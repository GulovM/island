//! Минимальный поставщик UI Automation для экранного диктора (Narrator): один виртуальный
//! элемент-«прокси» клавиатурного фокуса, не дерево фрагментов. Диктор при `Tab` озвучивает имя
//! и роль элемента, в который `Ui` перевела клавиатурный фокус (см. `ui::Ui::kbd_focus`), но не
//! может самостоятельно обойти остров в режиме сканирования — для этого нужна была бы полная
//! реализация `IRawElementProviderFragment`, которую сознательно не делаем: она требует отдавать
//! «пустой» указатель на соседний элемент (его в этой библиотеке безопасно не сконструировать),
//! вызывается из процесса диктора, и её нельзя проверить без живого запуска с Narrator.
//!
//! Имя/роль читаются из общего состояния под `Mutex`, а не из `thread_local`: UI Automation может
//! опрашивать поставщика с чужого потока (в том числе из процесса диктора через маршалинг COM).

use std::sync::Mutex;

use windows::core::{Error, IUnknown, Result};
use windows::Win32::Foundation::{E_NOTIMPL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    IRawElementProviderSimple, IRawElementProviderSimple_Impl, ProviderOptions, ProviderOptions_ServerSideProvider,
    UiaHostProviderFromHwnd, UiaRaiseAutomationEvent, UiaReturnRawElementProvider, UIA_AutomationFocusChangedEventId,
    UIA_ButtonControlTypeId, UIA_ControlTypePropertyId, UIA_HasKeyboardFocusPropertyId, UIA_IsContentElementPropertyId,
    UIA_IsControlElementPropertyId, UIA_IsKeyboardFocusablePropertyId, UIA_NamePropertyId, UIA_PATTERN_ID,
    UIA_PROPERTY_ID,
};
use windows::Win32::UI::WindowsAndMessaging::OBJID_CLIENT;

static FOCUS_NAME: Mutex<String> = Mutex::new(String::new());
/// Последний id, о котором уже сообщили диктору — чтобы не слать событие каждый кадр, а только
/// когда клавиатурный фокус действительно сменился.
static LAST_FOCUS_ID: Mutex<Option<u32>> = Mutex::new(None);

#[windows::core::implement(IRawElementProviderSimple)]
struct FocusProvider {
    hwnd: HWND,
}

impl IRawElementProviderSimple_Impl for FocusProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider)
    }

    /// Паттернов (ValuePattern и т. п.) не поддерживаем: у элемента-прокси нет своего состояния
    /// кроме имени и роли.
    fn GetPatternProvider(&self, _patternid: UIA_PATTERN_ID) -> Result<IUnknown> {
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn GetPropertyValue(&self, propertyid: UIA_PROPERTY_ID) -> Result<VARIANT> {
        if propertyid == UIA_NamePropertyId {
            let name = FOCUS_NAME.lock().map(|n| n.clone()).unwrap_or_default();
            return Ok(VARIANT::from(name.as_str()));
        }
        if propertyid == UIA_ControlTypePropertyId {
            return Ok(VARIANT::from(UIA_ButtonControlTypeId.0));
        }
        if propertyid == UIA_HasKeyboardFocusPropertyId
            || propertyid == UIA_IsKeyboardFocusablePropertyId
            || propertyid == UIA_IsControlElementPropertyId
            || propertyid == UIA_IsContentElementPropertyId
        {
            return Ok(VARIANT::from(true));
        }
        Ok(VARIANT::default())
    }

    /// Оконные свойства (в т. ч. ограничивающий прямоугольник) берём готовые у хоста — элемент сам
    /// не реализует `IRawElementProviderFragment`, своего прямоугольника не считает.
    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        unsafe { UiaHostProviderFromHwnd(self.hwnd) }
    }
}

/// `WM_GETOBJECT`: отдаёт диктору поставщика для клиентской области окна. `None` — сообщение не
/// по нашу часть (не `OBJID_CLIENT`), пусть идёт в `DefWindowProc` как обычно.
pub(crate) unsafe fn handle_get_object(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    if lparam.0 != OBJID_CLIENT.0 as isize {
        return None;
    }
    let provider: IRawElementProviderSimple = FocusProvider { hwnd }.into();
    Some(UiaReturnRawElementProvider(hwnd, wparam, lparam, &provider))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Сообщения не про клиентскую область (меню, заголовок и т. п.) не наш случай — идут в
    /// `DefWindowProc`, не создавая COM-объект поставщика.
    #[test]
    fn ignores_non_client_get_object() {
        unsafe {
            let not_client = LPARAM(OBJID_CLIENT.0 as isize - 1);
            assert!(handle_get_object(HWND::default(), WPARAM(0), not_client).is_none());
        }
    }

    /// Повторный отчёт о том же `id` не должен снова слать событие диктору (здесь — просто не
    /// падать и не менять сохранённое имя без смены фокуса).
    #[test]
    fn report_focus_is_idempotent_for_same_id() {
        unsafe {
            report_focus(HWND::default(), Some(42), "Кнопка");
            assert_eq!(*LAST_FOCUS_ID.lock().unwrap(), Some(42));
            report_focus(HWND::default(), Some(42), "Кнопка"); // не должно паниковать
            assert_eq!(*LAST_FOCUS_ID.lock().unwrap(), Some(42));
            report_focus(HWND::default(), None, "");
            assert_eq!(*LAST_FOCUS_ID.lock().unwrap(), None);
        }
    }
}

/// Сообщает дикторское имя клавиатурно сфокусированного элемента (`None` — фокуса сейчас нет).
/// Шлёт `UiaRaiseAutomationEvent` только при смене `id`, а не каждый кадр.
pub(crate) unsafe fn report_focus(hwnd: HWND, id: Option<u32>, name: &str) {
    if let Ok(mut n) = FOCUS_NAME.lock() {
        n.clear();
        n.push_str(name);
    }
    let Ok(mut last) = LAST_FOCUS_ID.lock() else { return };
    if *last == id {
        return;
    }
    *last = id;
    if id.is_some() {
        let provider: IRawElementProviderSimple = FocusProvider { hwnd }.into();
        let _ = UiaRaiseAutomationEvent(&provider, UIA_AutomationFocusChangedEventId);
    }
}

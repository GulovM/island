//! Перетаскивание на остров и с него: надпись «Быстрый доступ», итог броска, перетаскивание
//! записей наружу и горячая клавиша раздела. Системная часть (OLE) — в `dnd.rs`.
//!
//! Пока на остров наведено перетаскиваемое, свёрнутый остров (в том числе «полоска») становится
//! плашкой с надписью, раскрытый — закрывает содержимое такой же надписью. После броска надпись
//! на секунду показывает итог.

use super::*;
use crate::ui::{self, icon, Canvas, Rect};

/// Сколько итог броска остаётся на острове.
const RESULT_MS: u32 = 1300;

/// Что сказано на надписи перетаскивания.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DropMsg {
    /// Можно бросать.
    Ready,
    /// «Быстрый доступ» заполнен.
    Full,
    /// Перетаскиваемое нельзя добавить (не файл, не ссылка, не текст).
    Unsupported,
    /// Добавлено столько записей.
    Added(usize),
    /// Добавлено, но место кончилось.
    AddedFull(usize),
    /// Всё брошенное уже было в быстром доступе.
    Existing,
}

impl DropMsg {
    /// (заголовок, пояснение) на языке интерфейса.
    pub(crate) fn text(self) -> (String, String) {
        use crate::i18n::pick;
        let max = tabs::clipboard::QUICK_MAX;
        match self {
            DropMsg::Ready => (pick("Быстрый доступ", "Quick Access").into(), pick("Отпустите, чтобы добавить", "Release to add").into()),
            DropMsg::Full => (
                pick("Быстрый доступ заполнен", "Quick Access is full").into(),
                if crate::i18n::en() { format!("Limit is {max} items") } else { format!("Лимит — {max} записей") },
            ),
            DropMsg::Unsupported => (
                pick("Это нельзя добавить", "Can't add this").into(),
                pick("Подходят файлы, ссылки и текст", "Files, links and text only").into(),
            ),
            DropMsg::Added(n) => (
                if crate::i18n::en() { format!("Added: {n}") } else { format!("Добавлено: {n}") },
                pick("Быстрый доступ", "Quick Access").into(),
            ),
            DropMsg::AddedFull(n) => (
                if crate::i18n::en() { format!("Added: {n}") } else { format!("Добавлено: {n}") },
                if crate::i18n::en() { format!("Limit of {max} reached") } else { format!("Достигнут лимит {max}") },
            ),
            DropMsg::Existing => (pick("Уже в быстром доступе", "Already in Quick Access").into(), String::new()),
        }
    }

    /// Значок и его цвет.
    fn glyph(self, accent: D2D1_COLOR_F) -> (&'static str, D2D1_COLOR_F) {
        match self {
            DropMsg::Ready => (icon::ADD, accent),
            DropMsg::Added(_) | DropMsg::Existing => (icon::CHECK, ui::status_green()),
            DropMsg::Full | DropMsg::AddedFull(_) | DropMsg::Unsupported => (icon::CLOSE, ui::status_orange()),
        }
    }
}

/// Итог броска по тому, что получилось добавить.
fn result_msg(added: &tabs::clipboard::Added) -> DropMsg {
    match (added.new, added.existing, added.full) {
        (0, 0, true) => DropMsg::Full,
        (0, 0, false) => DropMsg::Unsupported,
        (0, _, _) => DropMsg::Existing,
        (n, _, true) => DropMsg::AddedFull(n),
        (n, _, false) => DropMsg::Added(n),
    }
}

impl App {
    /// Показывает надпись (или убирает её при `None`). Размеры свёрнутого острова пересчитываются.
    fn set_drop_msg(&mut self, msg: Option<DropMsg>) {
        // SAFETY: таймер этого окна.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), DROP_TIMER);
        }
        self.drop_msg = msg;
        self.drag_a.target = if msg.is_some() { 1.0 } else { 0.0 };
        self.retarget();
    }

    /// Итог броска остаётся на острове на секунду.
    fn show_drop_result(&mut self, msg: DropMsg) {
        self.set_drop_msg(Some(msg));
        // SAFETY: таймер этого окна.
        unsafe {
            SetTimer(Some(self.hwnd), DROP_TIMER, RESULT_MS, None);
        }
    }

    /// На остров навели перетаскиваемое. `ok` — это можно добавить.
    pub(crate) unsafe fn on_drag_enter(&mut self, ok: bool) -> bool {
        if self.hidden {
            return false;
        }
        let full = self.tabs.quick_full();
        dnd::set_full(full);
        self.set_drop_msg(Some(match (ok, full) {
            (false, _) => DropMsg::Unsupported,
            (true, true) => DropMsg::Full,
            (true, false) => DropMsg::Ready,
        }));
        true
    }

    /// Перетаскиваемое ушло с острова, не будучи брошенным.
    pub(crate) unsafe fn on_drag_leave(&mut self) -> bool {
        let shown = self.drop_msg.is_some();
        self.set_drop_msg(None);
        self.check_hover();
        shown
    }

    /// Брошено на остров: записи идут в «Быстрый доступ».
    pub(crate) unsafe fn on_dropped(&mut self, items: Vec<String>) -> bool {
        let msg = if items.is_empty() {
            DropMsg::Unsupported
        } else {
            let added = self.tabs.add_quick(&items);
            dnd::set_full(self.tabs.quick_full());
            result_msg(&added)
        };
        self.show_drop_result(msg);
        true
    }

    /// Итог броска показан достаточно.
    pub(crate) unsafe fn on_drop_timer(&mut self) -> bool {
        let shown = self.drop_msg.is_some();
        self.set_drop_msg(None);
        self.check_hover();
        shown
    }

    /// Запись «Быстрого доступа» тащат наружу. Блокирует до броска или отмены.
    pub(crate) unsafe fn run_drag_out(&mut self, out: dnd::Out) {
        // Кнопка уходит к OLE: ни долгое нажатие, ни перетаскивание внутри острова не должны продолжиться.
        let _ = KillTimer(Some(self.hwnd), LONG_TIMER);
        self.press_pos = None;
        self.ui.down = false;
        self.ui.pressed = false;
        self.ui.released = false;
        let taken = dnd::drag_out(out);
        log::info(if taken { "перетаскивание записи наружу: получатель принял" } else { "перетаскивание записи наружу: отменено или получатель не принял" });
        // Отпускание кнопки досталось не нам.
        self.ui.down = false;
        self.check_hover();
    }

    // ---------- Горячая клавиша раздела ----------

    /// Горячая клавиша раздела: остров раскрывается на «Быстром доступе» с фокусом в поиске.
    pub(crate) unsafe fn on_quick_hotkey(&mut self) -> bool {
        if self.hidden {
            return false;
        }
        // Вкладка «Буфер» выключена в настройках — открывать нечего.
        let Some(i) = self.order.iter().position(|&t| t == tabs::CLIPBOARD) else { return false };
        self.tabs.open_quick();
        self.open_from_hotkey(i, tabs::CLIPBOARD);
        true
    }

    /// Новое сочетание для раздела (пустое — снять). Занятое системой или Apps не принимается.
    pub(crate) unsafe fn set_quick_hotkey(&mut self, combo: String) {
        let old = std::mem::replace(&mut self.cfg.quick_hotkey, combo.clone());
        let _ = UnregisterHotKey(Some(self.hwnd), QUICK_HOTKEY_ID);
        if combo.is_empty() {
            config::save_str("quick_hotkey", "");
            self.tabs.set_quick_hotkey("", None);
            return;
        }
        let same_as_apps = combo.eq_ignore_ascii_case(&self.cfg.hotkey);
        if !same_as_apps && self.register_hotkey_id(QUICK_HOTKEY_ID, &combo) {
            config::save_str("quick_hotkey", &combo);
            self.tabs.set_quick_hotkey(&combo, None);
            return;
        }
        // Не вышло: остаётся прежнее сочетание.
        self.cfg.quick_hotkey = old.clone();
        if !old.is_empty() {
            self.register_hotkey_id(QUICK_HOTKEY_ID, &old);
        }
        let note = if same_as_apps {
            "Это сочетание уже открывает вкладку Apps"
        } else {
            "Сочетание занято другой программой"
        };
        self.tabs.set_quick_hotkey(&old, Some(note));
    }

    /// Клавиша, пока вкладка ждёт сочетание для горячей клавиши раздела.
    pub(crate) unsafe fn record_key(&mut self, vk: u32) -> bool {
        let search = tabs::id(tabs::CLIPBOARD, 0);
        if vk == VK_ESCAPE.0 as u32 {
            self.tabs.cancel_hotkey_recording();
            self.ui.focus = Some(search);
            return true;
        }
        let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
        if let Some(combo) = tabs::clipboard::hotkey_combo(down(VK_CONTROL), down(VK_MENU), down(VK_SHIFT), vk) {
            self.tabs.hotkey_captured(combo);
            self.ui.focus = Some(search);
        }
        // Одни модификаторы или клавиша без них — ждём дальше.
        true
    }

    // ---------- Рисование ----------

    /// Плашка на свёрнутом острове (`r` — её прямоугольник в логических пикселях).
    pub(crate) unsafe fn draw_drop_pill(&self, cv: &Canvas, r: Rect) {
        let Some(msg) = self.drop_msg else { return };
        let f = &cv.g.fonts;
        let (title, sub) = msg.text();
        let (g, col) = msg.glyph(cv.accent);
        let ic = Rect::new(r.x + 16.0, r.cy() - 9.0, 18.0, 18.0);
        cv.glyph(g, ic, false, col);
        let tx = ic.right() + 10.0;
        let tw = (r.right() - tx - 14.0).max(10.0);
        if sub.is_empty() {
            cv.text(&title, Rect::new(tx, r.cy() - 8.0, tw, 16.0), &f.body, ui::collapsed_txt());
        } else {
            cv.text(&title, Rect::new(tx, r.y + 6.0, tw, 16.0), &f.body, ui::collapsed_txt());
            cv.text(&sub, Rect::new(tx, r.y + 22.0, tw, 12.0), &f.small, ui::collapsed_dim());
        }
    }

    /// Надпись поверх содержимого раскрытого острова (`r` — область содержимого).
    pub(crate) unsafe fn draw_drop_overlay(&self, cv: &Canvas, r: Rect) {
        let Some(msg) = self.drop_msg else { return };
        let f = &cv.g.fonts;
        let (title, sub) = msg.text();
        let (g, col) = msg.glyph(cv.accent);
        let pal = theme::get();
        // Содержимое под надписью не должно читаться: почти сплошная заливка цветом тела.
        cv.rr(r, 14.0, D2D1_COLOR_F { a: 0.985, ..pal.bg });
        cv.rr_line(r.inset(1.0), 13.0, D2D1_COLOR_F { a: 0.8, ..col }, 1.5);
        let cy = r.cy();
        cv.glyph(g, Rect::new(r.cx() - 16.0, cy - 40.0, 32.0, 32.0), true, col);
        cv.text(&title, Rect::new(r.x, cy - 4.0, r.w, 18.0), &f.body_c, ui::txt());
        if !sub.is_empty() {
            cv.text(&sub, Rect::new(r.x, cy + 15.0, r.w, 14.0), &f.small_c, ui::dim());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tabs::clipboard::Added;

    #[test]
    fn result_reflects_what_was_added() {
        assert_eq!(result_msg(&Added { new: 3, existing: 0, full: false }), DropMsg::Added(3));
        assert_eq!(result_msg(&Added { new: 2, existing: 1, full: true }), DropMsg::AddedFull(2));
        assert_eq!(result_msg(&Added { new: 0, existing: 2, full: false }), DropMsg::Existing);
        assert_eq!(result_msg(&Added { new: 0, existing: 0, full: true }), DropMsg::Full);
        assert_eq!(result_msg(&Added { new: 0, existing: 0, full: false }), DropMsg::Unsupported);
    }

    #[test]
    fn messages_have_a_title() {
        for m in [DropMsg::Ready, DropMsg::Full, DropMsg::Unsupported, DropMsg::Added(2), DropMsg::AddedFull(1), DropMsg::Existing] {
            assert!(!m.text().0.is_empty(), "{m:?}");
        }
        assert!(DropMsg::Full.text().1.contains("30"), "лимит назван числом");
    }
}

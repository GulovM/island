//! Перетаскивание через OLE: остров принимает файлы, ссылки и текст, брошенные на него
//! (`IDropTarget`), и сам отдаёт записи «Быстрого доступа» другим окнам (`DoDragDrop`).
//! Весь `unsafe` OLE сосредоточен здесь. OLE работает только в STA-потоке, поэтому главный
//! поток инициализируется через [`init`] до создания окна.
//!
//! Колбэки приёма вызываются системой из цикла сообщений главного потока и ничего не меняют
//! сами: кладут событие в очередь (`wnd::push`), остальное делает `App`.

use std::cell::Cell;
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::{implement, w, Ref, Result, BOOL, HRESULT};
use windows::Win32::Foundation::{
    DATA_S_SAMEFORMATETC, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, DV_E_FORMATETC,
    E_NOTIMPL, GlobalFree, HGLOBAL, HWND, OLE_E_ADVISENOTSUPPORTED, POINTL, S_OK,
};
use windows::Win32::System::Com::{
    IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, DVASPECT_CONTENT, FORMATETC, STGMEDIUM,
    STGMEDIUM_0, TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, IDropTarget, IDropTarget_Impl, OleInitialize, RegisterDragDrop,
    ReleaseStgMedium, RevokeDragDrop, CF_DIB, CF_HDROP, CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY,
    DROPEFFECT_LINK, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::Shell::{DragQueryFileW, SHCreateStdEnumFmtEtc, HDROP};

use crate::wnd::{push, Ev};
use crate::{img, log, sys};

/// Быстрый доступ заполнен: приём отклоняется, система показывает «нельзя».
static FULL: AtomicBool = AtomicBool::new(false);
/// Остров сам тащит запись наружу: бросок обратно на него (даже в пути) ничего не добавляет.
static SELF_DRAG: AtomicBool = AtomicBool::new(false);

pub fn set_full(full: bool) {
    FULL.store(full, Ordering::Relaxed);
}

/// Включает OLE в главном потоке (STA). Нужно до `register`/`drag_out`.
pub fn init() -> bool {
    // SAFETY: вызывается один раз из главного потока до создания окон.
    unsafe { OleInitialize(None) }.is_ok()
}

// ---------- Приём ----------

/// Какой эффект выбрать из разрешённых источником: копирование, иначе ссылка, иначе отказ.
fn choose_effect(allowed: u32, ok: bool) -> u32 {
    if !ok {
        DROPEFFECT_NONE.0
    } else if allowed & DROPEFFECT_COPY.0 != 0 {
        DROPEFFECT_COPY.0
    } else if allowed & DROPEFFECT_LINK.0 != 0 {
        DROPEFFECT_LINK.0
    } else {
        DROPEFFECT_NONE.0
    }
}

unsafe fn set_effect(effect: *mut DROPEFFECT, ok: bool) {
    if let Some(e) = effect.as_mut() {
        *e = DROPEFFECT(choose_effect(e.0, ok));
    }
}

#[implement(IDropTarget)]
struct Target {
    /// Данные текущего перетаскивания можно добавить.
    accept: Cell<bool>,
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        // SAFETY: `data` и `effect` действительны на время вызова.
        unsafe {
            if SELF_DRAG.load(Ordering::Relaxed) {
                self.accept.set(false);
                set_effect(effect, false);
                return Ok(());
            }
            let ok = data.as_ref().is_some_and(|d| acceptable(d));
            self.accept.set(ok);
            push(Ev::DragEnter(ok));
            set_effect(effect, ok && !FULL.load(Ordering::Relaxed));
        }
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        // SAFETY: `effect` действителен на время вызова.
        unsafe { set_effect(effect, self.accept.get() && !FULL.load(Ordering::Relaxed)) };
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        if !SELF_DRAG.load(Ordering::Relaxed) {
            push(Ev::DragLeave);
        }
        Ok(())
    }

    fn Drop(&self, data: Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        // SAFETY: данные читаются здесь же, пока объект источника жив.
        unsafe {
            if SELF_DRAG.load(Ordering::Relaxed) {
                set_effect(effect, false);
                return Ok(());
            }
            let ok = self.accept.get() && !FULL.load(Ordering::Relaxed);
            let items = if ok { data.as_ref().map(|d| extract(d)).unwrap_or_default() } else { Vec::new() };
            set_effect(effect, !items.is_empty());
            push(Ev::Dropped(items));
        }
        Ok(())
    }
}

/// Делает окно получателем перетаскивания. `false` — OLE недоступен (приём отключён).
pub unsafe fn register(hwnd: HWND) -> bool {
    let target: IDropTarget = Target { accept: Cell::new(false) }.into();
    match RegisterDragDrop(hwnd, &target) {
        Ok(()) => true,
        Err(e) => {
            log::error(&format!("не удалось включить приём перетаскивания: {e}"));
            false
        }
    }
}

pub unsafe fn revoke(hwnd: HWND) {
    let _ = RevokeDragDrop(hwnd);
}

fn url_format() -> u16 {
    // SAFETY: простой вызов Win32 со статической строкой.
    unsafe { RegisterClipboardFormatW(w!("UniformResourceLocatorW")) as u16 }
}

fn format_of(cf: u16) -> FORMATETC {
    FORMATETC { cfFormat: cf, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
}

/// В перетаскиваемом есть то, что можно добавить: файлы, адрес или текст.
unsafe fn acceptable(d: &IDataObject) -> bool {
    [CF_HDROP.0, url_format(), CF_UNICODETEXT.0].iter().any(|&cf| d.QueryGetData(&format_of(cf)) == S_OK)
}

/// Записи для добавления: пути файлов (по одной на файл), иначе адрес, иначе текст.
unsafe fn extract(d: &IDataObject) -> Vec<String> {
    if let Some(files) = hdrop_paths(d).filter(|f| !f.is_empty()) {
        return files;
    }
    for cf in [url_format(), CF_UNICODETEXT.0] {
        if let Some(t) = hglobal_text(d, cf).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
            return vec![t];
        }
    }
    Vec::new()
}

unsafe fn hdrop_paths(d: &IDataObject) -> Option<Vec<String>> {
    let mut med = d.GetData(&format_of(CF_HDROP.0)).ok()?;
    let mut out = Vec::new();
    if med.tymed == TYMED_HGLOBAL.0 as u32 {
        let hdrop = HDROP(med.u.hGlobal.0);
        let count = DragQueryFileW(hdrop, u32::MAX, None);
        for i in 0..count {
            let len = DragQueryFileW(hdrop, i, None) as usize;
            let mut buf = vec![0u16; len + 1];
            let n = DragQueryFileW(hdrop, i, Some(&mut buf)) as usize;
            out.push(String::from_utf16_lossy(&buf[..n.min(len)]));
        }
    }
    ReleaseStgMedium(&mut med);
    Some(out)
}

unsafe fn hglobal_text(d: &IDataObject, cf: u16) -> Option<String> {
    let mut med = d.GetData(&format_of(cf)).ok()?;
    let mut out = None;
    if med.tymed == TYMED_HGLOBAL.0 as u32 {
        let hg = med.u.hGlobal;
        let p = GlobalLock(hg) as *const u16;
        if !p.is_null() {
            // Не дальше размера блока: строка без нуля не должна выводить за его границу.
            let max = (GlobalSize(hg) / 2).min(1 << 20);
            let mut n = 0;
            while n < max && *p.add(n) != 0 {
                n += 1;
            }
            out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
            let _ = GlobalUnlock(hg);
        }
    }
    ReleaseStgMedium(&mut med);
    out
}

// ---------- Отдача ----------

/// Что остров отдаёт при перетаскивании записи наружу.
pub enum Out {
    /// Файлы или папки по путям.
    Files(Vec<String>),
    Text(String),
    /// Адрес: отдаётся и как текст, и как интернет-ссылка.
    Url(String),
    /// Зашифрованный PNG из хранилища: отдаётся картинкой и временным файлом.
    Image(PathBuf),
}

/// Содержимое HGLOBAL-формата `DROPFILES`: заголовок 20 байт, пути UTF-16 через нуль, двойной нуль.
fn build_hdrop(paths: &[String]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&20u32.to_le_bytes()); // pFiles
    v.extend_from_slice(&[0u8; 8]); // pt
    v.extend_from_slice(&0u32.to_le_bytes()); // fNC
    v.extend_from_slice(&1u32.to_le_bytes()); // fWide
    for p in paths {
        for u in p.encode_utf16() {
            v.extend_from_slice(&u.to_le_bytes());
        }
        v.extend_from_slice(&0u16.to_le_bytes());
    }
    v.extend_from_slice(&0u16.to_le_bytes());
    v
}

fn wide_bytes(s: &str) -> Vec<u8> {
    s.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect()
}

unsafe fn global_from(bytes: &[u8]) -> Result<HGLOBAL> {
    let hg = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
    let p = GlobalLock(hg) as *mut u8;
    if p.is_null() {
        let _ = GlobalFree(Some(hg));
        return Err(E_NOTIMPL.into());
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    let _ = GlobalUnlock(hg);
    Ok(hg)
}

#[implement(IDataObject)]
struct Data {
    files: Vec<String>,
    text: Option<String>,
    url: Option<String>,
    /// Зашифрованный PNG для формата «изображение».
    image: Option<PathBuf>,
    url_cf: u16,
}

impl Data {
    fn formats(&self) -> Vec<FORMATETC> {
        let mut f = Vec::new();
        if !self.files.is_empty() {
            f.push(format_of(CF_HDROP.0));
        }
        if self.url.is_some() {
            f.push(format_of(self.url_cf));
        }
        if self.text.is_some() {
            f.push(format_of(CF_UNICODETEXT.0));
        }
        if self.image.is_some() {
            f.push(format_of(CF_DIB.0));
        }
        f
    }

    fn bytes_for(&self, cf: u16) -> Option<Vec<u8>> {
        if cf == CF_HDROP.0 && !self.files.is_empty() {
            Some(build_hdrop(&self.files))
        } else if cf == CF_UNICODETEXT.0 {
            self.text.as_deref().map(wide_bytes)
        } else if cf == self.url_cf {
            self.url.as_deref().map(wide_bytes)
        } else if cf == CF_DIB.0 {
            // SAFETY: WIC-вызовы блокирующие, но здесь это одна небольшая картинка.
            self.image.as_deref().and_then(|p| unsafe { img::load_dib(p) })
        } else {
            None
        }
    }
}

impl IDataObject_Impl for Data_Impl {
    fn GetData(&self, pformatetcin: *const FORMATETC) -> Result<STGMEDIUM> {
        // SAFETY: система передаёт действительный указатель на время вызова.
        let f = unsafe { pformatetcin.as_ref() }.ok_or(DV_E_FORMATETC)?;
        if f.tymed & TYMED_HGLOBAL.0 as u32 == 0 {
            return Err(DV_E_FORMATETC.into());
        }
        let bytes = self.bytes_for(f.cfFormat).ok_or(DV_E_FORMATETC)?;
        // SAFETY: память отдаётся получателю, он освобождает её через `ReleaseStgMedium`.
        let hg = unsafe { global_from(&bytes) }?;
        Ok(STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: hg }, pUnkForRelease: ManuallyDrop::new(None) })
    }

    fn GetDataHere(&self, _f: *const FORMATETC, _m: *mut STGMEDIUM) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn QueryGetData(&self, pformatetc: *const FORMATETC) -> HRESULT {
        // SAFETY: система передаёт действительный указатель на время вызова.
        let Some(f) = (unsafe { pformatetc.as_ref() }) else { return DV_E_FORMATETC };
        if f.tymed & TYMED_HGLOBAL.0 as u32 != 0 && self.formats().iter().any(|x| x.cfFormat == f.cfFormat) {
            S_OK
        } else {
            DV_E_FORMATETC
        }
    }

    fn GetCanonicalFormatEtc(&self, _fin: *const FORMATETC, fout: *mut FORMATETC) -> HRESULT {
        // SAFETY: `fout` действителен на время вызова.
        if let Some(o) = unsafe { fout.as_mut() } {
            o.ptd = std::ptr::null_mut();
        }
        DATA_S_SAMEFORMATETC
    }

    fn SetData(&self, _f: *const FORMATETC, _m: *const STGMEDIUM, _release: BOOL) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn EnumFormatEtc(&self, dwdirection: u32) -> Result<IEnumFORMATETC> {
        // DATADIR_GET = 1; записывать в объект нельзя.
        if dwdirection != 1 {
            return Err(E_NOTIMPL.into());
        }
        // SAFETY: список копируется внутрь перечислителя.
        unsafe { SHCreateStdEnumFmtEtc(&self.formats()) }
    }

    fn DAdvise(&self, _f: *const FORMATETC, _advf: u32, _sink: Ref<'_, windows::Win32::System::Com::IAdviseSink>) -> Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn DUnadvise(&self, _connection: u32) -> Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumDAdvise(&self) -> Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

#[implement(IDropSource)]
struct Source;

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, fescapepressed: BOOL, grfkeystate: MODIFIERKEYS_FLAGS) -> HRESULT {
        if fescapepressed.as_bool() {
            DRAGDROP_S_CANCEL
        } else if grfkeystate.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Папка с расшифрованными копиями картинок, отданных перетаскиванием.
fn drag_tmp_dir() -> PathBuf {
    std::env::temp_dir().join("island-drag")
}

/// Расшифрованная копия PNG: получатель может скопировать файл уже после броска, поэтому
/// удаляется копия не сразу, а при следующем перетаскивании (старше часа).
fn plain_png(sealed: &Path) -> Option<String> {
    let dir = drag_tmp_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_secs() > 3600);
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let plain = sys::unseal(&std::fs::read(sealed).ok()?)?;
    let path = dir.join(format!("island-{}.png", sys::now_secs()));
    std::fs::write(&path, plain).ok()?;
    path.to_str().map(str::to_string)
}

/// Тащит запись наружу. Блокирует до броска или отмены (внутри свой цикл сообщений).
/// `true` — получатель принял.
pub unsafe fn drag_out(out: Out) -> bool {
    let url_cf = url_format();
    let data = match out {
        Out::Files(files) => Data { files, text: None, url: None, image: None, url_cf },
        Out::Text(t) => Data { files: Vec::new(), text: Some(t), url: None, image: None, url_cf },
        Out::Url(u) => Data { files: Vec::new(), text: Some(u.clone()), url: Some(u), image: None, url_cf },
        Out::Image(p) => {
            let files = plain_png(&p).into_iter().collect();
            Data { files, text: None, url: None, image: Some(p), url_cf }
        }
    };
    let data: IDataObject = data.into();
    let source: IDropSource = Source.into();
    // Мышь была захвачена окном при нажатии; дальше её ведёт OLE.
    let _ = ReleaseCapture();
    // Только копирование и ссылка: перенос забрал бы файл из исходной папки.
    let mut effect = DROPEFFECT_NONE;
    SELF_DRAG.store(true, Ordering::Relaxed);
    let hr = DoDragDrop(&data, &source, DROPEFFECT_COPY | DROPEFFECT_LINK, &mut effect);
    SELF_DRAG.store(false, Ordering::Relaxed);
    hr == DRAGDROP_S_DROP && effect != DROPEFFECT_NONE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdrop_layout_matches_dropfiles() {
        let b = build_hdrop(&["C:\\a.txt".to_string(), "D:\\б".to_string()]);
        assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), 20, "смещение списка");
        assert_eq!(u32::from_le_bytes(b[16..20].try_into().unwrap()), 1, "fWide");
        let list: Vec<u16> = b[20..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(list.last(), Some(&0));
        assert_eq!(list[list.len() - 2], 0, "список заканчивается двойным нулём");
        let s = String::from_utf16_lossy(&list);
        assert_eq!(s.split('\0').filter(|x| !x.is_empty()).collect::<Vec<_>>(), ["C:\\a.txt", "D:\\б"]);
    }

    #[test]
    fn effect_prefers_copy_and_refuses_without_support() {
        let all = DROPEFFECT_COPY.0 | DROPEFFECT_LINK.0 | 2;
        assert_eq!(choose_effect(all, true), DROPEFFECT_COPY.0);
        assert_eq!(choose_effect(DROPEFFECT_LINK.0, true), DROPEFFECT_LINK.0);
        assert_eq!(choose_effect(2, true), DROPEFFECT_NONE.0, "только перенос не подходит");
        assert_eq!(choose_effect(all, false), DROPEFFECT_NONE.0);
    }

    #[test]
    fn wide_text_is_nul_terminated_utf16() {
        assert_eq!(wide_bytes("a"), [b'a', 0, 0, 0]);
        assert_eq!(wide_bytes("").len(), 2);
    }
}

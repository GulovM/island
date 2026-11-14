//! Текстовое поле ввода: курсор, выделение, навигация, IME.

use windows::core::BOOL;
use windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED;
use windows::Win32::Graphics::DirectWrite::{DWRITE_HIT_TEST_METRICS, DWRITE_TEXT_METRICS};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_SHIFT};

use super::{card, dim, fg, mix, txt, Canvas, Rect, Ui};

#[derive(Default)]
pub(super) struct Field {
    chars: Vec<char>,
    caret: usize,
    anchor: usize,
    scroll: f32,
    submit: Submit,
    /// Поле пароля: символы показываются точками, копировать нельзя.
    secret: bool,
    /// Только цифры (не больше 5 знаков): поля времени.
    digits: bool,
    /// Подсказка, не влезшая в поле, один раз прокручивается по кругу (узкие поля с длинной подсказкой).
    hint_marquee: bool,
}

/// Что делает Enter в текстовом поле.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Submit {
    /// Снимает фокус (поиск).
    #[default]
    None,
    /// Сообщает текст, очищает поле, фокус остаётся (добавление задачи).
    Clear,
    /// Сообщает текст, поле и фокус остаются (поиск с действием по Enter).
    Keep,
}

/// Навигация по списку с клавиатуры, пока в поле поиска есть фокус.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Nav {
    Up,
    Down,
    Pin,
    /// Ctrl+C без выделенного текста в поле: копировать выбранную запись списка.
    Copy,
    /// Delete при пустом поле: удалить выбранную запись.
    Delete,
    /// Tab: следующий фильтр.
    Tab,
}

impl Field {
    fn range(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }
    fn has_sel(&self) -> bool {
        self.caret != self.anchor
    }
    fn delete_sel(&mut self) -> bool {
        if !self.has_sel() {
            return false;
        }
        let (lo, hi) = self.range();
        self.chars.drain(lo..hi);
        self.caret = lo;
        self.anchor = lo;
        true
    }
    fn selected(&self) -> String {
        let (lo, hi) = self.range();
        self.chars[lo..hi].iter().collect()
    }
    fn insert(&mut self, s: &str) {
        self.delete_sel();
        let digits = self.digits;
        for c in s.chars().filter(|c| !c.is_control() && (!digits || c.is_ascii_digit())) {
            if self.chars.len() >= if digits { 5 } else { FIELD_MAX } {
                break;
            }
            self.chars.insert(self.caret, c);
            self.caret += 1;
        }
        self.anchor = self.caret;
    }
}

const FIELD_MAX: usize = 256;

/// Клавиатурное событие для сфокусированного текстового поля.
pub enum KeyEv {
    Char(char),
    Left(bool),
    Right(bool),
    Home(bool),
    End(bool),
    Backspace,
    BackspaceWord,
    Delete,
    DeleteWord,
    DeleteLine,
    SelectAll,
    Copy,
    Cut,
    Paste(String),
    Enter,
    Escape,
    Up,
    Down,
    Pin,
    Tab,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Позиция начала слова слева от `caret` для Ctrl+Backspace.
fn prev_word_start(chars: &[char], caret: usize) -> usize {
    let mut i = caret.min(chars.len());
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    if i == 0 {
        return 0;
    }
    if is_word_char(chars[i - 1]) {
        while i > 0 && is_word_char(chars[i - 1]) {
            i -= 1;
        }
    } else {
        while i > 0 && !chars[i - 1].is_whitespace() && !is_word_char(chars[i - 1]) {
            i -= 1;
        }
    }
    i
}

/// Позиция конца слова справа от `caret` для Ctrl+Delete.
fn next_word_end(chars: &[char], caret: usize) -> usize {
    let n = chars.len();
    let mut i = caret.min(n);
    while i < n && chars[i].is_whitespace() {
        i += 1;
    }
    if i == n {
        return n;
    }
    if is_word_char(chars[i]) {
        while i < n && is_word_char(chars[i]) {
            i += 1;
        }
    } else {
        while i < n && !chars[i].is_whitespace() && !is_word_char(chars[i]) {
            i += 1;
        }
    }
    i
}

/// Границы `[lo, hi)` строки, в которой находится `caret` (включая перевод строки).
fn current_line_bounds(chars: &[char], caret: usize) -> (usize, usize) {
    let n = chars.len();
    let pos = caret.min(n);
    let mut lo = pos;
    while lo > 0 && chars[lo - 1] != '\n' && chars[lo - 1] != '\r' {
        lo -= 1;
    }
    let mut hi = pos;
    while hi < n && chars[hi] != '\n' && chars[hi] != '\r' {
        hi += 1;
    }
    if hi < n {
        if chars[hi] == '\r' && hi + 1 < n && chars[hi + 1] == '\n' {
            hi += 2;
        } else {
            hi += 1;
        }
    } else if lo > 0 {
        if chars[lo - 1] == '\n' && lo >= 2 && chars[lo - 2] == '\r' {
            lo -= 2;
        } else {
            lo -= 1;
        }
    }
    (lo, hi)
}

fn utf16_len(chars: &[char]) -> u32 {
    chars.iter().map(|c| c.len_utf16() as u32).sum()
}

fn char_at_utf16(chars: &[char], pos16: u32) -> usize {
    let mut acc = 0;
    for (i, c) in chars.iter().enumerate() {
        if acc >= pos16 {
            return i;
        }
        acc += c.len_utf16() as u32;
    }
    chars.len()
}

fn shift_down() -> bool {
    unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 }
}

impl Ui {
    /// Забирает текст поля (как по Enter) — для кнопки «добавить» рядом с полем.
    pub fn submit_field(&mut self, id: u32) -> Option<String> {
        let f = self.fields.get_mut(&id)?;
        let text: String = f.chars.iter().collect();
        f.chars.clear();
        f.caret = 0;
        f.anchor = 0;
        f.scroll = 0.0;
        Some(text)
    }

    /// Задаёт текст поля программно (поле не должно быть в фокусе, иначе оно перебьёт ввод).
    pub fn set_field_text(&mut self, id: u32, s: &str) {
        let f = self.fields.entry(id).or_default();
        f.chars = s.chars().take(FIELD_MAX).collect();
        f.caret = f.chars.len();
        f.anchor = f.caret;
        f.scroll = 0.0;
    }

    /// Поле принимает только цифры.
    pub fn set_digits(&mut self, id: u32, on: bool) {
        self.fields.entry(id).or_default().digits = on;
    }

    /// Делает поле полем пароля (точки вместо символов, без Ctrl+C/X).
    pub fn set_secret(&mut self, id: u32, on: bool) {
        self.fields.entry(id).or_default().secret = on;
    }

    /// Подсказка поля, если не влезает, один раз медленно прокручивается по кругу и встаёт на место.
    pub fn set_hint_marquee(&mut self, id: u32, on: bool) {
        self.fields.entry(id).or_default().hint_marquee = on;
    }

    /// Выделяет весь текст поля (ввод заменит его).
    pub fn select_all(&mut self, id: u32) {
        let f = self.fields.entry(id).or_default();
        f.anchor = 0;
        f.caret = f.chars.len();
    }

    pub fn has_focus(&self, id: u32) -> bool {
        self.focus == Some(id)
    }

    // --- текстовое поле ---

    /// Текст, отправленный клавишей Enter из поля `id` (однократно).
    pub fn take_submitted(&mut self, id: u32) -> Option<String> {
        match &self.submitted {
            Some((i, _)) if *i == id => self.submitted.take().map(|(_, t)| t),
            _ => None,
        }
    }

    /// Забирает клавиши навигации; не у интерактивного слоя (уходящая вкладка) их нет.
    pub fn take_nav(&mut self) -> Vec<Nav> {
        if self.interactive {
            std::mem::take(&mut self.nav)
        } else {
            Vec::new()
        }
    }

    pub fn field_text(&self, id: u32) -> String {
        self.fields.get(&id).map(|f| f.chars.iter().collect()).unwrap_or_default()
    }

    /// Однострочное поле ввода: курсор, выделение мышью и клавишами, прокрутка по caret.
    /// Поле ввода с иконкой и режимом Enter.
    pub unsafe fn text_field_ex(&mut self, cv: &Canvas, id: u32, r: Rect, placeholder: &str, glyph: &str, submit: Submit) {
        self.text_field_pad(cv, id, r, placeholder, glyph, submit, 10.0);
    }

    /// То же, но с заданным отступом справа: текст не заезжает под кнопки поверх поля.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn text_field_pad(&mut self, cv: &Canvas, id: u32, r: Rect, placeholder: &str, glyph: &str, submit: Submit, right: f32) {
        self.text_field_at(cv, id, r, placeholder, glyph, submit, 30.0, right);
    }

    /// Поле с заданными отступами слева (под значок) и справа (под кнопки или единицу измерения).
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn text_field_at(&mut self, cv: &Canvas, id: u32, r: Rect, placeholder: &str, glyph: &str, submit: Submit, left: f32, right: f32) {
        self.fields.entry(id).or_default().submit = submit;
        // В полях поиска справа появляется «×»: очищает введённый текст одним нажатием.
        let clearable = glyph == super::icon::SEARCH;
        let right = if clearable { right.max(26.0) } else { right };
        let mut clear_hot = false;
        if clearable && self.fields.get(&id).is_some_and(|f| !f.chars.is_empty()) {
            let cr = Rect::new(r.right() - 24.0, r.y, 22.0, r.h);
            let (hot, clicked) = self.interact(id ^ 0x4000_0000, cr);
            clear_hot = hot;
            if clicked {
                if let Some(f) = self.fields.get_mut(&id) {
                    f.chars.clear();
                    f.caret = 0;
                    f.anchor = 0;
                    f.scroll = 0.0;
                }
                self.focus = Some(id);
                self.focus_claimed = true;
                self.caret_on = true;
            }
        }
        let (hover, _) = self.interact(id, r);
        let inner = Rect::new(r.x + left, r.y, (r.w - left - right).max(1.0), r.h);
        let w = self.widget(id);
        let (hv, ov) = (w.hover.value, w.on.value);
        let focused_now = self.focus == Some(id);
        self.widget(id).on.target = if focused_now { 1.0 } else { 0.0 };

        let mine = self.drag == Some(id);
        let (pressed, down, mouse_x, origin_x) = (self.pressed, self.down, self.mouse.0, self.origin_x);
        if pressed && hover {
            self.focus = Some(id);
            self.focus_claimed = true;
            self.caret_on = true;
        }
        let focused = self.focus == Some(id);
        let caret_on = self.caret_on;

        // Бегущая подсказка: сдвиг считаем до заимствования поля (таймер прокрутки живёт в `self`).
        const HINT_GAP: f32 = 24.0;
        let hint = self.fields.get(&id).filter(|f| f.hint_marquee && f.chars.is_empty()).map(|f| f.digits);
        let hint_off = match hint {
            Some(digits) => {
                let fmt = if digits { &cv.g.fonts.mono_c } else { &cv.g.fonts.body };
                let hint_w = cv.text_w(placeholder, fmt);
                if hint_w > inner.w {
                    self.marquee_loop_offset(id ^ 0x2000_0000, hint_w + HINT_GAP).map(|dx| (dx, hint_w))
                } else {
                    None
                }
            }
            None => None,
        };

        let f = self.fields.entry(id).or_default();
        // Точка «•» — одна единица UTF-16, поэтому позиции курсора совпадают с символами.
        let shown: String = if f.secret { "•".repeat(f.chars.len()) } else { f.chars.iter().collect() };
        let text: Vec<u16> = shown.encode_utf16().collect();
        let fonts = &cv.g.fonts;
        let fmt = if f.digits { &fonts.mono } else { &fonts.body };
        let fmt_c = if f.digits { &fonts.mono_c } else { &fonts.body };
        // Меряем так же, как рисуем (с трекингом формата), иначе курсор уходит вперёд с длиной текста.
        let layout = fonts.layout_exact(&shown, fmt, 100000.0, r.h);
        let text_w = layout
            .as_ref()
            .and_then(|l| {
                let mut tm = DWRITE_TEXT_METRICS::default();
                l.GetMetrics(&mut tm).ok().map(|()| tm.widthIncludingTrailingWhitespace)
            })
            .unwrap_or(0.0);
        let pad_x = if f.digits { ((inner.w - text_w) * 0.5).max(0.0) } else { 0.0 };

        // Позиция курсора по мышке (нажатие и протяжка).
        if mine && down {
            if let Some(l) = &layout {
                let (mut trailing, mut inside) = (BOOL(0), BOOL(0));
                let mut m = DWRITE_HIT_TEST_METRICS::default();
                let x = mouse_x - (inner.x + pad_x) + f.scroll;
                if l.HitTestPoint(x, r.h / 2.0, &mut trailing, &mut inside, &mut m).is_ok() {
                    let p16 = m.textPosition + if trailing.as_bool() { m.length } else { 0 };
                    f.caret = if f.secret { (p16 as usize).min(f.chars.len()) } else { char_at_utf16(&f.chars, p16) };
                    if pressed && !shift_down() {
                        f.anchor = f.caret;
                    }
                }
            }
        }

        // Положение курсора и прокрутка, чтобы он оставался видимым.
        let caret16 = if f.secret { f.caret.min(f.chars.len()) as u32 } else { utf16_len(&f.chars[..f.caret.min(f.chars.len())]) };
        let mut caret_x = 0.0f32;
        let mut m = DWRITE_HIT_TEST_METRICS::default();
        let mut cy = 0.0f32;
        if let Some(l) = &layout {
            let mut cx = 0.0f32;
            if l.HitTestTextPosition(caret16, false, &mut cx, &mut cy, &mut m).is_ok() {
                caret_x = cx;
            }
        }
        if caret_x - f.scroll > inner.w - 2.0 {
            f.scroll = caret_x - inner.w + 2.0;
        }
        if caret_x - f.scroll < 0.0 {
            f.scroll = caret_x;
        }
        if f.scroll < 0.0 || text.is_empty() || pad_x > 0.0 {
            f.scroll = 0.0;
        }

        // Фон и рамка.
        cv.rr(r, 10.0, mix(card(), fg(), 0.035 * hv));
        let icon_base = mix(dim(), txt(), 0.35);
        cv.glyph(glyph, Rect::new(r.x + 4.0, r.y, 26.0, r.h), r.h >= 27.0, mix(icon_base, cv.accent, ov));

        // Текст, выделение и курсор — внутри клипа.
        cv.g.dc.PushAxisAlignedClip(&inner.d2d(), D2D1_ANTIALIAS_MODE_ALIASED);
        if text.is_empty() {
            let hint_c = mix(dim(), txt(), 0.25);
            match hint_off {
                // Не влезшая подсказка: две копии с зазором, сдвиг на период — ровно один круг.
                Some((dx, hint_w)) => {
                    let w = hint_w + 4.0;
                    cv.text(placeholder, Rect::new(inner.x + dx, r.y, w, r.h), fmt_c, hint_c);
                    cv.text(placeholder, Rect::new(inner.x + dx + hint_w + HINT_GAP, r.y, w, r.h), fmt_c, hint_c);
                }
                None => cv.text(placeholder, inner, fmt_c, hint_c),
            }
        } else if let Some(l) = &layout {
            if f.has_sel() {
                let (lo, hi) = f.range();
                let (mut x0, mut x1) = (0.0f32, 0.0f32);
                let (p0, p1) = if f.secret { (lo as u32, hi as u32) } else { (utf16_len(&f.chars[..lo]), utf16_len(&f.chars[..hi])) };
                if l.HitTestTextPosition(p0, false, &mut x0, &mut cy, &mut m).is_ok()
                    && l.HitTestTextPosition(p1, false, &mut x1, &mut cy, &mut m).is_ok()
                {
                    let mut c = cv.accent;
                    c.a = 0.35;
                    cv.rr(
                        Rect::new(inner.x + pad_x + x0 - f.scroll, r.cy() - 7.0, (x1 - x0).max(1.0), 14.0),
                        3.0,
                        c,
                    );
                }
            }
            let tr = Rect::new(inner.x + pad_x - f.scroll, r.y, caret_x.max(inner.w) + 200.0, r.h);
            cv.text(&shown, tr, fmt, txt());
        }
        if focused && caret_on {
            cv.rr(Rect::new(inner.x + pad_x + caret_x - f.scroll, r.cy() - 6.0, 1.2, 12.0), 0.6, txt());
        }
        cv.g.dc.PopAxisAlignedClip();

        if clearable && !text.is_empty() {
            let cr = Rect::new(r.right() - 24.0, r.y, 22.0, r.h);
            cv.glyph(super::icon::CLOSE, cr.inset(5.0), false, mix(dim(), txt(), if clear_hot { 1.0 } else { 0.35 }));
        }

        if focused {
            self.ime_pos = Some((origin_x + inner.x + pad_x + caret_x - f.scroll, r.bottom()));
        }
    }

    /// Клавиатурное событие в сфокусированное поле.
    pub fn key(&mut self, ev: KeyEv) {
        let Some(id) = self.focus else { return };
        let f = self.fields.entry(id).or_default();
        let n = f.chars.len();
        self.caret_on = true;
        match ev {
            KeyEv::Char(c) => {
                let mut b = [0u8; 4];
                f.insert(c.encode_utf8(&mut b));
            }
            KeyEv::Paste(s) => f.insert(&s),
            KeyEv::Left(sh) => {
                if !sh && f.has_sel() {
                    f.caret = f.range().0;
                } else {
                    f.caret = f.caret.saturating_sub(1);
                }
                if !sh {
                    f.anchor = f.caret;
                }
            }
            KeyEv::Right(sh) => {
                if !sh && f.has_sel() {
                    f.caret = f.range().1;
                } else {
                    f.caret = (f.caret + 1).min(n);
                }
                if !sh {
                    f.anchor = f.caret;
                }
            }
            KeyEv::Home(sh) => {
                f.caret = 0;
                if !sh {
                    f.anchor = 0;
                }
            }
            KeyEv::End(sh) => {
                f.caret = n;
                if !sh {
                    f.anchor = n;
                }
            }
            KeyEv::Backspace => {
                if !f.delete_sel() && f.caret > 0 {
                    f.chars.remove(f.caret - 1);
                    f.caret -= 1;
                    f.anchor = f.caret;
                }
            }
            KeyEv::BackspaceWord => {
                if !f.delete_sel() && f.caret > 0 {
                    let lo = if f.secret { 0 } else { prev_word_start(&f.chars, f.caret) };
                    f.chars.drain(lo..f.caret);
                    f.caret = lo;
                    f.anchor = lo;
                }
            }
            KeyEv::Delete => {
                if n == 0 {
                    self.nav.push(Nav::Delete);
                } else if !f.delete_sel() && f.caret < n {
                    f.chars.remove(f.caret);
                }
            }
            KeyEv::DeleteWord => {
                if n == 0 {
                    self.nav.push(Nav::Delete);
                } else if !f.delete_sel() && f.caret < n {
                    let hi = if f.secret { n } else { next_word_end(&f.chars, f.caret) };
                    f.chars.drain(f.caret..hi);
                }
            }
            KeyEv::DeleteLine => {
                if n > 0 {
                    let (lo, hi) = if f.secret { (0, n) } else { current_line_bounds(&f.chars, f.caret) };
                    f.chars.drain(lo..hi);
                    f.caret = lo;
                    f.anchor = lo;
                }
            }
            KeyEv::SelectAll => {
                f.anchor = 0;
                f.caret = n;
            }
            KeyEv::Copy => {
                if f.secret {
                } else if f.has_sel() {
                    self.clipboard_out = Some(f.selected());
                } else {
                    self.nav.push(Nav::Copy);
                }
            }
            KeyEv::Cut => {
                if f.has_sel() && !f.secret {
                    self.clipboard_out = Some(f.selected());
                    f.delete_sel();
                }
            }
            KeyEv::Enter => match f.submit {
                Submit::None => self.focus = None,
                Submit::Keep => self.submitted = Some((id, f.chars.iter().collect())),
                Submit::Clear => {
                    self.submitted = Some((id, f.chars.iter().collect()));
                    f.chars.clear();
                    f.caret = 0;
                    f.anchor = 0;
                    f.scroll = 0.0;
                }
            },
            KeyEv::Escape => self.focus = None,
            KeyEv::Up => self.nav.push(Nav::Up),
            KeyEv::Down => self.nav.push(Nav::Down),
            KeyEv::Pin => self.nav.push(Nav::Pin),
            KeyEv::Tab => self.nav.push(Nav::Tab),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_edit() {
        let mut f = Field::default();
        f.insert("привет");
        assert_eq!(f.chars.len(), 6);
        f.anchor = 0;
        assert_eq!(f.selected(), "привет");
        f.insert("мир");
        assert_eq!(f.chars.iter().collect::<String>(), "мир");
        assert_eq!(f.caret, 3);
    }

    #[test]
    fn utf16_positions() {
        let chars: Vec<char> = "a😀b".chars().collect();
        assert_eq!(utf16_len(&chars), 4);
        assert_eq!(char_at_utf16(&chars, 3), 2);
        assert_eq!(char_at_utf16(&chars, 4), 3);
    }

    #[test]
    fn ctrl_backspace_and_delete_word() {
        let mut ui = Ui::default();
        let id = 42;
        ui.set_field_text(id, "привет,   world_1  test");
        ui.focus = Some(id);

        // Ctrl+Backspace удаляет слово целиком.
        ui.key(KeyEv::BackspaceWord);
        assert_eq!(ui.field_text(id), "привет,   world_1  ");

        // Повторный Ctrl+Backspace пропускает хвостовые пробелы и удаляет "world_1".
        ui.key(KeyEv::BackspaceWord);
        assert_eq!(ui.field_text(id), "привет,   ");

        // Пропускает пробелы и удаляет знак препинания.
        ui.key(KeyEv::BackspaceWord);
        assert_eq!(ui.field_text(id), "привет");

        // Перемещаемся в начало и проверяем Delete / Ctrl+Delete.
        ui.key(KeyEv::Home(false));
        ui.key(KeyEv::Delete);
        assert_eq!(ui.field_text(id), "ривет");
        ui.key(KeyEv::DeleteWord);
        assert_eq!(ui.field_text(id), "");

        // Выделение удаляется целиком одним Ctrl+Backspace / Delete.
        ui.set_field_text(id, "alpha beta");
        ui.key(KeyEv::Left(true));
        ui.key(KeyEv::Left(true));
        ui.key(KeyEv::BackspaceWord);
        assert_eq!(ui.field_text(id), "alpha be");

        // В поле пароля Ctrl+Backspace стирает до начала, не выдавая границ слов.
        ui.set_secret(id, true);
        ui.set_field_text(id, "sec ret");
        ui.key(KeyEv::BackspaceWord);
        assert_eq!(ui.field_text(id), "");
    }

    #[test]
    fn fn_backspace_deletes_current_line() {
        let mut ui = Ui::default();
        let id = 11;
        ui.focus = Some(id);

        // Однострочный ввод очищается полностью.
        ui.set_field_text(id, "hello world");
        ui.key(KeyEv::Left(false));
        ui.key(KeyEv::DeleteLine);
        assert_eq!(ui.field_text(id), "");
        assert_eq!(ui.nav.len(), 0);

        // Пустое поле не отправляет Nav::Delete.
        ui.key(KeyEv::DeleteLine);
        assert_eq!(ui.nav.len(), 0);

        // В многострочном тексте удаляется только строка с курсором.
        ui.set_field_text(id, "one\ntwo\nthree");
        // Ставим курсор в середину "two" (индекс 5).
        ui.fields.get_mut(&id).unwrap().caret = 5;
        ui.key(KeyEv::DeleteLine);
        assert_eq!(ui.field_text(id), "one\nthree");
        assert_eq!(ui.fields[&id].caret, 4);

        // Удаление последней строки убирает предшествующий перевод строки.
        ui.fields.get_mut(&id).unwrap().caret = 6;
        ui.key(KeyEv::DeleteLine);
        assert_eq!(ui.field_text(id), "one");
        assert_eq!(ui.fields[&id].caret, 3);
    }
}

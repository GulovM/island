//! Текстовое поле ввода: курсор, выделение, навигация, IME.

use windows::core::BOOL;
use windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED;
use windows::Win32::Graphics::DirectWrite::DWRITE_HIT_TEST_METRICS;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_SHIFT};

use super::{card, fg, mix, rgb, txt, Canvas, Rect, Ui};

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
    Delete,
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

        let f = self.fields.entry(id).or_default();
        // Точка «•» — одна единица UTF-16, поэтому позиции курсора совпадают с символами.
        let shown: String = if f.secret { "•".repeat(f.chars.len()) } else { f.chars.iter().collect() };
        let text: Vec<u16> = shown.encode_utf16().collect();
        let fonts = &cv.g.fonts;
        let layout = fonts.factory.CreateTextLayout(&text, &fonts.body, 100000.0, r.h).ok();

        // Позиция курсора по мышке (нажатие и протяжка).
        if mine && down {
            if let Some(l) = &layout {
                let (mut trailing, mut inside) = (BOOL(0), BOOL(0));
                let mut m = DWRITE_HIT_TEST_METRICS::default();
                let x = mouse_x - inner.x + f.scroll;
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
        if f.scroll < 0.0 || text.is_empty() {
            f.scroll = 0.0;
        }

        // Фон и рамка.
        cv.rr(r, 10.0, mix(card(), fg(), 0.035 * hv));
        cv.glyph(glyph, Rect::new(r.x + 4.0, r.y, 26.0, r.h), r.h >= 27.0, mix(rgb(0.7, 0.7, 0.7), cv.accent, ov));

        // Текст, выделение и курсор — внутри клипа.
        cv.g.dc.PushAxisAlignedClip(&inner.d2d(), D2D1_ANTIALIAS_MODE_ALIASED);
        if text.is_empty() {
            cv.text(placeholder, inner, &fonts.body, rgb(0.5, 0.5, 0.5));
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
                        Rect::new(inner.x + x0 - f.scroll, r.cy() - 7.0, (x1 - x0).max(1.0), 14.0),
                        3.0,
                        c,
                    );
                }
            }
            let tr = Rect::new(inner.x - f.scroll, r.y, caret_x.max(inner.w) + 200.0, r.h);
            cv.text(&shown, tr, &fonts.body, txt());
        }
        if focused && caret_on {
            cv.rr(Rect::new(inner.x + caret_x - f.scroll, r.cy() - 6.0, 1.2, 12.0), 0.6, txt());
        }
        cv.g.dc.PopAxisAlignedClip();

        if focused {
            self.ime_pos = Some((origin_x + inner.x + caret_x - f.scroll, r.bottom()));
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
            KeyEv::Delete => {
                if n == 0 {
                    self.nav.push(Nav::Delete);
                } else if !f.delete_sel() && f.caret < n {
                    f.chars.remove(f.caret);
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
}

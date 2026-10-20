//! Список задач: хранение в `todo.txt`, анимация строк, карточка задач.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use super::Today;
use crate::anim::Spring;
use crate::tabs::{id, read_lines, write_lines, TODAY};
use crate::ui::{chip_bg, dim, fg, icon, ink, inset, mix, track, txt, Canvas, Rect, Submit, Ui, GREEN};

const MAX_TODO: usize = 100;

const TODO_FILE: &str = "todo.txt";

/// Плавность строки задачи: смещение при вставке/удалении соседей, появление и отметка «выполнено».
pub(super) struct RowAnim {
    pos: Spring,
    fade: Spring,
    done: Spring,
    /// Положение уже выставлено (первая отрисовка ставит строку на место без движения).
    placed: bool,
}

impl RowAnim {
    fn new(done: bool, entering: bool) -> Self {
        let mut fade = Spring::unit(if entering { 0.0 } else { 1.0 }, 220.0, 24.0);
        fade.target = 1.0;
        let mut d = Spring::unit(done as u8 as f32, 260.0, 26.0);
        d.target = done as u8 as f32;
        Self { pos: Spring::new(0.0, 300.0, 28.0), fade, done: d, placed: entering }
    }

    pub(super) fn settled(&self) -> bool {
        self.pos.settled() && self.fade.settled() && self.done.settled()
    }

    pub(super) fn step(&mut self, dt: f32) {
        self.pos.step(dt);
        self.fade.step(dt);
        self.done.step(dt);
    }
}

impl Today {
    pub fn load(&mut self) {
        self.todo = read_lines(TODO_FILE)
            .iter()
            .filter_map(|l| l.split_once('|'))
            .map(|(d, t)| (d == "1", t.to_string()))
            .take(MAX_TODO)
            .collect();
        self.anim = self.todo.iter().map(|(d, _)| RowAnim::new(*d, false)).collect();
    }

    fn save(&self) {
        let lines: Vec<String> = self.todo.iter().map(|(d, t)| format!("{}|{t}", if *d { 1 } else { 0 })).collect();
        write_lines(TODO_FILE, &lines);
    }

    pub(super) unsafe fn todo_card(&mut self, ui: &mut Ui, cv: &Canvas, card: Rect) {
        cv.card(card);
        let f = &cv.g.fonts;
        let done = self.todo.iter().filter(|(d, _)| *d).count();
        cv.text("Задачи", Rect::new(card.x + 11.0, card.y + 12.0, 90.0, 14.0), &f.body, txt());
        // Счётчик «выполнено/всего» в кружке с дугой прогресса.
        let (bx, by) = (card.right() - 21.0, card.y + 18.0);
        cv.circle(bx, by, 9.0, chip_bg());
        if !self.todo.is_empty() {
            cv.ring(bx, by, 8.0, 1.4, done as f32 / self.todo.len() as f32, GREEN);
        }
        cv.text(&format!("{done}/{}", self.todo.len()), Rect::new(bx - 9.0, by - 6.0, 18.0, 12.0), &f.caps_c, txt());

        // Поле «добавить» внизу и золотая кнопка со стрелкой.
        let fid = id(TODAY, 30);
        let field = Rect::new(card.x + 10.0, card.bottom() - 38.0, card.w - 20.0, 28.0);
        let editing = self.edit.is_some_and(|i| i < self.todo.len());
        if !editing {
            self.edit = None;
        }
        let (hint, glyph) = if editing { ("Изменить задачу…", icon::EDIT) } else { ("Добавить задачу…", icon::ADD) };
        // Нажатие достаётся первой зарегистрированной области под курсором, поэтому кнопки поверх
        // поля регистрируются раньше него, а рисуются после.
        let add = Rect::new(field.right() - 22.0, field.y + 5.0, 16.0, 16.0);
        let cancel = Rect::new(add.x - 19.0, add.y, 16.0, 16.0);
        let (_, add_clicked) = ui.click_area(id(TODAY, 31), add);
        let cancel_clicked = editing && ui.click_area(id(TODAY, 32), cancel).1;
        // Справа поверх поля лежат кнопки: текст до них не доходит.
        ui.text_field_pad(cv, fid, field, hint, glyph, Submit::Clear, if editing { 45.0 } else { 28.0 });
        cv.rr_line(field, 8.0, if editing { cv.accent } else { track() }, 1.0);
        let hv = ui.hover_of(id(TODAY, 31));
        cv.rr(add, 4.0, mix(cv.accent, fg(), 0.12 * hv));
        cv.glyph(if editing { icon::CHECK } else { icon::FORWARD }, add, false, ink());
        if editing {
            // Отмена правки: поле очищается, задача остаётся прежней.
            let hv = ui.hover_of(id(TODAY, 32));
            cv.rr(cancel, 8.0, D2D1_COLOR_F { a: 0.10 * hv, ..fg() });
            cv.glyph(icon::CLOSE, cancel, false, mix(dim(), txt(), hv));
        }
        if cancel_clicked {
            let _ = ui.submit_field(fid);
            self.edit = None;
        }
        let mut submitted = ui.take_submitted(fid);
        if add_clicked {
            submitted = ui.submit_field(fid);
        }
        if let Some(text) = submitted {
            let text = text.trim();
            match self.edit.take() {
                // Пустой текст при правке — отмена, задача не пропадает.
                Some(i) if !text.is_empty() && i < self.todo.len() => {
                    self.todo[i].1 = text.to_string();
                    self.save();
                }
                Some(_) => {}
                None if !text.is_empty() && self.todo.len() < MAX_TODO => {
                    self.todo.insert(0, (false, text.to_string()));
                    self.anim.insert(0, RowAnim::new(false, true));
                    self.save();
                }
                None => {}
            }
        }

        const ROW: f32 = 27.0;
        const GAP: f32 = 3.0;
        /// Длинная задача переносится максимум на столько строк, дальше — «…» (целиком — в правке).
        const MAX_LINES: usize = 3;
        let view = Rect::new(card.x + 9.0, card.y + 38.0, card.w - 18.0, (card.h - 38.0 - 48.0).max(ROW));
        let text_w = (view.w - 4.0 - 25.0 - 8.0).max(20.0);
        // Раскладка строк: (смещение, высота, число строк, высота строки текста).
        let mut rows: Vec<(f32, f32, usize, f32)> = Vec::with_capacity(self.todo.len());
        let mut total = 0.0f32;
        for (_, text) in &self.todo {
            let (lines, lh) = cv.wrapped(text, &f.wrap_l, text_w);
            let lines = lines.clamp(1, MAX_LINES);
            let h = (lines as f32 * lh + 11.0).max(ROW);
            rows.push((total, h, lines, lh));
            total += h + GAP;
        }
        self.anim.resize_with(self.todo.len(), || RowAnim::new(false, false));
        for (a, row) in self.anim.iter_mut().zip(&rows) {
            a.pos.target = row.0;
            if !a.placed {
                a.pos.snap();
                a.placed = true;
            }
        }
        let sid = id(TODAY, 100);
        let off = ui.scroll_begin(cv, sid, view, total);
        let mut toggled = None;
        let mut removed = None;
        let mut edit_req = None;
        for (i, (is_done, text)) in self.todo.iter().enumerate() {
            let (_, rh, lines, lh) = rows[i];
            let (fade, dn) = (self.anim[i].fade.value.clamp(0.0, 1.0), self.anim[i].done.value.clamp(0.0, 1.0));
            let rcv = Canvas { g: cv.g, alpha: cv.alpha * fade, accent: cv.accent };
            let cv = &rcv;
            let y = view.y + self.anim[i].pos.value - off;
            if y + rh < view.y || y > view.bottom() {
                continue;
            }
            let r = Rect::new(view.x, y, view.w - 4.0, rh);
            let rid = id(TODAY, 200 + i as u32);
            // Кнопки справа («изменить», «удалить») видны при наведении; их области регистрируются
            // раньше строки, иначе строка заберёт нажатие себе.
            let (eid, did) = (id(TODAY, 600 + i as u32), id(TODAY, 400 + i as u32));
            let del = Rect::new(r.right() - 21.0, r.y + ROW / 2.0 - 9.0, 18.0, 18.0);
            let edit = Rect::new(del.x - 20.0, del.y, 18.0, 18.0);
            let show = ui.hover_of(rid) > 0.3 || ui.hover_of(did) > 0.3 || ui.hover_of(eid) > 0.3;
            let (mut edit_clicked, mut del_clicked) = (false, false);
            if show {
                edit_clicked = ui.click_area(eid, edit).1;
                del_clicked = ui.click_area(did, del).1;
            }
            let (_, clicked) = ui.click_area(rid, r);
            let hv = ui.hover_of(rid);
            cv.rr(r, 8.0, mix(inset(), fg(), 0.035 * hv));
            if self.edit == Some(i) {
                cv.rr_line(r, 8.0, cv.accent, 1.0);
            }
            let (cx, cy) = (r.x + 13.0, r.y + ROW / 2.0);
            // Отметка выполнения проявляется плавно: кольцо гаснет, зелёный круг с галочкой нарастает.
            if dn < 0.99 {
                let ring = D2D1_COLOR_F { a: dim().a * (1.0 - dn), ..dim() };
                cv.rr_line(Rect::new(cx - 6.0, cy - 6.0, 12.0, 12.0), 6.0, ring, 1.2);
            }
            if dn > 0.01 {
                cv.circle(cx, cy, 6.0 * (0.5 + 0.5 * dn), D2D1_COLOR_F { a: dn, ..GREEN });
                cv.text(icon::CHECK, Rect::new(cx - 6.0, cy - 6.0, 12.0, 12.0), &f.icon_small, D2D1_COLOR_F { a: ink().a * dn, ..ink() });
            }
            let th = lines as f32 * lh;
            let tr = Rect::new(r.x + 25.0, r.y + (r.h - th) / 2.0, text_w, th + 2.0);
            let shown = cv.wrap_clip(text, &f.wrap_l, text_w, MAX_LINES);
            cv.text_strike(&shown, tr, &f.wrap_l, mix(txt(), dim(), dn), *is_done);
            if show {
                // Кнопки лежат поверх конца первой строки: подложка цвета строки.
                let patch = Rect::new(edit.x - 4.0, edit.y - 1.0, r.right() - edit.x + 2.0, 20.0);
                cv.rr(patch, 8.0, mix(inset(), fg(), 0.035 * hv));
            }
            if clicked {
                toggled = Some(i);
            }
            if show {
                for (bid, rect, glyph, active) in [(eid, edit, icon::EDIT, self.edit == Some(i)), (did, del, icon::DELETE, false)] {
                    let h = ui.hover_of(bid);
                    cv.rr(rect, 9.0, D2D1_COLOR_F { a: 0.10 * h, ..fg() });
                    cv.glyph(glyph, rect, false, if active { cv.accent } else { mix(dim(), txt(), h) });
                }
            }
            if del_clicked {
                removed = Some(i);
            } else if edit_clicked {
                edit_req = Some(i);
            }
        }
        ui.scroll_end(cv, sid);
        if let Some(i) = removed {
            self.todo.remove(i);
            if i < self.anim.len() {
                self.anim.remove(i);
            }
            match self.edit {
                Some(e) if e == i => {
                    let _ = ui.submit_field(fid);
                    self.edit = None;
                }
                Some(e) if e > i => self.edit = Some(e - 1),
                _ => {}
            }
            self.save();
        } else if let Some(i) = edit_req {
            self.edit = Some(i);
            ui.set_field_text(fid, &self.todo[i].1);
            ui.focus = Some(fid);
            ui.caret_on = true;
        } else if let Some(i) = toggled {
            self.todo[i].0 = !self.todo[i].0;
            if let Some(a) = self.anim.get_mut(i) {
                a.done.target = self.todo[i].0 as u8 as f32;
            }
            self.save();
        }
        if self.todo.is_empty() {
            cv.text("Список пуст", Rect::new(view.x + 4.0, view.y + 4.0, view.w, 14.0), &f.small, dim());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_todo_lines() {
        let lines = [
            "0|Купить молоко".to_string(),
            "1|Сдать отчёт".to_string(),
            "invalid_line".to_string(),
            "".to_string(),
        ];
        let items: Vec<(bool, String)> = lines
            .iter()
            .filter_map(|l| l.split_once('|'))
            .map(|(d, t)| (d == "1", t.to_string()))
            .take(MAX_TODO)
            .collect();
        assert_eq!(items, vec![
            (false, "Купить молоко".to_string()),
            (true, "Сдать отчёт".to_string()),
        ]);
    }
}

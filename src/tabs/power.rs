//! Вкладка Power: блокировка, сон, перезагрузка, выключение. Перезагрузка и выключение
//! требуют второго нажатия в течение трёх секунд — случайный клик ничего не сделает.

use std::os::windows::process::CommandExt;
use std::time::{Duration, Instant};

use crate::ui::{icon, Canvas, Rect, Ui};

use super::{id, PAD, POWER, TOP};

const CONFIRM: Duration = Duration::from_secs(3);
/// `CREATE_NO_WINDOW`: shutdown.exe не мигает консолью.
const NO_WINDOW: u32 = 0x0800_0000;

const LOCK: usize = 0;
const SLEEP: usize = 1;
const RESTART: usize = 2;
const SHUTDOWN: usize = 3;

#[derive(Default)]
pub struct Power {
    /// Кнопка, ждущая подтверждения, и момент, когда подтверждение истечёт.
    armed: Option<(usize, Instant)>,
}

impl Power {
    pub fn armed(&self) -> bool {
        self.armed.is_some()
    }

    pub fn reset(&mut self) {
        self.armed = None;
    }

    /// Секундный тик: подтверждение просроченной кнопки снимается.
    pub fn poll(&mut self) -> bool {
        match self.armed {
            Some((_, until)) if Instant::now() >= until => {
                self.armed = None;
                true
            }
            Some(_) => true,
            None => false,
        }
    }

    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, w: f32) {
        let items = [
            (icon::LOCK, "Блокировка"),
            (icon::MOON, "Сон"),
            (icon::RESTART, "Перезагрузка"),
            (icon::POWER, "Выключение"),
        ];
        let gap = 10.0;
        let bw = (w - 2.0 * PAD - 3.0 * gap) / 4.0;
        for (i, (g, l)) in items.iter().enumerate() {
            let r = Rect::new(PAD + i as f32 * (bw + gap), TOP, bw, 56.0);
            let armed = self.armed.is_some_and(|(a, _)| a == i);
            let label = if armed { "Подтвердить" } else { l };
            let clicked = ui.button(cv, id(POWER, i as u32), r, g, label);
            if armed {
                cv.rr_line(r, crate::ui::card_radius(), cv.accent, 1.5);
            }
            if clicked {
                self.press(i);
            }
        }
    }

    fn press(&mut self, idx: usize) {
        match idx {
            LOCK => {
                self.armed = None;
                unsafe {
                    let _ = windows::Win32::System::Shutdown::LockWorkStation();
                }
            }
            SLEEP => {
                self.armed = None;
                // SetSuspendState возвращается только после пробуждения — не блокируем остров.
                std::thread::spawn(|| unsafe {
                    let _ = windows::Win32::System::Power::SetSuspendState(false, false, false);
                });
            }
            RESTART | SHUTDOWN => {
                if self.armed.is_some_and(|(a, _)| a == idx) {
                    self.armed = None;
                    let flag = if idx == RESTART { "/r" } else { "/s" };
                    // Полный путь: имя без пути ищется в текущем каталоге и PATH (подмена exe).
                    let exe = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
                        .join("System32")
                        .join("shutdown.exe");
                    if let Err(e) = std::process::Command::new(exe)
                        .args([flag, "/t", "0"])
                        .creation_flags(NO_WINDOW)
                        .spawn()
                    {
                        crate::log::error(&format!("shutdown.exe не запустился: {e}"));
                    }
                } else {
                    self.armed = Some((idx, Instant::now() + CONFIRM));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_needs_second_press() {
        let mut p = Power::default();
        p.press(RESTART);
        assert!(p.armed());
        p.press(SHUTDOWN);
        assert_eq!(p.armed.map(|(a, _)| a), Some(SHUTDOWN));
        p.reset();
        assert!(!p.armed());
    }
}

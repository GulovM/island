//! Замер кадров анимации. Включается переменной окружения `ISLAND_PERF=1`;
//! в обычной работе стоит один `bool`. После каждой серии кадров пишет в лог итог:
//! сколько стоит отрисовка на CPU и ровно ли идут кадры.

use std::cell::Cell;
use std::time::{Duration, Instant};

use crate::log;

/// Интервал между кадрами длиннее этого считается пропуском (при 60 Гц кадр — 16,7 мс).
const SLOW_MS: f32 = 25.0;

thread_local! {
    static DRAW: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

/// Отрисовка (без ожидания vsync) закончена: `since` — начало кадра.
pub fn draw_done(since: Instant) {
    DRAW.set(since.elapsed());
}

#[derive(Default, Debug, PartialEq)]
struct Series {
    frames: u32,
    draw_sum: f32,
    draw_max: f32,
    gaps: u32,
    gap_sum: f32,
    gap_max: f32,
    slow: u32,
}

impl Series {
    fn add(&mut self, draw_ms: f32, gap_ms: Option<f32>) {
        self.frames += 1;
        self.draw_sum += draw_ms;
        self.draw_max = self.draw_max.max(draw_ms);
        if let Some(g) = gap_ms {
            self.gaps += 1;
            self.gap_sum += g;
            self.gap_max = self.gap_max.max(g);
            if g > SLOW_MS {
                self.slow += 1;
            }
        }
    }

    fn summary(&self) -> String {
        let n = self.frames.max(1) as f32;
        let g = self.gaps.max(1) as f32;
        format!(
            "кадров {}: отрисовка ср {:.2} / макс {:.2} мс; интервал ср {:.1} / макс {:.1} мс ({:.0} кадр/с); медленных (>{SLOW_MS:.0} мс) {}",
            self.frames,
            self.draw_sum / n,
            self.draw_max,
            self.gap_sum / g,
            self.gap_max,
            1000.0 / (self.gap_sum / g).max(0.001),
            self.slow
        )
    }
}

pub struct Perf {
    on: bool,
    series: Series,
    last: Option<Instant>,
}

impl Perf {
    pub fn new() -> Self {
        let on = std::env::var_os("ISLAND_PERF").is_some_and(|v| v != "0" && !v.is_empty());
        if on {
            log::info("замер кадров включён (ISLAND_PERF)");
        }
        Self { on, series: Series::default(), last: None }
    }

    /// Кадр показан (после `Present`).
    pub fn frame(&mut self) {
        if !self.on {
            return;
        }
        let now = Instant::now();
        let gap = self.last.map(|t| (now - t).as_secs_f32() * 1000.0);
        let draw = DRAW.get().as_secs_f32() * 1000.0;
        self.series.add(draw, gap);
        self.last = Some(now);
    }

    /// Анимация закончилась: записать итог серии. Короткие серии (несколько кадров) не пишем.
    pub fn flush(&mut self) {
        if !self.on || self.series.frames == 0 {
            return;
        }
        if self.series.frames >= 5 {
            log::info(&self.series.summary());
        }
        self.series = Series::default();
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_slow_frames_and_averages() {
        let mut s = Series::default();
        s.add(1.0, None);
        s.add(2.0, Some(16.7));
        s.add(3.0, Some(40.0));
        assert_eq!(s.frames, 3);
        assert_eq!(s.slow, 1);
        assert!((s.draw_sum / 3.0 - 2.0).abs() < 1e-6);
        assert_eq!(s.gaps, 2);
        assert!((s.gap_max - 40.0).abs() < 1e-6);
        assert!(s.summary().contains("кадров 3"));
    }

    #[test]
    fn disabled_does_nothing() {
        let mut p = Perf { on: false, series: Series::default(), last: None };
        p.frame();
        p.flush();
        assert_eq!(p.series, Series::default());
    }
}

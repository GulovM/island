//! Чистая логика графиков, спарклайнов и шкал для виджетов интерфейса.

/// Кольцевой буфер фиксированной ёмкости без динамических аллокаций.
#[derive(Clone, Copy, Debug)]
pub struct History<const N: usize> {
    buf: [f32; N],
    head: usize,
    len: usize,
}

impl<const N: usize> History<N> {
    pub const fn new() -> Self {
        Self {
            buf: [0.0; N],
            head: 0,
            len: 0,
        }
    }

    pub fn push(&mut self, val: f32) {
        if N == 0 {
            return;
        }
        self.buf[self.head] = val;
        self.head = (self.head + 1) % N;
        if self.len < N {
            self.len += 1;
        }
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Итератор по элементам буфера в хронологическом порядке (от старых к новым).
    pub fn iter(&self) -> HistoryIter<'_, N> {
        let start = if self.len < N { 0 } else { self.head };
        HistoryIter {
            hist: self,
            start,
            idx: 0,
        }
    }
}

impl<const N: usize> Default for History<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Итератор по элементам `History`.
#[derive(Clone, Copy)]
pub struct HistoryIter<'a, const N: usize> {
    hist: &'a History<N>,
    start: usize,
    idx: usize,
}

impl<'a, const N: usize> Iterator for HistoryIter<'a, N> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.idx < self.hist.len {
            let val = self.hist.buf[(self.start + self.idx) % N];
            self.idx += 1;
            Some(val)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let rem = self.hist.len.saturating_sub(self.idx);
        (rem, Some(rem))
    }
}

impl<'a, const N: usize> ExactSizeIterator for HistoryIter<'a, N> {}

/// Вычисляет углы дуги в градусах (отсчёт от верхней точки по часовой стрелке) для открытой дуги 270°.
/// Полная дуга: от -135° (низ-лево) до +135° (низ-право).
pub fn arc_span(frac: f32) -> (f32, f32) {
    let f = frac.clamp(0.0, 1.0);
    (-135.0, -135.0 + 270.0 * f)
}

/// Расчёт сегментной шкалы памяти RAM:
/// возвращает (общее число делений <= 24, число заполненных делений, цена одного деления в ГБ).
pub fn ram_segments(total_gb: f32, used_gb: f32) -> (usize, usize, f32) {
    let mut step = 1.0f32;
    let tot = total_gb.max(1.0);
    while (tot / step).round() > 24.0 {
        step *= 2.0;
    }
    let count = (tot / step).round().max(1.0) as usize;
    let filled = ((used_gb.max(0.0) / step).round() as usize).min(count);
    (count, filled, step)
}

/// Ближайший порог шкалы спидометра (100, 250, 500, 1000, 2000 или выше с шагом 1000),
/// не меньший входящей скорости `down`.
pub fn speed_scale(down: f32) -> f32 {
    const THRESHOLDS: [f32; 5] = [100.0, 250.0, 500.0, 1000.0, 2000.0];
    // Не число или бесконечность (сбой замера) не должны ни зациклить расчёт ниже, ни раздуть шкалу.
    if !down.is_finite() {
        return THRESHOLDS[0];
    }
    for &th in &THRESHOLDS {
        if down <= th {
            return th;
        }
    }
    let mut th = 2000.0f32;
    while down > th {
        th += 1000.0;
    }
    th
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_push_and_order() {
        let mut h = History::<4>::new();
        assert_eq!(h.len(), 0);
        assert!(h.is_empty());
        assert_eq!(h.iter().collect::<Vec<_>>(), Vec::<f32>::new());

        h.push(10.0);
        h.push(20.0);
        assert_eq!(h.len(), 2);
        assert_eq!(h.iter().collect::<Vec<_>>(), vec![10.0, 20.0]);

        h.push(30.0);
        h.push(40.0);
        assert_eq!(h.len(), 4);
        assert_eq!(h.iter().collect::<Vec<_>>(), vec![10.0, 20.0, 30.0, 40.0]);

        // Переполнение: вытесняются самые старые, порядок остаётся хронологическим
        h.push(50.0);
        assert_eq!(h.len(), 4);
        assert_eq!(h.iter().collect::<Vec<_>>(), vec![20.0, 30.0, 40.0, 50.0]);

        h.push(60.0);
        assert_eq!(h.iter().collect::<Vec<_>>(), vec![30.0, 40.0, 50.0, 60.0]);
    }

    #[test]
    fn test_arc_span_angles() {
        assert_eq!(arc_span(0.0), (-135.0, -135.0));
        assert_eq!(arc_span(0.5), (-135.0, 0.0));
        assert_eq!(arc_span(1.0), (-135.0, 135.0));
        // Clamp
        assert_eq!(arc_span(-0.5), (-135.0, -135.0));
        assert_eq!(arc_span(1.5), (-135.0, 135.0));
    }

    #[test]
    fn test_ram_segments_calculation() {
        // 16 ГБ всего, 10.9 ГБ занято: 16 делений, 11 заполнено, шаг 1 ГБ
        let (count, filled, step) = ram_segments(16.0, 10.9);
        assert_eq!(count, 16);
        assert_eq!(filled, 11);
        assert_eq!(step, 1.0);

        // 8 ГБ: 8 делений, 4 заполнено
        let (count, filled, step) = ram_segments(8.0, 4.0);
        assert_eq!(count, 8);
        assert_eq!(filled, 4);
        assert_eq!(step, 1.0);

        // 32 ГБ (> 24): шаг 2 ГБ, 16 делений
        let (count, filled, step) = ram_segments(32.0, 20.0);
        assert_eq!(count, 16);
        assert_eq!(filled, 10);
        assert_eq!(step, 2.0);

        // 64 ГБ: шаг 4 ГБ, 16 делений
        let (count, filled, step) = ram_segments(64.0, 32.0);
        assert_eq!(count, 16);
        assert_eq!(filled, 8);
        assert_eq!(step, 4.0);
    }

    #[test]
    fn test_speed_scale_thresholds() {
        assert_eq!(speed_scale(10.0), 100.0);
        assert_eq!(speed_scale(100.0), 100.0);
        assert_eq!(speed_scale(101.0), 250.0);
        assert_eq!(speed_scale(250.0), 250.0);
        assert_eq!(speed_scale(251.0), 500.0);
        assert_eq!(speed_scale(487.0), 500.0);
        assert_eq!(speed_scale(500.0), 500.0);
        assert_eq!(speed_scale(800.0), 1000.0);
        assert_eq!(speed_scale(1000.0), 1000.0);
        assert_eq!(speed_scale(1500.0), 2000.0);
        assert_eq!(speed_scale(2000.0), 2000.0);
        assert_eq!(speed_scale(2500.0), 3000.0);
        // Сбойные значения не зацикливают расчёт.
        assert_eq!(speed_scale(f32::INFINITY), 100.0);
        assert_eq!(speed_scale(f32::NAN), 100.0);
    }
}

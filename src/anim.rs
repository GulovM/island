/// Пружина: плавно тянет `value` к `target`. Даёт «живое» ощущение вместо фиксированных кривых.
pub struct Spring {
    pub value: f32,
    pub target: f32,
    vel: f32,
    stiffness: f32,
    damping: f32,
    /// Допуск «успокоилась»: 0,05 для пикселей, мельче для величин 0..1.
    eps: f32,
}

impl Spring {
    pub fn new(value: f32, stiffness: f32, damping: f32) -> Self {
        Self { value, target: value, vel: 0.0, stiffness, damping, eps: 0.05 }
    }

    /// Пружина для безразмерной величины 0..1 (наведение, прозрачность, ползунок).
    pub fn unit(value: f32, stiffness: f32, damping: f32) -> Self {
        Self { value, target: value, vel: 0.0, stiffness, damping, eps: 0.002 }
    }

    /// Меняет жёсткость и затухание на лету (скорость сохраняется).
    pub fn tune(&mut self, stiffness: f32, damping: f32) {
        self.stiffness = stiffness;
        self.damping = damping;
    }

    pub fn step(&mut self, dt: f32) {
        // Системная настройка «Показывать анимацию» выключена: пружина сходится в разы быстрее
        // вместо мгновенного прыжка без перехода — само движение остаётся, просто не воспринимается
        // глазом (как и просит системная настройка), а не исчезает как класс.
        let dt = if crate::theme::reduced_motion() { dt * 8.0 } else { dt };
        // Мелкие подшаги держат интегрирование устойчивым при редких кадрах.
        let n = ((dt / 0.004).ceil() as u32).clamp(1, 16);
        let h = dt / n as f32;
        for _ in 0..n {
            let acc = self.stiffness * (self.target - self.value) - self.damping * self.vel;
            self.vel += acc * h;
            self.value += self.vel * h;
        }
    }

    pub fn settled(&self) -> bool {
        (self.target - self.value).abs() < self.eps && self.vel.abs() < self.eps * 10.0
    }

    pub fn snap(&mut self) {
        self.value = self.target;
        self.vel = 0.0;
    }
}

/// Размер тела острова: одна интерполяция между свёрнутой и раскрытой формами.
/// Ширина и высота не бегут каждая своей пружиной, а выводятся из одного прогресса раскрытия,
/// поэтому остров растёт и сжимается как единое тело. Концы (`c`, `e`) — свои пружины: они
/// сглаживают смену формы (мини-плеер, уведомление) и размера вкладки.
pub struct Body {
    c: [Spring; 2],
    e: [Spring; 2],
}

impl Body {
    pub fn new(collapsed: (f32, f32)) -> Self {
        let s = |v| Spring::new(v, 420.0, 30.0);
        Self { c: [s(collapsed.0), s(collapsed.1)], e: [s(collapsed.0), s(collapsed.1)] }
    }

    /// Размер, к которому идёт раскрытая форма.
    pub fn expanded_goal(&self) -> (f32, f32) {
        (self.e[0].target, self.e[1].target)
    }

    pub fn set_targets(&mut self, collapsed: (f32, f32), expanded: (f32, f32)) {
        self.c[0].target = collapsed.0;
        self.c[1].target = collapsed.1;
        self.e[0].target = expanded.0;
        self.e[1].target = expanded.1;
    }

    /// Раскрытая форма не видна, пока остров свёрнут, — к цели она переходит без анимации.
    pub fn snap_expanded(&mut self) {
        self.e.iter_mut().for_each(Spring::snap);
    }

    /// Свёрнутая форма не видна, пока остров раскрыт целиком.
    pub fn snap_collapsed(&mut self) {
        self.c.iter_mut().for_each(Spring::snap);
    }

    pub fn snap(&mut self) {
        self.snap_expanded();
        self.snap_collapsed();
    }

    pub fn step(&mut self, dt: f32) {
        self.c.iter_mut().chain(self.e.iter_mut()).for_each(|s| s.step(dt));
    }

    pub fn settled(&self) -> bool {
        self.c.iter().chain(self.e.iter()).all(Spring::settled)
    }

    /// Размер тела при прогрессе раскрытия `open` (0 — свёрнут, 1 — раскрыт).
    pub fn size(&self, open: f32) -> (f32, f32) {
        let t = open.max(0.0);
        let at = |i: usize| self.c[i].value + (self.e[i].value - self.c[i].value) * t;
        (at(0), at(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ширина и высота при любом прогрессе лежат на одной прямой между формами:
    /// доля пройденного пути у них одинакова, отдельных растущих слоёв нет.
    #[test]
    fn body_axes_move_together() {
        let mut b = Body::new((100.0, 20.0));
        b.set_targets((100.0, 20.0), (500.0, 320.0));
        b.snap();
        for open in [0.0_f32, 0.25, 0.5, 0.8, 1.0] {
            let (w, h) = b.size(open);
            let (fw, fh) = ((w - 100.0) / 400.0, (h - 20.0) / 300.0);
            assert!((fw - fh).abs() < 1e-5, "open={open}: {fw} против {fh}");
        }
    }

    #[test]
    fn body_does_not_shrink_below_collapsed_on_undershoot() {
        let mut b = Body::new((100.0, 20.0));
        b.set_targets((100.0, 20.0), (500.0, 320.0));
        b.snap();
        assert_eq!(b.size(-0.05), (100.0, 20.0));
    }

    #[test]
    fn spring_converges() {
        let mut s = Spring::new(0.0, 420.0, 30.0);
        s.target = 100.0;
        for _ in 0..300 {
            s.step(1.0 / 60.0);
        }
        assert!(s.settled());
        assert!((s.value - 100.0).abs() < 0.1);
    }

    /// При выключенной системной настройке «Показывать анимацию» та же пружина доходит до цели
    /// за меньшее число кадров — движение быстрее, а не отсутствует совсем.
    #[test]
    fn reduced_motion_converges_faster() {
        let steps_to_settle = |reduced: bool| {
            crate::theme::set_reduced_motion(reduced);
            let mut s = Spring::new(0.0, 420.0, 30.0);
            s.target = 100.0;
            let mut n = 0;
            while !s.settled() && n < 300 {
                s.step(1.0 / 60.0);
                n += 1;
            }
            n
        };
        let normal = steps_to_settle(false);
        let reduced = steps_to_settle(true);
        crate::theme::set_reduced_motion(false);
        assert!(reduced < normal, "reduced={reduced} против normal={normal}");
    }

    /// Пружина считает по реальному `dt`, поэтому результат не зависит от частоты кадров.
    #[test]
    fn same_motion_at_60_120_144_hz() {
        let at = |hz: f32, secs: f32| {
            let mut s = Spring::new(0.0, 420.0, 30.0);
            s.target = 100.0;
            for _ in 0..(hz * secs).round() as u32 {
                s.step(1.0 / hz);
            }
            s.value
        };
        // Кратно 1/12 с: на 60/120/144/240 Гц это целое число кадров.
        for t in [1.0 / 12.0, 2.0 / 12.0, 4.0 / 12.0] {
            let base = at(60.0, t);
            for hz in [120.0, 144.0, 240.0] {
                assert!((at(hz, t) - base).abs() < 1.0, "{hz} Гц, t={t}: {} против {base}", at(hz, t));
            }
        }
    }

    /// Редкий кадр (подвисание) не должен «выстрелить» пружину за цель или в бесконечность.
    #[test]
    fn long_frame_stays_stable() {
        let mut s = Spring::new(0.0, 420.0, 30.0);
        s.target = 100.0;
        s.step(0.05);
        assert!(s.value.is_finite() && s.value < 150.0, "{}", s.value);
        for _ in 0..200 {
            s.step(0.05);
        }
        assert!(s.settled());
    }

    /// Успокоенная пружина перестаёт требовать кадров: на этом держится 0% CPU в простое.
    #[test]
    fn settles_within_a_second() {
        for (k, d) in [(420.0, 30.0), (500.0, 36.0)] {
            let mut s = Spring::unit(0.0, k, d);
            s.target = 1.0;
            let mut t = 0.0;
            while !s.settled() && t < 2.0 {
                s.step(1.0 / 60.0);
                t += 1.0 / 60.0;
            }
            assert!(t < 1.0, "k={k}: {t} с");
        }
    }

    #[test]
    fn unit_spring_is_finer() {
        let mut s = Spring::unit(0.0, 500.0, 36.0);
        s.target = 1.0;
        s.value = 0.99;
        assert!(!s.settled());
    }
}

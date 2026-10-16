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

#[cfg(test)]
mod tests {
    use super::*;

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

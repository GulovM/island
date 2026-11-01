//! Периодический и мгновенный peek лица (раскладка, время, Wi-Fi) на свёрнутом острове
//! во время воспроизведения музыки в мини-плеере.

/// Интервал между периодическими показами лица (секунды).
pub const PEEK_INTERVAL_SECS: f32 = 15.0;
/// Время удержания лица на экране перед возвратом в мини-плеер (секунды).
pub const PEEK_HOLD_SECS: f32 = 1.2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeekPhase {
    /// Ожидание следующего цикла (накопление времени).
    Idle,
    /// Плавное появление лица (мини-плеер гаснет, лицо проявляется).
    In,
    /// Удержание лица на экране (~1.2 сек).
    Hold,
    /// Плавное возвращение мини-плеера (лицо гаснет, мини-плеер проявляется).
    Out,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeekTransition {
    None,
    /// Переход в фазу In: лицо начинает проявляться.
    Start,
    /// Переход в фазу Out: лицо начинает гаснуть, мини-плеер возвращается.
    FadeOut,
    /// Завершение фазы Out: мини-плеер вернулся, переход в Idle.
    Finish,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Peek {
    phase: PeekPhase,
    elapsed: f32,
    hold_remaining: f32,
}

impl Default for Peek {
    fn default() -> Self {
        Self::new()
    }
}

impl Peek {
    pub fn new() -> Self {
        Self {
            phase: PeekPhase::Idle,
            elapsed: 0.0,
            hold_remaining: 0.0,
        }
    }

    #[cfg(test)]
    pub fn phase(&self) -> PeekPhase {
        self.phase
    }

    pub fn is_active(&self) -> bool {
        self.phase != PeekPhase::Idle
    }

    /// Лицо уже показывается или входит в показ - повторный trigger() должен только
    /// обновить данные (refresh_all), не перезапускать отсчёт удержания.
    pub fn is_showing(&self) -> bool {
        matches!(self.phase, PeekPhase::In | PeekPhase::Hold)
    }

    #[cfg(test)]
    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }

    #[cfg(test)]
    pub fn hold_remaining(&self) -> f32 {
        self.hold_remaining
    }

    /// Нужно ли сбрасывать состояние peek (активен показ или началось накопление времени).
    pub fn needs_reset(&self) -> bool {
        self.is_active() || self.elapsed > 0.0
    }

    /// Активен ли процесс показа (вход, показ или выход).
    /// Держит главный цикл активным (animating == true), пока не закончится переход.
    pub fn animating(&self) -> bool {
        self.phase != PeekPhase::Idle
    }

    /// Целевые альфы для (mini_a, face_alpha), если peek активен.
    /// Возвращает None, когда peek в Idle (альфами управляет обычный retarget).
    pub fn target_alphas(&self) -> Option<(f32, f32)> {
        match self.phase {
            PeekPhase::Idle => None,
            PeekPhase::In | PeekPhase::Hold => Some((0.0, 1.0)),
            PeekPhase::Out => Some((1.0, 0.0)),
        }
    }

    /// Запуск перехода peek (периодический или внеплановый по смене раскладки).
    pub fn trigger(&mut self) {
        self.phase = PeekPhase::In;
        self.elapsed = 0.0;
        self.hold_remaining = PEEK_HOLD_SECS;
    }

    /// Отмена peek и сброс накопленного времени (условие мини-плеера перестало выполняться).
    pub fn cancel(&mut self) {
        self.phase = PeekPhase::Idle;
        self.elapsed = 0.0;
        self.hold_remaining = 0.0;
    }

    /// Шаг состояния.
    /// `dt`: прошедшее время в секундах.
    /// `paused`: курсор над островом или идёт перетаскивание (приостанавливает накопление 15с).
    /// `face_settled`: пружина лица достигла цели (alpha >= 0.95 и settled).
    /// `mini_settled`: пружина мини-плеера достигла цели (mini_a >= 0.95 и settled).
    pub fn step(
        &mut self,
        dt: f32,
        paused: bool,
        face_settled: bool,
        mini_settled: bool,
    ) -> PeekTransition {
        match self.phase {
            PeekPhase::Idle => {
                if !paused {
                    self.elapsed += dt;
                    if self.elapsed >= PEEK_INTERVAL_SECS {
                        self.trigger();
                        return PeekTransition::Start;
                    }
                }
                PeekTransition::None
            }
            PeekPhase::In => {
                if face_settled {
                    self.phase = PeekPhase::Hold;
                    self.hold_remaining = PEEK_HOLD_SECS;
                }
                PeekTransition::None
            }
            PeekPhase::Hold => {
                self.hold_remaining -= dt;
                if self.hold_remaining <= 0.0 {
                    self.phase = PeekPhase::Out;
                    return PeekTransition::FadeOut;
                }
                PeekTransition::None
            }
            PeekPhase::Out => {
                if mini_settled {
                    self.phase = PeekPhase::Idle;
                    self.elapsed = 0.0;
                    return PeekTransition::Finish;
                }
                PeekTransition::None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_periodic_cycle_ticks() {
        let mut peek = Peek::new();
        assert_eq!(peek.phase(), PeekPhase::Idle);
        assert!(!peek.animating());
        assert_eq!(peek.target_alphas(), None);

        // Симуляция тиков по ~33 мс в течение 14.9 секунд: всё ещё Idle
        let dt = 0.033;
        let steps = (14.9 / dt) as usize;
        for _ in 0..steps {
            let tr = peek.step(dt, false, false, true);
            assert_eq!(tr, PeekTransition::None);
            assert_eq!(peek.phase(), PeekPhase::Idle);
            assert!(!peek.animating());
        }

        // Превышаем 15.0 секунд: должен сработать PeekTransition::Start
        let tr = peek.step(0.2, false, false, true);
        assert_eq!(tr, PeekTransition::Start);
        assert_eq!(peek.phase(), PeekPhase::In);
        assert!(peek.animating());
        assert_eq!(peek.target_alphas(), Some((0.0, 1.0)));

        // Фаза In: пока лицо не успокоилось у цели
        for _ in 0..5 {
            let tr = peek.step(dt, false, false, false);
            assert_eq!(tr, PeekTransition::None);
            assert_eq!(peek.phase(), PeekPhase::In);
        }

        // Лицо успокоилось у 1.0: переход в Hold
        let tr = peek.step(dt, false, true, false);
        assert_eq!(tr, PeekTransition::None);
        assert_eq!(peek.phase(), PeekPhase::Hold);
        assert!(peek.animating());
        assert_eq!(peek.target_alphas(), Some((0.0, 1.0)));

        // Фаза Hold: держится ~1.2 секунды
        let tr = peek.step(0.6, false, true, false);
        assert_eq!(tr, PeekTransition::None);
        assert_eq!(peek.phase(), PeekPhase::Hold);

        // Время удержания истекло: FadeOut
        let tr = peek.step(0.7, false, true, false);
        assert_eq!(tr, PeekTransition::FadeOut);
        assert_eq!(peek.phase(), PeekPhase::Out);
        assert!(peek.animating());
        assert_eq!(peek.target_alphas(), Some((1.0, 0.0)));

        // Фаза Out: мини-плеер возвращается
        for _ in 0..5 {
            let tr = peek.step(dt, false, false, false);
            assert_eq!(tr, PeekTransition::None);
            assert_eq!(peek.phase(), PeekPhase::Out);
        }

        // Мини-плеер успокоился у 1.0: возврат в Idle
        let tr = peek.step(dt, false, false, true);
        assert_eq!(tr, PeekTransition::Finish);
        assert_eq!(peek.phase(), PeekPhase::Idle);
        assert!(!peek.animating());
        assert_eq!(peek.target_alphas(), None);
        assert_eq!(peek.elapsed(), 0.0);
    }

    #[test]
    fn test_hover_drag_pauses_accumulation() {
        let mut peek = Peek::new();

        // Накопили 5 секунд
        peek.step(5.0, false, false, false);
        assert!((peek.elapsed() - 5.0).abs() < 1e-4);

        // Курсор над островом (paused = true) в течение 20 секунд: накопление заморожено
        for _ in 0..100 {
            let tr = peek.step(0.2, true, false, false);
            assert_eq!(tr, PeekTransition::None);
            assert_eq!(peek.phase(), PeekPhase::Idle);
        }
        assert!((peek.elapsed() - 5.0).abs() < 1e-4);

        // Курсор ушёл (paused = false): продолжаем ровно с 5.0 секунд
        peek.step(9.9, false, false, false);
        assert!((peek.elapsed() - 14.9).abs() < 1e-4);
        assert_eq!(peek.phase(), PeekPhase::Idle);

        // На 15.0 срабатывает
        let tr = peek.step(0.2, false, false, false);
        assert_eq!(tr, PeekTransition::Start);
        assert_eq!(peek.phase(), PeekPhase::In);
    }

    #[test]
    fn test_layout_change_triggers_instant_peek_and_resets_counter() {
        let mut peek = Peek::new();

        // Прошло 10 секунд ожидания
        peek.step(10.0, false, false, false);
        assert!((peek.elapsed() - 10.0).abs() < 1e-4);

        // Смена раскладки вызывает мгновенный trigger()
        peek.trigger();
        assert_eq!(peek.phase(), PeekPhase::In);
        assert_eq!(peek.elapsed(), 0.0);
        assert!(peek.animating());

        // Завершаем переход в Hold и Out до возврата в Idle
        peek.step(0.1, false, true, false); // -> Hold
        peek.step(1.3, false, true, false); // -> Out
        peek.step(0.1, false, false, true); // -> Idle
        assert_eq!(peek.phase(), PeekPhase::Idle);
        assert_eq!(peek.elapsed(), 0.0);

        // Проверяем, что после внепланового peek периодический таймер отсчитывает полные 15 секунд
        peek.step(14.9, false, false, false);
        assert_eq!(peek.phase(), PeekPhase::Idle);

        let tr = peek.step(0.2, false, false, false);
        assert_eq!(tr, PeekTransition::Start);
    }

    #[test]
    fn test_cancel_when_mini_form_lost() {
        let mut peek = Peek::new();
        peek.step(15.0, false, false, false); // -> In
        peek.step(0.1, false, true, false);  // -> Hold
        assert_eq!(peek.phase(), PeekPhase::Hold);
        assert!(peek.animating());

        // Трек поставлен на паузу или остров развёрнут: сброс без паники
        peek.cancel();
        assert_eq!(peek.phase(), PeekPhase::Idle);
        assert_eq!(peek.elapsed(), 0.0);
        assert_eq!(peek.hold_remaining(), 0.0);
        assert!(!peek.animating());
        assert_eq!(peek.target_alphas(), None);
    }

    #[test]
    fn test_needs_reset() {
        let mut peek = Peek::new();
        assert!(!peek.needs_reset());

        peek.step(1.0, false, false, false);
        assert!(peek.needs_reset());

        peek.cancel();
        assert!(!peek.needs_reset());

        peek.trigger();
        assert!(peek.needs_reset());
    }

    #[test]
    fn test_is_showing() {
        let mut peek = Peek::new();
        assert!(!peek.is_showing()); // Idle

        peek.trigger();
        assert!(peek.is_showing()); // In

        peek.step(0.05, false, true, false);
        assert_eq!(peek.phase(), PeekPhase::Hold);
        assert!(peek.is_showing()); // Hold

        let rem = peek.hold_remaining();
        peek.step(rem + 0.05, false, true, false);
        assert_eq!(peek.phase(), PeekPhase::Out);
        assert!(!peek.is_showing()); // Out
    }
}

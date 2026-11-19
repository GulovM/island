//! Замер пикового уровня звука для живых волн мини-плеера.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[allow(dead_code)]
static CAPTURE_OK: AtomicBool = AtomicBool::new(false);
#[allow(dead_code)]
static LAST_PEAK: Mutex<[f32; 2]> = Mutex::new([0.0, 0.0]);

#[derive(Default)]
pub struct AudioMeter {
    pub active: bool,
}

impl AudioMeter {
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(dead_code)]
    pub fn open(&mut self) {
        self.active = false;
        CAPTURE_OK.store(false, Ordering::Relaxed);
    }

    pub fn close(&mut self) {
        self.active = false;
        CAPTURE_OK.store(false, Ordering::Relaxed);
    }

    pub fn invalidate(&self) {}

    pub fn reset(&mut self) {
        self.close();
    }

    #[cfg_attr(test, allow(dead_code))]
    pub fn sample(&mut self) -> [f32; 2] {
        LAST_PEAK.lock().map(|g| *g).unwrap_or([0.0, 0.0])
    }
}

#[cfg(test)]
pub(crate) fn fold_stereo(ch: &[f32]) -> [f32; 2] {
    match ch.len() {
        0 => [0.0, 0.0],
        1 => [ch[0], ch[0]],
        2 => [ch[0], ch[1]],
        n => {
            let mut l = ch[0].max(ch.get(2).copied().unwrap_or(0.0));
            let mut r = ch[1].max(ch.get(2).copied().unwrap_or(0.0));
            for (i, &val) in ch.iter().enumerate().take(n).skip(3) {
                if i == 3 {
                    l = l.max(val);
                    r = r.max(val);
                } else if i % 2 == 0 {
                    l = l.max(val);
                } else {
                    r = r.max(val);
                }
            }
            [l, r]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fold_stereo;

    #[test]
    fn fold_stereo_mono_and_stereo() {
        assert_eq!(fold_stereo(&[]), [0.0, 0.0]);
        assert_eq!(fold_stereo(&[0.5]), [0.5, 0.5]);
        assert_eq!(fold_stereo(&[0.2, 0.8]), [0.2, 0.8]);
        let f = fold_stereo(&[0.1, 0.2, 0.6, 0.0, 0.4, 0.9]);
        assert_eq!(f, [0.6, 0.9]);
    }
}

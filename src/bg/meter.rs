//! Измерение стерео-пиков системного аудио (`IAudioMeterInformation`) для волн свёрнутой чёлки.
//! Задействуется только пока на обычной свёрнутой чёлке отображаются волны; при паузе,
//! разворачивании или режиме полоски COM-интерфейс освобождается и опрос полностью прекращается.

use std::time::{Duration, Instant};

use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::System::Com::CLSCTX_ALL;

use super::devices::device;

#[cfg_attr(test, allow(dead_code))]
const MAX_CH: usize = 8;
#[cfg_attr(test, allow(dead_code))]
const RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// Кэшированный COM-измеритель пиков активного устройства вывода.
#[derive(Default)]
pub struct AudioMeter {
    meter: Option<IAudioMeterInformation>,
    channels: usize,
    last_retry: Option<Instant>,
}

impl AudioMeter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Освобождает кэшированный COM-интерфейс (простой с нулевым потреблением).
    pub fn reset(&mut self) {
        self.meter = None;
        self.channels = 0;
        self.last_retry = None;
    }

    /// Сбрасывает привязку к устройству вывода, чтобы на следующем кадре переключиться на новое.
    pub fn invalidate(&mut self) {
        self.reset();
    }

    /// Возвращает текущие нормализованные пики левого и правого каналов в диапазоне `[0.0, 1.0]`.
    #[cfg_attr(test, allow(dead_code))]
    pub unsafe fn sample(&mut self) -> [f32; 2] {
        if self.meter.is_none() {
            if self.last_retry.is_some_and(|t| t.elapsed() < RETRY_INTERVAL) {
                return [0.0, 0.0];
            }
            if !self.connect() {
                self.last_retry = Some(Instant::now());
                return [0.0, 0.0];
            }
        }

        let Some(meter) = self.meter.as_ref() else {
            return [0.0, 0.0];
        };

        let n = self.channels.clamp(1, MAX_CH);
        let mut buf = [0.0f32; MAX_CH];
        if meter.GetChannelsPeakValues(&mut buf[..n]).is_ok() {
            return fold_stereo(&buf[..n]);
        }

        match meter.GetPeakValue() {
            Ok(p) => {
                let v = clean_peak(p);
                [v, v]
            }
            Err(_) => {
                // Устройство могло отключиться или смениться (AUDCLNT_E_DEVICE_INVALIDATED и т.п.).
                self.meter = None;
                self.last_retry = Some(Instant::now());
                [0.0, 0.0]
            }
        }
    }

    #[cfg_attr(test, allow(dead_code))]
    unsafe fn connect(&mut self) -> bool {
        let Some(dev) = device() else { return false };
        let Ok(meter) = dev.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None) else {
            return false;
        };
        let ch = meter.GetMeteringChannelCount().unwrap_or(2).clamp(1, MAX_CH as u32) as usize;
        self.meter = Some(meter);
        self.channels = ch;
        self.last_retry = None;
        true
    }
}

#[inline]
fn clean_peak(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

/// Сводит N-канальный массив пиков WASAPI (`FL, FR, FC, LFE, BL, BR, SL, SR`) к стерео `[L, R]`.
pub(crate) fn fold_stereo(ch: &[f32]) -> [f32; 2] {
    match ch.len() {
        0 => [0.0, 0.0],
        1 => {
            let v = clean_peak(ch[0]);
            [v, v]
        }
        2 => [clean_peak(ch[0]), clean_peak(ch[1])],
        _ => {
            let center = clean_peak(ch[2]) * 0.7;
            let lfe = if ch.len() > 3 { clean_peak(ch[3]) * 0.55 } else { 0.0 };
            let mut l = clean_peak(ch[0]).max(center).max(lfe);
            let mut r = clean_peak(ch[1]).max(center).max(lfe);
            let mut idx = 4;
            while idx < ch.len() {
                l = l.max(clean_peak(ch[idx]));
                if idx + 1 < ch.len() {
                    r = r.max(clean_peak(ch[idx + 1]));
                }
                idx += 2;
            }
            [l, r]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_stereo_handles_mono_stereo_surround_and_nan() {
        assert_eq!(fold_stereo(&[]), [0.0, 0.0]);
        assert_eq!(fold_stereo(&[0.42]), [0.42, 0.42]);
        assert_eq!(fold_stereo(&[0.2, 0.8]), [0.2, 0.8]);
        assert_eq!(fold_stereo(&[f32::NAN, 1.5]), [0.0, 1.0]);
        let surround = [0.1, 0.2, 0.5, 0.4, 0.9, 0.3];
        assert_eq!(fold_stereo(&surround), [0.9, 0.5 * 0.7]);
    }
}

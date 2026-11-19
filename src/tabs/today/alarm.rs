//! Сигнал окончания таймера: WAV из памяти и воспроизведение в отдельном потоке.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// WAV (16 бит, моно) с тремя двойными гудками: звучит через звуковое устройство по умолчанию,
/// в отличие от `Beep` и звуков схемы Windows, которые на многих системах молчат.
fn alarm_wav() -> Vec<u8> {
    let mut pcm: Vec<i16> = Vec::new();
    for _ in 0..2 {
        tone(&mut pcm, 523.0, 220);
        tone(&mut pcm, 659.0, 220);
        tone(&mut pcm, 784.0, 420);
        pcm.extend(std::iter::repeat_n(0i16, (RATE * 400 / 1000) as usize));
    }
    pcm_wav(&pcm)
}

/// Одиночный короткий сигнал для напоминания о событии.
fn ding_wav() -> Vec<u8> {
    let mut pcm: Vec<i16> = Vec::new();
    tone(&mut pcm, 784.0, 160);
    tone(&mut pcm, 1047.0, 260);
    pcm_wav(&pcm)
}

const RATE: u32 = 44_100;

/// Добавляет в `pcm` тон `freq` Гц длиной `ms` мс.
fn tone(pcm: &mut Vec<i16>, freq: f32, ms: u32) {
    let n = RATE * ms / 1000;
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        // Короткое нарастание и затухание, чтобы не щёлкало.
        let env = (i.min(n - i) as f32 / (RATE as f32 * 0.01)).min(1.0);
        pcm.push((env * 0.4 * i16::MAX as f32 * (std::f32::consts::TAU * freq * t).sin()) as i16);
    }
}

fn pcm_wav(pcm: &[i16]) -> Vec<u8> {
    let bytes = (pcm.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + bytes as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + bytes).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&(RATE * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&bytes.to_le_bytes());
    for v in pcm {
        w.extend_from_slice(&v.to_le_bytes());
    }
    w
}

static PLAYING: AtomicBool = AtomicBool::new(false);
/// Просьба оборвать сигнал (уведомление закрыли крестиком или Esc). Сам звук глушит поток
/// воспроизведения: вызов из потока интерфейса блокировался бы до конца мелодии.
static STOP: AtomicBool = AtomicBool::new(false);

/// Сигнал ещё звучит: уведомление на острове остаётся, пока он не закончится.
pub fn is_playing() -> bool {
    PLAYING.load(Ordering::Acquire)
}

/// Обрывает звучащий сигнал: повторы и гудки не начинаются, текущая мелодия глушится.
pub fn stop_alarm() {
    if PLAYING.load(Ordering::Acquire) {
        STOP.store(true, Ordering::Release);
    }
}

/// Длительность PCM-WAV по его заголовку.
fn wav_duration(w: &[u8]) -> Option<Duration> {
    if w.len() < 12 || &w[..4] != b"RIFF" || &w[8..12] != b"WAVE" {
        return None;
    }
    let (mut byte_rate, mut data) = (0u32, None);
    let mut at = 12;
    while at + 8 <= w.len() {
        let size = u32::from_le_bytes(w[at + 4..at + 8].try_into().ok()?) as usize;
        match &w[at..at + 4] {
            b"fmt " if at + 20 <= w.len() => byte_rate = u32::from_le_bytes(w[at + 16..at + 20].try_into().ok()?),
            b"data" => {
                data = Some(size.min(w.len() - at - 8));
                break;
            }
            _ => {}
        }
        at += 8 + size + (size & 1);
    }
    let data = data?;
    (byte_rate > 0).then(|| Duration::from_secs_f64(data as f64 / f64::from(byte_rate)))
}

/// Играет WAV на macOS через системный `afplay` (`repeats` раз), поддерживая прерывание по `STOP`.
unsafe fn play_wav(wav: &[u8], repeats: u32) -> bool {
    let _len = wav_duration(wav);
    let tmp = std::env::temp_dir().join("kite-island-alarm.wav");
    if std::fs::write(&tmp, wav).is_err() {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        for _ in 0..repeats {
            if STOP.load(Ordering::Acquire) {
                return true;
            }
            let Ok(mut child) = std::process::Command::new("/usr/bin/afplay").arg(&tmp).spawn() else {
                return false;
            };
            loop {
                if STOP.load(Ordering::Acquire) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return true;
                }
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                    Err(_) => break,
                }
            }
        }
        true
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = repeats;
        false
    }
}

/// Сигнал окончания таймера в отдельном потоке (не блокирует интерфейс).
pub(super) fn play_alarm() {
    if PLAYING.swap(true, Ordering::AcqRel) {
        return;
    }
    STOP.store(false, Ordering::Release);
    std::thread::spawn(|| {
        unsafe {
            if !play_wav(&alarm_wav(), 2) && !STOP.load(Ordering::Acquire) {
                #[cfg(target_os = "macos")]
                let _ = std::process::Command::new("/usr/bin/afplay")
                    .arg("/System/Library/Sounds/Glass.aiff")
                    .status();
            }
        }
        PLAYING.store(false, Ordering::Release);
    });
}

/// Один короткий сигнал о событии календаря. Не мешает долгому сигналу таймера: пока тот звучит,
/// второй не запускается.
pub(super) fn play_ding() {
    if PLAYING.swap(true, Ordering::AcqRel) {
        return;
    }
    STOP.store(false, Ordering::Release);
    std::thread::spawn(|| {
        unsafe {
            if !play_wav(&ding_wav(), 1) && !STOP.load(Ordering::Acquire) {
                #[cfg(target_os = "macos")]
                let _ = std::process::Command::new("/usr/bin/afplay")
                    .arg("/System/Library/Sounds/Tink.aiff")
                    .status();
            }
        }
        PLAYING.store(false, Ordering::Release);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Слышимая проверка: `cargo test alarm_plays -- --ignored`.
    #[test]
    #[ignore]
    fn alarm_plays() {
        let wav = alarm_wav();
        let tmp = std::env::temp_dir().join("island-alarm-test.wav");
        assert!(std::fs::write(&tmp, &wav).is_ok());
        #[cfg(target_os = "macos")]
        {
            let ok = std::process::Command::new("/usr/bin/afplay")
                .arg(&tmp)
                .status()
                .is_ok_and(|s| s.success());
            let _ = std::fs::remove_file(&tmp);
            assert!(ok);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn wav_length_from_header() {
        let d = wav_duration(&alarm_wav()).unwrap();
        // Два раза по 220 + 220 + 420 + 400 мс.
        assert!((d.as_secs_f32() - 2.52).abs() < 0.01, "{d:?}");
        assert!(wav_duration(b"nope").is_none());
    }

    #[test]
    fn ding_is_short_and_valid() {
        let d = wav_duration(&ding_wav()).unwrap();
        assert!(d.as_secs_f32() < 0.6, "{d:?}");
    }

    #[test]
    fn alarm_wav_is_valid() {
        let w = alarm_wav();
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[8..12], b"WAVE");
        let data = u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize;
        assert_eq!(w.len(), 44 + data);
    }
}

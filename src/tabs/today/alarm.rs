//! Сигнал окончания таймера: WAV из памяти и воспроизведение в отдельном потоке.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT, SND_PURGE};
use windows::Win32::System::Diagnostics::Debug::Beep;

/// WAV (16 бит, моно) с тремя двойными гудками: звучит через звуковое устройство по умолчанию,
/// в отличие от `Beep` и звуков схемы Windows, которые на многих системах молчат.
fn alarm_wav() -> Vec<u8> {
    const RATE: u32 = 44_100;
    let mut pcm: Vec<i16> = Vec::new();
    let tone = |pcm: &mut Vec<i16>, freq: f32, ms: u32| {
        let n = RATE * ms / 1000;
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            // Короткое нарастание и затухание, чтобы не щёлкало.
            let env = (i.min(n - i) as f32 / (RATE as f32 * 0.01)).min(1.0);
            pcm.push((env * 0.4 * i16::MAX as f32 * (std::f32::consts::TAU * freq * t).sin()) as i16);
        }
    };
    for _ in 0..2 {
        tone(&mut pcm, 523.0, 220);
        tone(&mut pcm, 659.0, 220);
        tone(&mut pcm, 784.0, 420);
        pcm.extend(std::iter::repeat_n(0i16, (RATE * 400 / 1000) as usize));
    }
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

/// Играет WAV из памяти `repeats` раз, не блокируясь внутри PlaySound, чтобы его можно было оборвать.
/// `false` — воспроизвести не удалось.
unsafe fn play_wav(wav: &[u8], repeats: u32) -> bool {
    let Some(len) = wav_duration(wav) else { return false };
    for _ in 0..repeats {
        if STOP.load(Ordering::Acquire) {
            return true;
        }
        if !PlaySoundW(windows::core::PCWSTR(wav.as_ptr().cast()), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT).as_bool() {
            return false;
        }
        // Буфер должен жить, пока звук играет.
        let end = Instant::now() + len + Duration::from_millis(80);
        while Instant::now() < end {
            if STOP.load(Ordering::Acquire) {
                let _ = PlaySoundW(windows::core::PCWSTR::null(), None, SND_PURGE);
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    true
}

/// Сигнал окончания таймера в отдельном потоке (не блокирует интерфейс).
pub(super) fn play_alarm() {
    if PLAYING.swap(true, Ordering::AcqRel) {
        return;
    }
    STOP.store(false, Ordering::Release);
    std::thread::spawn(|| {
        unsafe {
            // Стандартный сигнал будильника Windows (дважды); нет файла — мягкий звон из памяти,
            // и только если не звучит ничего — системный гудок.
            let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
            let mut ok = std::fs::read(format!(r"{root}\Media\Alarm01.wav")).is_ok_and(|w| play_wav(&w, 2));
            if !ok {
                ok = play_wav(&alarm_wav(), 1);
            }
            if !ok {
                for _ in 0..3 {
                    if STOP.load(Ordering::Acquire) {
                        break;
                    }
                    let _ = Beep(880, 150);
                    let _ = Beep(1175, 200);
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        }
        PLAYING.store(false, Ordering::Release);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Media::Audio::SND_SYNC;

    /// Слышимая проверка: `cargo test alarm_plays -- --ignored`.
    #[test]
    #[ignore]
    fn alarm_plays() {
        let wav = alarm_wav();
        let ok = unsafe { PlaySoundW(windows::core::PCWSTR(wav.as_ptr().cast()), None, SND_MEMORY | SND_SYNC | SND_NODEFAULT).as_bool() };
        assert!(ok);
    }

    #[test]
    fn wav_length_from_header() {
        let d = wav_duration(&alarm_wav()).unwrap();
        // Два раза по 220 + 220 + 420 + 400 мс.
        assert!((d.as_secs_f32() - 2.52).abs() < 0.01, "{d:?}");
        assert!(wav_duration(b"nope").is_none());
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

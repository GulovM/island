//! Сигнал окончания таймера: WAV из памяти и воспроизведение в отдельном потоке.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows::Win32::Media::Audio::{PlaySoundW, SND_FILENAME, SND_MEMORY, SND_NODEFAULT, SND_SYNC};
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

/// Сигнал окончания таймера в отдельном потоке (не блокирует интерфейс).
pub(super) fn play_alarm() {
    static PLAYING: AtomicBool = AtomicBool::new(false);
    if PLAYING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(|| {
        unsafe {
            // Стандартный сигнал будильника Windows (дважды); нет файла — мягкий звон из памяти,
            // и только если не звучит ничего — системный гудок.
            let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
            let file: Vec<u16> = format!(r"{root}\Media\Alarm01.wav").encode_utf16().chain(Some(0)).collect();
            let mut ok = false;
            for _ in 0..2 {
                ok = PlaySoundW(windows::core::PCWSTR(file.as_ptr()), None, SND_FILENAME | SND_SYNC | SND_NODEFAULT).as_bool();
                if !ok {
                    break;
                }
            }
            if !ok {
                let wav = alarm_wav();
                ok = PlaySoundW(windows::core::PCWSTR(wav.as_ptr().cast()), None, SND_MEMORY | SND_SYNC | SND_NODEFAULT).as_bool();
            }
            if !ok {
                for _ in 0..3 {
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

    /// Слышимая проверка: `cargo test alarm_plays -- --ignored`.
    #[test]
    #[ignore]
    fn alarm_plays() {
        let wav = alarm_wav();
        let ok = unsafe { PlaySoundW(windows::core::PCWSTR(wav.as_ptr().cast()), None, SND_MEMORY | SND_SYNC | SND_NODEFAULT).as_bool() };
        assert!(ok);
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

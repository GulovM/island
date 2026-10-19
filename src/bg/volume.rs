//! Подписка на изменения громкости устройства вывода по умолчанию.

use std::cell::RefCell;
use std::sync::atomic::Ordering;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::AUDIO_VOLUME_NOTIFICATION_DATA;
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use super::devices::{device, device_id};
use super::{HWND_MAIN, WM_VOLUME};
use crate::log;

/// Приёмник уведомлений об изменении громкости: вызывается системой в своём потоке, только
/// пересылает значения главному окну — тяжёлого здесь ничего нет.
#[windows::core::implement(IAudioEndpointVolumeCallback)]
struct VolumeWatch;

#[allow(non_snake_case)]
impl IAudioEndpointVolumeCallback_Impl for VolumeWatch_Impl {
    fn OnNotify(&self, pnotify: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        let h = HWND_MAIN.load(Ordering::Relaxed);
        if let (Some(n), true) = (unsafe { pnotify.as_ref() }, h != 0) {
            let permille = (n.fMasterVolume.clamp(0.0, 1.0) * 1000.0).round() as usize;
            unsafe {
                let _ = PostMessageW(Some(HWND(h as *mut _)), WM_VOLUME, WPARAM(permille), LPARAM(n.bMuted.as_bool() as isize));
            }
        }
        Ok(())
    }
}

thread_local! {
    /// Подписка на громкость устройства по умолчанию: (конечная точка, приёмник, id устройства).
    static VOL_WATCH: RefCell<Option<(IAudioEndpointVolume, IAudioEndpointVolumeCallback, String)>> =
        const { RefCell::new(None) };
}

/// Подписывается на изменения громкости устройства вывода по умолчанию (повторно — только если
/// устройство сменилось). Событийная подписка: в простое потоку делать нечего.
pub(super) unsafe fn watch_volume() {
    let Some(dev) = device() else { return };
    let id = device_id(&dev).unwrap_or_default();
    VOL_WATCH.with(|w| {
        let mut w = w.borrow_mut();
        if w.as_ref().is_some_and(|(_, _, old)| *old == id) {
            return;
        }
        if let Some((ep, cb, _)) = w.take() {
            let _ = ep.UnregisterControlChangeNotify(&cb);
        }
        let Ok(ep) = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) else { return };
        let cb: IAudioEndpointVolumeCallback = VolumeWatch.into();
        match ep.RegisterControlChangeNotify(&cb) {
            Ok(()) => *w = Some((ep, cb, id)),
            Err(e) => log::error(&format!("не удалось подписаться на громкость: {e}")),
        }
    });
}

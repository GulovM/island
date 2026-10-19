//! Переключение звукового устройства по умолчанию через IPolicyConfig.

use windows::core::{GUID, PCWSTR};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

use crate::log;

// Недокументированный, но давно стабильный интерфейс смены устройства по умолчанию
// (им пользуются SoundSwitch, nircmd и др.). Порядок методов — по vtable IPolicyConfig.
#[allow(non_snake_case, dead_code)]
mod policy {
    use windows::core::{HRESULT, PCWSTR};

    #[windows::core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    pub unsafe trait IPolicyConfig: windows::core::IUnknown {
        pub unsafe fn GetMixFormat(&self, device: PCWSTR, format: *mut *mut std::ffi::c_void) -> HRESULT;
        pub unsafe fn GetDeviceFormat(&self, device: PCWSTR, default: i32, format: *mut *mut std::ffi::c_void) -> HRESULT;
        pub unsafe fn ResetDeviceFormat(&self, device: PCWSTR) -> HRESULT;
        pub unsafe fn SetDeviceFormat(&self, device: PCWSTR, a: *mut std::ffi::c_void, b: *mut std::ffi::c_void) -> HRESULT;
        pub unsafe fn GetProcessingPeriod(&self, device: PCWSTR, default: i32, a: *mut i64, b: *mut i64) -> HRESULT;
        pub unsafe fn SetProcessingPeriod(&self, device: PCWSTR, a: *mut i64) -> HRESULT;
        pub unsafe fn GetShareMode(&self, device: PCWSTR, mode: *mut std::ffi::c_void) -> HRESULT;
        pub unsafe fn SetShareMode(&self, device: PCWSTR, mode: *mut std::ffi::c_void) -> HRESULT;
        pub unsafe fn GetPropertyValue(&self, device: PCWSTR, fx: i32, key: *const std::ffi::c_void, value: *mut std::ffi::c_void) -> HRESULT;
        pub unsafe fn SetPropertyValue(&self, device: PCWSTR, fx: i32, key: *const std::ffi::c_void, value: *mut std::ffi::c_void) -> HRESULT;
        pub unsafe fn SetDefaultEndpoint(&self, device: PCWSTR, role: i32) -> HRESULT;
        pub unsafe fn SetEndpointVisibility(&self, device: PCWSTR, visible: i32) -> HRESULT;
    }
}
use policy::IPolicyConfig;

pub(super) unsafe fn audio_set_default(id: &str) {
    const CLSID_POLICY_CONFIG: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
    let cfg = match CoCreateInstance::<_, IPolicyConfig>(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL) {
        Ok(c) => c,
        Err(e) => {
            log::error(&format!("не удалось получить IPolicyConfig: {e}"));
            return;
        }
    };
    let wide = crate::sys::wz(id);
    // eConsole, eMultimedia, eCommunications.
    for role in 0..3 {
        let _ = cfg.SetDefaultEndpoint(PCWSTR(wide.as_ptr()), role);
    }
}

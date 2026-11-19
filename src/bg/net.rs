//! Состояние сетевого подключения macOS (Ethernet / Wi-Fi).

use std::sync::Mutex;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetStatus {
    pub wifi_connected: bool,
    pub quality: u8,
    pub wired_up: bool,
}

static STATE: Mutex<NetStatus> = Mutex::new(NetStatus {
    wifi_connected: false,
    quality: 0,
    wired_up: false,
});

pub fn net_status() -> NetStatus {
    STATE.lock().map(|g| *g).unwrap_or_default()
}

pub(super) unsafe fn watch_network() {}

pub(super) unsafe fn refresh_net() {
    let wifi = super::wifi::wifi_status();
    let wired_up = has_ethernet_mac();
    let wifi_connected = wifi.as_ref().is_some_and(|w| w.radio_on && w.ssid.is_some());
    let next = NetStatus {
        wifi_connected,
        quality: if wifi_connected { 80 } else { 0 },
        wired_up,
    };
    if let Ok(mut g) = STATE.lock() {
        *g = next;
    }
    super::notify();
}

fn has_ethernet_mac() -> bool {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("/sbin/route")
            .args(["-n", "get", "default"])
            .output()
        {
            let s = String::from_utf8_lossy(&out.stdout);
            for line in s.lines() {
                if let Some(iface) = line.trim().strip_prefix("interface:") {
                    let iface = iface.trim();
                    if let Ok(hw) = std::process::Command::new("/usr/sbin/networksetup")
                        .args(["-listallhardwareports"])
                        .output()
                    {
                        let hws = String::from_utf8_lossy(&hw.stdout);
                        let mut prev_port = "";
                        for hl in hws.lines() {
                            if let Some(p) = hl.strip_prefix("Hardware Port:") {
                                prev_port = p.trim();
                            } else if hl.trim() == format!("Device: {iface}") {
                                return !prev_port.contains("Wi-Fi") && !prev_port.contains("AirPort");
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

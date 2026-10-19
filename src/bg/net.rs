//! Состояние сети: Wi-Fi подключение, качество сигнала, Ethernet.

use std::sync::Mutex;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIfTable2, NotifyIpInterfaceChange, MIB_IF_TABLE2, MIB_IPINTERFACE_ROW, MIB_NOTIFICATION_TYPE,
};
use windows::Win32::NetworkManagement::WiFi::*;
use windows::Win32::Networking::WinSock::AF_UNSPEC;

use super::{send, Cmd};
use crate::log;

/// Состояние сети для значка на свёрнутом острове.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetStatus {
    /// Подключены к сети Wi-Fi.
    pub wifi_connected: bool,
    /// Качество сигнала Wi-Fi, 0–100.
    pub quality: u8,
    /// Есть работающий проводной адаптер.
    pub wired_up: bool,
}

static NET: Mutex<NetStatus> = Mutex::new(NetStatus { wifi_connected: false, quality: 0, wired_up: false });

pub fn net_status() -> NetStatus {
    NET.lock().map(|n| *n).unwrap_or_default()
}

/// Подписка на смену сетевых интерфейсов (подключение, отключение, смена адреса): событие шлёт
/// `Cmd::Net`, поэтому значок Wi-Fi обновляется без опроса.
pub(super) unsafe fn watch_network() {
    unsafe extern "system" fn on_change(_ctx: *const std::ffi::c_void, _row: *const MIB_IPINTERFACE_ROW, _kind: MIB_NOTIFICATION_TYPE) {
        send(Cmd::Net);
    }
    // Хэндл подписки живёт до конца процесса.
    let mut handle = HANDLE::default();
    let r = NotifyIpInterfaceChange(AF_UNSPEC, Some(on_change), None, false, &mut handle);
    if r.0 != 0 {
        log::error(&format!("NotifyIpInterfaceChange вернул {}", r.0));
    }
}

/// Читает состояние Wi-Fi и проводной сети в общий снимок и будит главное окно, если оно изменилось.
pub(super) unsafe fn refresh_net() {
    let mut st = NetStatus::default();

    let (mut ver, mut h) = (0u32, HANDLE::default());
    if WlanOpenHandle(2, None, &mut ver, &mut h) == 0 {
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        if WlanEnumInterfaces(h, None, &mut list) == 0 && !list.is_null() {
            let items = std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), (*list).dwNumberOfItems as usize);
            for it in items {
                if it.isState != wlan_interface_state_connected {
                    continue;
                }
                let (mut size, mut data) = (0u32, std::ptr::null_mut::<std::ffi::c_void>());
                if WlanQueryInterface(h, &it.InterfaceGuid, wlan_intf_opcode_current_connection, None, &mut size, &mut data, None) == 0
                    && !data.is_null()
                {
                    let attr = &*(data as *const WLAN_CONNECTION_ATTRIBUTES);
                    st.wifi_connected = true;
                    st.quality = attr.wlanAssociationAttributes.wlanSignalQuality.min(100) as u8;
                    WlanFreeMemory(data);
                    break;
                }
            }
            WlanFreeMemory(list as *const std::ffi::c_void);
        }
        let _ = WlanCloseHandle(h, None);
    }

    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    if GetIfTable2(&mut table).is_ok() && !table.is_null() {
        let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        // Физический адаптер Ethernet (тип 6) в рабочем состоянии (OperStatusUp = 1).
        st.wired_up = rows.iter().any(|r| r.InterfaceAndOperStatusFlags._bitfield & 1 != 0 && r.Type == 6 && r.OperStatus.0 == 1);
        FreeMibTable(table as *const std::ffi::c_void);
    }

    if let Ok(mut n) = NET.lock() {
        *n = st;
    }
}

//! Win32 notification transport; no quota state or configuration lives here.

use super::{BalloonCommand, NotificationSink, wide_array};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_REALTIME, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIIF_NOSOUND,
    NIIF_RESPECT_QUIET_TIME, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICON_VERSION_4,
    NOTIFYICONDATAW, QUERY_USER_NOTIFICATION_STATE, QUNS_ACCEPTS_NOTIFICATIONS,
    SHQueryUserNotificationState, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{IDI_APPLICATION, KillTimer, LoadIconW, SetTimer};
use windows::core::PCWSTR;

const CLEANUP_BASE: usize = 0x10000;
const CLEANUP_MS: u32 = 300_000;

pub struct WindowsNotificationSink {
    pub hwnd: HWND,
    pub callback: u32,
    pub shown: fn(u16) -> bool,
}

fn cleanup_delay(shown: bool) -> u32 {
    if shown { CLEANUP_MS } else { 5_000 }
}

fn accepts_state(state: Option<QUERY_USER_NOTIFICATION_STATE>) -> bool {
    state == Some(QUNS_ACCEPTS_NOTIFICATIONS)
}

/// Timer identities cannot overlap the strip's regular timer IDs, e.g. icon7 != icon8.
pub fn cleanup_timer(id: u16) -> usize {
    CLEANUP_BASE + usize::from(id)
}

/// Returns the icon owning a cleanup timer, or None for ordinary strip timers.
pub fn timer_icon(timer: usize) -> Option<u16> {
    u16::try_from(timer.checked_sub(CLEANUP_BASE)?)
        .ok()
        .filter(|&id| id != 0)
}

/// Version4 callbacks encode icon ID in HIWORD and the notification in LOWORD.
pub fn decode_callback(lparam: LPARAM) -> (u16, u32) {
    ((lparam.0 as usize >> 16) as u16, (lparam.0 as u32) & 0xffff)
}

fn packet(
    hwnd: HWND,
    id: u16,
    callback: u32,
    command: BalloonCommand,
    title: &str,
    body: &str,
) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: u32::from(id),
        ..Default::default()
    };
    match command {
        BalloonCommand::Add => {
            data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
            data.uCallbackMessage = callback;
            data.szTip = wide_array("CCT");
        }
        BalloonCommand::Version => data.Anonymous.uVersion = NOTIFYICON_VERSION_4,
        BalloonCommand::Show => {
            data.uFlags = NIF_INFO | NIF_REALTIME;
            data.dwInfoFlags = NIIF_INFO | NIIF_RESPECT_QUIET_TIME | NIIF_NOSOUND;
            data.szInfoTitle = wide_array(title);
            data.szInfo = wide_array(body);
        }
        BalloonCommand::ArmCleanup => {}
    }
    data
}

impl NotificationSink for WindowsNotificationSink {
    fn accepts(&self) -> bool {
        // SAFETY: read-only shell query; an error conservatively suppresses delivery.
        accepts_state(unsafe { SHQueryUserNotificationState() }.ok())
    }

    fn send(&self, command: BalloonCommand, id: u16, title: &str, body: &str) -> bool {
        // SAFETY: hwnd belongs to this UI thread; buffers and shared icons outlive each call.
        unsafe {
            if command == BalloonCommand::ArmCleanup {
                return SetTimer(
                    Some(self.hwnd),
                    cleanup_timer(id),
                    cleanup_delay((self.shown)(id)),
                    None,
                ) != 0;
            }
            let mut data = packet(self.hwnd, id, self.callback, command, title, body);
            let message = match command {
                BalloonCommand::Add => {
                    // LoadIcon returns shared handles; they must not be destroyed by us.
                    data.hIcon = GetModuleHandleW(None)
                        .ok()
                        .and_then(|h| LoadIconW(Some(h.into()), PCWSTR(1usize as _)).ok())
                        .or_else(|| LoadIconW(None, IDI_APPLICATION).ok())
                        .unwrap_or_default();
                    NIM_ADD
                }
                BalloonCommand::Version => NIM_SETVERSION,
                BalloonCommand::Show => NIM_MODIFY,
                BalloonCommand::ArmCleanup => unreachable!(),
            };
            Shell_NotifyIconW(message, &data).as_bool()
        }
    }

    fn remove(&self, id: u16) {
        let data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: u32::from(id),
            ..Default::default()
        };
        // SAFETY: each ID identifies only its own icon and timer, including stale callbacks.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), cleanup_timer(id));
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Shell::{
        QUNS_APP, QUNS_BUSY, QUNS_NOT_PRESENT, QUNS_PRESENTATION_MODE, QUNS_QUIET_TIME,
        QUNS_RUNNING_D3D_FULL_SCREEN,
    };

    #[test]
    fn only_explicit_shell_permission_accepts_notifications() {
        assert!(accepts_state(Some(QUNS_ACCEPTS_NOTIFICATIONS)));
        for state in [
            QUNS_APP,
            QUNS_BUSY,
            QUNS_NOT_PRESENT,
            QUNS_PRESENTATION_MODE,
            QUNS_QUIET_TIME,
            QUNS_RUNNING_D3D_FULL_SCREEN,
            QUERY_USER_NOTIFICATION_STATE(-1),
        ] {
            assert!(!accepts_state(Some(state)), "{state:?}");
        }
        assert!(!accepts_state(None), "query failure is not permission");
    }

    #[test]
    fn native_packets_use_version_four_and_realtime_quiet_silent_payloads() {
        let hwnd = HWND(0x1234usize as _);
        let info = packet(hwnd, 7, 0x8004, BalloonCommand::Show, "Título", "Aviso 💡");
        assert_eq!(info.cbSize, size_of::<NOTIFYICONDATAW>() as u32);
        assert_eq!((info.hWnd, info.uID), (hwnd, 7));
        assert_eq!(info.uFlags, NIF_INFO | NIF_REALTIME);
        assert_eq!(
            info.dwInfoFlags,
            NIIF_INFO | NIIF_RESPECT_QUIET_TIME | NIIF_NOSOUND
        );
        assert_eq!(info.szInfoTitle, wide_array::<64>("Título"));
        assert_eq!(info.szInfo, wide_array::<256>("Aviso 💡"));
        assert_eq!(*info.szInfoTitle.last().unwrap(), 0);
        assert_eq!(*info.szInfo.last().unwrap(), 0);
        let add = packet(hwnd, 7, 0x8004, BalloonCommand::Add, "", "");
        assert_eq!(add.uFlags, NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP);
        assert_eq!(add.uCallbackMessage, 0x8004);
        assert_eq!(add.szTip, wide_array::<128>("CCT"));
        let version = packet(hwnd, 7, 0x8004, BalloonCommand::Version, "", "");
        // SAFETY: packet initialized the uVersion union member for this command.
        assert_eq!(unsafe { version.Anonymous.uVersion }, NOTIFYICON_VERSION_4);
    }

    #[test]
    fn callbacks_and_cleanup_timers_identify_their_own_balloon() {
        assert_eq!(cleanup_delay(false), 5_000);
        assert_eq!(cleanup_delay(true), 300_000);
        assert_eq!(
            decode_callback(LPARAM(((7usize << 16) | 1028) as isize)),
            (7, 1028)
        );
        assert_eq!(timer_icon(cleanup_timer(7)), Some(7));
        assert_eq!(timer_icon(cleanup_timer(8)), Some(8));
        assert_ne!(cleanup_timer(7), cleanup_timer(8));
        for unrelated in [0, 1, 2, 3, 4, 5, CLEANUP_BASE, CLEANUP_BASE + 65536] {
            assert_eq!(timer_icon(unrelated), None);
        }
    }
}

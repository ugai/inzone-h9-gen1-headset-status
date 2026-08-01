//! System notifications, sent through the tray icon the app already owns.

/// Shows a native notification through the tray icon we already own. Windows 10/11 turn a
/// Shell_NotifyIcon balloon into a real toast, so this needs no AppUserModelID, no Start
/// menu shortcut and no extra dependency.
///
/// tray-icon does not expose the uID it registered, and the value is not 1: the builder
/// burns one counter tick on the TrayIconId before the platform layer takes the next one
/// for the uID. Rather than hardcode today's answer, ask the shell — Shell_NotifyIcon
/// returns FALSE for an (hwnd, uID) pair it does not know, so the first one that succeeds
/// is ours. Self-correcting across tray-icon versions, and only a handful of cheap calls
/// a few times a day.
pub fn notify(hwnd: windows_sys::Win32::Foundation::HWND, body: &str) {
    use windows_sys::Win32::UI::Shell::{NIF_INFO, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW};

    fn wide<const N: usize>(dst: &mut [u16; N], s: &str) {
        for (slot, ch) in dst.iter_mut().zip(s.encode_utf16().take(N - 1)) {
            *slot = ch;
        }
    }

    for uid in 1..=8u32 {
        unsafe {
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = uid;
            nid.uFlags = NIF_INFO;
            wide(&mut nid.szInfoTitle, "INZONE H9 / H7");
            wide(&mut nid.szInfo, body);
            if Shell_NotifyIconW(NIM_MODIFY, &nid) != 0 {
                return;
            }
        }
    }
}

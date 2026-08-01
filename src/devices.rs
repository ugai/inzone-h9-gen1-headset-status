//! Noticing the moment the dongle's COM port appears or disappears.
//!
//! Without this the poller runs on its own clock: 15 seconds normally, and 30, 60 or 120
//! after repeated failures, so replugging the receiver could take two minutes to show in
//! the tray. Waking from sleep re-enumerates the device, so it arrives through the same
//! path.

use std::sync::mpsc::Sender;
use std::sync::{Mutex, PoisonError};

use windows_sys::core::GUID;

/// `GUID_DEVINTERFACE_COMPORT`, the device interface class every serial port is published
/// under. windows-sys does not export it, and the value is stable since Windows 2000.
const GUID_DEVINTERFACE_COMPORT: GUID = GUID::from_u128(0x86e0d1e0_8089_11d0_9ce4_08003e301f73);

/// Windows runs the notification callback on a thread of its own, so the only way to hand
/// it the channel is a static. A `Mutex` because nothing promises the callback is never
/// entered twice at once, and an `Option` so a failed registration can put it back to
/// empty rather than leaving a sender nothing will ever send on.
static WAKE: Mutex<Option<Sender<()>>> = Mutex::new(None);

/// The only thing under the lock is a `Sender`, and there is no invariant across it that a
/// panic could break, so a poisoned lock is worth taking anyway. The alternative is a
/// poller that silently stops waking early for the rest of the session.
fn wake_slot() -> std::sync::MutexGuard<'static, Option<Sender<()>>> {
    WAKE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Asks Windows to tell us when a serial port arrives or goes away, and reports whether it
/// agreed to. On failure the poller simply keeps its own clock, which is what it did before,
/// and the slot is left empty so a later attempt is not mistaken for one already made.
///
/// `CM_Register_Notification` rather than `WM_DEVICECHANGE`: the window message is *sent*
/// rather than posted, so it never surfaces in a `GetMessageW` loop, and we would have had
/// to own a window with a real wndproc just to see it. This asks for the same interface
/// class arrivals and delivers them to a callback instead.
///
/// Calling this twice replaces the sender and adds a second registration rather than
/// failing; the app calls it once. The registration handle is deliberately dropped: it is
/// wanted for the whole life of the process, and unregistering from inside the callback's
/// own thread is a deadlock.
pub fn wake_on_serial_port_change(wake: Sender<()>) -> bool {
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_NOTIFY_ACTION, CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL, CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL,
        CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER, CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, CM_Register_Notification,
        CR_SUCCESS, HCMNOTIFICATION,
    };

    unsafe extern "system" fn on_change(
        _notify: HCMNOTIFICATION,
        _context: *const core::ffi::c_void,
        action: CM_NOTIFY_ACTION,
        _data: *const CM_NOTIFY_EVENT_DATA,
        _size: u32,
    ) -> u32 {
        // Windows also sends query/cancel actions for removals it is still negotiating.
        // Only the settled ones are news, and the poller is what decides what they mean.
        let settled = matches!(
            action,
            CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL | CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL
        );
        if settled && let Some(wake) = wake_slot().as_ref() {
            // A poller that has gone away is not this callback's problem to report.
            let _ = wake.send(());
        }
        // The documented "carry on" return. A callback that fails is not retried.
        0
    }

    // Published before registering, because the callback can fire the moment the
    // registration call returns.
    *wake_slot() = Some(wake);

    let mut filter: CM_NOTIFY_FILTER = unsafe { std::mem::zeroed() };
    filter.cbSize = std::mem::size_of::<CM_NOTIFY_FILTER>() as u32;
    filter.FilterType = CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE;
    filter.u.DeviceInterface.ClassGuid = GUID_DEVINTERFACE_COMPORT;

    let mut handle: HCMNOTIFICATION = std::ptr::null_mut();
    let ok = unsafe { CM_Register_Notification(&filter, std::ptr::null(), Some(on_change), &mut handle) };
    if ok != CR_SUCCESS {
        *wake_slot() = None;
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The filter has to be one cfgmgr32 will actually take. A wrong `cbSize` or filter
    /// type is a runtime `CONFIGRET`, not a compile error, and the only symptom in the app
    /// would be a poller that quietly never wakes early: the tray would still be right,
    /// just up to two minutes late, which is the bug this whole module exists to fix.
    #[test]
    fn windows_accepts_the_serial_port_filter() {
        let (tx, _rx) = std::sync::mpsc::channel();
        assert!(
            wake_on_serial_port_change(tx),
            "CM_Register_Notification refused the filter"
        );
        assert!(wake_slot().is_some(), "a successful registration must leave a sender");
        // Registering again has to work rather than latch: a second call replaces the
        // sender, and a caller retrying after a transient failure must not be turned away
        // by state left behind from the first attempt.
        let (tx, _rx) = std::sync::mpsc::channel();
        assert!(wake_on_serial_port_change(tx), "the second registration must also take");
    }
}

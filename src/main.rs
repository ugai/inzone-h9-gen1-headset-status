#![windows_subsystem = "windows"]

//! Tray indicator for the Sony INZONE H9 / H7 (first generation) headset power state.
//!
//! The dongle (USB 054C:0E53) speaks a Sony HCI-framed protocol over its USB CDC
//! serial interface. `protocol` has the frame layout.

mod cli;
mod devices;
mod icon;
mod notify;
mod protocol;
mod settings;
mod startup;
mod state;
mod text;
mod transport;

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::Duration;

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use crate::icon::make_icon;
use crate::notify::notify;
use crate::settings::Settings;
use crate::state::{Badge, BatteryWatch, Cause, State, notification_for};
use crate::text::Lang;
use crate::transport::poll;

const POLL_INTERVAL: Duration = Duration::from_secs(15);
/// Consecutive failures stretch the interval, so a wedged or absent dongle is not
/// hammered with an open/DTR/write cycle every 15 s forever.
const POLL_BACKOFF_MAX: Duration = Duration::from_secs(120);
/// How long to let a device change settle before believing it. A single replug publishes
/// several interfaces within a second, and the COM port is not openable the instant its
/// arrival is announced. Everything that lands during the wait is folded into one poll.
const DEVICE_SETTLE: Duration = Duration::from_millis(700);
/// Where a newer build would be. Nothing here fetches it: the menu item hands the address to
/// the shell and the user's browser is what connects, which is what lets README say the app
/// itself never touches the network. A check that read this page would have to change that
/// sentence before it changed any code.
const RELEASES_URL: &str = "https://github.com/ugai/inzone-h9-gen1-headset-status/releases";

/// Whether this process should go on to put an icon in the tray.
///
/// It tries to take the name that says "an instance of this app is running in this
/// session". True means either that we took it, or that we could not ask.
///
/// Two instances mean two tray icons and two pollers opening the same COM port, which
/// doubles the window in which INZONE Hub can fail to acquire it. Windows keeps a named
/// mutex alive for as long as any handle to it is open, so the handle is deliberately
/// leaked: the process holds it until it exits, and the kernel cleans up after that.
///
/// `Local\` rather than `Global\`: the tray is per session, so one instance per session is
/// the right granularity, and it needs no privilege that a plain user might not have.
fn claim_single_instance() -> bool {
    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let name: Vec<u16> = "Local\\inzone-h9-gen1-headset-status\0".encode_utf16().collect();
    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 1, name.as_ptr());
        // GetLastError has to be read before anything else can overwrite it. It is set to
        // ERROR_ALREADY_EXISTS even on the success path, which is the whole signal here.
        let err = GetLastError();
        // Not being able to ask is not evidence of a second instance. A duplicate tray icon
        // is a smaller failure than an app that refuses to start.
        handle.is_null() || err != ERROR_ALREADY_EXISTS
    }
}

/// Opens a URL with whatever the user has set as their browser, and reports whether the
/// shell took it. A menu item that silently does nothing is the same failure as a tick that
/// never reached the registry, and that one is already reported rather than left to be
/// guessed at.
///
/// `ShellExecuteW` answers above 32 on success and with an error code at or below it, which
/// is the one place in Win32 where an `HINSTANCE` is not a handle.
fn open_url(url: &str) -> bool {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let target: Vec<u16> = url.encode_utf16().chain([0]).collect();
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        ) as isize
            > 32
    }
}

/// Polls on a clock, and again straight away whenever `wake` says the device set changed.
fn spawn_poller(wake: Receiver<()>) -> Receiver<State> {
    let (tx, rx): (Sender<State>, Receiver<State>) = mpsc::channel();
    std::thread::spawn(move || {
        let mut tid: u16 = 1;
        let mut wait = POLL_INTERVAL;
        loop {
            let state = poll(tid);
            let healthy = !matches!(state, State::Error(_) | State::NoDongle { .. });
            if tx.send(state).is_err() {
                return;
            }
            wait = if healthy {
                POLL_INTERVAL
            } else {
                (wait * 2).min(POLL_BACKOFF_MAX)
            };
            // A poll spends three transaction ids (link, battery, Bluetooth); stepping by
            // four keeps the next round clear of them.
            tid = tid.wrapping_add(4);
            match wake.recv_timeout(wait) {
                Ok(()) => {
                    std::thread::sleep(DEVICE_SETTLE);
                    while wake.try_recv().is_ok() {}
                    // The device set changed, so whatever the backoff was measuring is no
                    // longer the situation. A receiver that just came back must not wait
                    // two minutes to be noticed.
                    wait = POLL_INTERVAL;
                }
                Err(RecvTimeoutError::Timeout) => {}
                // Nobody left to wake us. The clock still works, so keep polling.
                Err(RecvTimeoutError::Disconnected) => std::thread::sleep(wait),
            }
        }
    });
    rx
}

fn main() {
    // Before anything reads SM_CXSMICON, and before parse_args can put a MessageBox on
    // screen, so --help from Explorer is drawn sharp too.
    let dpi_aware = icon::declare_dpi_aware();
    // Settled before anything speaks, so --help comes out in the right language. The
    // settings file is read first for the same reason: its language= overrides Windows,
    // and both have to be known before the first word.
    let mut settings = Settings::load();
    let lang = settings.language.unwrap_or_else(Lang::from_windows);
    let args = cli::parse_args(lang);

    if !claim_single_instance() {
        cli::emit(text::already_running(lang));
        std::process::exit(cli::EXIT_ALREADY_RUNNING);
    }

    let (wake_tx, wake_rx) = mpsc::channel();
    let rx = spawn_poller(wake_rx);
    // Losing this only costs the early poll; the clock in spawn_poller still runs.
    devices::wake_on_serial_port_change(wake_tx);

    let status_item = MenuItem::new(text::menu_starting(lang), false, None);
    let power_item = CheckMenuItem::new(text::menu_notify_power(lang), true, settings.notify_power, None);
    let battery_item = CheckMenuItem::new(text::menu_notify_battery(lang), true, settings.notify_battery, None);
    // Read from the registry rather than from settings.conf: the Run key is the real
    // record, and the user can have changed it from Settings since we last ran.
    let startup_item = CheckMenuItem::new(text::menu_run_at_logon(lang), true, startup::is_enabled(), None);
    // Disabled, so it reads as a label rather than something to click. It names the app as
    // well as the version: the menu is the one place someone looks at this without having
    // typed its name, and "Version 0.1.0" alone does not say whose. The tooltip is left
    // alone, since the version number is not what anyone opened it for.
    let version_item = MenuItem::new(cli::VERSION_LINE, false, None);
    // Directly under the version, which is the number it is there to be compared against.
    let updates_item = MenuItem::new(text::menu_check_for_updates(lang), true, None);
    let quit_item = MenuItem::new(text::menu_quit(lang), true, None);
    let menu = Menu::new();
    menu.append_items(&[
        &status_item,
        &PredefinedMenuItem::separator(),
        &power_item,
        &battery_item,
        &startup_item,
        &PredefinedMenuItem::separator(),
        &version_item,
        &updates_item,
        &quit_item,
    ])
    .unwrap();

    let mut icon_n = icon::tray_icon_size(dpi_aware);
    let tray: TrayIcon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(text::tip_starting(lang))
        .with_icon(make_icon(icon_n, [0x6E, 0x6E, 0x78], true, Badge::None, None))
        .build()
        .expect("tray icon");

    let quit_id = quit_item.id().clone();
    let power_id = power_item.id().clone();
    let battery_id = battery_item.id().clone();
    let startup_id = startup_item.id().clone();
    let updates_id = updates_item.id().clone();
    // What the user has been told, which is what decides whether a reading is news and what
    // `notification_for` compares against. It is updated whether or not the shell accepted
    // the drawing, and must stay that way: making it conditional would leave the next
    // identical reading looking like a fresh transition and announce it a second time.
    let mut shown: Option<State> = None;
    // Whether the shell took the last drawing. Separate from `shown` because it answers a
    // different question, and because a state skipped for being unchanged would otherwise
    // leave a failed drawing on screen until the headset itself changed.
    let mut painted = false;
    let mut battery_watch = BatteryWatch::default();

    // Reports whether the shell took it. Both calls are made either way: a failed icon must
    // not cost the tooltip, which is the more informative of the two.
    let show = |state: &State, n: i32| -> bool {
        let icon = tray
            .set_icon(Some(make_icon(
                n,
                state.color(),
                state.hollow(),
                state.badge(),
                state.battery_arc(),
            )))
            .is_ok();
        let tip = tray.set_tooltip(Some(state.tooltip(lang))).is_ok();
        status_item.set_text(state.tooltip(lang));
        icon && tip
    };

    if args.test_toast {
        notify(tray.window_handle(), text::notify_test(lang));
    }

    // Bare Win32 message loop. A 500 ms timer is the wake-up we use to drain the poller
    // channel; the tray icon itself needs the pump regardless.
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, MSG, SetTimer, TranslateMessage, WM_TIMER,
        };
        SetTimer(std::ptr::null_mut(), 1, 500, None);
        let mut msg: MSG = std::mem::zeroed();
        // GetMessageW returns -1 on error, where msg holds nothing worth dispatching.
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);

            // We own no window, so WM_DPICHANGED goes to tray-icon's hidden one and never
            // reaches here. The timer tick is the cheapest place to notice instead:
            // GetSystemMetrics reads a shared memory page, and twice a second is nothing.
            if msg.message == WM_TIMER {
                let n = icon::tray_icon_size(dpi_aware);
                let resized = n != icon_n;
                icon_n = n;
                // The retry is the other half: Shell_NotifyIcon can refuse while Explorer is
                // restarting, and the reading that failed will not be sent again if the
                // headset stays as it is. Without this the tray would keep showing whatever
                // was there before, which is the stale-reading lie in a different costume.
                if let Some(state) = &shown
                    && (resized || !painted)
                {
                    painted = show(state, icon_n);
                }
            }

            loop {
                let state = match rx.try_recv() {
                    Ok(state) => state,
                    Err(TryRecvError::Empty) => break,
                    // The poller is gone, so every reading from here on is stale. Freezing
                    // on the last confident state would keep claiming the headset is on
                    // long after it was switched off, which is the one thing this app is
                    // built not to do. A closed channel answers the same way forever, so
                    // the drain has to stop once that has been said: it is the one arm the
                    // `continue` below would otherwise spin on.
                    Err(TryRecvError::Disconnected) => {
                        let stopped = State::Error(Cause::PollerStopped);
                        if shown.as_ref() == Some(&stopped) {
                            break;
                        }
                        stopped
                    }
                };
                // A reading identical to what is on screen is not news, but the ones queued
                // behind it may be. Ending the drain here left them until the next timer
                // tick, one per tick, so a UI that had been blocked took half a second per
                // stale reading to catch up with the headset.
                if shown.as_ref() == Some(&state) {
                    continue;
                }
                // The watch is fed every reading whether or not the warning is wanted, so
                // turning it back on does not fire for a fall that happened while it was off.
                let low = battery_watch.observe(lang, &state);
                if let Some(body) = notification_for(lang, shown.as_ref(), &state)
                    && settings.notify_power
                {
                    notify(tray.window_handle(), &body);
                }
                if let Some(body) = low
                    && settings.notify_battery
                {
                    notify(tray.window_handle(), &body);
                }
                painted = show(&state, icon_n);
                shown = Some(state);
            }
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id == quit_id {
                    return;
                }
                if event.id == startup_id {
                    let wanted = startup_item.is_checked();
                    if !startup::set_enabled(wanted) {
                        // The tick was already drawn, so put it back rather than leave the
                        // menu claiming something the registry never accepted.
                        startup_item.set_checked(!wanted);
                        notify(tray.window_handle(), text::notify_startup_failed(lang));
                    }
                    continue;
                }
                if event.id == updates_id {
                    if !open_url(RELEASES_URL) {
                        notify(tray.window_handle(), text::notify_browser_failed(lang));
                    }
                    continue;
                }
                if event.id == power_id {
                    settings.notify_power = power_item.is_checked();
                } else if event.id == battery_id {
                    settings.notify_battery = battery_item.is_checked();
                } else {
                    continue;
                }
                // A toggle that cannot be written down still holds for this session, and
                // saying so beats letting it quietly revert at the next launch.
                if !settings.save(lang) {
                    notify(tray.window_handle(), text::notify_settings_unsaved(lang));
                }
            }
        }
    }
}

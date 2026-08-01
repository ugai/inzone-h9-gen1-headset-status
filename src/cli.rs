//! Command line handling, and getting text out of a GUI-subsystem process.

use crate::text::{self, Lang};

/// A second instance found the first one already running and stood down. Distinct from 0
/// so a script that launches the app can tell "started" from "was already up", and from
/// the 2 that an unusable command line returns.
pub const EXIT_ALREADY_RUNNING: i32 = 3;

/// What `--version` prints, and what the tray menu shows.
///
/// The same string in both, so the two cannot drift. It carries the program's name because
/// the menu is the one place the app is looked at without anyone having typed its name: a
/// line reading only "Version 0.1.0" does not say whose version it is.
///
/// The name is not translated. It is the name of the executable, of the folder under
/// `%APPDATA%`, and of the `Run` value, and one thing should have one name. Both halves are
/// read from `Cargo.toml`, so this line cannot fall behind a rename.
pub const VERSION_LINE: &str = concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION"));
/// What the command line asked for. Anything that only prints has already exited by the
/// time this is returned.
#[derive(Default)]
pub struct Args {
    /// Fire one notification at startup, to check that the notification path works.
    pub test_toast: bool,
}

/// Prints to the parent console when there is one, and falls back to a dialog so that
/// `--help` from Explorer is not silently swallowed. A GUI-subsystem process has no
/// console of its own, but it does inherit a redirected stdout.
pub fn emit(text: &str) {
    use std::io::Write;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_OK, MessageBoxW};

    unsafe {
        // GetStdHandle has two ways of not giving us one, and they are different values:
        // null when the process has no standard handles, INVALID_HANDLE_VALUE when the call
        // itself failed. Only a real handle means "somebody redirected our output". Reading
        // the failure as one wrote the text into it, dropped it, and returned before the
        // dialog below could show it, which is the whole point of this function.
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        let redirected = !out.is_null() && out != INVALID_HANDLE_VALUE;
        if redirected || AttachConsole(ATTACH_PARENT_PROCESS) != 0 {
            let _ = writeln!(std::io::stdout(), "{text}");
            return;
        }
        let body: Vec<u16> = text.encode_utf16().chain([0]).collect();
        let title: Vec<u16> = "inzone-h9-gen1-headset-status".encode_utf16().chain([0]).collect();
        MessageBoxW(std::ptr::null_mut(), body.as_ptr(), title.as_ptr(), MB_OK);
    }
}

/// Reads the command line, or exits after printing for the options that only print.
pub fn parse_args(lang: Lang) -> Args {
    let mut args = Args::default();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                emit(text::help(lang));
                std::process::exit(0);
            }
            "-V" | "--version" => {
                emit(VERSION_LINE);
                std::process::exit(0);
            }
            "--test-toast" => args.test_toast = true,
            other => {
                emit(&format!(
                    "{}\n\n{}",
                    text::unknown_option(lang, other),
                    text::help(lang)
                ));
                std::process::exit(2);
            }
        }
    }
    args
}

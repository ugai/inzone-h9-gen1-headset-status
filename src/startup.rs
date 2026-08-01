//! Starting with Windows, through the `Run` key under `HKCU`.
//!
//! The registry rather than a shortcut in the Startup folder or an installer, because the
//! `Run` key is what Settings > Apps > Startup reads: the entry shows up there as a toggle
//! the user already knows how to find, and the app stays a single exe that runs from
//! wherever it happens to sit.
//!
//! Note that Settings can switch the entry off without deleting it. Windows records that
//! refusal somewhere else, so `is_enabled` below reports what we wrote, not whether Windows
//! will honor it. Reporting the second would mean reading a key Microsoft does not
//! document, and getting it wrong would be worse than the gap.
//!
//! `is_enabled` compares the stored command against this exe's own path, which is the right
//! question for a program the user drops wherever they like. It would read as off after
//! every update of a program that installs itself under a versioned directory, and the
//! repair for that belongs to whatever does the updating, not here: an entry naming the
//! previous version's path launches nothing whatever this function reports, so loosening
//! the comparison would go back to hiding a startup that is already broken. Point the entry
//! at a path that survives the update, or rewrite it as part of the update.

use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW,
    RegDeleteValueW, RegQueryValueExW, RegSetValueExW,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// The value name is what Settings shows in its list, so it is the program's own name.
const VALUE_NAME: &str = "inzone-h9-gen1-headset-status";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Opens the `Run` key, creating it if a bare profile is missing it. Returns `None` when
/// the registry refuses, which is the same answer as "we cannot manage startup".
fn open_run_key(access: u32) -> Option<HKEY> {
    let mut key: HKEY = std::ptr::null_mut();
    let err = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            0,
            std::ptr::null(),
            0,
            access,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    (err == ERROR_SUCCESS).then_some(key)
}

/// Whether we have written a startup entry. See the module note: Windows can still be
/// ignoring it, and this does not claim otherwise.
pub fn is_enabled() -> bool {
    is_enabled_for(VALUE_NAME)
}

/// Writes or removes the entry, and reports whether the registry agreed.
pub fn set_enabled(on: bool) -> bool {
    set_enabled_for(VALUE_NAME, on)
}

/// The command the entry has to hold for this exe: the path in quotes, nul terminated.
///
/// Built from the `OsStr`'s own wide form, not through a Rust `String`. Windows paths are
/// UTF-16 and need not be valid Unicode, and `Path::display` would replace what it could
/// not decode, leaving a `Run` entry pointing at a file that is not there.
///
/// The quotes are not decoration. Windows splits an unquoted `Run` command on spaces, so an
/// exe under `C:\Program Files\...` would be launched as its first word plus arguments.
fn command_for_this_exe() -> Option<Vec<u16>> {
    let exe = std::env::current_exe().ok()?;
    let mut command: Vec<u16> = Vec::with_capacity(exe.as_os_str().len() + 3);
    command.push(u16::from(b'"'));
    command.extend(exe.as_os_str().encode_wide());
    command.push(u16::from(b'"'));
    command.push(0);
    Some(command)
}

/// Longest value worth reading, in bytes. A `Run` command is a path and perhaps a few
/// arguments, and Windows caps a path at 32767 wide characters, so this is twice what one
/// can hold.
///
/// The size that decides the allocation below comes from the registry, which is outside
/// this program the same way a length byte off the wire is. `panic = "abort"` makes a
/// failed allocation the end of the process and leaves a tray icon nothing is behind, so
/// the number is bounded before it is believed rather than after. Same reasoning as
/// `decode_ret` refusing to index on a length it was handed.
const VALUE_MAX_BYTES: u32 = 128 * 1024;

/// Reads a `Run` value back as the UTF-16 it really is. `None` covers "no such value", "the
/// registry would not say", "too big to be a command" and "not the kind of value we write",
/// which are the same answer to every caller here: not a value naming this exe.
///
/// The type is asked for and checked, because a registry value is bytes plus a label saying
/// what they mean, and reading `REG_BINARY` as if it were a string would let the menu tick
/// itself for an entry Windows never launches anything from. An odd byte count is refused
/// for the same reason: half a UTF-16 unit is not a string we were handed, and
/// `chunks_exact` would drop it without saying so.
fn read_value(value_name: &str) -> Option<Vec<u16>> {
    let key = open_run_key(KEY_READ)?;
    let name = wide(value_name);
    let mut size = 0u32;
    let mut kind: REG_VALUE_TYPE = 0;
    let read = |buf: *mut u8, size: &mut u32, kind: &mut REG_VALUE_TYPE| unsafe {
        RegQueryValueExW(key, name.as_ptr(), std::ptr::null(), kind, buf, size)
    };
    // A size query with a null buffer is the documented way to ask how much to allocate.
    if read(std::ptr::null_mut(), &mut size, &mut kind) != ERROR_SUCCESS
        || kind != REG_SZ
        || size > VALUE_MAX_BYTES
        || !size.is_multiple_of(2)
    {
        unsafe { RegCloseKey(key) };
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    let err = read(buf.as_mut_ptr(), &mut size, &mut kind);
    unsafe { RegCloseKey(key) };
    // The second call reports both again, and a value rewritten between the two would come
    // back with the type or the length it has now rather than the one we allocated for.
    (err == ERROR_SUCCESS && kind == REG_SZ && size as usize <= buf.len() && size.is_multiple_of(2)).then(|| {
        buf.truncate(size as usize);
        buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
    })
}

/// The value name is a parameter so the test can exercise the real registry without
/// touching the entry the person running it actually depends on.
///
/// The value has to name *this* exe, not merely exist. A menu tick is a claim that the
/// program you are looking at starts with Windows, and an entry left behind by a copy that
/// has since been moved or deleted would make that claim while nothing started. Ticking the
/// item then rewrites the entry and makes it true again, which is the repair a menu that
/// showed a confident tick would never have offered.
fn is_enabled_for(value_name: &str) -> bool {
    let (Some(stored), Some(ours)) = (read_value(value_name), command_for_this_exe()) else {
        return false;
    };
    // Compared without case, because the paths Windows itself hands back for one file can
    // differ in it and the filesystem does not. Only ASCII is folded: that covers the drive
    // letter and everything else this has been seen to differ in, and a full Unicode fold
    // would be a table this app has no other use for.
    let fold = |u: &u16| match u {
        0x41..=0x5A => u + 0x20,
        _ => *u,
    };
    stored.len() == ours.len() && std::iter::zip(&stored, &ours).all(|(a, b)| fold(a) == fold(b))
}

/// The path is re-read on every enable rather than remembered, so moving the exe and
/// toggling again fixes the entry. `is_enabled_for` above is what makes that discoverable:
/// after a move the item reads as off, which is both true and an invitation to switch it
/// back on.
///
/// Switching off deletes whatever the value holds, this exe's path or another copy's. One
/// value name means one entry, and the caller asked for there not to be one.
fn set_enabled_for(value_name: &str, on: bool) -> bool {
    let Some(key) = open_run_key(KEY_WRITE) else {
        return false;
    };
    let name = wide(value_name);
    let err = if on {
        let Some(command) = command_for_this_exe() else {
            unsafe { RegCloseKey(key) };
            return false;
        };
        unsafe {
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr() as *const u8,
                // REG_SZ is measured in bytes and includes the terminating nul.
                (command.len() * 2) as u32,
            )
        }
    } else {
        let err = unsafe { RegDeleteValueW(key, name.as_ptr()) };
        // Deleting something that was never there is the state the caller asked for.
        if err == ERROR_FILE_NOT_FOUND {
            ERROR_SUCCESS
        } else {
            err
        }
    };
    unsafe { RegCloseKey(key) };
    err == ERROR_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a value the way a stale entry or another copy would leave one: the same name,
    /// a command that is not this exe's.
    /// Takes UTF-16 rather than a `&str`, because a Windows path need not be valid Unicode
    /// and the interesting cases here are built from a real one.
    fn write_foreign(value_name: &str, command: &[u16]) {
        let key = open_run_key(KEY_WRITE).expect("the Run key has to open");
        let name = wide(value_name);
        let err = unsafe {
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr() as *const u8,
                (command.len() * 2) as u32,
            )
        };
        unsafe { RegCloseKey(key) };
        assert_eq!(err, ERROR_SUCCESS, "the registry refused the write");
    }

    /// Writes raw bytes under a type of the test's choosing, which `set_enabled_for` cannot
    /// do: it only ever writes `REG_SZ`, and the wrong type is the thing being exercised.
    fn write_as(value_name: &str, kind: REG_VALUE_TYPE, bytes: &[u8]) {
        let key = open_run_key(KEY_WRITE).expect("the Run key has to open");
        let name = wide(value_name);
        let err = unsafe { RegSetValueExW(key, name.as_ptr(), 0, kind, bytes.as_ptr(), bytes.len() as u32) };
        unsafe { RegCloseKey(key) };
        assert_eq!(err, ERROR_SUCCESS, "the registry refused the write");
    }

    /// Takes the entry away again whichever way the test ends, so a failed assertion
    /// cannot leave the test binary registered to launch at every login.
    struct Scrub(String);
    impl Drop for Scrub {
        fn drop(&mut self) {
            set_enabled_for(&self.0, false);
        }
    }

    /// Writes a real entry under HKCU and takes it away again.
    ///
    /// Under its own value name, never the app's. Whoever runs the tests may well have the
    /// real entry enabled and pointing at an installed copy, and this test writes
    /// `current_exe()`, which here is the test binary: restoring "it was enabled" would
    /// have quietly repointed their startup at `target\debug\deps`.
    #[test]
    fn an_entry_can_be_written_and_taken_away() {
        let name = format!("{VALUE_NAME}-test-{}", std::process::id());
        let scrub = Scrub(name.clone());
        let name = &scrub.0;

        assert!(!is_enabled_for(name), "the test name must start out unused");
        assert!(set_enabled_for(name, true), "the registry refused the write");
        assert!(is_enabled_for(name), "a written entry has to read back as enabled");

        // The exe path in its own UTF-16, wrapped in quotes, nul terminated, and nothing
        // else. A path Windows cannot express in UTF-8 has to survive, the quotes have to
        // be there or Windows splits the command on spaces, and the byte count passed to
        // RegSetValueExW has to have covered the terminator.
        let exe = std::env::current_exe().expect("the test binary has a path");
        let mut want = vec![u16::from(b'"')];
        want.extend(exe.as_os_str().encode_wide());
        want.push(u16::from(b'"'));
        want.push(0);
        assert_eq!(
            read_value(name).as_deref(),
            Some(&want[..]),
            "the entry must be exactly the quoted exe path"
        );
        // Writing twice must not fail on the value already being there.
        assert!(set_enabled_for(name, true), "overwriting an existing entry has to work");
        assert!(is_enabled_for(name));

        assert!(set_enabled_for(name, false), "the registry refused the delete");
        assert!(!is_enabled_for(name), "a removed entry has to read back as disabled");
        // And removing what is already gone is the state the caller asked for, not an error.
        assert!(set_enabled_for(name, false), "removing an absent entry must not fail");

        // The app's own entry is not this test's business, but reading it must not blow up.
        let _ = is_enabled();
    }

    /// A registry value is bytes plus a label saying what they mean, and only `REG_SZ` is a
    /// command Windows will launch. Reading the bytes without asking for the label let an
    /// entry written as `REG_BINARY` tick the menu while nothing started at logon.
    #[test]
    fn an_entry_of_the_wrong_type_is_not_a_startup_command() {
        use windows_sys::Win32::System::Registry::REG_BINARY;

        let name = format!("{VALUE_NAME}-type-{}", std::process::id());
        let scrub = Scrub(name.clone());
        let name = &scrub.0;

        // Our own command, byte for byte, so the type is the only thing that differs.
        let command = command_for_this_exe().expect("the test binary has a path");
        let bytes: Vec<u8> = command.iter().flat_map(|u| u.to_le_bytes()).collect();

        write_as(name, REG_BINARY, &bytes);
        assert!(read_value(name).is_none(), "REG_BINARY is not a string we were handed");
        assert!(
            !is_enabled_for(name),
            "the right bytes under the wrong type must not tick the menu"
        );

        // The same bytes under REG_SZ do read back, which is what makes the assertion above
        // about the type rather than about the bytes.
        write_as(name, REG_SZ, &bytes);
        assert_eq!(read_value(name).as_deref(), Some(&command[..]));
        assert!(is_enabled_for(name));

        // Half a UTF-16 unit at the end is not a string either, and `chunks_exact` would
        // drop it without saying so.
        write_as(name, REG_SZ, &bytes[..bytes.len() - 1]);
        assert!(read_value(name).is_none(), "an odd byte count is not UTF-16");
        assert!(!is_enabled_for(name));
    }

    /// An entry naming some other exe is not this one starting with Windows.
    ///
    /// The tick claims that the program whose menu you have open starts at logon. A copy
    /// that has since been moved or deleted leaves its `Run` value behind under the same
    /// name, and reading only the value's presence kept the tick on while nothing started.
    /// Reading as off is the true answer and is also the repair: switching it back on
    /// rewrites the entry to this exe.
    #[test]
    fn an_entry_naming_another_exe_is_not_this_one_enabled() {
        let name = format!("{VALUE_NAME}-test-elsewhere-{}", std::process::id());
        let scrub = Scrub(name.clone());
        let name = &scrub.0;

        let elsewhere = wide(r#""C:\nowhere\inzone-h9-gen1-headset-status.exe""#);
        write_foreign(name, &elsewhere);
        assert_eq!(
            read_value(name).as_deref(),
            Some(&elsewhere[..]),
            "the entry has to be there for its absence not to be what the next line proves"
        );
        assert!(
            !is_enabled_for(name),
            "an entry naming another exe must not read as enabled"
        );

        assert!(set_enabled_for(name, true), "switching it on has to repair the entry");
        assert!(is_enabled_for(name), "and then it names this exe");

        // The same length as ours, so the comparison has to reach the characters. The path
        // above is a different length, which let the length check alone carry the test: with
        // the two joined by `or` instead of `and`, an entry naming any other exe of the same
        // length read as enabled and nothing here noticed.
        let ours = command_for_this_exe().expect("the test binary has a path");
        let mut same_length = ours.clone();
        // The last character before the closing quote and the nul, so the result is still a
        // quoted path and differs from ours only in what it names.
        let last = same_length.len() - 3;
        same_length[last] = if same_length[last] == u16::from(b'x') {
            u16::from(b'y')
        } else {
            u16::from(b'x')
        };
        assert_eq!(same_length.len(), ours.len());
        assert_ne!(same_length, ours);
        write_foreign(name, &same_length);
        assert!(
            !is_enabled_for(name),
            "another exe whose path is the same length must not read as enabled"
        );
    }

    /// A value too big to be a command is refused, not allocated.
    ///
    /// The size comes from the registry, and under `panic = "abort"` a failed allocation
    /// ends the process and leaves a tray icon with nothing behind it. Opening a menu must
    /// not be able to do that, whatever some other program has written under our name.
    #[test]
    fn an_oversized_value_is_refused_rather_than_read() {
        let name = format!("{VALUE_NAME}-test-oversized-{}", std::process::id());
        let scrub = Scrub(name.clone());
        let name = &scrub.0;

        // One wide character past the cap, so it is the cap being tested and not a round
        // number that happens to sit near it.
        let huge = vec![u16::from(b'x'); VALUE_MAX_BYTES as usize / 2 + 1];
        write_foreign(name, &huge);
        assert!(read_value(name).is_none(), "a value past the cap must not be read");
        assert!(!is_enabled_for(name), "and it is certainly not this exe");
    }

    /// Case is not what tells two Windows paths apart, and the filesystem agrees.
    ///
    /// `current_exe()` goes through `GetModuleFileNameW`, which has been seen to differ in
    /// the drive letter's case from the path a value was written with. Folding only ASCII
    /// is the deliberate limit: it covers the drive letter, and a full Unicode fold would
    /// be a table this app has no other use for.
    #[test]
    fn the_same_path_in_another_case_is_still_this_exe() {
        let name = format!("{VALUE_NAME}-test-case-{}", std::process::id());
        let scrub = Scrub(name.clone());
        let name = &scrub.0;

        assert!(set_enabled_for(name, true), "the registry refused the write");
        let shouted: Vec<u16> = read_value(name)
            .expect("just written")
            .iter()
            .map(|u| match u {
                0x61..=0x7A => u - 0x20,
                _ => *u,
            })
            .collect();
        write_foreign(name, &shouted);
        assert!(
            is_enabled_for(name),
            "the same path in another case still names this exe"
        );
    }
}

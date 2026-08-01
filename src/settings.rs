//! The handful of preferences that have to outlive the process.
//!
//! A plain text file under `%APPDATA%`, not the registry: the file is readable and
//! deletable by hand, and it keeps the single-exe, runs-from-anywhere property that an
//! installer would cost. `key=value`, one line each, because it needs no parser crate and
//! stays readable when someone opens it to see what the app remembered.

use std::path::{Path, PathBuf};

use crate::text::{self, Lang};

/// What the tray menu can turn on and off. Both default to on, which is what the app did
/// before there was anything to turn off.
#[derive(PartialEq, Clone, Copy, Debug)]
pub struct Settings {
    /// Announce the headset being switched on or off.
    pub notify_power: bool,
    /// Announce the battery falling past a threshold.
    pub notify_battery: bool,
    /// Which language the interface speaks, or `None` to follow Windows. Not on the menu:
    /// it is the one preference whose wrong value makes the menu hard to read, so it is
    /// set by editing the file, and read once at startup.
    pub language: Option<Lang>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            notify_power: true,
            notify_battery: true,
            language: None,
        }
    }
}

/// `%APPDATA%\inzone-h9-gen1-headset-status\settings.conf`, or `None` when Windows does not tell
/// us where roaming application data lives. Nothing else in the app depends on the file, so
/// that case simply means the settings last for the session.
fn path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    if appdata.is_empty() {
        return None;
    }
    Some(
        PathBuf::from(appdata)
            .join("inzone-h9-gen1-headset-status")
            .join("settings.conf"),
    )
}

impl Settings {
    pub fn load() -> Settings {
        path().and_then(|p| Settings::read_from(&p)).unwrap_or_default()
    }

    /// Writes the file, and reports whether it got there. The caller says so out loud
    /// rather than letting a toggle quietly forget itself between sessions.
    pub fn save(&self, lang: Lang) -> bool {
        path().is_some_and(|p| self.write_to(lang, &p))
    }

    /// Split out from `save` and `load` so the create-then-write path can be tested
    /// without depending on where `%APPDATA%` happens to point.
    fn write_to(&self, lang: Lang, path: &Path) -> bool {
        let Some(dir) = path.parent() else {
            return false;
        };
        // `create_dir` rather than `create_dir_all`: `%APPDATA%` is always there, so only
        // the one level below it can be missing, and the recursive version costs about a
        // kilobyte of binary for a case that cannot arise.
        match std::fs::create_dir(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return false,
        }
        let text = self.as_text(lang);
        // Written beside the file and moved over it, so a crash between the truncate and the
        // write cannot leave half a settings file behind. `%APPDATA%` is one volume, so the
        // move is a rename and not a copy.
        //
        // The fallback is not belt and braces. A process holding the file open with
        // `FILE_SHARE_READ | FILE_SHARE_WRITE` and not `FILE_SHARE_DELETE`, which is an
        // ordinary way to read a file, lets the plain write through and refuses the replace
        // with "access is denied" (measured, both halves). Atomicity without the second
        // attempt would turn saves that work today into a toast saying the setting was lost.
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, &text).is_ok() && std::fs::rename(&tmp, path).is_ok() {
            return true;
        }
        // README tells people to delete this directory by hand to uninstall, and a file the
        // docs do not mention is a file nobody knows to delete.
        let _ = std::fs::remove_file(&tmp);
        std::fs::write(path, &text).is_ok()
    }

    fn read_from(path: &Path) -> Option<Settings> {
        std::fs::read_to_string(path).ok().map(|text| Settings::parse(&text))
    }

    /// Anything the file does not say keeps its default, and anything it says that we do
    /// not understand is skipped. A settings file is not worth refusing to start over, and
    /// a half-written one must not turn a preference into its opposite.
    fn parse(text: &str) -> Settings {
        let mut settings = Settings::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            if key == "language" {
                // A value we cannot read leaves the preference where it was, the same rule
                // the on/off keys follow below: not understanding the file is not a
                // decision to change anything.
                settings.language = match value {
                    "auto" => None,
                    named => Lang::parse(named).or(settings.language),
                };
                continue;
            }
            let on = match value {
                "on" => true,
                "off" => false,
                _ => continue,
            };
            match key {
                "notify_power" => settings.notify_power = on,
                "notify_battery" => settings.notify_battery = on,
                _ => {}
            }
        }
        settings
    }

    fn as_text(&self, lang: Lang) -> String {
        let onoff = |b: bool| if b { "on" } else { "off" };
        format!(
            "{}\nnotify_power={}\nnotify_battery={}\nlanguage={}\n",
            text::settings_file_header(lang),
            onoff(self.notify_power),
            onoff(self.notify_battery),
            self.language.map_or("auto", Lang::as_str),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What we write has to be what we read back, or a toggle survives the session it was
    /// made in and then quietly reverts on the next launch.
    #[test]
    fn a_saved_setting_reads_back_the_same() {
        for notify_power in [true, false] {
            for notify_battery in [true, false] {
                for language in [None, Some(Lang::Ja), Some(Lang::En)] {
                    let want = Settings {
                        notify_power,
                        notify_battery,
                        language,
                    };
                    assert_eq!(Settings::parse(&want.as_text(Lang::Ja)), want);
                }
            }
        }
    }

    /// The round-trip above only proves the text is right. This one puts it through the
    /// filesystem, because the first save of a fresh install has to create the directory
    /// too, and a toggle that silently fails to persist is the failure worth catching.
    #[test]
    fn a_toggle_survives_a_write_and_a_read() {
        // One level below the temp directory, matching the one level below %APPDATA% that
        // is all `write_to` is allowed to create. The process id keeps two `cargo test`
        // runs on the same machine from deleting each other's directory mid-test.
        let dir = std::env::temp_dir().join(format!("inzone-h9-gen1-headset-status-settings-{}", std::process::id()));
        let file = dir.join("settings.conf");
        let _ = std::fs::remove_dir_all(&dir);

        let first = Settings {
            notify_power: false,
            notify_battery: true,
            language: Some(Lang::En),
        };
        assert!(
            first.write_to(Lang::Ja, &file),
            "the directory has to be created on first save"
        );
        assert_eq!(Settings::read_from(&file), Some(first));

        // And again, over a directory and a file that already exist.
        let second = Settings {
            notify_power: true,
            notify_battery: false,
            language: None,
        };
        assert!(
            second.write_to(Lang::Ja, &file),
            "saving over an existing file has to work"
        );
        assert_eq!(Settings::read_from(&file), Some(second));

        assert_eq!(Settings::read_from(&dir.join("absent.conf")), None);

        // The save goes through a file beside this one. Leaving it behind would put a file
        // in `%APPDATA%` that README does not mention, in the directory README tells people
        // to delete by hand.
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "the save must leave settings.conf and nothing else"
        );

        // And a reader holding the file must not cost the save. The share mode is spelled
        // out because it is the whole point: `File::open` asks for share-delete as well, and
        // under that the replace succeeds and this proves nothing. Share-read-write without
        // share-delete is what an ordinary reader takes, and it is the case where the replace
        // is refused with "access is denied" and the plain write is not.
        use std::os::windows::fs::OpenOptionsExt;
        // `FILE_SHARE_READ | FILE_SHARE_WRITE`, written out rather than turning on another
        // windows-sys feature for two numbers that have not moved since Win32 existed.
        const SHARE_READ_WRITE: u32 = 0x0000_0001 | 0x0000_0002;
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(SHARE_READ_WRITE)
            .open(&file)
            .expect("the settings file has to open");
        let third = Settings {
            notify_power: false,
            notify_battery: false,
            language: Some(Lang::Ja),
        };
        assert!(
            third.write_to(Lang::Ja, &file),
            "a save must not fail because something else is reading the file"
        );
        drop(held);
        assert_eq!(Settings::read_from(&file), Some(third));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file we cannot fully understand must leave every preference it did not speak
    /// about alone, rather than resetting the lot or flipping one.
    #[test]
    fn a_damaged_file_never_flips_a_preference() {
        let default = Settings::default();
        assert_eq!(Settings::parse(""), default);
        assert_eq!(Settings::parse("# comment only\n\n   \n"), default);
        assert_eq!(Settings::parse("garbage without an equals sign"), default);
        assert_eq!(Settings::parse("notify_power=maybe"), default, "an unknown value");
        // A real language code, because that is what someone will actually type. It has to
        // leave the interface where it was rather than half-applying.
        assert_eq!(Settings::parse("language=fr"), default, "a language we do not speak");
        // A language we do understand, and the word that means "ask Windows".
        assert_eq!(Settings::parse("language=en").language, Some(Lang::En));
        assert_eq!(Settings::parse("language=auto").language, None);
        // An unreadable value leaves an earlier good one alone, rather than resetting it.
        assert_eq!(Settings::parse("language=ja\nlanguage=zz").language, Some(Lang::Ja));

        // The header is the only prose in the file, and it is the only place `language`'s
        // other values are written down. It has been wrong once already, when it still
        // claimed every value was on or off.
        for lang in [Lang::Ja, Lang::En] {
            let header = Settings::default().as_text(lang).lines().next().unwrap().to_string();
            assert!(header.starts_with('#'), "{lang:?} header must be a comment: {header}");
            for value in ["auto", "ja", "en"] {
                assert!(header.contains(value), "{lang:?} header omits {value}: {header}");
            }
            assert!(!header.contains('\n'), "{lang:?} header must stay one line");
        }
        // An explicit `on` has to be read as a decision, and the only way to see that is over
        // a line that said otherwise. Both preferences default to on, so a parser that
        // dropped the word entirely would still satisfy every round trip above.
        assert!(Settings::parse("notify_power=off\nnotify_power=on").notify_power);
        assert!(Settings::parse("notify_battery=off\nnotify_battery=on").notify_battery);
        assert_eq!(Settings::parse("notify_power="), default, "an empty value");
        assert_eq!(Settings::parse("unknown_key=off"), default, "a key from the future");

        // Truncated halfway through: the key that did arrive is honored, the one that did
        // not keeps its default rather than being read as off.
        let half = Settings::parse("notify_power=off\nnotify_bat");
        assert!(!half.notify_power);
        assert!(half.notify_battery, "a line that never arrived is not a decision");

        // Whitespace and a trailing comment block are ordinary, not damage.
        let spaced = Settings::parse("  notify_power = off  \n#note\n\tnotify_battery\t=\toff\t");
        assert_eq!(
            spaced,
            Settings {
                notify_power: false,
                notify_battery: false,
                language: None
            }
        );
    }
}

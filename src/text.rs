//! Every word the app says to a person, in each language it says it in.
//!
//! One function per message, and each one is a `match lang` and nothing else. That is the
//! whole mechanism: no catalog, no message ids, no lookup at run time, no fourth crate.
//! An exhaustive match is what guarantees a message exists in both languages, and it does
//! it at compile time.
//!
//! What is *not* here: `Cause::describe`, which is diagnostic and English in every
//! language (see CLAUDE.md), and names that are not words in any language, like the
//! notification title `INZONE H9 / H7` and the settings keys.
//!
//! Every string a person reads belongs here, and nowhere else. Nothing enforces that: a
//! message written straight into the module that shows it would reach the tray in one
//! language, and the compiler has no opinion. Keeping it out is a review job, the same way
//! it is in a project with a message catalog, where a string that was never wrapped for
//! extraction is invisible to the tooling too.

/// The languages the interface speaks.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Lang {
    Ja,
    En,
}

impl Lang {
    /// Which language Windows says this user reads, for when the settings file has no
    /// opinion. English is the answer for every language we do not speak, rather than a
    /// third state: a tray icon has to say something.
    pub fn from_windows() -> Lang {
        use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
        /// The low ten bits of a LANGID are the primary language. `0x11` is Japanese; the
        /// sublanguage above them distinguishes regional variants, and Japanese has one.
        const LANG_JAPANESE: u16 = 0x11;
        let langid = unsafe { GetUserDefaultUILanguage() };
        if langid & 0x3FF == LANG_JAPANESE {
            Lang::Ja
        } else {
            Lang::En
        }
    }

    /// The spelling used in `settings.conf`, and what `language=` accepts there.
    pub fn parse(name: &str) -> Option<Lang> {
        match name {
            "ja" => Some(Lang::Ja),
            "en" => Some(Lang::En),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Lang::Ja => "ja",
            Lang::En => "en",
        }
    }
}

/// Prefix every tray string carries, so the icon is identifiable in a crowded taskbar.
/// A product name, so it is the same in both languages and lives outside the matches.
const TAG: &str = "INZONE";

// ------------------------------------------------------------------ tooltip

pub fn tip_off(lang: Lang) -> String {
    match lang {
        Lang::Ja => format!("{TAG}: ヘッドセット電源オフ"),
        Lang::En => format!("{TAG}: headset off"),
    }
}

pub fn tip_pairing(lang: Lang) -> String {
    match lang {
        Lang::Ja => format!("{TAG}: ペアリング中"),
        Lang::En => format!("{TAG}: pairing"),
    }
}

/// The powered-on tooltip, assembled from the level and the Bluetooth side so the pieces
/// can be ordered differently in a language that wants them differently.
pub fn tip_on(lang: Lang, battery: Option<u8>, charging: bool, bluetooth: Option<&str>) -> String {
    let power = match (lang, battery, charging) {
        (Lang::Ja, Some(b), true) => format!("オン（充電中 {b}%）"),
        (Lang::En, Some(b), true) => format!("On (charging, {b}%)"),
        (Lang::Ja, Some(b), false) => format!("オン（バッテリー {b}%）"),
        (Lang::En, Some(b), false) => format!("On (battery {b}%)"),
        (Lang::Ja, None, true) => "オン（充電中）".to_string(),
        (Lang::En, None, true) => "On (charging)".to_string(),
        (Lang::Ja, None, false) => "オン".to_string(),
        (Lang::En, None, false) => "On".to_string(),
    };
    match (lang, bluetooth) {
        (_, None) => format!("{TAG}: {power}"),
        (Lang::Ja, Some(bt)) => format!("{TAG}: {power}、{bt}"),
        (Lang::En, Some(bt)) => format!("{TAG}: {power}, {bt}"),
    }
}

pub fn bluetooth_connected(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "Bluetooth 接続済み",
        Lang::En => "Bluetooth connected",
    }
}

pub fn bluetooth_pairing(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "Bluetooth ペアリング中",
        Lang::En => "Bluetooth pairing",
    }
}

pub fn bluetooth_unconnected(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "Bluetooth オン（未接続）",
        Lang::En => "Bluetooth on, not connected",
    }
}

pub fn bluetooth_off(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "Bluetooth オフ",
        Lang::En => "Bluetooth off",
    }
}

pub fn tip_hub_busy(lang: Lang) -> String {
    match lang {
        Lang::Ja => format!("{TAG}: ポートが使用中（INZONE Hub を閉じると復帰します）"),
        Lang::En => format!("{TAG}: port in use (close INZONE Hub to resume)"),
    }
}

/// `unidentified` is how many serial ports we could not identify while failing to find the
/// dongle. At zero the list was read to the end and understood, so "not found" is the whole
/// truth. Above zero it is still what the user should act on, and the rest says the answer
/// may be ours to get wrong rather than theirs.
pub fn tip_no_dongle(lang: Lang, unidentified: usize) -> String {
    match (lang, unidentified) {
        (Lang::Ja, 0) => format!("{TAG}: レシーバーが見つかりません"),
        (Lang::Ja, n) => format!("{TAG}: レシーバーが見つかりません（判別できないポートが {n} 個あります）"),
        (Lang::En, 0) => format!("{TAG}: receiver not found"),
        (Lang::En, n) => format!("{TAG}: receiver not found (could not identify {n} of the ports)"),
    }
}

/// `reason` comes from `Cause::describe` and is English whatever the language. Only the
/// sentence around it is translated.
pub fn tip_error(lang: Lang, reason: &str) -> String {
    match lang {
        Lang::Ja => format!("{TAG}: エラー（{reason}）"),
        Lang::En => format!("{TAG}: error ({reason})"),
    }
}

pub fn tip_starting(lang: Lang) -> String {
    match lang {
        Lang::Ja => format!("{TAG}: 状態を取得中…"),
        Lang::En => format!("{TAG}: reading the state…"),
    }
}

// ------------------------------------------------------------------ notifications

pub fn notify_on_with_battery(lang: Lang, battery: u8) -> String {
    match lang {
        Lang::Ja => format!("ヘッドセットがオンになりました（バッテリー {battery}%）"),
        Lang::En => format!("Headset switched on (battery {battery}%)"),
    }
}

pub fn notify_on(lang: Lang) -> String {
    match lang {
        Lang::Ja => "ヘッドセットがオンになりました".into(),
        Lang::En => "Headset switched on".into(),
    }
}

pub fn notify_pairing(lang: Lang) -> String {
    match lang {
        Lang::Ja => "ヘッドセットがペアリングを開始しました".into(),
        Lang::En => "Headset started pairing".into(),
    }
}

pub fn notify_off(lang: Lang) -> String {
    match lang {
        Lang::Ja => "ヘッドセットがオフになりました".into(),
        Lang::En => "Headset switched off".into(),
    }
}

pub fn notify_battery_critical(lang: Lang, level: u8) -> String {
    match lang {
        Lang::Ja => format!("バッテリー残量がわずかです（{level}%）"),
        Lang::En => format!("Battery is nearly empty ({level}%)"),
    }
}

pub fn notify_battery_low(lang: Lang, level: u8) -> String {
    match lang {
        Lang::Ja => format!("バッテリーが残り {level}% です"),
        Lang::En => format!("Battery is down to {level}%"),
    }
}

pub fn notify_test(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "通知の動作確認です",
        Lang::En => "This is a test notification",
    }
}

pub fn notify_startup_failed(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "自動起動の設定を変更できませんでした",
        Lang::En => "Could not change the startup setting",
    }
}

pub fn notify_browser_failed(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "ブラウザを開けませんでした",
        Lang::En => "Could not open the browser",
    }
}

pub fn notify_settings_unsaved(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "設定を保存できませんでした（この起動中だけ有効です）",
        Lang::En => "Could not save the settings (they hold for this session only)",
    }
}

// ------------------------------------------------------------------ menu

pub fn menu_starting(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "状態を取得中…",
        Lang::En => "Reading the state…",
    }
}

pub fn menu_notify_power(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "ヘッドセットのオン/オフ通知",
        Lang::En => "Headset on/off notifications",
    }
}

pub fn menu_notify_battery(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "バッテリー低下の通知",
        Lang::En => "Low battery notifications",
    }
}

pub fn menu_run_at_logon(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "Windows 起動時に開始",
        Lang::En => "Start with Windows",
    }
}

/// Names the page as well as the errand. "Check for updates" alone reads as something the
/// app goes and does, and it does not: it opens a page and the reader compares the number
/// against the version line above it.
pub fn menu_check_for_updates(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "更新を確認（リリースページ）",
        Lang::En => "Check for updates (releases page)",
    }
}

pub fn menu_quit(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "終了",
        Lang::En => "Quit",
    }
}

// ------------------------------------------------------------------ command line

pub fn already_running(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "inzone-h9-gen1-headset-status はすでに起動しています。タスクトレイを確認してください。",
        Lang::En => "inzone-h9-gen1-headset-status is already running. Look in the notification area.",
    }
}

pub fn unknown_option(lang: Lang, option: &str) -> String {
    match lang {
        Lang::Ja => format!("不明なオプション: {option}"),
        Lang::En => format!("unknown option: {option}"),
    }
}

pub fn help(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => HELP_JA,
        Lang::En => HELP_EN,
    }
}

// ------------------------------------------------------------------ settings file

/// The header written into `settings.conf`. The keys below it are ASCII because they are a
/// format, but the line explaining them is read by whoever opened the file.
///
/// It names what `language` accepts, because `auto` is the only value they will see there
/// and it gives away none of the others. The on/off keys need no such help: their two
/// values are the two words already written next to them.
pub fn settings_file_header(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => {
            "# inzone-h9-gen1-headset-status の設定です。消すと既定に戻ります。language は auto か ja か en です。"
        }
        Lang::En => "# inzone-h9-gen1-headset-status settings. Delete the file to reset. language is auto, ja or en.",
    }
}

const HELP_JA: &str = "\
INZONE H9 / H7（初代）のヘッドセット電源状態をタスクトレイに表示します。

使い方:
  inzone-h9-gen1-headset-status [オプション]

オプション:
  -h, --help       このヘルプを表示して終了
  -V, --version    バージョンを表示して終了
      --test-toast 起動時に通知を 1 回出す（通知経路の確認用）

アイコン:
  緑    ヘッドセットがオン
  黄    オン、バッテリー 35% 以下
  赤    オン、バッテリー 15% 以下
  明るい色 オン、充電中（残量の色を明るくしたもの。満充電で元に戻ります）
  青    ペアリング中
  灰    ヘッドセットの電源がオフ
  紫    COM ポートを INZONE Hub が使用中（リング）
  暗灰  レシーバー未接続、またはエラー（リング）

  円は残量のぶんだけ塗られます（真上から時計回り）。
  右下の青いバッジが Bluetooth で、塗りが接続済み、
  中空がオンで未接続またはペアリング中です。

右クリックメニュー:
  現在の状態、通知の on/off、自動起動の on/off、バージョン、更新の確認が出ます。

  このアプリはネットワークに接続しません。「更新を確認」はリリースページを
  既定のブラウザで開くだけで、新しい版があるかどうかの確認はブラウザ側で行います。

  通知の設定は %APPDATA%\\inzone-h9-gen1-headset-status\\settings.conf に保存され、
  次回の起動でも維持されます。ファイルを消すと既定（どちらも on）に戻ります。

  自動起動は HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run に登録します。
  Windows の設定のアプリ、スタートアップの一覧からも切り替えられます。

終了コード:
  0  正常に終了した
  2  オプションが解釈できなかった
  3  すでに起動していたので何もせず終了した

INZONE Hub が起動しているとポートを握られて状態を読めません。Hub を閉じれば
自動で復帰します。EQ などの音響効果は Hub を常駐させなくても効き続けます。";

const HELP_EN: &str = "\
Shows the power state of a Sony INZONE H9 / H7 (first generation) headset in the
notification area.

Usage:
  inzone-h9-gen1-headset-status [options]

Options:
  -h, --help       print this help and exit
  -V, --version    print the version and exit
      --test-toast fire one notification at startup, to check that path

Icon:
  green   headset is on
  amber   on, battery at 35% or below
  red     on, battery at 15% or below
  pale    on and charging: the band color, lightened. Back to plain when full.
  blue    pairing
  gray    headset is off
  violet  INZONE Hub is holding the COM port (ring)
  dim     no receiver, or an error (ring)

  The disc fills with what charge is left, clockwise from the top.
  The blue badge in the corner is Bluetooth: filled is connected, hollow is on
  but not connected, or pairing.

Context menu:
  The current state, the notification switches, the startup switch, the version,
  and a way to check for updates.

  This app never connects to the network. The updates item opens the releases page
  in your browser, and it is the browser that goes and looks.

  The notification settings are kept in
  %APPDATA%\\inzone-h9-gen1-headset-status\\settings.conf and survive a restart. Delete the
  file to go back to the defaults, which are both on.

  language=auto|ja|en in that file picks the interface language. auto follows
  Windows. A change takes effect at the next start.

  Startup registers under HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run,
  so Settings > Apps > Startup can switch it off as well.

Exit codes:
  0  finished normally
  2  the command line could not be understood
  3  another instance was already running, so this one did nothing

While INZONE Hub is running it holds the port and the state cannot be read. Closing
the Hub is enough; this recovers on its own. The equalizer and the other audio
effects keep working with the Hub closed.";

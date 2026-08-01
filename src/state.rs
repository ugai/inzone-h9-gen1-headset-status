//! What the app believes about the headset, and everything that reads off that belief:
//! the tooltip, the icon's color and shape, and the decision to notify.

use crate::protocol::{
    BATT_CHARGING, BATT_DISCHARGING, BT_CONN_CONNECTED, BT_CONN_PAIRING, BT_CONN_UNCONNECTED, BT_OFF, BT_ON,
};
use crate::text::{self, Lang};

/// The headset's Bluetooth side, which is independent of the 2.4 GHz dongle link: the
/// feature can be on while nothing is paired to it.
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Bt {
    Off,
    Unconnected,
    Pairing,
    Connected,
}

/// Where the Bluetooth state goes on the icon. The color channel is already carrying
/// power and battery, so Bluetooth gets its own corner instead of its own hue.
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Badge {
    None,
    Hollow,
    Filled,
}

/// Why a poll came back without a reading.
///
/// A cause rather than a sentence, so the serial transport does not decide how the tray
/// words things. It used to hand `State` a `String` assembled across three modules, half
/// of it English and half Japanese, and there was no one place that could have fixed that.
///
/// The variants that carry text carry someone else's: a Windows message that arrives
/// already localized, or a protocol decode failure that is diagnostic. Both are quoted,
/// never composed.
///
/// `describe` is English in every language the UI speaks. These are symbolic: what a
/// person does with "unknown link state 0xAB" is quote it into a bug report, and the
/// decode failures they sit beside have always been English. Translating half of a
/// sentence whose other half comes from `protocol` would make the mixture look accidental
/// rather than deliberate. The tray words the sentence *around* them, and that is
/// translated.
#[derive(PartialEq, Clone, Debug)]
pub enum Cause {
    /// DTR could not be asserted. The exchange was abandoned before a byte went out,
    /// because writing without it wedges the dongle until someone replugs it.
    Dtr(String),
    /// The serial ports could not be listed, so whether the dongle is there is unknown.
    /// Distinct from `NoDongle`, which is the answer to a list we actually got back.
    Enumerate(String),
    /// Opening the port failed for a reason that is not "another process holds it".
    Open(String),
    /// A read or a write failed mid-exchange.
    Io(String),
    /// The port went away under us.
    PortClosed,
    /// The exchange ended without a reply we could use. `detail` is the first decode
    /// failure, if there was one at all, and `unusable` counts every byte that arrived
    /// during it, all of which the frame scan made nothing of. It is a running total of
    /// bytes read rather than what the scan had left over, because the scan drains what it
    /// rejects: a dongle that said nothing and one that streamed a kilobyte of rubbish
    /// would otherwise both report a byte or two, and telling those apart is the point.
    NoReply { detail: Option<String>, unusable: usize },
    /// A reply arrived where a parameter was required, and carried none.
    EmptyReply,
    /// The dongle named a link state we have no meaning for.
    UnknownLink(u8),
    /// The poller thread is gone, so every reading from here on would be stale. Not a
    /// transport failure at all, but it lands in the same place: we cannot say.
    PollerStopped,
}

/// Longest run of someone else's text to quote inside a tooltip, in UTF-16 units. `szTip`
/// holds 128 of them and the shell truncates without a word, so a Windows message long
/// enough to spend the whole budget would silently cut the sentence wrapped around it.
const QUOTED_MAX_UNITS: usize = 60;

impl Cause {
    /// The reason, in a form short enough to sit inside a tooltip.
    fn describe(&self) -> String {
        // The budget is the shell's, so it is counted in the shell's units. Characters
        // would be the wrong ruler: anything outside the basic plane is two UTF-16 units,
        // so 60 of those would be 120 and blow the very limit this exists to respect.
        // Bytes would be wrong too, and would panic mid-character on the way.
        fn quote(s: &str) -> String {
            let mut out = String::new();
            let mut units = 0;
            for c in s.chars() {
                if units + c.len_utf16() > QUOTED_MAX_UNITS {
                    out.push('…');
                    return out;
                }
                units += c.len_utf16();
                out.push(c);
            }
            out
        }
        match self {
            Cause::Dtr(e) => format!("cannot assert DTR: {}", quote(e)),
            Cause::Enumerate(e) => format!("cannot list serial ports: {}", quote(e)),
            Cause::Open(e) => quote(e),
            Cause::Io(e) => quote(e),
            Cause::PortClosed => "port closed".into(),
            Cause::NoReply { detail, unusable } => match (detail, unusable) {
                (Some(e), 0) => quote(e),
                (Some(e), n) => format!("{} ({n} bytes unusable)", quote(e)),
                (None, 0) => "no reply".into(),
                (None, n) => format!("no usable reply ({n} bytes)"),
            },
            Cause::EmptyReply => "empty reply".into(),
            Cause::UnknownLink(v) => format!("unknown link state 0x{v:02X}"),
            Cause::PollerStopped => "poller stopped".into(),
        }
    }
}

#[derive(PartialEq, Clone, Debug)]
pub enum State {
    /// Dongle answered: the headset is not powered on / not linked.
    Off,
    Pairing,
    On {
        battery: Option<u8>,
        charging: bool,
        bt: Option<Bt>,
    },
    /// COM port exists but is held by another process — in practice INZONE Hub.
    HubBusy,
    /// No port on the list was the dongle. `unidentified` counts the entries we could not
    /// identify at all, which the dongle may be one of: `serialport` resolves a composite
    /// device's VID and PID through its parent, this dongle's port always sits under an
    /// interface, and a parent that will not resolve leaves the port listed as `Unknown`.
    /// Ports found only in the registry are `Unknown` too. Saying "receiver not found" with
    /// a receiver plugged in is the confident wrong answer the rest of this program exists
    /// to avoid, so the count travels with the state and the tooltip says it out loud.
    NoDongle {
        unidentified: usize,
    },
    Error(Cause),
}

impl State {
    pub fn tooltip(&self, lang: Lang) -> String {
        match self {
            State::Off => text::tip_off(lang),
            State::Pairing => text::tip_pairing(lang),
            State::On { battery, charging, bt } => {
                let bluetooth = bt.map(|bt| match bt {
                    Bt::Connected => text::bluetooth_connected(lang),
                    Bt::Pairing => text::bluetooth_pairing(lang),
                    Bt::Unconnected => text::bluetooth_unconnected(lang),
                    Bt::Off => text::bluetooth_off(lang),
                });
                text::tip_on(lang, *battery, *charging, bluetooth)
            }
            State::HubBusy => text::tip_hub_busy(lang),
            State::NoDongle { unidentified } => text::tip_no_dongle(lang, *unidentified),
            State::Error(cause) => text::tip_error(lang, &cause.describe()),
        }
    }

    /// Bluetooth off and "we never asked" both mean no badge: an absent mark must not be
    /// read as a positive claim that Bluetooth is off.
    pub fn badge(&self) -> Badge {
        match self {
            State::On {
                bt: Some(Bt::Connected),
                ..
            } => Badge::Filled,
            State::On {
                bt: Some(Bt::Unconnected | Bt::Pairing),
                ..
            } => Badge::Hollow,
            _ => Badge::None,
        }
    }

    /// Icon fill color, on a traffic-light reading: green is fine, amber is getting low,
    /// red needs charging. Battery level tints the "on" state so a flat headset is visible
    /// without opening the tooltip.
    pub fn color(&self) -> [u8; 3] {
        match self {
            State::On { battery, charging, .. } => {
                let band = band_color(*battery);
                // A pack on the cable is lit rather than recolored, so the level still
                // reads off the hue while the paleness says "filling". Charging used to
                // take the hue outright, which meant a pack at 10% and one at 90% were the
                // same color, and the one hue it took turned out not to say "charging" to
                // anybody. Full is not filling, so it goes back to the plain band.
                if *charging && *battery != Some(100) {
                    lighten(band)
                } else {
                    band
                }
            }
            State::Pairing => [0x3B, 0x82, 0xF6],                           // blue
            State::Off => [0x8A, 0x8A, 0x94],                               // gray
            State::HubBusy => [0xA7, 0x8B, 0xFA],                           // violet
            State::NoDongle { .. } | State::Error(_) => [0x71, 0x71, 0x7A], // dim gray
        }
    }

    /// Battery level as a fraction of the disc to fill, so remaining charge survives the
    /// loss of hue. Green and amber differ by a contrast ratio of 1.06, and red and the
    /// powered-off gray by 1.10: on hue alone, "nearly flat" and "switched off" are the
    /// same icon. `None` leaves the disc whole, which is the honest shape for "unknown".
    pub fn battery_arc(&self) -> Option<u8> {
        match self {
            State::On { battery: Some(b), .. } => Some(*b),
            _ => None,
        }
    }

    /// Hollow ring rather than a solid dot, for the states where we have no real answer.
    pub fn hollow(&self) -> bool {
        matches!(self, State::NoDongle { .. } | State::Error(_) | State::HubBusy)
    }

    /// Whether the headset is powered, for states where we actually know. `None` means the
    /// dongle did not tell us, which must never be reported as a power transition.
    fn powered(&self) -> Option<bool> {
        match self {
            State::On { .. } | State::Pairing => Some(true),
            State::Off => Some(false),
            State::HubBusy | State::NoDongle { .. } | State::Error(_) => None,
        }
    }

    fn notification_body(&self, lang: Lang) -> Option<String> {
        match self {
            State::On { battery: Some(b), .. } => Some(text::notify_on_with_battery(lang, *b)),
            State::On { battery: None, .. } => Some(text::notify_on(lang)),
            State::Pairing => Some(text::notify_pairing(lang)),
            State::Off => Some(text::notify_off(lang)),
            _ => None,
        }
    }
}

/// Battery levels at or below these announce, and they are the same two `State::color`
/// turns amber and red on, so the warning and the color cannot come to disagree.
const BATTERY_LOW: u8 = 35;
const BATTERY_CRITICAL: u8 = 15;
/// How far above a threshold the level has to climb before that warning can fire again. A
/// pack sitting on 35 would otherwise announce on every poll, four times a minute.
const BATTERY_HYSTERESIS: u8 = 5;

/// The band a level falls in. The same two constants `BatteryWatch` announces on, so the
/// color and the warning cannot come to disagree.
fn band_color(battery: Option<u8>) -> [u8; 3] {
    match battery {
        Some(b) if b <= BATTERY_CRITICAL => [0xEF, 0x44, 0x44], // red
        Some(b) if b <= BATTERY_LOW => [0xF5, 0x9E, 0x0B],      // amber
        _ => [0x22, 0xC5, 0x5E],                                // green
    }
}

/// How far a charging color is washed towards `PAPER`. Measured against every band, the
/// pale version sits at a contrast ratio of 1.5 or better from the color it came from,
/// where this project has judged 1.06 and 1.10 too low to rely on.
const CHARGING_WASH: f32 = 0.60;
const PAPER: [u8; 3] = [0xF8, 0xFA, 0xFC];

fn lighten(color: [u8; 3]) -> [u8; 3] {
    std::array::from_fn(|i| (color[i] as f32 + (PAPER[i] as f32 - color[i] as f32) * CHARGING_WASH).round() as u8)
}

/// Watches the battery for a threshold crossed downward.
///
/// The rule that binds `notification_for` binds this too: only a transition between two
/// readings we actually have is news. A reading we failed to take leaves the watch exactly
/// as it was, so coming back from a spell of HubBusy announces nothing by itself, and the
/// first known reading after launch seeds the watch instead of firing.
#[derive(Default, Debug)]
pub struct BatteryWatch {
    /// Severity already announced, or `None` before the first reading we understood.
    announced: Option<u8>,
}

/// 0 is fine, 1 is low, 2 is nearly flat. `slack` raises both thresholds, which is how a
/// band is held on to until the level has clearly escaped it.
fn severity(level: u8, slack: u8) -> u8 {
    if level <= BATTERY_CRITICAL + slack {
        2
    } else if level <= BATTERY_LOW + slack {
        1
    } else {
        0
    }
}

impl BatteryWatch {
    /// Feeds one reading in and returns the text to announce, or `None` to stay silent.
    pub fn observe(&mut self, lang: Lang, state: &State) -> Option<String> {
        // Not a level we have, so not a crossing we can claim. The watch keeps its memory:
        // forgetting here would let the next known reading announce all over again.
        let State::On {
            battery: Some(level),
            charging,
            ..
        } = state
        else {
            return None;
        };
        let (level, charging) = (*level, *charging);

        let now = severity(level, 0);
        // Falling takes effect at the threshold; rising has to clear it by the hysteresis
        // before the band is given up.
        let held = severity(level, BATTERY_HYSTERESIS);

        let Some(announced) = self.announced else {
            // Nothing to cross from. Seed and stay quiet, the same way the first poll after
            // launch never announces a power transition.
            self.announced = Some(now);
            return None;
        };
        if now <= announced {
            self.announced = Some(announced.min(held));
            return None;
        }
        // A pack on the cable is already being dealt with, and the level is on its way up.
        // Checked before the crossing is written down rather than after: recording it and
        // then staying quiet spent it, so a pack that fell past a threshold while plugged in
        // had nothing left to announce when the cable came out at the very level the warning
        // exists for. Not recording it costs nothing, because the same `charging` returns
        // here on every poll until the cable is out.
        if charging {
            return None;
        }
        self.announced = Some(now);
        Some(match now {
            2 => text::notify_battery_critical(lang, level),
            _ => text::notify_battery_low(lang, level),
        })
    }
}

/// The notification text for a state change, or `None` to stay silent.
///
/// Silence is the default and it matters: the first poll after launch has no previous state,
/// and a trip through HubBusy / an error / an unplugged dongle tells us nothing about the
/// headset. Announcing those would mean claiming the headset turned off when it did not.
pub fn notification_for(lang: Lang, prev: Option<&State>, next: &State) -> Option<String> {
    let before = prev?.powered()?;
    let now = next.powered()?;
    if before == now {
        return None;
    }
    next.notification_body(lang)
}

/// Reads a BATTERY_INFO payload into (level, charging).
///
/// Every way of not understanding the reply collapses to "level unknown". While charging,
/// the level byte has been seen coming back as 0xC8 (200), which is not a percentage;
/// masking the high bit would give a plausible 72, but nothing confirms that encoding. `BATTERY_STATUS::ERROR` means the device is telling us its own reading is
/// bad, so the number that comes with it is not worth showing either.
///
/// A reply too short to carry a level forfeits its status byte too, so `charging` stays a
/// plain `bool`. The honest `Option<bool>` would have to thread through `State`, the color,
/// the tooltip and the notification text, and it would buy truth only for a frame shape
/// never seen on hardware. Unlike the Bluetooth case the omission does not assert anything:
/// a missing cyan is an absent mark, not a claim of "not charging". Revisit if a one-byte
/// battery reply ever turns up in a capture.
pub fn interpret_battery(params: &[u8]) -> (Option<u8>, bool) {
    match params {
        [BATT_DISCHARGING, level, ..] => ((*level <= 100).then_some(*level), false),
        [BATT_CHARGING, level, ..] => ((*level <= 100).then_some(*level), true),
        _ => (None, false),
    }
}

/// Reads a BT_STATUS payload. `None` means the headset did not give us an answer we
/// understand, which must stay distinct from a confident "Bluetooth is off": a one-byte
/// reply whose only byte says *on* used to be reported as off.
///
/// Note this is stricter than `interpret_battery`, which keeps a status it trusts even when
/// it throws the level away. The difference is in what the icon can say: "charging, level
/// unknown" has an honest rendering, whereas "on, connection unknown" does not (a hollow
/// badge positively claims not-connected). So an unrecognized connection byte discards the
/// whole answer rather than dressing it up. Do not harmonize the two without fixing that.
pub fn interpret_bt(params: &[u8]) -> Option<Bt> {
    match params {
        [BT_OFF, ..] => Some(Bt::Off),
        [BT_ON, BT_CONN_UNCONNECTED, ..] => Some(Bt::Unconnected),
        [BT_ON, BT_CONN_CONNECTED, ..] => Some(Bt::Connected),
        [BT_ON, BT_CONN_PAIRING, ..] => Some(Bt::Pairing),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icon::BT_BLUE;

    /// Charging lights the band rather than replacing it.
    ///
    /// It used to replace it, with cyan. Two things were wrong with that. A pack at 10% and
    /// one at 90% came out the same color, so the level survived only in the arc. And the
    /// hue said nothing: cyan does not read as "charging" to anyone, which is what finally
    /// settled it after a long look at bolts, plugs and corner marks.
    #[test]
    fn charging_lightens_the_band_until_full() {
        let on = |b, charging| State::On {
            battery: Some(b),
            charging,
            bt: None,
        };
        let (green, amber, red) = (on(70, false).color(), on(30, false).color(), on(10, false).color());

        // The level still reads off the hue while the cable is in.
        assert_eq!(on(70, true).color(), lighten(green));
        assert_eq!(on(30, true).color(), lighten(amber));
        assert_eq!(on(10, true).color(), lighten(red));
        assert_ne!(
            on(10, true).color(),
            on(70, true).color(),
            "charging must not flatten the bands"
        );

        // Full is not filling, so it drops the wash.
        assert_eq!(on(100, true).color(), on(100, false).color());
        assert_eq!(on(99, true).color(), lighten(green));

        // A level we could not read still shows that the cable is in.
        let unknown = State::On {
            battery: None,
            charging: true,
            bt: None,
        };
        assert_eq!(unknown.color(), lighten(green));
        assert!(unknown.tooltip(Lang::Ja).contains("充電中"));
        assert!(
            !State::On {
                battery: None,
                charging: false,
                bt: None
            }
            .tooltip(Lang::Ja)
            .contains("充電中")
        );

        /// WCAG relative luminance, for the one thing this design rests on: a washed band
        /// has to be visibly lighter than the band it came from.
        fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
            let lum = |c: [u8; 3]| {
                let f = |v: u8| {
                    let v = v as f32 / 255.0;
                    if v <= 0.04045 {
                        v / 12.92
                    } else {
                        ((v + 0.055) / 1.055).powf(2.4)
                    }
                };
                0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
            };
            let (x, y) = (lum(a), lum(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        }
        // Washed *towards* paper, and not past it. The contrast check below only asks for a
        // gap, so a wash that overshot to white, or one that ran the wrong way and saturated
        // there, would satisfy it just as well and look nothing like the design.
        for band in [green, amber, red] {
            let washed = lighten(band);
            for i in 0..3 {
                assert!(
                    band[i] < washed[i] && washed[i] < PAPER[i],
                    "channel {i} of {band:?} washed to {}, outside {}..{}",
                    washed[i],
                    band[i],
                    PAPER[i]
                );
            }
        }

        // The 1.5 `CHARGING_WASH` documents, held here so the two cannot drift apart.
        for band in [green, amber, red] {
            let ratio = contrast(band, lighten(band));
            assert!(
                ratio >= 1.5,
                "washed {band:?} is only {ratio:.2} from its band, and 1.06 was already judged too low"
            );
        }

        // Every state a person could mistake for charging has to stay far away. The washed
        // bands are deliberately absent from this list: telling "charging at 30%" from
        // "charging at 10%" is a level question, and the arc is what answers those.
        let others = [
            green,
            amber,
            red,
            State::Pairing.color(),
            State::Off.color(),
            State::HubBusy.color(),
            State::NoDongle { unidentified: 0 }.color(),
            BT_BLUE,
        ];
        for band in [green, amber, red] {
            let washed = lighten(band);
            for other in others {
                let gap: f32 = (0..3)
                    .map(|i| (washed[i] as f32 - other[i] as f32).powi(2))
                    .sum::<f32>()
                    .sqrt();
                assert!(gap > 55.0, "washed {band:?} is too close to {other:?} (distance {gap})");
            }
        }
    }

    /// An absent badge must mean "no positive claim", never "Bluetooth is off".
    #[test]
    fn badge_only_appears_when_bluetooth_answered() {
        let with = |bt| State::On {
            battery: Some(50),
            charging: false,
            bt,
        };
        assert_eq!(with(Some(Bt::Connected)).badge(), Badge::Filled);
        assert_eq!(with(Some(Bt::Unconnected)).badge(), Badge::Hollow);
        assert_eq!(with(Some(Bt::Pairing)).badge(), Badge::Hollow);
        assert_eq!(with(Some(Bt::Off)).badge(), Badge::None);
        assert_eq!(with(None).badge(), Badge::None);
        for s in [
            State::Off,
            State::Pairing,
            State::HubBusy,
            State::NoDongle { unidentified: 0 },
        ] {
            assert_eq!(s.badge(), Badge::None, "{s:?} knows nothing about Bluetooth");
        }
    }

    #[test]
    fn notifies_only_on_a_real_power_transition() {
        let on = State::On {
            battery: Some(80),
            charging: false,
            bt: Some(Bt::Connected),
        };
        let off = State::Off;

        assert!(
            notification_for(Lang::Ja, Some(&off), &on).is_some(),
            "off -> on must announce"
        );
        assert!(
            notification_for(Lang::Ja, Some(&on), &off).is_some(),
            "on -> off must announce"
        );

        // Nothing to compare against yet: launching must not fire a toast.
        assert!(notification_for(Lang::Ja, None, &on).is_none());
        assert!(notification_for(Lang::Ja, None, &off).is_none());

        // A trip through a state that tells us nothing must not be read as a transition.
        for unknown in [
            State::HubBusy,
            State::NoDongle { unidentified: 0 },
            State::Error(Cause::PortClosed),
        ] {
            assert!(
                notification_for(Lang::Ja, Some(&on), &unknown).is_none(),
                "{unknown:?} is not news"
            );
            assert!(
                notification_for(Lang::Ja, Some(&unknown), &off).is_none(),
                "resuming from {unknown:?}"
            );
        }

        // Battery drifting or Bluetooth changing while powered on is a tooltip change,
        // not a power transition.
        let on_lower = State::On {
            battery: Some(30),
            charging: false,
            bt: Some(Bt::Off),
        };
        assert!(notification_for(Lang::Ja, Some(&on), &on_lower).is_none());
        // Pairing counts as powered, so it must not announce again once connected.
        assert!(notification_for(Lang::Ja, Some(&State::Pairing), &on).is_none());

        // And the words. Every assertion above is just as happy with an empty string, and a
        // toast with nothing in it is worse than no toast: it says something happened and
        // then does not say what.
        let pairing = State::Pairing;
        let on_no_level = State::On {
            battery: None,
            charging: false,
            bt: None,
        };
        for lang in [Lang::Ja, Lang::En] {
            let bodies: Vec<String> = [(&off, &on), (&off, &on_no_level), (&off, &pairing), (&on, &off)]
                .iter()
                .map(|(prev, next)| notification_for(lang, Some(prev), next).expect("a transition announces"))
                .collect();
            for body in &bodies {
                assert!(!body.trim().is_empty(), "{lang:?} announced an empty toast");
            }
            assert!(bodies[0].contains("80"), "{lang:?} dropped the level: {}", bodies[0]);
            for i in 0..bodies.len() {
                for j in (i + 1)..bodies.len() {
                    assert_ne!(bodies[i], bodies[j], "{lang:?} says the same thing for two states");
                }
            }
        }
    }

    /// The icon's shape carries what its color cannot, and these two functions decide it.
    ///
    /// Neither had a test. The geometry tests in `icon` are handed an arc value and a hollow
    /// flag directly and never ask a `State` for either, so replacing either function with a
    /// constant changed every icon the app draws and nothing failed.
    #[test]
    fn the_shape_says_what_the_color_cannot() {
        let on = |battery| State::On {
            battery,
            charging: false,
            bt: None,
        };
        let no_answer = [
            State::HubBusy,
            State::NoDongle { unidentified: 0 },
            State::Error(Cause::PortClosed),
        ];

        // A level we have fills that much of the disc. One we do not leaves it whole, which
        // is the honest shape for "unknown" rather than a claim that the pack is full.
        assert_eq!(on(Some(0)).battery_arc(), Some(0));
        assert_eq!(on(Some(42)).battery_arc(), Some(42));
        assert_eq!(on(Some(100)).battery_arc(), Some(100));
        assert_eq!(on(None).battery_arc(), None);
        for s in no_answer.iter().chain(&[State::Off, State::Pairing]) {
            assert_eq!(s.battery_arc(), None, "{s:?} has no level to draw");
        }

        // A ring says we have no answer and a solid disc says we do. Collapsing the two would
        // make "the dongle told us the headset is off" and "we could not ask" one picture,
        // which is the lie the rest of this module is built around not telling.
        for s in &no_answer {
            assert!(s.hollow(), "{s:?} is not an answer");
        }
        for s in [State::Off, State::Pairing, on(Some(50)), on(None)] {
            assert!(!s.hollow(), "{s:?} is an answer");
        }
    }

    /// The warning fires on the same two numbers the icon changes color on, and a level
    /// parked on a threshold announces once rather than four times a minute.
    ///
    /// `BATTERY_LOW` and `BATTERY_CRITICAL` are read rather than written out here. The two
    /// last drifted apart when the color kept one copy of the literals and the warning
    /// kept another, so the whole point is that this test cannot be passed by a constant
    /// that only one of them moved.
    #[test]
    fn low_battery_announces_once_per_crossing() {
        let on = |b| State::On {
            battery: Some(b),
            charging: false,
            bt: None,
        };
        let (green, amber, red) = ([0x22, 0xC5, 0x5E], [0xF5, 0x9E, 0x0B], [0xEF, 0x44, 0x44]);
        assert_eq!(on(BATTERY_LOW).color(), amber, "amber has to start at BATTERY_LOW");
        assert_eq!(on(BATTERY_LOW + 1).color(), green);
        assert_eq!(
            on(BATTERY_CRITICAL).color(),
            red,
            "red has to start at BATTERY_CRITICAL"
        );
        assert_eq!(on(BATTERY_CRITICAL + 1).color(), amber);

        let mut watch = BatteryWatch::default();
        // Nothing to cross from, so the first reading only seeds.
        assert!(watch.observe(Lang::Ja, &on(BATTERY_LOW + 1)).is_none());
        // The threshold itself is inside the band, which is what the icon says too.
        let low = watch
            .observe(Lang::Ja, &on(BATTERY_LOW))
            .expect("the crossing announces");
        assert!(low.contains(&BATTERY_LOW.to_string()), "{low}");
        for _ in 0..4 {
            assert!(
                watch.observe(Lang::Ja, &on(BATTERY_LOW)).is_none(),
                "a level that has not moved is not a crossing"
            );
        }

        // Climbing back by less than the hysteresis does not give the band up, so falling
        // into it again is the same crossing rather than a new one. The single point below
        // is what fails if the hysteresis is ever taken back out: without it one poll on the
        // other side of the line counts as a recovery and the next fall announces again.
        assert!(watch.observe(Lang::Ja, &on(BATTERY_LOW + 1)).is_none());
        assert!(
            watch.observe(Lang::Ja, &on(BATTERY_LOW)).is_none(),
            "one point of recovery must not rearm the warning"
        );
        assert!(watch.observe(Lang::Ja, &on(BATTERY_LOW + BATTERY_HYSTERESIS)).is_none());
        assert!(
            watch.observe(Lang::Ja, &on(BATTERY_LOW)).is_none(),
            "the top of the hysteresis is still inside the band"
        );
        // Clearing it by more than the hysteresis does, and then the fall is news again.
        assert!(
            watch
                .observe(Lang::Ja, &on(BATTERY_LOW + BATTERY_HYSTERESIS + 1))
                .is_none()
        );
        assert!(watch.observe(Lang::Ja, &on(BATTERY_LOW)).is_some());

        // The lower threshold is its own crossing, reached from inside the band above it.
        let critical = watch
            .observe(Lang::Ja, &on(BATTERY_CRITICAL))
            .expect("falling into the critical band announces");
        assert!(critical.contains(&BATTERY_CRITICAL.to_string()), "{critical}");
        assert_ne!(critical, low, "the two warnings have to read differently");
        // Two different warnings, not one warning with a smaller number in it. The line
        // above passes either way, because the levels differ on their own.
        assert_ne!(
            critical,
            text::notify_battery_low(Lang::Ja, BATTERY_CRITICAL),
            "the critical warning must not collapse into the low one"
        );
        assert!(watch.observe(Lang::Ja, &on(BATTERY_CRITICAL)).is_none());

        // The same escape and re-entry at the lower threshold. The hysteresis is added to
        // both, and everything above exercises only the upper one.
        assert!(watch.observe(Lang::Ja, &on(BATTERY_CRITICAL + 1)).is_none());
        assert!(
            watch.observe(Lang::Ja, &on(BATTERY_CRITICAL)).is_none(),
            "one point of recovery must not rearm the critical warning"
        );
        assert!(
            watch
                .observe(Lang::Ja, &on(BATTERY_CRITICAL + BATTERY_HYSTERESIS + 1))
                .is_none()
        );
        assert!(watch.observe(Lang::Ja, &on(BATTERY_CRITICAL)).is_some());

        // A pack on the cable is already being dealt with and the level is on its way up.
        let plugged = |b| State::On {
            battery: Some(b),
            charging: true,
            bt: None,
        };
        let mut charging = BatteryWatch::default();
        assert!(charging.observe(Lang::Ja, &on(BATTERY_LOW + 20)).is_none());
        for _ in 0..3 {
            assert!(
                charging.observe(Lang::Ja, &plugged(BATTERY_CRITICAL)).is_none(),
                "a charging pack has no warning to give"
            );
        }
        // And staying quiet must not spend the crossing. Unplugging at the level the warning
        // exists for is exactly when it has to arrive, so the silence above cannot be paid
        // for by writing the crossing down.
        let unplugged = charging
            .observe(Lang::Ja, &on(BATTERY_CRITICAL))
            .expect("the cable coming out at a critical level announces");
        assert!(unplugged.contains(&BATTERY_CRITICAL.to_string()), "{unplugged}");
        assert!(charging.observe(Lang::Ja, &on(BATTERY_CRITICAL)).is_none());
    }

    /// A reading we could not take is never itself a crossing, and it does not erase the
    /// readings either side of it.
    ///
    /// Two rules meet here and they are easy to confuse. Feeding the watch a state that
    /// carries no level must announce nothing, which is the same rule `notification_for`
    /// follows. But the memory survives the gap, because the warning names the level the
    /// pack is at now rather than a fall anyone watched happen: a drop that straddles a
    /// spell of HubBusy is still worth saying out loud once the port comes back. Dropping
    /// the memory instead would be worse than it sounds, since `Off` carries no level
    /// either, and a headset switched off between sessions would then never warn again.
    #[test]
    fn a_missed_reading_is_never_a_battery_crossing() {
        let on = |b| State::On {
            battery: Some(b),
            charging: false,
            bt: None,
        };
        let blind = [
            State::HubBusy,
            State::NoDongle { unidentified: 0 },
            State::Error(Cause::PollerStopped),
            State::Off,
            State::Pairing,
            State::On {
                battery: None,
                charging: false,
                bt: None,
            },
        ];

        for state in &blind {
            // On its own it says nothing, from whichever side of a threshold it arrives.
            for start in [BATTERY_LOW + 1, BATTERY_LOW, BATTERY_CRITICAL] {
                let mut watch = BatteryWatch::default();
                assert!(watch.observe(Lang::Ja, &on(start)).is_none(), "the first reading seeds");
                assert!(
                    watch.observe(Lang::Ja, state).is_none(),
                    "{state:?} carries no level to cross"
                );
            }

            // The warning already given is remembered across it, so the gap cannot rearm it.
            let mut watch = BatteryWatch::default();
            assert!(watch.observe(Lang::Ja, &on(BATTERY_LOW + 1)).is_none());
            assert!(watch.observe(Lang::Ja, &on(BATTERY_LOW)).is_some());
            assert!(watch.observe(Lang::Ja, state).is_none());
            assert!(
                watch.observe(Lang::Ja, &on(BATTERY_LOW)).is_none(),
                "{state:?} must not rearm a warning already given"
            );

            // A fall that straddles the gap is announced on the far side of it: the level
            // is one we now have, and it is the level the warning is about.
            let mut watch = BatteryWatch::default();
            assert!(watch.observe(Lang::Ja, &on(BATTERY_LOW + 40)).is_none());
            assert!(watch.observe(Lang::Ja, state).is_none());
            assert!(
                watch.observe(Lang::Ja, &on(BATTERY_CRITICAL)).is_some(),
                "a flat pack is worth saying so, gap or no gap"
            );

            // Launching into the gap is the case where nothing fires: the first level we
            // ever get seeds the watch, so a fall that happened before we could see anything
            // is not announced.
            let mut watch = BatteryWatch::default();
            assert!(watch.observe(Lang::Ja, state).is_none());
            assert!(
                watch.observe(Lang::Ja, &on(BATTERY_CRITICAL)).is_none(),
                "the first level we have is a seed, not a crossing"
            );
        }
    }

    /// The tooltip is assembled from pieces, and moving the pieces into `text` moved the
    /// seams with them: the tag, the particle before the Bluetooth clause, and the brackets
    /// around a reason. Those are what a restructuring can break without any single string
    /// looking wrong, so the finished sentences are written out here in full.
    #[test]
    fn the_tooltip_reads_as_one_sentence() {
        let on = |battery, charging, bt| State::On { battery, charging, bt };

        assert_eq!(State::Off.tooltip(Lang::Ja), "INZONE: ヘッドセット電源オフ");
        assert_eq!(State::Pairing.tooltip(Lang::Ja), "INZONE: ペアリング中");
        assert_eq!(
            State::HubBusy.tooltip(Lang::Ja),
            "INZONE: ポートが使用中（INZONE Hub を閉じると復帰します）"
        );
        assert_eq!(
            State::NoDongle { unidentified: 0 }.tooltip(Lang::Ja),
            "INZONE: レシーバーが見つかりません"
        );
        // A port we could not identify does not take the instruction away, it qualifies it.
        // The user still has a receiver to plug in; the rest says we may be the ones at
        // fault. Guards the whole sentence, because dropping the first half would leave a
        // tooltip that says nothing anyone can act on.
        let unsure = State::NoDongle { unidentified: 2 }.tooltip(Lang::Ja);
        assert!(unsure.starts_with("INZONE: レシーバーが見つかりません"), "{unsure}");
        assert!(unsure.contains("2 個"), "{unsure}");
        assert!(
            State::NoDongle { unidentified: 1 }
                .tooltip(Lang::En)
                .starts_with("INZONE: receiver not found")
        );

        // No Bluetooth clause: no particle either, or the sentence ends on a comma.
        assert_eq!(
            on(Some(72), false, None).tooltip(Lang::Ja),
            "INZONE: オン（バッテリー 72%）"
        );
        assert_eq!(on(Some(72), true, None).tooltip(Lang::Ja), "INZONE: オン（充電中 72%）");
        assert_eq!(on(None, true, None).tooltip(Lang::Ja), "INZONE: オン（充電中）");
        assert_eq!(on(None, false, None).tooltip(Lang::Ja), "INZONE: オン");

        // With one: the particle joins the two clauses and belongs to neither.
        assert_eq!(
            on(Some(72), false, Some(Bt::Connected)).tooltip(Lang::Ja),
            "INZONE: オン（バッテリー 72%）、Bluetooth 接続済み"
        );
        assert_eq!(
            on(None, false, Some(Bt::Off)).tooltip(Lang::Ja),
            "INZONE: オン、Bluetooth オフ"
        );
        assert_eq!(
            on(Some(5), true, Some(Bt::Unconnected)).tooltip(Lang::Ja),
            "INZONE: オン（充電中 5%）、Bluetooth オン（未接続）"
        );
        assert_eq!(
            on(Some(5), false, Some(Bt::Pairing)).tooltip(Lang::Ja),
            "INZONE: オン（バッテリー 5%）、Bluetooth ペアリング中"
        );

        // The reason is English inside a Japanese sentence, brackets and all.
        assert_eq!(
            State::Error(Cause::PortClosed).tooltip(Lang::Ja),
            "INZONE: エラー（port closed）"
        );

        // English has its own seams: a comma where Japanese has a particle, and half-width
        // brackets where Japanese has full-width ones. Written out for the same reason.
        assert_eq!(State::Off.tooltip(Lang::En), "INZONE: headset off");
        assert_eq!(on(Some(72), false, None).tooltip(Lang::En), "INZONE: On (battery 72%)");
        assert_eq!(on(None, false, None).tooltip(Lang::En), "INZONE: On");
        assert_eq!(
            on(None, false, Some(Bt::Off)).tooltip(Lang::En),
            "INZONE: On, Bluetooth off"
        );
        assert_eq!(
            on(Some(5), true, Some(Bt::Unconnected)).tooltip(Lang::En),
            "INZONE: On (charging, 5%), Bluetooth on, not connected"
        );
        assert_eq!(
            State::Error(Cause::PortClosed).tooltip(Lang::En),
            "INZONE: error (port closed)"
        );
    }

    /// A cause is diagnostic and stays English whatever language the tray is speaking, so
    /// the words it composes must not drift back into Japanese. Only the composed words:
    /// what it quotes is someone else's, and a Windows message on a Japanese install is
    /// Japanese by rights.
    #[test]
    fn a_cause_composes_only_english() {
        let quoted = "a message from somewhere else";
        let causes = [
            Cause::Dtr(quoted.into()),
            Cause::Enumerate(quoted.into()),
            Cause::Open(quoted.into()),
            Cause::Io(quoted.into()),
            Cause::PortClosed,
            Cause::NoReply {
                detail: Some(quoted.into()),
                unusable: 0,
            },
            Cause::NoReply {
                detail: Some(quoted.into()),
                unusable: 12,
            },
            Cause::NoReply {
                detail: None,
                unusable: 0,
            },
            Cause::NoReply {
                detail: None,
                unusable: 12,
            },
            Cause::EmptyReply,
            Cause::UnknownLink(0xAB),
            Cause::PollerStopped,
        ];
        for cause in causes {
            let described = cause.describe();
            assert!(
                described.is_ascii(),
                "{cause:?} composed something that is not English: {described}"
            );
            assert!(!described.is_empty(), "{cause:?} said nothing");
        }

        // And the other half of the line: a Windows message arrives already localized, so
        // it passes through untouched rather than being dropped for being Japanese.
        let from_windows = "アクセスが拒否されました。";
        assert_eq!(Cause::Open(from_windows.into()).describe(), from_windows);
    }

    /// Every cause the transport can hand us has to come out as a tooltip the shell will
    /// show whole. `szTip` holds 128 UTF-16 units and truncates without a word, so a
    /// Windows message long enough to spend the budget would cut off the sentence wrapped
    /// around it and leave the user with half a reason.
    #[test]
    fn a_long_reason_is_cut_before_the_shell_cuts_the_sentence() {
        /// The documented capacity of NOTIFYICONDATAW::szTip, terminator included.
        const SZTIP: usize = 128;
        // Three rulers disagree about how long a string is, so the oversized inputs cover
        // all three. Bytes: multi-byte, so a byte-wise cut would panic mid-character.
        // Characters: outside the basic plane, where one character is two UTF-16 units, so
        // counting characters would let twice the budget through. ASCII, as the baseline
        // where every ruler agrees.
        let long_bmp = "指定されたデバイスは応答していません。".repeat(20);
        // Every character here is outside the basic plane and so costs two UTF-16 units.
        // Mixing in a one-unit character would let the two rulers agree by accident, and a
        // truncation that counted characters would slip through.
        let long_astral = "🎧".repeat(200);
        assert_eq!(
            long_astral.encode_utf16().count(),
            long_astral.chars().count() * 2,
            "the astral sample has to be twice as long in units as in characters"
        );
        let long_ascii = "the specified device is not responding. ".repeat(20);

        let causes = [
            Cause::Dtr(long_bmp.clone()),
            // The longest fixed prefix any cause composes, so this is the one with the least
            // room left for what it quotes.
            Cause::Enumerate(long_bmp.clone()),
            Cause::Open(long_bmp.clone()),
            Cause::Io(long_bmp.clone()),
            Cause::Dtr(long_astral.clone()),
            Cause::Enumerate(long_astral.clone()),
            Cause::Open(long_astral.clone()),
            Cause::Io(long_astral.clone()),
            Cause::Enumerate(long_ascii.clone()),
            Cause::Open(long_ascii.clone()),
            Cause::PortClosed,
            Cause::NoReply {
                detail: Some(long_astral.clone()),
                unusable: 65535,
            },
            Cause::NoReply {
                detail: Some(long_bmp.clone()),
                unusable: 65535,
            },
            Cause::NoReply {
                detail: None,
                unusable: 65535,
            },
            Cause::NoReply {
                detail: None,
                unusable: 0,
            },
            Cause::EmptyReply,
            Cause::UnknownLink(0xAB),
            Cause::PollerStopped,
        ];
        // Both languages, because English says the same thing in more UTF-16 units than
        // Japanese does, and the budget is counted in those.
        for lang in [Lang::Ja, Lang::En] {
            for cause in &causes {
                let tooltip = State::Error(cause.clone()).tooltip(lang);
                let units = tooltip.encode_utf16().count();
                assert!(
                    units < SZTIP,
                    "{lang:?} {cause:?} renders {units} UTF-16 units, which the shell would cut"
                );
                assert!(tooltip.starts_with("INZONE: "), "{lang:?} {cause:?} lost its prefix");
                // Every cause has to say something. Both bracket shapes are checked against
                // both languages: a mix-up would show as an empty pair either way.
                assert!(
                    !tooltip.contains("（）") && !tooltip.contains("()"),
                    "{lang:?} {cause:?} left the brackets empty: {tooltip}"
                );
            }
        }

        // Short reasons are passed through untouched, so the truncation only ever costs
        // something in the case it exists for.
        let short = "アクセスが拒否されました。";
        assert!(
            State::Error(Cause::Open(short.into()))
                .tooltip(Lang::Ja)
                .contains(short)
        );
    }

    /// Not understanding a reply has to read as "unknown", never as a confident value.
    #[test]
    fn unreadable_payloads_never_become_confident_answers() {
        assert_eq!(interpret_battery(&[BATT_DISCHARGING, 70]), (Some(70), false));
        assert_eq!(interpret_battery(&[BATT_CHARGING, 70]), (Some(70), true));
        // The 0xC8 the dongle really sends while charging.
        assert_eq!(interpret_battery(&[BATT_CHARGING, 200]), (None, true));
        // BATTERY_STATUS::ERROR means the device distrusts its own reading, so we do too.
        assert_eq!(interpret_battery(&[0x02, 70]), (None, false));
        assert_eq!(interpret_battery(&[BATT_CHARGING]), (None, false));
        assert_eq!(interpret_battery(&[]), (None, false));

        assert_eq!(interpret_bt(&[BT_OFF, BT_CONN_UNCONNECTED]), Some(Bt::Off));
        // Deliberate: "Bluetooth is off" is complete on its own, so one byte is enough.
        // The connection state cannot add anything once the radio is off.
        assert_eq!(interpret_bt(&[BT_OFF]), Some(Bt::Off));
        assert_eq!(interpret_bt(&[BT_ON, BT_CONN_CONNECTED]), Some(Bt::Connected));
        assert_eq!(interpret_bt(&[BT_ON, BT_CONN_PAIRING]), Some(Bt::Pairing));
        assert_eq!(interpret_bt(&[BT_ON, BT_CONN_UNCONNECTED]), Some(Bt::Unconnected));
        // A single byte saying "on" must never be reported as off.
        assert_eq!(interpret_bt(&[BT_ON]), None);
        assert_eq!(interpret_bt(&[BT_ON, 0x77]), None, "unknown connection state");
        assert_eq!(interpret_bt(&[0x55, 0x01]), None, "unknown bt state");
        assert_eq!(interpret_bt(&[]), None);
    }
}

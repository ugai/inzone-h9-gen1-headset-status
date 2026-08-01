//! Drawing the tray icon from the state's color, shape and badge, at the size the shell
//! actually draws it.

use tray_icon::Icon;

use crate::state::Badge;

/// The canvas every length below is written for. Also the fallback size, and the size the
/// shell asks for at 200% scaling.
const REF_N: i32 = 32;

/// Disc radius. The badged variant is smaller so the mark has somewhere to sit.
const OUTER: f32 = 15.0;
const OUTER_BADGED: f32 = 13.5;
/// Width of the darkened rim that outlines the disc against a taskbar of any color.
const RIM: f32 = 2.0;
/// Inner radius of the ring shape, used for the states where we have no real answer.
const HOLE: f32 = 9.0;
/// Badge center, chosen so the mark sits in the lower-right corner without leaving the box.
const BADGE_C: f32 = 23.0;
const BADGE_R: f32 = 7.0;
/// Transparent moat between badge and disc. It is what makes the badge legible against a
/// taskbar of any color, so `badge_keeps_a_transparent_moat` checks it at every size.
const BADGE_GAP: f32 = 2.5;
/// Inner radius of the hollow badge.
const BADGE_HOLE: f32 = 3.5;

pub const BT_BLUE: [u8; 3] = [0x2E, 0x9B, 0xFF];

/// Tells Windows this process understands display scaling, and reports whether the process
/// is DPI-aware afterwards, however it got that way.
///
/// This has to happen before anything reads `SM_CXSMICON`, because Windows tells a
/// DPI-unaware process 16 at every scaling factor. Believing that at 150% would render a
/// 16 px icon into the 24 px slot the shell has, which is worse than the fixed 32 it
/// replaces — so the metric is only trusted once this returns true.
///
/// The return value of the set call is deliberately ignored. It fails with
/// `ERROR_ACCESS_DENIED` when awareness has already been set, by a manifest or by the
/// shell's high-DPI compatibility options, and in that case the process is aware and the
/// metric is perfectly good. Asking what we actually ended up with answers the question
/// that matters; the set call's own verdict does not.
///
/// Per-monitor v2 rather than system-aware: a system-aware process keeps the DPI it
/// started with, which would leave `tray_icon_size` frozen and the re-render on a scaling
/// change permanently dead. System-aware is still trusted if that is what we inherit,
/// because its `SM_CXSMICON` is right at startup even though it never moves.
pub fn declare_dpi_aware() -> bool {
    use windows_sys::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, DPI_AWARENESS_PER_MONITOR_AWARE, DPI_AWARENESS_SYSTEM_AWARE,
        GetAwarenessFromDpiAwarenessContext, GetThreadDpiAwarenessContext, SetProcessDpiAwarenessContext,
    };
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let awareness = GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext());
        // Anything else is UNAWARE or INVALID, and neither can be believed.
        matches!(awareness, DPI_AWARENESS_SYSTEM_AWARE | DPI_AWARENESS_PER_MONITOR_AWARE)
    }
}

/// Edge length to render at, in pixels: what the shell will draw a tray icon at, so the
/// bitmap goes up without a resample. Measured at 24 on a 150% display and 32 at 200%.
///
/// `dpi_aware` is the result of `declare_dpi_aware()`. Without it the metric is a fixed
/// 16 and worth nothing, and the reference size is the better guess.
///
/// The DPI comes from the monitor rather than from the process, because the process-wide
/// answers do not move when the user changes the scaling. Measured across a live change
/// from 150% to 200% and back, with the process per-monitor v2 aware throughout:
///
/// ```text
///                                       150%   200%   150%
///   GetSystemMetrics(SM_CXSMICON)         24     24     24
///   GetDpiForSystem()                    144    144    144
///   GetDpiForMonitor(primary)            144    192    144
///   GetSystemMetricsForDpi(.., monitor)   24     32     24
/// ```
///
/// Only the last row tracks it. This used to read the first, which is why the icon stayed
/// coarse until the app was restarted, and why the re-render in the message loop could
/// never have fired.
///
/// The primary monitor, not the one the taskbar happens to be on. With two monitors at
/// different scalings and the taskbar on the secondary, this reads the wrong one; there is
/// no such setup to test against, and guessing at a fix that cannot be checked is worse
/// than a limitation that is written down.
pub fn tray_icon_size(dpi_aware: bool) -> i32 {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint};
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};

    if !dpi_aware {
        return REF_N;
    }
    let n = unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let (mut dpi, mut unused) = (0u32, 0u32);
        // S_OK is 0. A monitor that will not tell us its DPI still leaves the fixed metric,
        // which is right at startup even though it never moves afterwards.
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused) == 0 {
            GetSystemMetricsForDpi(SM_CXSMICON, dpi)
        } else {
            GetSystemMetrics(SM_CXSMICON)
        }
    };
    // 0 comes back on failure, and a wild value would cost real pixels for no gain. 16 is
    // what 100% scaling asks for and the smallest size the geometry is tested at, so
    // anything under it is a metric we have no reason to believe.
    if (16..=256).contains(&n) { n } else { REF_N }
}

/// `n` x `n` RGBA circle with a darkened rim, so the shape reads against a light taskbar as
/// well as a dark one. Anti-aliased by sampling each pixel's distance from the center,
/// which is enough at tray size and needs no image crate.
///
/// Rebuilt from scratch on every state change. That is a few thousand f32 ops a handful of
/// times a day, so caching the possible icons would save nothing measurable.
pub fn make_icon(n: i32, color: [u8; 3], hollow: bool, badge: Badge, battery: Option<u8>) -> Icon {
    let rgba = icon_rgba(n, color, hollow, badge, battery);
    Icon::from_rgba(rgba, n as u32, n as u32).expect("icon")
}

fn icon_rgba(n: i32, color: [u8; 3], hollow: bool, badge: Badge, battery: Option<u8>) -> Vec<u8> {
    // Every length scales with the canvas, so the 32 px rendering is unchanged to the byte.
    // The `clamp(0.0, 1.0)` anti-aliasing ramps are deliberately left alone: an edge is one
    // pixel wide whatever the icon measures.
    let s = n as f32 / REF_N as f32;
    let center = (n as f32 - 1.0) / 2.0;
    let outer = if badge == Badge::None { OUTER } else { OUTER_BADGED } * s;
    let (rim_w, hole_r) = (RIM * s, HOLE * s);
    let (badge_c, badge_r) = (BADGE_C * s, BADGE_R * s);
    let (badge_moat, badge_hole) = ((BADGE_R + BADGE_GAP) * s, BADGE_HOLE * s);
    let rim: [u8; 3] = std::array::from_fn(|i| (color[i] as f32 * 0.45) as u8);

    let mut rgba = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = (x as f32 - center, y as f32 - center);
            let d = (dx * dx + dy * dy).sqrt();
            let bd = ((x as f32 - badge_c).powi(2) + (y as f32 - badge_c).powi(2)).sqrt();

            // Outer edge gives the alpha; the fill mask decides rim color vs body color.
            // The inner cut only applies to the ring shape, otherwise the exact center of a
            // solid disc comes out semi-transparent.
            let hole = if hollow { (d - hole_r).clamp(0.0, 1.0) } else { 1.0 };
            let mut alpha = (outer - d).clamp(0.0, 1.0) * hole;
            let fill = (outer - rim_w - d).clamp(0.0, 1.0);

            // Charge fills clockwise from the top; the spent sector drops to the rim color
            // so the level reads as a shape, not only as a hue.
            let body = match battery {
                Some(pct) => {
                    let turn = dx.atan2(-dy).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
                    if turn > pct as f32 / 100.0 { rim } else { color }
                }
                None => color,
            };
            let mut px: [u8; 3] = std::array::from_fn(|i| (rim[i] as f32 + (body[i] - rim[i]) as f32 * fill) as u8);

            if badge != Badge::None {
                alpha *= (bd - badge_moat).clamp(0.0, 1.0);
                let hole = if badge == Badge::Hollow {
                    (bd - badge_hole).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                let ba = (badge_r - bd).clamp(0.0, 1.0) * hole;
                if ba > 0.0 {
                    px = std::array::from_fn(|i| (px[i] as f32 * (1.0 - ba) + BT_BLUE[i] as f32 * ba) as u8);
                    alpha = alpha.max(ba);
                }
            }
            rgba.extend_from_slice(&[px[0], px[1], px[2], (alpha * 255.0) as u8]);
        }
    }
    rgba
}

/// A PNG writer, for the README's sample icons only.
///
/// Hand-rolled rather than pulled in, because the dependency list is capped at three and a
/// dev-dependency would still be a fourth crate for anyone auditing the tree. It costs
/// nothing at runtime: the whole module is `cfg(test)`, so it is never in the binary.
///
/// The compressor is the one deflate mode that needs no compressor: stored blocks. A 48 px
/// icon is nine kilobytes and these are written by hand a few times a year.
#[cfg(test)]
mod png {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    0xEDB8_8320 ^ (crc >> 1)
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn adler32(data: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for &byte in data {
            a = (a + byte as u32) % 65521;
            b = (b + a) % 65521;
        }
        (b << 16) | a
    }

    fn chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = (body.len() as u32).to_be_bytes().to_vec();
        let mut checked = kind.to_vec();
        checked.extend_from_slice(body);
        out.extend_from_slice(&checked);
        out.extend_from_slice(&crc32(&checked).to_be_bytes());
        out
    }

    /// 8-bit RGBA, no interlacing, every row filtered as "none".
    ///
    /// The preconditions are asserted rather than assumed. `chunks_exact` drops a partial
    /// tail without a word, so a buffer that is not exactly `n` rows would quietly produce
    /// a PNG whose IHDR promises more than its IDAT delivers, and the failure would surface
    /// as a broken picture in the README rather than as a red test.
    pub fn encode(n: i32, rgba: &[u8]) -> Vec<u8> {
        assert!(n > 0, "an icon has to have pixels, got {n}");
        let n = n as usize;
        let stride = n * 4;
        assert_eq!(rgba.len(), n * stride, "the buffer has to be exactly {n} x {n} RGBA");

        let mut raw = Vec::with_capacity(n * (1 + stride));
        for row in rgba.chunks_exact(stride) {
            raw.push(0);
            raw.extend_from_slice(row);
        }

        const BLOCK: usize = 65_535;
        let blocks = raw.len().div_ceil(BLOCK);
        let mut zlib = vec![0x78, 0x01];
        for (i, block) in raw.chunks(BLOCK).enumerate() {
            zlib.push(u8::from(i + 1 == blocks));
            zlib.extend_from_slice(&(block.len() as u16).to_le_bytes());
            zlib.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
            zlib.extend_from_slice(block);
        }
        zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

        let mut header = (n as u32).to_be_bytes().to_vec();
        header.extend_from_slice(&(n as u32).to_be_bytes());
        header.extend_from_slice(&[8, 6, 0, 0, 0]);

        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        out.extend(chunk(b"IHDR", &header));
        out.extend(chunk(b"IDAT", &zlib));
        out.extend(chunk(b"IEND", &[]));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every size the shell can ask for, plus the fallback. 16 is 100% scaling, 24 is 150%
    /// and 32 is 200%; 48 stands in for anything beyond. The geometry has to hold at all of
    /// them, because the icon is now drawn at whichever one is current.
    const SIZES: [i32; 4] = [16, 24, REF_N, 48];

    /// Fraction of the disc still wearing the state color rather than the dim rim.
    /// Sampled well inside the outer edge, so the rim band that outlines every icon is
    /// not counted as spent charge.
    fn lit_fraction(rgba: &[u8], n: i32, color: [u8; 3], rim: [u8; 3]) -> f32 {
        let center = (n as f32 - 1.0) / 2.0;
        let sample_r = 11.0 * n as f32 / REF_N as f32;
        let (mut lit, mut total) = (0u32, 0u32);
        for y in 0..n {
            for x in 0..n {
                if ((x as f32 - center).powi(2) + (y as f32 - center).powi(2)).sqrt() > sample_r {
                    continue;
                }
                let px = &rgba[((y * n + x) * 4) as usize..][..3];
                total += 1;
                let near = |t: [u8; 3]| (0..3).map(|i| (px[i] as i32 - t[i] as i32).pow(2)).sum::<i32>();
                if near(color) < near(rim) {
                    lit += 1;
                }
            }
        }
        lit as f32 / total.max(1) as f32
    }

    /// The parts of the size decision that can be checked without a display to rescale.
    /// That the monitor DPI is the one that moves is a measurement, written down above
    /// `tray_icon_size`, not something a test on one machine can demonstrate.
    #[test]
    fn the_tray_size_falls_back_rather_than_guessing() {
        assert_eq!(
            tray_icon_size(false),
            REF_N,
            "a process that is not DPI aware is told 16 at every scaling factor, so it has to \
             fall back rather than believe it"
        );
        let n = tray_icon_size(declare_dpi_aware());
        assert!(
            (16..=256).contains(&n),
            "a size outside the believable range should have become the fallback, got {n}"
        );
    }

    /// The whole point of the arc: charge has to be readable without hue. Green and amber
    /// sit at a contrast ratio of 1.06, and red and the powered-off gray at 1.10, so a
    /// hue-only icon collapses exactly where it matters most.
    #[test]
    fn charge_reads_as_shape_not_only_hue() {
        let green = [0x22, 0xC5, 0x5E];
        let rim: [u8; 3] = std::array::from_fn(|i| (green[i] as f32 * 0.45) as u8);

        for n in SIZES {
            let at = |pct| lit_fraction(&icon_rgba(n, green, false, Badge::None, Some(pct)), n, green, rim);

            let (full, half, low) = (at(100), at(50), at(10));
            assert!(full > 0.95, "at {n} px a full battery must fill the disc, got {full}");
            assert!(
                (half - 0.5).abs() < 0.08,
                "at {n} px half should light about half the disc, got {half}"
            );
            assert!(low < 0.2, "at {n} px 10% must look nearly spent, got {low}");
            assert!(full > half && half > low, "at {n} px the arc must shrink monotonically");

            // The collision this replaces: 10% charge versus a powered-off headset. Both are
            // dark discs under hue-only encoding; they must differ in shape.
            let gray = [0x8A, 0x8A, 0x94];
            let off = icon_rgba(n, gray, false, Badge::None, None);
            let off_rim: [u8; 3] = std::array::from_fn(|i| (gray[i] as f32 * 0.45) as u8);
            assert!(
                lit_fraction(&off, n, gray, off_rim) > 0.95,
                "at {n} px powered off must stay a whole disc, or it reads as a flat battery"
            );
        }
    }

    /// The badge is only legible if the transparent moat around it actually survives.
    /// A geometry change that swallows the gap makes the badge merge into the disc, and so
    /// does a size the moat is too thin to survive: at 16 px it is 1.25 px wide.
    #[test]
    fn badge_keeps_a_transparent_moat() {
        for n in SIZES {
            let alpha_at = |rgba: &[u8], x: i32, y: i32| rgba[((y * n + x) * 4 + 3) as usize];
            let s = n as f32 / REF_N as f32;
            let (badge_c, center) = (BADGE_C * s, (n as f32 - 1.0) / 2.0);
            let bc = badge_c.round() as i32;
            let cc = center.round() as i32;

            let plain = icon_rgba(n, [0x22, 0xC5, 0x5E], false, Badge::None, None);
            let badged = icon_rgba(n, [0x22, 0xC5, 0x5E], false, Badge::Filled, None);

            assert_eq!(
                alpha_at(&badged, bc, bc),
                255,
                "at {n} px the badge center must be opaque"
            );
            // Walk the diagonal from the badge center to the disc center. Both ends are
            // opaque, so a fully clear pixel anywhere between them is the moat.
            let steps = 4 * n;
            let moat = (0..=steps).any(|i| {
                let t = i as f32 / steps as f32;
                let p = (badge_c + (center - badge_c) * t).round() as i32;
                alpha_at(&badged, p, p) == 0
            });
            assert!(moat, "at {n} px no transparent pixel between badge and disc");
            assert_eq!(
                alpha_at(&plain, cc, cc),
                255,
                "at {n} px the disc center must stay opaque"
            );
        }
    }

    /// Writes the README's sample icons, straight from the states they document:
    ///   cargo test -- --ignored readme_icons
    ///
    /// Generated rather than screenshotted, so the pictures cannot drift away from the
    /// code. If a color or a radius changes, rerun this and the table is right again.
    ///
    /// 48 px rather than the 16 to 32 the shell draws. The geometry is the same either way,
    /// and at tray size a reader cannot see what the table is pointing at.
    #[test]
    #[ignore]
    fn readme_icons() {
        use crate::state::{Bt, State};

        const N: i32 = 48;
        let on = |battery, charging, bt| State::On { battery, charging, bt };
        let samples: [(&str, State); 16] = [
            ("green", on(Some(80), false, None)),
            ("amber", on(Some(30), false, None)),
            ("red", on(Some(10), false, None)),
            ("charging-green", on(Some(70), true, None)),
            ("charging-amber", on(Some(30), true, None)),
            ("charging-red", on(Some(10), true, None)),
            ("pairing", State::Pairing),
            ("off", State::Off),
            ("hub-busy", State::HubBusy),
            ("no-dongle", State::NoDongle { unidentified: 0 }),
            ("badge-filled", on(Some(80), false, Some(Bt::Connected))),
            ("badge-hollow", on(Some(80), false, Some(Bt::Unconnected))),
            ("arc-100", on(Some(100), false, None)),
            ("arc-70", on(Some(70), false, None)),
            ("arc-40", on(Some(40), false, None)),
            ("arc-10", on(Some(10), false, None)),
        ];

        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("docs")
            .join("icons");
        std::fs::create_dir_all(&dir).expect("docs/icons");
        for (name, state) in samples {
            let rgba = icon_rgba(N, state.color(), state.hollow(), state.badge(), state.battery_arc());
            let path = dir.join(format!("{name}.png"));
            std::fs::write(&path, png::encode(N, &rgba)).expect("write the sample");
            println!("{}", path.display());
        }
    }

    /// The hand-rolled encoder has to produce something a decoder will actually take, and
    /// the parts most likely to be wrong are silent: a bad CRC, a stored block whose length
    /// and its complement disagree, an Adler sum over the wrong bytes. This pins the header
    /// and the framing; `readme_icons` output opening in a viewer is the rest of the proof.
    #[test]
    fn the_png_encoder_frames_its_chunks() {
        let n = 16;
        let rgba = icon_rgba(n, [0x22, 0xC5, 0x5E], false, Badge::Filled, Some(50));
        let png = png::encode(n, &rgba);

        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        // Length, type, then width and height, then 8-bit RGBA with no interlacing.
        assert_eq!(&png[8..16], &[0, 0, 0, 13, b'I', b'H', b'D', b'R']);
        assert_eq!(&png[16..24], &(n as u32).to_be_bytes().repeat(2)[..]);
        assert_eq!(&png[24..29], &[8, 6, 0, 0, 0]);
        assert_eq!(
            &png[png.len() - 12..],
            &[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]
        );

        // Walk the chunks the way a decoder does: every length has to land exactly on the
        // next one, or a CRC we computed over the wrong span would go unnoticed.
        let mut at = 8;
        let mut kinds = Vec::new();
        while at < png.len() {
            let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            kinds.push(String::from_utf8(png[at + 4..at + 8].to_vec()).unwrap());
            at += 12 + len;
        }
        assert_eq!(at, png.len(), "the chunk lengths must tile the file exactly");
        assert_eq!(kinds, ["IHDR", "IDAT", "IEND"]);
    }

    /// A buffer that is not exactly n x n RGBA has to stop the encoder, not get silently
    /// trimmed by `chunks_exact` into a file whose header promises rows that are not there.
    #[test]
    #[should_panic(expected = "exactly")]
    fn the_png_encoder_refuses_a_buffer_of_the_wrong_size() {
        let mut rgba = icon_rgba(16, [0x22, 0xC5, 0x5E], false, Badge::None, None);
        rgba.pop();
        png::encode(16, &rgba);
    }

    /// Prints the icons as text so the geometry can be eyeballed without a screenshot:
    ///   cargo test -- --ignored --nocapture badge_art
    #[test]
    #[ignore]
    fn badge_art() {
        for n in SIZES {
            for (label, badge) in [
                ("none", Badge::None),
                ("hollow", Badge::Hollow),
                ("filled", Badge::Filled),
            ] {
                println!("\n=== {n} px, {label} ===");
                let rgba = icon_rgba(n, [0x22, 0xC5, 0x5E], false, badge, None);
                for y in 0..n {
                    let row: String = (0..n)
                        .map(|x| {
                            let i = ((y * n + x) * 4) as usize;
                            let (a, blue) = (rgba[i + 3], rgba[i + 2] > rgba[i + 1]);
                            match (a, blue) {
                                (0, _) => ' ',
                                (1..=127, _) => '.',
                                (_, true) => 'B',
                                _ => '#',
                            }
                        })
                        .collect();
                    println!("{row}");
                }
            }
        }
    }
}

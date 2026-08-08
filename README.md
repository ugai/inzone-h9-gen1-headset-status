# inzone-h9-gen1-headset-status

[日本語](README.ja.md)

## Overview

A small Windows tray utility for the Sony INZONE H9 and H7 wireless headsets (1st generation).

The headset connects through a USB receiver and stays present as an audio device even when it is switched off, so Windows itself cannot tell you whether the headset is powered on.
The official companion app, INZONE Hub, knows, but you have to open its window to find out.

This utility reads the same state from the receiver and shows it as a colored tray icon.

![Tray icon and menu](docs/screenshots/tray-en.avif)

## Unsupported models

The INZONE H9 II is not supported, and there is no plan to support it. Its transceiver is not detected, and the tray stays in the no-receiver state.

## Usage

Run it with `--help`.

## Installing

Download the exe from [Releases](https://github.com/ugai/inzone-h9-gen1-headset-status/releases) and put it wherever you like. There is also an installer (`-setup.exe`), which installs for the current user and keeps the startup setting working across upgrades.

You can check the hash of a downloaded exe:

```powershell
Get-FileHash .\inzone-h9-gen1-headset-status-*.exe -Algorithm SHA256
```

An intact download matches the value in the `.sha256` file of the same name in the release. Both the exe and the installer have one.

That says the file arrived whole, not where it came from. For the second, releases carry a build attestation, which [GitHub CLI](https://cli.github.com/) checks against the workflow that produced them:

```powershell
gh attestation verify .\inzone-h9-gen1-headset-status-<version>.exe -R ugai/inzone-h9-gen1-headset-status
```

The exe is not signed, so the first launch raises a SmartScreen warning saying Windows protected your PC. Click **More info** and then **Run anyway** to start it.

## Uninstalling

Deleting the exe leaves two things behind:

- the settings file, under `%APPDATA%`
- the startup entry in the registry

[Settings locations](#settings-locations) gives both paths.

## Building

Install the Rust toolchain and run:

```bash
cargo build --release
```

## Conflict with INZONE Hub

Only one program at a time can talk to the USB receiver, so this utility cannot read the state while INZONE Hub is running.
It holds the port for as little time as it can, so that the Hub can still start.

> [!NOTE]
> Settings such as the equalizer appear to be kept without the Hub running.
> Unless you are changing settings or turning on one of its extra features, the Hub does not need to stay open.

## Footprint

One executable, about 300 KiB on disk, and a few MB of memory while it runs.

> [!NOTE]
> CI checks the size with the `.github/check-size.ps1` script.

## Network use

It does not connect to any network. Nothing is sent about how you use it, and it never checks for updates on its own.
Reading the state needs nothing beyond the USB connection to the receiver.

## Tray icon states

The color and the shape say what the headset is doing.
It is a filled disc normally, and a ring when something is wrong.

| Icon | Color | State |
|:---:|---|---|
| ![green disc](docs/icons/green.png) | green | on |
| ![amber disc](docs/icons/amber.png) | amber | on, battery at 35% or below |
| ![red disc](docs/icons/red.png) | red | on, battery at 15% or below |
| ![blue disc](docs/icons/pairing.png) | blue | pairing |
| ![gray disc](docs/icons/off.png) | gray | off |
| ![violet ring](docs/icons/hub-busy.png) | violet (ring) | INZONE Hub is using the device |
| ![dim gray ring](docs/icons/no-dongle.png) | dim gray (ring) | no receiver, or an error |

Charging lightens the color. A full charge brings it back to plain green.

| ![charging, green](docs/icons/charging-green.png) | ![charging, amber](docs/icons/charging-amber.png) | ![charging, red](docs/icons/charging-red.png) |
|:---:|:---:|:---:|
| charging, above 35% | charging, 35% or below | charging, 15% or below |

While the headset is on, the disc fills with the charge that is left.

| ![100% left](docs/icons/arc-100.png) | ![70% left](docs/icons/arc-70.png) | ![40% left](docs/icons/arc-40.png) | ![10% left](docs/icons/arc-10.png) |
|:---:|:---:|:---:|:---:|
| 100% | 70% | 40% | 10% |

A blue badge in the bottom right corner shows the Bluetooth state.

| Icon | Badge | State |
|:---:|---|---|
| ![filled badge](docs/icons/badge-filled.png) | filled disc | Bluetooth connected |
| ![hollow badge](docs/icons/badge-hollow.png) | ring | Bluetooth on, not connected or pairing |
| ![no badge](docs/icons/green.png) | none | Bluetooth off, or no answer |

> [!NOTE]
> Green against amber is a contrast ratio of 1.06, and red against the powered-off gray is 1.10, so the colors are hard to tell apart on their own. That is why the charge also fills an arc, which follows WCAG 2.2 SC 1.4.1 (Use of Color).
> Contrast against the background (SC 1.4.11) has not been checked, since it moves with the taskbar color and the theme.

> [!NOTE]
> The sample icons above come from the drawing code itself. Regenerate them with:
>
> ```bash
> cargo test -- --ignored readme_icons
> ```
>
> They are drawn at 48px to stay legible here. In the notification area the real size is 16px to 32px.

## Languages

Japanese and English, following the Windows display language. The settings file can override it.
Some diagnostic text stays English whichever language is in use, so a Japanese interface shows lines like `INZONE: エラー（no reply）`.

## Display scaling

The icon is drawn at the size Windows asks for, which follows the scaling factor: 24px at 150%, 32px at 200%, and so on. The size comes from the monitor's DPI, and a change made while the app is running is picked up.

## Notifications

There are two.

| Notification | When |
|---|---|
| Headset on/off | The headset is switched on or off, or starts pairing |
| Low battery | The charge reaches 35% or below, and 15% or below |

Either can be switched off on its own from the context menu.

The low-battery notification fires once as the charge crosses a threshold, and not again until the charge has recovered by some margin.

The on/off notification does not fire across a stretch where the power state could not be read. Switch the headset off while INZONE Hub is running, and closing the Hub turns the icon gray without announcing anything.

The low-battery notification names the charge as it is now, so it does fire across such a stretch. The first reading after launch is the exception, and is never announced even when it is already below a threshold.

> [!NOTE]
> The notification path can be checked with:
>
> ```bash
> cargo run --release -- --test-toast
> ```

## Settings locations

### Notification settings

The notification switches in the context menu are kept in:

```
%APPDATA%\inzone-h9-gen1-headset-status\settings.conf
```

It is plain `key=value` text. Delete it and these defaults apply:

```
notify_power=on
notify_battery=on
language=auto
```

`language` takes `auto`, `ja` or `en`. `auto` follows the Windows display language, which means English anywhere other than a Japanese installation. A change takes effect at the next start.

### Startup

The context menu can start the app at logon. It registers here, and should also show up under Settings > Apps > Startup:

```
HKCU\Software\Microsoft\Windows\CurrentVersion\Run
  inzone-h9-gen1-headset-status = "<full path to the exe>"
```

## Polling interval

The state is read every 15 seconds.

A missing receiver or a run of errors stretches that to 30, 60 and then 120 seconds. It stays at 15 while INZONE Hub holds the port, so closing the Hub brings the display back at once.

Replugging the receiver reads it straight away and puts the interval back to 15 seconds.
Windows announces serial ports arriving and leaving, so waking from sleep should prompt the same re-read.

For a few seconds after waking the icon stays gray while the headset reconnects. Wait and it goes back to its usual state, green if the headset is on.

## Other behavior

- Launching a second copy shows a message and exits, so only one instance runs
- Check for updates in the context menu opens the releases page in your default browser

## License

[0BSD](LICENSE)

Modify and redistribute it freely. It comes with no warranty, and you use it at your own risk.

### Notes

Two things the license does not cover.

- Sony and INZONE are trademarks of Sony. This project is unofficial and unaffiliated, and the names appear here only to say which product it works with
- No claim is made over the protocol itself

The dependencies are under their own licenses:

- serialport: [MPL-2.0](https://github.com/serialport/serialport-rs/blob/main/LICENSE.txt)
- tray-icon: [Apache-2.0 or MIT](https://github.com/tauri-apps/tray-icon/blob/dev/LICENSE.spdx)
- windows-sys: [Apache-2.0](https://github.com/microsoft/windows-rs/blob/master/license-apache-2.0) or [MIT](https://github.com/microsoft/windows-rs/blob/main/license-mit)

## Tested environments

This version has been checked with an INZONE H9 (1st generation), one particular receiver (`054C:0E53`), and Windows 11 25H2.

Nothing else has been tried.

These are implemented, but the conditions to see them on real hardware have not been available:

- an INZONE H7
- picking the language automatically on a non-Japanese Windows (`language=en` in `settings.conf` has been checked)
- the pairing display, meaning the blue disc and the hollow badge
- how accurate the low-charge display and notifications are (amber, red, the warning)

A firmware update to the receiver or the headset could change the shape of the reply and leave the state unreadable.

## Known limitations

Seen, and accepted on purpose.

- INZONE Hub can fail to take the port when it starts at the moment this utility is holding it (a few hundred milliseconds once every 15 seconds, or up to about 8 seconds when the headset does not answer)
- With different scaling on each monitor and the taskbar on the secondary one, the icon comes out the wrong size
- Switching startup off in Windows Settings does not clear the tick in the context menu

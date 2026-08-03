# CLAUDE.md

A Rust tray app that shows the power state of a Sony INZONE H9 / H7 headset.
First generation only: the H9 II and every other second-generation model talks HID, not COM.
[README.md](README.md) is for people using the app, and [README.ja.md](README.ja.md) is the
same document in Japanese.
This file carries only what breaks if you skip it.

**The two READMEs are one document in two languages, section for section.** A change to
either is not finished until the other says the same thing. There is no rule about which
details each one carries, on purpose: a rule with a judgment in it gets a different answer
every time, where "translate it" gets the same answer forever. English is `README.md`
because that is the file GitHub puts in front of everyone.

**Do not add notes on where the protocol came from, to this file or to any other.** The
constants in `protocol` are what the app needs and they stay; how anyone arrived at them is
not this repository's business, and a comment that reaches for it is the one thing to take
back out in review.

## Do not brick the dongle

**Always assert DTR before writing to the COM port.**
Writing without it makes the dongle's firmware stop reading its OUT endpoint.
Every subsequent write then fails with `ERROR_SEM_TIMEOUT` (os error 121), and a driver
reset does not clear it. Only unplugging and replugging the receiver does, which means
asking the user to touch hardware. This has actually happened once.
The `write_data_terminal_ready(true)` and the 50 ms settle in `poll()` are load-bearing, and
a failure there must abandon the exchange *before* anything is written. Do not soften it back
to `.ok()`: a missed reading costs 15 seconds, a wedged dongle costs a trip to the hardware.
This binds every site that writes, `raw_exchange()` in the tests included, since that is the
one that gets run casually while iterating.

**Never write to HID vendor collection `0xFF03` (reports `0xA0` / `0xA1`).**
That collection is a firmware-update surface, so a stray write there is not a failed poll,
it is a headset that may not come back. The app never opens it, and nothing should.

**The HCI opcode is `0xFC00`.**
`0xFFF3` is the over-the-air transport opcode and must not wrap ordinary commands. It reads
like the right one and was nearly used once, which is why it is written down here.

**Do not send a byte sequence you have not confirmed.** Every frame in `protocol` has been
seen answered by the real dongle. Anything new has to be watched working on hardware before
it goes in, not reasoned into place.

## Where things live

`protocol` is the frame format and the payload vocabulary, and `transport` is the port: it
finds the dongle, runs one exchange and turns the reply into a `State`. `state` holds that
belief and everything read off it (tooltip, color, badge, arc, and the two notification
decisions). `icon` draws it, `notify` sends it through the tray icon.
`devices` is the COM-port arrival watch, `settings` the `%APPDATA%` preferences file,
`startup` the HKCU `Run` entry, `cli` the argument handling and the print-from-a-GUI-process
trick. `main` is the single-instance claim, the poller thread and the message loop.

## The installer

`packaging/installer.nsi` builds a per-user installer beside the portable exe, because the
`Run` entry holds a full path and the released file name carries its version, so that path
stops naming anything the moment the next version lands. The portable exe stays the main way
to run this and nothing here is required.

**No "start with Windows" checkbox in the installer.** The tray menu owns that switch and
says so when the registry refuses it. A checkbox in an installer re-asserts itself at every
upgrade, so somebody who turned startup off would find it back on after an update they ran
for an unrelated reason.

**The uninstaller deletes the `Run` value only when it names the exe inside `$INSTDIR`.** A
portable copy writes the same value under the same name, and removing the installed copy must
not stop that one from starting. Deleting it unconditionally is the tempting simplification.
Not deleting it at all is worse still: it would leave an entry naming an exe that no longer
exists, which is the failure this installer was written for.

**Find a running instance through the mutex, never by process name.** A running exe cannot be
replaced or deleted, so both halves stop before touching anything. The name is the thing that
moves (`...-v0.1.0.exe` downloaded, `...exe` once installed), so a name match finds one and
misses the other. `OpenMutexW` on `Local\inzone-h9-gen1-headset-status` finds it either way.
Measured with the app running, with it stopped, and against a name that exists nowhere.

**The `.nsi` needs a UTF-8 BOM.** Without one NSIS reads it as the system code page and dies
on the Japanese strings with `Bad text encoding`. `Unicode true` is about the installer being
built, not about the source being read.

**An uninstaller's exit code says nothing.** NSIS copies it to `%TEMP%` and re-runs it there,
so the process you waited on returns 0 whether or not anything happened. Pass `_?=$INSTDIR`
to keep it in place and synchronous, which is the only way to test it; that form then cannot
delete itself, so the leftover `uninstall.exe` is expected rather than a bug.

## Running the tests

```bash
cargo test                                     # unit tests only, no hardware
cargo test -- --ignored --nocapture            # real dongle, INZONE Hub must be closed
cargo run --release -- --test-toast            # verify the notification path
cargo test -- --ignored --nocapture badge_art  # print the icon geometry as text
cargo test -- --ignored readme_icons           # regenerate the README's icon PNGs
cargo build --release && pwsh -File .github/check-size.ps1   # the size gate CI enforces
```

`readme_icons` writes `docs/icons/*.png` from the real drawing code, so a color or a radius
change is not finished until it has been rerun and the PNGs committed.

**Hardware tests need INZONE Hub closed.** The Hub holds the COM port exclusively for as
long as it runs: 30 open attempts over 27 seconds were all refused, so there is no gap to
slip through. Ask the user before closing it. The equalizer and other audio effects run as
an APO inside `audiodg.exe`, so they keep working while the Hub is closed.

**`an_entry_can_be_written_and_taken_away` writes to the real HKCU `Run` key**, under a
pid-suffixed name of its own and never `VALUE_NAME`. It writes `current_exe()`, which under
`cargo test` is the test binary, so pointing it at the app's own value name would repoint a
user's startup at `target\debug\deps`. Do not "simplify" it to use `VALUE_NAME`.

## Invariants worth protecting

**Battery level must never ride on hue alone.**
Green against amber is a contrast ratio of 1.06, and red against the powered-off gray is
1.10, so hue-only encoding collapses exactly where it matters. That is why charge also
fills an arc (WCAG 2.2 SC 1.4.1). Guarded by `charge_reads_as_shape_not_only_hue`.

**An absent mark is not a positive claim.**
No Bluetooth badge means either "off" or "we could not ask". The same rule binds the
tooltip: a reply we do not fully understand becomes `None`, never a confident `Bt::Off`.
Guarded by `badge_only_appears_when_bluetooth_answered` and
`unreadable_payloads_never_become_confident_answers`.

**`decode_ret` must stay total, and a dead poller must be visible.**
Its input is bytes off the wire, so no length byte may reach an index or a `split_last` that
can fail. A panic there aborts the process under `panic = "abort"` and leaves a ghost icon;
worse, in a debug build only the poller thread dies and the tray freezes on the last
confident reading, which is the exact lie the rest of these invariants exist to prevent.
That is why the message loop treats `TryRecvError::Disconnected` as an error state rather
than as "nothing new". Guarded by `survives_any_length_byte` and
`survives_truncation_and_mutation`.

**Notify only on a transition between two *known* states.**
Announcing "headset turned off" merely because the user launched the Hub is the easiest lie
this app can tell. The decision lives in `notification_for()`, and it compares two states we
have: a trip through HubBusy, an error or an unplugged dongle is not a transition, and the
first reading after launch has nothing to be a transition from. Guarded by
`notifies_only_on_a_real_power_transition`.

**`BatteryWatch` follows the first half of that rule and deliberately not the second.**
A state carrying no level never announces, and the first level we get seeds the watch rather
than firing. But the watch *keeps* its memory across a stretch it could not read, and a fall
that straddles that stretch is announced on the far side of it. The two differ because the
warning names the level the pack is at now, where a power notification claims a change
happened. Invalidating the memory instead is the tempting change and it is wrong: `Off`
carries no level either, so a headset switched off between sessions would never warn again.
Guarded by `a_missed_reading_is_never_a_battery_crossing`, whose name is about the missed
reading itself and not about the readings either side of it.

**`BATTERY_LOW` and `BATTERY_CRITICAL` are the single source of truth**, for the icon color
and for the low-battery warning both. Repeating the literals is how the two last drifted
apart. `BATTERY_HYSTERESIS` is what keeps a level parked on a threshold from announcing four
times a minute. Guarded by `low_battery_announces_once_per_crossing`, which checks the
colors and the warning against the same two constants.

**The settings parser must stay total.** A truncated file leaves every preference it did not
speak about at its default; a half-written line is never read as "off". Guarded by
`a_damaged_file_never_flips_a_preference`.

**Range-check whatever the device reports.**
`BATTERY_INFO` has been seen returning `0xC8` (200) for the level while charging. It is
neither a fixed sentinel (a charging pack at 100% returns `0x64`) nor a high-bit charging
flag (that would make 100% read `0xE4`). The encoding is still unexplained, so `poll()`
drops any level above 100 to "unknown" rather than guessing. Do not display a number the
device has not clearly given you.

**Every exchange has to terminate.**
`query()` skips frames it cannot use, which means the read loop no longer stops at the first
complete frame; a device streaming rubbish would otherwise keep it alive forever and hang
the poller, freezing the tray on its last confident reading. `QUERY_DEADLINE` bounds the
whole exchange, not the gap between reads. On a decode failure the scan steps one byte, not
one frame: a length byte only means something in a frame that decoded, and trusting it while
misaligned eats the reply behind it. The same reasoning is why `take_answer` takes a `Scan`
mode. While bytes may still arrive, an incomplete frame is worth waiting for; once the port
falls quiet, a length that was never satisfied was never a length, so the scan steps past it
and can still find the answer stranded behind the phantom. Guarded by
`resynchronizes_a_byte_at_a_time` and `waits_for_an_incomplete_frame_instead_of_dropping_it`.

**Do not call `flush()` on the port.** `serialport` writes with a bare `WriteFile` and keeps
no buffer of its own, so there is nothing for a flush to push out. What it does call is
`FlushFileBuffers`, which on a communications device waits for the transmit queue to drain
under flow control and is outside the write timeout: it is the one call in the exchange
`QUERY_DEADLINE` cannot bound, and a poller stopped in it holds the COM port against the Hub
for as long as it lasts. It looks like the responsible thing to do after a write, which is
why it is written down. No hang has been observed; this is reasoned from the crate source.

**Where the testing stops, and why.**
The frame scan is a pure function (`take_answer`) and is tested directly. The I/O framing
around it, chunked reads and the deadline, is exercised only against real hardware. A stub
`SerialPort` would mean implementing about twenty-five trait methods and then keeping that
fake in step with the crate; it was judged not worth it. If you want that layer covered,
extract further rather than adding a fake.

**A mutation run has been done and its survivors are accounted for**, so do not re-derive
this from scratch. `cargo mutants` less `main`, `notify` and `cli` left 139 of 427 alive.
Everything worth closing was closed, and what still survives is allowed to: the arithmetic
in `icon.rs`, which would need golden-pixel tests; the message bodies in `text.rs`, where
the property that matters is that a language missing from an arm does not compile; the I/O
in `transport.rs`, the layer above; and the `%APPDATA%` wrappers in `settings.rs`, split out
precisely so the tests do not depend on where it points. A few are equivalent rather than
uncovered, and those are the ones that look most like gaps: `|` and `^` in `encode_get`
never disagree, because `dst << 4` and `SRC_PC` share no bits, and every boundary mutation
in `quote` truncates *earlier*, so none of them can breach the budget it exists for. Rerun
it with `--output` somewhere outside the tree. It drives `startup.rs`'s tests, which write
real `HKCU\Run` values, and a mutant that breaks the delete path leaves them behind: check
for `inzone-h9-gen1-headset-status-*` afterwards and delete what you find. Four were left
last time.

**`icon.rs` geometry is written for a 32 px reference and scaled by `n / 32`.**
The one-pixel anti-aliasing ramps (`clamp(0.0, 1.0)`) are deliberately *not* scaled: an edge
is one pixel wide whatever the icon measures. Guarded at 16, 24, 32 and 48 by
`charge_reads_as_shape_not_only_hue` and `badge_keeps_a_transparent_moat`.

**`SM_CXSMICON` is only worth reading once `declare_dpi_aware()` has returned true.**
A DPI-unaware process is told 16 at every scaling factor, so believing it would render
smaller than the fixed 32 it replaced.

**Take that metric from the monitor, never from `GetSystemMetrics`.**
`GetSystemMetrics(SM_CXSMICON)` does not move when the user changes the scaling, even with
per-monitor v2 declared, and neither does `GetDpiForSystem()`. Only
`GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForMonitor(primary))` does. Reading the process
answer once shipped a re-render in the message loop that could never fire, plus a README
claiming it worked; the measurement is in the doc comment on `tray_icon_size`. `n` outside
16..=256 falls back to `REF_N` rather than being believed.

**Open and close the port on every poll.** Holding it would lock the Hub out.

**Keep the dependency list at three** (`serialport`, `tray-icon`, `windows-sys`).
Shipping a single exe that runs from anywhere is a requirement. WinRT toasts would force
an AppUserModelID registration and cost exactly that property.

**The binary size ceiling is written down once**, as `$Limit` in `.github/check-size.ps1`
(320 KiB). README says only that CI checks the size, on purpose: a figure repeated in prose
is a figure that drifts, and the script prints the measured size on every run. Do not put
the number back into either README or into a workflow yaml. The approximate one both READMEs
already carry (about 300 KiB) is not the ceiling and is fine where it is.

## After changing anything, check it on hardware

Paths never exercised on real hardware are listed in prose, in the 検証環境 section of
README.ja.md and in "What has been tested" in README.md. When one gets confirmed, take its
line out of **both** and add the captured frame to `decodes_a_real_reply`. That list is the
only record of what is unverified, so a line removed without a hardware run leaves nothing
behind that says so, and a line removed from one file only leaves the other one lying.

## Writing style for the docs

**To pick the language of a new string, ask whether it is a `Cause`.**

A `Cause` answers "why is there no reading". Nothing in it says what to do about that, its
reader's next move is to paste it into a bug report, and it sits beside decode failures
from `protocol` that have always been English (`checksum mismatch`, `short reply (13
bytes)`). So `Cause::describe` is English, whatever language the rest of the app is
speaking. Translating half of a sentence whose other half comes from `protocol` would make
the mixture look accidental rather than chosen.

Everything else is telling a person something they can act on, and is written in both
languages: tooltips, menu items, notifications, `--help`. **All of it lives in `text.rs`**,
one function per message, each an exhaustive `match lang` so a message missing from one
language does not compile. Writing one straight into the module that shows it compiles and
works and reaches the tray in a single language; nothing catches that, so catch it in
review. A project with a message catalog has the same hole, since a string nobody wrapped
for extraction is invisible to the extractor.

The two worked examples, one on each side. `Cause::UnknownLink(0xAB)` renders `unknown link
state 0xAB`: a number to quote, no action implied, English. `State::HubBusy` renders
`INZONE: ポートが使用中（INZONE Hub を閉じると復帰します）`: not a `Cause`, names the thing
to do, Japanese.

What a cause *quotes* is not its own words and passes through as it came, so a Windows
message on a Japanese install stays Japanese. Guarded by `a_cause_composes_only_english`,
which checks the composed words for kana and kanji and lets the quoted ones through.

README.ja.md is Japanese, and so is the Japanese half of `text.rs`. Keep them that way, in
です・ます調,
one sentence per line, paragraphs separated by blank lines. Do not use dashes (`—`, `―`,
`——`) or the nakaguro (`・`) as a list separator in Japanese prose: use parentheses for
asides and split into two sentences for restatements. A nakaguro joining two words into one
idiomatic compound (`改変・再配布`, `未実装・未検証`) is fine and has been signed off; do not
"fix" those. This file may stay in English.

**The Japanese carries an English rhythm unless you take it out, and there is one habit
underneath.** Everything above and around it, this file, the commit messages, the PR bodies,
the test names, is deliberately rhetorical and personified. Whoever writes the Japanese has
usually just written that, and the head does not switch on its own.

The habit is cutting a causal chain into separate sentences and reattaching the cause
afterwards: `X. Y. Because Z.` Everything else follows from it. Cut the sentences and the
subject goes missing, so 分裂文 (`〜のは…です`) props it back up; the contrast comes loose,
so `A ではなく B` pins it down. They are one import wearing three hats, not three habits.
The user rewrote README.ja.md by hand and the count went 90 sentences to 58, with 分裂文 5
to 0, `〜ためです` 6 to 1, `A ではなく B` 3 to 0.

So: **clauses joined by cause or contrast belong in one sentence; independent facts belong
in separate ones.** Not "shorter", not "longer" — a test you can apply. 一文一義 does not
conflict with this: it forbids cramming unrelated ideas into one sentence, not subordinate
clauses. Neither does the one-sentence-per-line rule above, which is about line breaks and
not about sentence length, though it reads like an invitation to split.

The rest of the layer, all of it observed in what the user removed:

- No 分裂文 (`チェックが付くのは、〜ときだけです`)
- No reason as its own trailing sentence (`〜ためです`, `〜からです`). Fold it into the clause
- No contrast by negation (`上限は目標ではなく歯止めで`). Say the second half only
- No personifying the app (`「下がった」とは言いません`). It displays, it notifies
- No metaphor (`通知の材料にしません`, `歯止め`, `リンクを張り直す`)
- No 前者 / 後者

**A second layer binds both READMEs.** Drop the reasoning behind a design decision, drop
internal state words (`充電中の扱いをやめ`), and prefer the readable figure to the exact one
(`5 ポイント` became `ある程度`). README.ja.md has been through a conclusion-first pass by
hand: state the conclusion, then the detail, and do not build up to it. README.md mirrors it
section for section, so it inherits the shape whether or not anyone re-derives it.

**The English half has the same problem in the other direction.** A section written from the
Japanese sentence by sentence comes out Japanese-shaped: topic fronted (`As for the
notification settings, they are…`), the same fact restated in the next sentence, `〜について
は` turned into `regarding`, and politeness hedges taken literally (`〜と思われます` becoming
`it is thought that` rather than `appear to`). The hand pass cut the Japanese from 90
sentences to 58, and translating it back sentence by sentence re-inflates it. Write each
section from what it means, not from where its sentences end.

**Do not point README at Issues, and do not tell anyone what to put in a report.** Both were
written once and taken back out. An invitation to report is read as an undertaking to
answer, and this project has not made one. The firmware paragraph at the end of the
tested-with section says what can stop working and stops there, which is the shape to keep: the reader learns what to expect,
without being asked for anything.

## Commits

Conventional Commits, in English: `type(scope): summary`, imperative mood, no trailing
period. Types in use here: `feat`, `fix`, `docs`, `test`, `refactor`, `chore`. Scope is
optional and names the area (`protocol`, `icon`, `notify`, `poll`).

Say what changed and why in the body, not how. When a commit fixes something that could
brick the dongle or make the app assert something it does not know, say so plainly, and name
the test that now guards it.

## Third-party code

Do not read repositories without an explicit license, not even for reference. If the user
shares a link to prior art, check the license first, and if there is none, say so and stop
reading it. No third-party code is vendored here; the dependency list in `Cargo.toml` is the
whole of it.

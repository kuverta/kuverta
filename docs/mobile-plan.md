# kuverta on a phone: the two mobile apps

2026-10-07. Reopens the "mobile" line that
[`implementation-plan.md`](implementation-plan.md) §0 deferred to month 9 and
§1 listed as a non-goal. There are two apps:

- **kuverta mail**: a mobile-first 1:1 of the desktop client.
- **kuverta scan**: a 1:1 of `scannerd`'s capture loop, for people with no
  Raspberry Pi under a camera.

This adds two platforms. Earlier decisions make that cheaper than it looks
(§1). Safe preview is the one part that costs more (§2).

---

## 0. Decisions (locked 2026-10-07)

| # | Decision | Choice | Why |
|---|---|---|---|
| 1 | Stack | **Tauri 2, same Rust core** | Already on 2.11; Android and iOS use the same toolchain. Anything else abandons `core-rpc` and `core-paper`. |
| 2 | Safe preview | **A WASM sandbox, shared with desktop** | A subprocess is impossible on both platforms (§4). In-process WASM restores the isolation boundary and closes the Linux and Windows gaps listed in §44. |
| 3 | Distribution | **Sideload first: APK and TestFlight** | No licence exception, no store review to design around. Both apps can exist before store compliance is a commitment. |
| 4 | Scanner target | **Paperless, or kuverta desktop over the LAN** | A phone-only person may have neither a Pi nor Paperless. Paperless when configured; otherwise desktop files it through `core-paper`. |

Licensing does not block this. Per `implementation-plan.md` §0.8, a sole
copyright holder can always dual-license. Decision 3 means the question need
not be answered yet.

---

## 1. Why this is cheaper than it looks

Three earlier decisions did most of the work.

The desktop app is only a shell. `apps/desktop/src` is 2,935 lines across
three files, against 16,405 in `core-rpc`. `main.rs`'s header states the rule:
every command is a thin wrapper over `core_rpc::Core`, and "the shell decides
what to draw and which key does what, and knows nothing else". A mobile app is
a second shell over the same core.

The desktop frontend has no build step. `tauri.conf.json` points
`frontendDist` straight at `ui/`, which is plain ES modules that load in a
mobile webview unchanged. The mail app's UI work is layout and touch targets.
`i18n.js`, `app.js` and the rest stay one copy.

`scannerd`'s camera is already an interface. `camera.rs` defines `Camera` as a
trait, "so the state machine can be driven by a directory of images on a
laptop." A phone camera is another implementor, and `tests/run.rs` already
runs the loop against a scripted one.

`scannerd` splits like this for a phone:

| | Lines | On a phone |
|---|---|---|
| `detect`, `straighten`, `quality`, `flatten`, `pdf`, `spool`, `upload`, `folders`, `locate` | ~4,300 | **Unchanged.** Pure Rust, no Pi. |
| `picture`, `settings` | ~430 | Mostly unchanged. |
| `lcd`, `epaper`, `touch`, `button`, `display`, `paint` | ~2,600 | **Dropped.** The phone is the screen and the button. |
| `camera`, `read` | ~610 | Replaced. Both shell out to a program. |
| `run`, `main`, `hub`, `web` | ~3,500 | The loop stays; its wiring is rewritten. |

`rppal` is already behind `cfg(target_os = "linux")`, so nothing needs
untangling first.

`web.rs` serves `ui.html`, already a mobile-first page with breakpoints at
520px and 860px and a viewport meta. Most of the scanner app's UI is written.

---

## 2. The one hard part: safe preview

§44 built safe preview on four layers. Layer 2 is "in a process of its own,
locked down before it looks": the app starts itself again with
`--preview-worker`, then takes away files, sockets and processes before parsing
a byte.

That layer cannot exist on either mobile platform. iOS does not let an app
`fork` or `exec` a helper binary. On Android a Tauri app ships as a `.so`
inside an APK, not an executable, so there is nothing to start again.
`setrlimit` is per-process, so a thread is not a substitute.

The two attacks §44 names, FORCEDENTRY and BLASTPASS, are both iOS attacks, so
the phone is where this layer is needed most.

### What replaces it

The decoders run in an in-process WASM sandbox: `hayro`, the `image` codecs
and the §43 reader compiled to `wasm32`, instantiated with no host imports and
no filesystem, given memory and fuel limits, and handed the file's bytes.
Layers 1, 3 and 4 are unchanged (Rust decoders, bounded work, only pixels come
back, PNG written by kuverta). Layer 2 is enforced by the runtime in place of
the kernel.

Cost: this is the largest item in the plan and the only one that is new code.
The decoders must build for `wasm32`. That is plausible, since §44's layer 1
requires pure Rust, but unproven: `hayro` is pinned at 0.5 by the 1.90
toolchain (§44) and its one lifetime transmute is in the parser. Drawing will
be slower than native.

Benefit on desktop: §44's "what it cannot cover" lists two gaps: "Linux has no
sandbox here beyond the limits: a syscall filter would be the next step.
Windows has only the separate process and the clock." A WASM boundary is the
same on every platform. Desktop can keep the subprocess *and* run the sandbox
inside it, which gives macOS both and gives Linux and Windows their first real
boundary. Decision 2 is worth its cost because it also closes those §44 gaps.

The existing test (the worker, once locked down, must fail to write a file,
open a socket, start a program and read a file) becomes the sandbox's test. It
should fail the same way when the sandbox is removed.

---

## 3. The scanner app

The loop is `Scanner::turn` with the clock passed in. It does not change; its
wiring does.

The camera gets two implementors. Detection wants a small greyscale frame
every second or two: `getUserMedia` in the webview, downscaled, handed across
as the Y plane. `detect.rs` already expects that, since `camera.rs` chose
`--encoding yuv420` so detection needs no JPEG decoder. Capture wants a
full-resolution JPEG from a native camera plugin. The `--roi` sensor crop has
no phone equivalent, so the captured frame is cropped instead, using the
geometry `straighten.rs` already does for the angled-camera case.

Where letters go: `upload.rs` and `spool.rs` are reused whole for Paperless,
including the backoff fixed in §23 and the offline spooling. The second target
is new: kuverta desktop on the LAN, which files the letter through
`core-paper`. It needs a pairing step and a transport, and is the only new
product surface in this app.

Reading a page: `read.rs` shells out to `tesseract` for the preview guess,
"badly and at once", and Paperless's answer replaces it when the letter is
sent. A phone has no `tesseract` binary, so the first version has no preview
text. `read.rs`'s header describes that as a supported state: "Without
`tesseract` installed there is simply no preview text, and everything else
works as before." The folder guess then arrives from Paperless as it did
before §32. On-device OCR is a later decision. It depends on whether the §32
property (saying where a letter goes while it is still in your hand) is worth
it.

Dropped: the e-paper panel, the LCD, the touch overlay and the GPIO button.
§23 decided multi-page letters with a button because grouping by time merges a
scanned stack. On the phone the button is on the screen, where `ui.html`
already puts **Finish letter**.

---

## 4. Everything else that must be replaced

| What | Where | On a phone |
|---|---|---|
| OS keychain | `core-accounts/src/lib.rs` | `keyring` needs iOS Keychain and Android Keystore backends. The `KEYRING_SERVICE` split per instance stays. |
| Apple Mail / Thunderbird import | `core-rpc/src/setup/` | `cfg`'d out. The `plist` read and the NSS password decryption do not apply on mobile. |
| Docker control | `core-rpc/src/setup/mod.rs` | `cfg`'d out. No phone starts a Paperless container. |
| `open` / `explorer` / `xdg-open` | `desktop/src/files.rs`, `main.rs`, `logging.rs` | The share sheet and the system opener, via Tauri plugins. |
| Quarantine marking | `desktop/src/files.rs` | No `xattr` binary. Both platforms sandbox per-app storage, so a saved file stays in the app's container unless shared, which does the mark's job. |
| Local Ollama | `core-ai` | `provider.rs` already abstracts hosted OpenAI-compatible services (§25), so this is configuration: a LAN Ollama or a service. No model runs on the phone. |
| Always-on sync thread | `desktop/src/main.rs` | Bounded by OS background-execution limits. A **behaviour change**, not a port. See risks. |

---

## 5. Sequencing

Scanner first. It is the smaller app, its UI exists, its core is portable, and
it does not need the WASM sandbox. It proves the Tauri mobile toolchain, the
signing and the camera plugin on a build that can ship while the mail app's
hard part is still being written.

| Phase | What | Gate |
|---|---|---|
| **A** | Tauri mobile toolchain, Android first. Empty shell, APK on the device. | It installs and opens. |
| **B** | Extract `scannerd`'s portable core into a crate. No behaviour change; `tests/run.rs` still passes. | Tests green, Pi build unchanged. |
| **C** | `Camera` over `getUserMedia` plus a native capture plugin. `ui.html` as the frontend. | A letter photographed on a phone reaches Paperless. |
| **D** | Desktop-over-LAN as the second target: pairing and transport. | A letter files with no Paperless configured. |
| **E** | iOS build of the scanner, TestFlight. | Same letter, same result, on an iPhone. |
| **F** | WASM preview sandbox, in `core-rpc`, proved on desktop first. | §44's test passes against the sandbox; fails without it. |
| **G** | Mail app shell, mobile layout over the existing `ui/` modules. | Mail syncs and reads on a phone. |
| **H** | Keychain backends, send, mutation, the rest of 1:1. | Apple Mail stays closed on the phone too. |

Phase F lands on desktop before any mobile mail build exists. It is the risky
piece, it can be tested on a laptop, and it is worth doing even if the mail
app slips.

---

## 6. Risks

Background sync is a different behaviour, not a port. The desktop syncs on a
thread that is always there, and a phone suspends the app. A 1:1 clone of the
*UI* cannot be a 1:1 clone of the *freshness*, and users will mistrust an app
that implies it is. Either mail is fresh when opened and sync is foreground,
or there is push, and neither platform readily grants background push over
IMAP IDLE. Decide before phase G, and say it in the UI.

`hayro` may not build for `wasm32`. Start phase F with a build, before any
design. If it fails, the fallback is the in-process decoders without the
sandbox layer on mobile only, the option decision 2 rejected.

Two shells drift. §1 recorded the same cost as "two implementations of the
same rules now exist. They will drift unless something checks." The
mitigation is the same: the mobile shell calls the same `core_rpc::Core`
commands, and tests target the command surface, not each shell's rendering.

The scanner app's quality bar is the Pi's. §23 and §36 list what real
photographs taught: exposure that the paper fools, edges the sensor loses, a
page that never lies exactly where setup said. A hand-held phone has all of
those plus camera shake. `quality.rs` matters most here, and the first real
test is a pile of post, not a unit test.

---

## 7. Not verified

- That `hayro` 0.5 and the `image` codecs build and draw correctly on `wasm32`.
- That `getUserMedia` gives a usable greyscale frame at a useful rate in both
  platforms' webviews, and that handing it to Rust is cheap enough at one or
  two frames a second.
- Whether `detect.rs`'s thresholds, tuned for a camera fixed above a fixed
  spot, mean anything for a hand-held phone. §23 still lists detection
  against a real surface as unverified even on the Pi.
- What the OS background-execution limits permit for IMAP, by measurement.
- Whether the desktop-over-LAN target wants pairing at all, or whether the
  existing `core-rpc` session model already answers it.

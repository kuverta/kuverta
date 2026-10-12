# kuverta on a phone — the two mobile apps

**2026-10-07.** Reopens the "mobile" line that
[`implementation-plan.md`](implementation-plan.md) §0 deferred to month 9 and
§1 listed as a non-goal. Two apps, not one:

- **kuverta mail** — a mobile-first 1:1 of the desktop client.
- **kuverta scan** — a 1:1 of `scannerd`'s capture loop, for people with no
  Raspberry Pi under a camera.

The non-goals list is where the eighteen months went, and this adds two
platforms to it. What follows is mostly an argument that it costs much less
than it looks like, because of decisions already made for other reasons — and
an honest account of the one place where it costs more.

---

## 0. Decisions (locked 2026-10-07)

| # | Decision | Choice | Why |
|---|---|---|---|
| 1 | Stack | **Tauri 2, same Rust core** | Already on 2.11; Android and iOS are the same toolchain. Anything else abandons `core-rpc` and `core-paper`, which is the whole asset. |
| 2 | Safe preview | **A WASM sandbox, shared with desktop** | A subprocess is impossible on both platforms (§4). In-process WASM restores the isolation boundary — and closes the Linux and Windows gaps §44 admits to. |
| 3 | Distribution | **Sideload first: APK and TestFlight** | No licence exception needed, no store review to design around. Both apps can exist before store compliance is a commitment. |
| 4 | Scanner target | **Paperless, or kuverta desktop over the LAN** | A phone-only person may have neither a Pi nor a Paperless install. Paperless when configured; otherwise desktop files it through `core-paper`. |

Licensing is not in the way. `implementation-plan.md` §0.8 already reasoned it
out: sole copyright holder, so dual-licensing is always available. Decision 3
means that question stays unasked for now rather than answered.

---

## 1. Why this is cheaper than it looks

Three decisions taken for unrelated reasons turn out to have done most of the
work.

**The desktop app is already a shell and nothing else.**
`apps/desktop/src` is 2,935 lines across three files, against 16,405 in
`core-rpc`. `main.rs`'s own header states the rule: every command is a thin
wrapper over `core_rpc::Core`, and "the shell decides what to draw and which
key does what, and knows nothing else". A second shell is therefore a second
shell — not a second application.

**The desktop frontend has no build step.** `tauri.conf.json` points
`frontendDist` straight at `ui/`, which is plain ES modules. They load in a
mobile webview unchanged. The mail app's UI work is layout and touch targets,
not a rewrite, and `i18n.js`, `app.js` and the rest stay one copy.

**`scannerd`'s camera is already an interface.** `camera.rs` defines `Camera`
as a trait, and says why: "so the state machine can be driven by a directory of
images on a laptop." That seam was cut for testability. A phone camera is just
another implementor, and `tests/run.rs` already proves the loop runs against a
scripted one.

The split inside `scannerd` falls almost exactly along the line a phone needs:

| | Lines | On a phone |
|---|---|---|
| `detect`, `straighten`, `quality`, `flatten`, `pdf`, `spool`, `upload`, `folders`, `locate` | ~4,300 | **Unchanged.** Pure Rust, no Pi. |
| `picture`, `settings` | ~430 | Mostly unchanged. |
| `lcd`, `epaper`, `touch`, `button`, `display`, `paint` | ~2,600 | **Dropped.** The phone is the screen and the button. |
| `camera`, `read` | ~610 | Replaced — both shell out to a program. |
| `run`, `main`, `hub`, `web` | ~3,500 | The loop stays; its wiring is rewritten. |

`rppal` is already behind `cfg(target_os = "linux")`, so nothing has to be
untangled first.

And `web.rs` serves `ui.html` — it is already a mobile-first page, with
breakpoints at 520px and 860px and a viewport meta. The scanner app's UI is
largely written, by someone building a thing to use from a phone in the first
place.

---

## 2. The one hard part: safe preview

§44 built safe preview on four layers. Layer 2 is "in a process of its own,
locked down before it looks" — the app starts itself again with
`--preview-worker`, then takes away files, sockets and processes before parsing
a byte.

**That layer cannot exist on either mobile platform.** iOS does not permit an
app to `fork` or `exec` a helper binary at all. On Android a Tauri app ships as
a `.so` inside an APK rather than an executable, so "the app starts itself
again" has nothing to start. And `setrlimit` is per-process, so a thread is not
a substitute — there is no smaller boundary to retreat to.

This matters more here than it would anywhere else, because the two attacks
§44 names are both iOS attacks. FORCEDENTRY and BLASTPASS are the reason the
layer exists, and the phone is simultaneously the platform that most needs it
and the one that cannot have it in this form.

### What replaces it

The decoders run inside a WASM sandbox, in-process: `hayro`, the `image`
codecs and the §43 reader compiled to `wasm32`, instantiated with no host
imports and no filesystem, given memory and fuel limits, and handed the file's
bytes. Layers 1, 3 and 4 survive as they are — Rust decoders, bounded work,
only pixels come back, PNG written by kuverta. Layer 2 becomes a boundary
enforced by the runtime rather than the kernel.

**What this costs.** It is the largest single item in this plan, and the only
one that is new code rather than ported code. The decoders must build for
`wasm32` — plausible, since the whole point of §44's layer 1 is that they are
pure Rust, but `hayro` is pinned at 0.5 by the 1.90 toolchain (§44) and its one
lifetime transmute is in the parser, so this is to be proved and not assumed.
Drawing will be slower than native.

**What it buys back, on desktop.** §44's own "what it cannot cover" lists two
gaps: "Linux has no sandbox here beyond the limits: a syscall filter would be
the next step. Windows has only the separate process and the clock." A WASM
boundary is the same boundary everywhere. Desktop can then keep the subprocess
*and* gain the sandbox inside it — belt and braces on macOS, and a real
boundary on Linux and Windows for the first time. This is why decision 2 is
worth its cost: it is not a mobile workaround, it is the missing half of §44.

The existing test — the worker, once locked down, must fail to write a file,
open a socket, start a program and read a file — becomes the sandbox's test,
and should fail the same way when the sandbox is removed.

---

## 3. The scanner app

The loop is `Scanner::turn` with the clock passed in, and it does not change.
What changes is what it is wired to.

**The camera.** Two implementors rather than one, because the Pi's two needs
differ on a phone too. Detection wants a small greyscale frame every second or
two: `getUserMedia` in the webview, downscaled, handed across as the Y plane —
which is what `detect.rs` already expects, since `camera.rs` chose
`--encoding yuv420` precisely so detection needs no JPEG decoder. Capture wants
a full-resolution JPEG, which is a native camera plugin. The `--roi` sensor
crop has no phone equivalent; the crop becomes a crop of the captured frame,
which is the geometry `straighten.rs` already does for the angled-camera case.

**Where letters go.** `upload.rs` and `spool.rs` are reused whole for
Paperless, including the backoff fixed in §23 and the offline spooling. The
second target is new: kuverta desktop on the LAN, which files the letter
through `core-paper`. That needs a pairing step and a transport, and it is the
one piece of this app that is a new product surface rather than a port.

**Reading a page.** `read.rs` shells out to `tesseract` for the preview guess —
"badly and at once", with Paperless's answer replacing it when the letter is
sent. There is no `tesseract` binary to shell out to on a phone. The honest
first version has no preview text, which `read.rs`'s own header already
describes as a supported state: "Without `tesseract` installed there is simply
no preview text, and everything else works as before." The folder guess then
arrives from Paperless as it did before §32. An on-device OCR library is a
later question, and should be decided on whether the §32 property — saying
where a letter goes while it is still in your hand — is worth it.

**What is simply gone.** The e-paper panel, the LCD, the touch overlay and the
GPIO button. §23 decided multi-page letters with a button because grouping by
time merges a scanned stack; the phone's button is on the screen, which is
where `ui.html` already puts **Finish letter**.

---

## 4. Everything else that must be replaced

| What | Where | On a phone |
|---|---|---|
| OS keychain | `core-accounts/src/lib.rs` | `keyring` needs iOS Keychain and Android Keystore backends. The `KEYRING_SERVICE` split per instance stays. |
| Apple Mail / Thunderbird import | `core-rpc/src/setup/` | `cfg`'d out. The `plist` read and the NSS password decryption have no mobile meaning. |
| Docker control | `core-rpc/src/setup/mod.rs` | `cfg`'d out. No phone starts a Paperless container. |
| `open` / `explorer` / `xdg-open` | `desktop/src/files.rs`, `main.rs`, `logging.rs` | The share sheet and the system opener, via Tauri plugins. |
| Quarantine marking | `desktop/src/files.rs` | No `xattr` binary. Both platforms sandbox per-app storage, so a file saved never leaves the app's container unless shared — the mark's job, done differently. |
| Local Ollama | `core-ai` | `provider.rs` already abstracts hosted OpenAI-compatible services (§25), so this is configuration: a LAN Ollama or a service. No model runs on the phone. |
| Always-on sync thread | `desktop/src/main.rs` | Bounded by OS background-execution limits. This is a **behaviour change**, not a port — see risks. |

---

## 5. Sequencing

Scanner first. It is the smaller app, its UI exists, its core is portable, and
it does not need the WASM sandbox at all — so it proves the Tauri mobile
toolchain, the signing, and the camera plugin on a build that can ship while
the mail app's hard part is still being written.

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

Phase F lands on desktop before any mobile mail build exists. That is
deliberate: it is the risky piece, it is testable on a laptop, and it is worth
doing even if the mail app slips.

---

## 6. Risks

**Background sync is not a port, it is a different behaviour.** The desktop
syncs on a thread that is always there. A phone suspends the app. A 1:1 clone
of the *UI* cannot be a 1:1 clone of the *freshness*, and pretending otherwise
is how the app gets mistrusted. Either mail is fresh when opened and sync is
foreground, or there is push — and push over IMAP IDLE in the background is
not something either platform grants generously. Decide it explicitly before
phase G, and say it in the UI.

**`hayro` may not build for `wasm32`.** Phase F's first hour should be a build,
not a design. If it does not, the fallback is the in-process decoders without
the sandbox layer on mobile only, which was the option decision 2 rejected —
so it is worth knowing early.

**Two shells drift.** This is §1's cost all over again, recorded there as "two
implementations of the same rules now exist. They will drift unless something
checks." The mitigation is the same shape: the mobile shell calls the same
`core_rpc::Core` commands, and the command surface is the thing to test, not
each shell's rendering.

**The scanner app's quality bar is the Pi's.** §23 and §36 are a long list of
things learned from real photographs — exposure that the paper fools, edges the
sensor loses, a page that never lies exactly where setup said. A phone held by
hand has all of those and camera shake besides. `quality.rs` is the file that
earns its keep here, and the first real test is a pile of post, not a unit test.

---

## 7. Not verified

- That `hayro` 0.5 and the `image` codecs build and draw correctly on `wasm32`.
- That `getUserMedia` gives a usable greyscale frame at a useful rate in both
  platforms' webviews, and that handing it to Rust is cheap enough at one or
  two frames a second.
- Whether `detect.rs`'s thresholds, tuned for a camera fixed above a fixed
  spot, mean anything for a hand-held phone — §23 still lists detection
  against a real surface as unverified even on the Pi.
- What the OS background-execution limits actually permit for IMAP, measured
  rather than read.
- Whether the desktop-over-LAN target wants pairing at all, or whether the
  existing `core-rpc` session model already answers it.

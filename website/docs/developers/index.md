---
description: Building kuverta from source — toolchain, system libraries, make targets, run.sh, tests, and keeping a development build apart from an installed one.
---

# Building from source

kuverta is a Rust workspace with a Tauri desktop app, plus a JavaScript triage
surface in `kuverta-bird/` that runs both inside the app and as a Thunderbird
extension. The repository is
[github.com/kuverta/kuverta](https://github.com/kuverta/kuverta); it is licensed
under the **AGPL-3.0-or-later**.

## What you need

| | |
| --- | --- |
| **Rust 1.90** | pinned in `rust-toolchain.toml`, with rustfmt, clippy and rust-analyzer. With [rustup](https://rustup.rs) installed, the right toolchain is fetched on the first `cargo` command. |
| **Node.js 18 or later** | for the triage surface's tests (`make test-js`); its only dependencies are jsdom and, for browser tests, Playwright. |
| **Docker** with Compose | for the [dev stack](dev-stack.md): an IMAP server, an SMTP sink and Paperless-ngx. |
| **Python 3** | for `docker/fill-mailbox.py`, which generates test mail. |
| **Tauri CLI 2** | only to build installers: `cargo install tauri-cli --version "^2" --locked`. |

On **Linux**, the app links against WebKitGTK and D-Bus. On Debian or Ubuntu —
the list CI installs:

```sh
sudo apt-get install -y --no-install-recommends \
  pkg-config libdbus-1-dev libssl-dev \
  libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
  libjavascriptcoregtk-4.1-dev librsvg2-dev libayatana-appindicator3-dev libxdo-dev
```

On **Windows**, building the TLS library needs [NASM](https://www.nasm.us/) on
the `PATH`. On **macOS**, the Xcode command line tools are enough.

## First build

```sh
git clone https://github.com/kuverta/kuverta
cd kuverta
make dev-up     # Dovecot with nine fixture messages, an SMTP sink, a Gmail-shaped Dovecot, Paperless-ngx
make e2e        # register the dev account in a scratch store, sync it, send one
make triage     # copy the triage surface into the app, build, open the window
```

Expected after `make e2e`: **9 fixtures, 8 messages, 9 locations.** The
newsletter fixture is in both INBOX and Archive under one Message-ID and must
collapse into a single message; if the count is 9, deduplication has regressed.

Nine messages prove that sync works but do not feel like a mailbox. For that:

```sh
make fill-dev   # ~250 varied messages in a second dev account, demo@kuverta.test
```

Both languages, every category, HTML-only marketing, attachments, a thread, and
dates spread over seven months.

## make targets

`make help` lists them. The ones you will use:

| Target | |
| --- | --- |
| `make dev-up` / `dev-down` / `dev-reset` / `dev-logs` | start, stop, reseed, and follow the [dev stack](dev-stack.md) |
| `make ai-up`, `make ai-model` | Ollama in a container, and pull the chat and embedding models into it |
| `make paperless-up` | the dev stack, then Paperless-ngx on `http://localhost:8000` (admin/admin) |
| `make e2e` | register the dev account in `.devdata`, sync it, send one message |
| `make fill-dev` | ~250 varied messages in `demo@kuverta.test`, synced |
| `make fill-mailbox HOST= USER= PASS=` | the same generated mail into any test mailbox |
| `make triage-ui` | copy the shared triage surface into `apps/desktop/ui` |
| `make triage` | `triage-ui`, then open the window on the scratch store |
| `make app` / `make app-real` | open the window on the scratch store / on the real data directory |
| `make build` | build the whole workspace |
| `make test` | Rust tests, then the triage surface's tests; integration tests skip if the dev stack is down |
| `make test-all` | Rust tests with the dev stack **required**, as CI runs them |
| `make test-js` | the triage surface's tests, in Node, no mail client needed |
| `make test-e2e` | browser tests of the triage page and settings, in Chromium via Playwright |
| `make fmt`, `make lint` | rustfmt; Clippy with warnings as errors |
| `make check` | what CI runs: format check, Clippy, `test-all` |
| `make bundle` | build `kuverta.app` and a `.dmg` for this Mac into `target/release/bundle` |
| `make release VERSION=x.y.z` | set the version, commit and tag — see [Releasing](releasing.md) |
| `make scannerd-pi`, `make scannerd-to-pi PI_HOST=…` | cross-compile the [scanner](scanner.md) and copy it to a Pi |
| `make clean` | remove build output and the scratch store |

Builds run with `-j 2` on purpose: a full-parallelism build of this workspace
can run out of memory on a laptop, and an OOM-killed build looks like a mystery
rather than like running out of memory.

## run.sh

`./run.sh` builds and opens the window.

```text
./run.sh              on your real mail, release build
./run.sh --dev        on the scratch store, against the Docker dev stack,
                      as the dev instance (its own keychain entries)
./run.sh --debug      a fast build, for iterating on the UI
./run.sh --data-dir ~/somewhere
```

Release is the default because the list needs it: a debug build roughly triples
the cost of moving rows across the Rust–JavaScript bridge, which decides whether
scrolling holds 60 fps. Use `--debug` when the change is markup.

## Dev and installed side by side

A development build and an installed kuverta share nothing they should not:

| | Installed app, `./run.sh` | Dev: `./run.sh --dev`, `make e2e`, … |
| --- | --- | --- |
| Data | `~/.local/share/kuverta` (Windows: `%APPDATA%\kuverta`) | `.devdata` in the repository |
| Keychain services | `kuverta`, `kuverta-oauth` | `kuverta-dev`, `kuverta-oauth-dev` |
| Window title | kuverta | kuverta — dev |
| Update check | twice a day | off (Settings → Check for updates still asks) |
| Paperless | the one the setup assistant installs: port 8010 or above, compose project `kuverta-paperless` | the dev stack's, port 8000, project `kuverta-dev` |

Two environment variables decide it:

- `KUVERTA_INSTANCE=dev` makes a process the dev instance: its own keychain
  entries and, without `KUVERTA_DATA_DIR`, its own data directory,
  `~/.local/share/kuverta-dev`. The dev targets set it.
- `KUVERTA_DATA_DIR` puts the data directory anywhere.

One window per data directory: `./run.sh` without `--dev` opens the installed
app's store and refuses to start while the installed app has it open, and the
other way round.

## Tests

```sh
make test        # everything that runs without the dev stack, plus the JS tests
make test-all    # with the dev stack required, as CI does
cd kuverta-bird && npm test        # the triage surface, in Node
cd kuverta-bird && npm run e2e     # the same page and the settings sheet, in Chromium
cargo test -p scannerd             # the scanner, no camera needed
```

CI (`.github/workflows/ci.yml`) runs format, Clippy and the full test suite on
Linux against the dev stack, and builds the app and CLI on Windows.

## Where things are decided

- The **readme** has the design rules worth knowing before changing anything —
  nothing destroys mail, no UID is trusted on its own, Bcc never reaches the
  message, dedup is per account, a quiet sync must be cheap.
- **`docs/decisions.md`** is the decisions log: why the classifier is ported to
  JavaScript, why one surface has two hosts, how the models were measured, how
  post is filed, and more — each with what it cost.
- **`docs/implementation-plan.md`** and the briefs in `docs/` are the plan the
  project was built against.

Next: [the dev stack](dev-stack.md), [the command line](cli.md),
[architecture](architecture.md).

# Releasing kuverta

A release is a tag. Pushing `vX.Y.Z` runs `.github/workflows/release.yml`, which
makes a **draft** release and builds into it:

| System | Built on | Installers |
| --- | --- | --- |
| macOS 11+ (Apple Silicon and Intel) | macos-14 | `.dmg`, and the `.app` as `.tar.gz` |
| Linux (x86-64) | ubuntu-22.04 | `.AppImage`, `.deb`, `.rpm` |
| Linux (64-bit ARM) | ubuntu-22.04-arm | `.AppImage`, `.deb`, `.rpm` |
| Windows 10+ (x86-64) | windows-latest | `-setup.exe`, `.msi` |

Nothing is public until the draft is published. The four builds are separate: if
one fails, the others still upload, and building the same tag again fills the
same draft.

Windows is compiled on every push (the `windows` job in CI) but has not been
used by anyone yet; the draft's text says so.

## Once, before the first release

1. **Make the repository public** (GitHub → Settings → General → Danger Zone →
   Change visibility). Releases of a private repository cannot be downloaded
   without signing in, and the app's update check asks GitHub anonymously — it
   treats a private repository like one with no releases.
2. **Allow the workflow to create releases.** GitHub → Settings → Actions →
   General → Workflow permissions → *Read and write permissions*. (The workflow
   also asks for `contents: write` itself.)
3. **Decide about signing.** Two ways, both work:

   - **Unsigned (free).** The app is signed *ad hoc*, which Apple Silicon needs
     to run it at all. macOS then refuses to open it the first time with "cannot
     be verified"; each person allows it once (see the readme's *Installing*).
   - **Signed and notarised.** Needs the Apple Developer Program (99 € a year).
     Create a *Developer ID Application* certificate, export it with its key as a
     `.p12`, and add these repository secrets (Settings → Secrets and variables →
     Actions):

     | Secret | What |
     | --- | --- |
     | `APPLE_CERTIFICATE` | the `.p12`, base64: `base64 -i cert.p12 \| pbcopy` |
     | `APPLE_CERTIFICATE_PASSWORD` | the password the `.p12` was exported with |
     | `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID)` |
     | `APPLE_ID` | the Apple ID email |
     | `APPLE_PASSWORD` | an app-specific password for that Apple ID (appleid.apple.com) |
     | `APPLE_TEAM_ID` | the ten-character team id |

     With them set, the next release is signed and notarised and opens without a
     warning. Nothing else changes.

## Each release

```sh
git switch main && git pull
make check                      # what CI runs
make release VERSION=0.2.0      # sets the version, commits "Release 0.2.0", tags v0.2.0
git push origin main v0.2.0     # starts the build
```

Then, on GitHub → Releases, read the draft the workflow made (it takes about
fifteen minutes the first time, less once its cache is warm), write what changed,
and **Publish**. Within twelve hours every running kuverta shows "kuverta 0.2.0 is
available" in its header; Settings → *Check for updates* asks at once.

The workflow refuses a tag that does not match the version in `Cargo.toml`: the
app compares its own version with the latest tag, and a mismatch would offer the
release to itself forever.

To try the bundle locally first: `make bundle` (needs
`cargo install tauri-cli --version "^2" --locked`), which leaves `kuverta.app` and
a `.dmg` for this Mac in `target/release/bundle/`.

## What each system still lacks

- **Windows and Linux are unsigned.** Windows SmartScreen warns about an
  unsigned program (*More info → Run anyway*); signing needs a code-signing
  certificate. Linux packages are not signed either; the apt repository that
  pages.yml publishes is (see the website's releasing page).
- **32-bit ARM Linux** (`armhf`) is not built.

## What a release does not include yet

- **The `kuverta` command-line tool.** OAuth2 sign-in (`kuverta login`) still
  needs it, from `cargo install --path apps/cli`.
- **Automatic updates.** The app says a release exists and links to the
  download; installing is dragging the new app over the old one. Tauri's updater
  would need an update signing key and a manifest per release.

## Dev and installed side by side

An installed kuverta and a development build do not share anything they should
not:

| | Installed app / `./run.sh` | Dev (`./run.sh --dev`, `make e2e`, …) |
| --- | --- | --- |
| Data | `~/.local/share/kuverta` (Windows: `%APPDATA%\kuverta`) | `.devdata` in the repository |
| Keychain service | `kuverta`, `kuverta-oauth` | `kuverta-dev`, `kuverta-oauth-dev` |
| Window title | kuverta | kuverta — dev |
| Update check | twice a day | off (Settings → Check for updates still asks) |
| Paperless | the one the setup assistant installs: port 8010 or above, compose project `kuverta-paperless` | the dev stack's, port 8000, project `kuverta-dev` |

`KUVERTA_INSTANCE=dev` is what makes a process the dev instance; the dev targets
set it. Without `KUVERTA_DATA_DIR`, an instance uses `~/.local/share/kuverta-<name>`.

One window per data directory: `./run.sh` without `--dev` opens the installed
app's store, and refuses to start while the installed app has it open (and the
other way round).

The Docker dev stack's optional Ollama (`make ai-up`) listens on 11434, like a
natively installed Ollama; run one or the other.

## Developing on Windows

Install Rustup, the Visual Studio C++ Build Tools (Desktop development with
C++), and NASM. WebView2 is also required; see the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/). In this
repository, `rustup toolchain install` installs the pinned Rust toolchain.
`run.ps1` also recognises portable NASM at `.devtools/nasm-3.02/nasm.exe`.

From PowerShell in the repository root:

```powershell
.\run.ps1                  # debug build, then open kuverta - dev
.\run.ps1 -BuildOnly       # compile without opening a window
.\run.ps1 -Release         # optimised build for performance checks
.\run.ps1 -Preview         # browser preview with test data; no Rust build
.\run.ps1 -DataDir .devdata/test-mailbox  # use a separate copy of a local mailbox
```

The script copies and verifies the shared UI, builds with the lockfile, and
uses `.devdata` plus the `dev` keychain namespace. No installer or push is
needed. Close the development window before rebuilding. The first build is
slow; later builds reuse compiled dependencies.

The browser preview is at `http://127.0.0.1:19326`. Open Settings → Models and
switch from Ollama to `Claude (Test)`. It uses the actual frontend files and
the tests' fake RPC bridge. Saving survives reopening the settings in that
tab; refreshing the browser resets the sample data. It does not test native
Windows integration. Ctrl+C in the terminal stops the preview.

If Windows blocks a freshly compiled Cargo build script with error 4551,
check the CodeIntegrity event log. A policy named `VerifiedAndReputableDesktop`
is Windows Smart App Control, which has no per-app exceptions. On a personal
development PC, its owner can choose to turn it off in Windows Security →
App & browser control → Smart App Control settings; this changes protection
for the whole PC. Recent Windows updates support turning it back on, which
can block unsigned development builds again. See the
[Microsoft FAQ](https://support.microsoft.com/en-us/windows/security/threat-malware-protection/smart-app-control-frequently-asked-questions).
If an organisation manages the policy, ask its administrator for an approved
development environment instead. Installing the compiler alone does not grant
permission to execute the programs it generates.

The model settings can also be tried without API keys or downloaded models:
run `.\run.ps1 -ModelDemo`, put aside first-run setup, and in Settings change
the local Ollama address to `http://127.0.0.1:19325/ollama`. Add a Claude
provider named `Claude (local test)` with address
`http://127.0.0.1:19325/claude/v1` and leave its key empty. These addresses
serve fixture model lists only; **Try them** does not run a real model.
Switch each job between the two providers, save, and reopen Models. A model
offered by the chosen provider should be filled in; an explicit model choice
should survive saving. The fixture server stops when the development app exits.

For automated tests, from `kuverta-bird`:

```powershell
npm.cmd ci --no-audit --no-fund
npx.cmd playwright install chromium
node --test e2e/settings.test.js
npm.cmd test
```

The browser tests use a fake Tauri bridge, so test Windows WebView2 behaviour
in the development app too. Installer testing is a separate final step: a
locally built installer must contain the change before the installed app can
be used to verify it.

## Local development changes and reproducible checks

This branch contains provider model selection and phishing-priority corrections,
rustls/HTML-fuzz fixes, Windows development support, eM Client import, readable
IMAP namespaces and reliable assistant folder creation. UI additions include
optional unread highlighting/filtering, read-on-open, mail threads with HTML,
navigation history, editable attention criteria, metadata-first assistant
review, email-only onboarding, DNS publication checks and Cleanup repairs.
See the user guide and the implementation's focused tests for behavior/limits.

Invoice collection combines accounts within one explicitly configured profile.
The setup wizard stores company/recipient policy, review terms, destination,
folder layout and default period. Each run accepts arbitrary dates, including
year boundaries. Evidence-backed document facts and potential versions stay
reviewable. Exact-document corrections persist per profile, and each export
requires joint preview approval. Scans remain manual. See
[filing](../website/docs/user-guide/filing.md).

Windows folder picking only returns a path. Date hierarchy uses verified invoice
dates, reuses ordinary folders and creates missing segments only after approval.
Existing documents are not reorganized or broadly indexed. Export preflight
rejects directory-name collisions and junctions before writing selected files.

Ollama assistant calls request 16,384 context tokens and up to 2,048 output
tokens. The 4,096 server default can be smaller than the assistant's initial
prompt/tool catalog. Context overflow produces an actionable error; no global
server change, silent history trimming or cloud fallback is performed. Large
conversations can still require a narrower scope/new chat. Memory requirements
depend on hardware and model. This is not a mail-classification quality claim.

Validated 2026-10-19 with synthetic data: 23 invoice SQLite/MIME/filesystem tests,
18 invoice browser tests, 18 provider HTTP-contract tests, release desktop check
and build. The provider tests require local socket access. A Windows junction
fixture uses native path separators. Tests cover profile boundaries, persisted
corrections, free periods, date folders, preflight, preservation of existing
files, retries, picker cancellation/errors and destination previews.

The built Windows app also passed native picker selection/cancellation against a
separate synthetic profile. Saving the setup draft preserved the chosen layout
and created no invoice files. The local installation was backed up and updated;
the executable hash and database integrity were verified, along with unchanged
account/profile/model settings and invoice setup, corrections and export records.
Detailed installation evidence stays in the ignored local audit directory.

Reproduce from the repository root:

```powershell
cargo test -p core-rpc --release --locked --test invoices
cargo test -p core-ai --release --locked --test providers
node --test kuverta-bird/e2e/invoices.test.js
cargo build -p kuverta-desktop --release --locked -j 1
```

Native smoke fixtures: create a NEW directory with the `invoice_fixture`
example, launch the development app against it, and test setup/review/picking
without production mail. `local_assistant_smoke` explicitly performs one
local inference with synthetic text and the production tool catalog; it has
no database/mail access and never executes returned tools. A successful call
checks capacity/function only, not quality or peak-memory use.

Before replacing a local installation, close the app normally, back up SQLite
consistently plus the previous executable/configuration, verify integrity and
hashes, then relaunch and audit account/profile/model settings. Database-only
backups do not include mail blob files. Personal paths, mailbox totals, previews,
export manifests, hardware snapshots and per-user installation logs belong
outside tracked files. Public reports use only synthetic examples.

## Publication hygiene

Never push a local practice branch merely because its latest files look clean:
earlier commits can contain private notes. Preserve private audit records locally
under ignored `.devdata`. Prepare a reviewed, sanitized patch on a branch from
the public base before publishing. Check the complete diff and new commit history
for real identities, accounts, paths, document content and secrets; exclude local
databases, imports, screenshots, backups and manifests. A local pre-push guard
may block the practice history. Do not bypass it to publish that history.

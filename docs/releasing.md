# Releasing kuverta

A release is a tag. Pushing `vX.Y.Z` runs `.github/workflows/release.yml`, which
builds a universal macOS app (Apple Silicon and Intel) and attaches the `.dmg` to
a **draft** release. Nothing is public until the draft is published.

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

## What a release does not include yet

- **The `kuverta` command-line tool.** OAuth2 sign-in (`kuverta login`) still
  needs it, from `cargo install --path apps/cli`.
- **Automatic updates.** The app says a release exists and links to the
  download; installing is dragging the new app over the old one. Tauri's updater
  would need an update signing key and a manifest per release.
- **Linux and Windows builds.** The code builds there; the workflow does not yet.

## Dev and installed side by side

An installed kuverta and a development build do not share anything they should
not:

| | Installed app / `./run.sh` | Dev (`./run.sh --dev`, `make e2e`, …) |
| --- | --- | --- |
| Data | `~/.local/share/kuverta` | `.devdata` in the repository |
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

---
description: How a kuverta release is made — tags, the release workflow, signing, and what a release does not include yet.
---

# Releasing

A release is a tag. Pushing `vX.Y.Z` runs `.github/workflows/release.yml`, which
makes a **draft** release and builds into it:

| System | Built on | Installers |
| --- | --- | --- |
| macOS 11+ (Apple Silicon and Intel) | macos-14 | `.dmg`, and the `.app` as `.tar.gz` |
| Linux (x86-64) | ubuntu-22.04 | `.AppImage`, `.deb`, `.rpm` |
| Windows 10+ (x86-64) | windows-latest | `-setup.exe`, `.msi` |

Nothing is public until the draft is published. The three builds are separate:
if one fails the others still upload, and building the same tag again fills the
same draft.

## Each release

```sh
git switch main && git pull
make check                      # what CI runs
make release VERSION=0.2.0      # sets the version, commits "Release 0.2.0", tags v0.2.0
git push origin main v0.2.0     # starts the build
```

Then, on GitHub → Releases, read the draft (about fifteen minutes the first
time, less with a warm cache), write what changed, and **Publish**. Within
twelve hours every running kuverta shows "kuverta 0.2.0 is available" in its
header; Settings → *Check for updates* asks at once.

The workflow refuses a tag that does not match the version in `Cargo.toml`: the
app compares its own version with the latest tag, and a mismatch would offer the
release to itself forever.

To try the bundle locally first: `make bundle` (needs
`cargo install tauri-cli --version "^2" --locked`) leaves `kuverta.app` and a
`.dmg` for this Mac in `target/release/bundle/`.

## Once, before the first release

1. **Make the repository public.** Releases of a private repository cannot be
   downloaded without signing in, and the app's update check asks GitHub
   anonymously.
2. **Allow the workflow to create releases**: Settings → Actions → General →
   Workflow permissions → *Read and write permissions*.
3. **Decide about signing.**
    - **Unsigned (free).** The app is signed *ad hoc*, which Apple Silicon needs
      to run it at all; macOS then says "cannot be verified" the first time, and
      each person allows it once.
    - **Signed and notarised.** Needs the Apple Developer Program. Add a
      *Developer ID Application* certificate and the Apple ID details as the
      repository secrets `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
      `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID`,
      and the next release is signed and notarised. `docs/releasing.md` has the
      details.

## What each system still lacks

- **Windows and Linux are unsigned.** SmartScreen warns (*More info → Run
  anyway*); signing needs a code-signing certificate.
- **Linux on ARM** is not built.

## What a release does not include yet

- **The `kuverta` command-line tool.** OAuth2 sign-in still needs it, from
  `cargo install --path apps/cli`.
- **Automatic updates.** The app says a release exists and links to it;
  installing is replacing the app. Tauri's updater would need an update signing
  key and a manifest per release.

## This website

The site is built from `website/` with MkDocs and the Material theme, by
`.github/workflows/pages.yml`, on every push to `main` that touches `website/` or
`assets/brand/`. Locally:

```sh
python3 -m venv .venv
.venv/bin/pip install -r website/requirements.txt
.venv/bin/mkdocs serve -f website/mkdocs.yml     # http://127.0.0.1:8000/kuverta/
```

The brand files the site uses are copies in `website/docs/assets/brand/`, so the
build never reaches outside `website/`; copy them again when the logo changes.
In the repository's settings, **Pages → Source** has to be *GitHub Actions*.

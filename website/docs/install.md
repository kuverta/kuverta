---
description: Installing kuverta on macOS, Linux and Windows, the first run, updating, and removing it again completely.
---

# Install

Every installer is on the **[latest release](https://github.com/kuverta/kuverta/releases/latest)**.
kuverta is usable but not finished; the [FAQ](user-guide/faq.md#what-is-not-there-yet)
lists what is still missing.

| System | Installers |
| --- | --- |
| macOS 11 or later, Apple Silicon and Intel | `.dmg` (and the `.app` as `.tar.gz`) |
| Linux, x86-64 | `.AppImage`, `.deb`, `.rpm` |
| Windows 10 or later, x86-64 | `-setup.exe`, `.msi` |

## macOS

1. Download the `.dmg`, open it, and drag **kuverta** into **Applications**.
2. Open kuverta from Applications.

Releases are not notarised by Apple yet, so the first time macOS says that
kuverta "cannot be verified" and will not open it. Allow it once:

- open **System Settings → Privacy & Security**, scroll down, and press
  **Open Anyway** next to the note about kuverta; or
- in Terminal:

    ```sh
    xattr -dr com.apple.quarantine /Applications/kuverta.app
    ```

After that it opens normally. Passwords are kept in your **login keychain**.

!!! tip "Apple Mail accounts and smart mailboxes"
    macOS only shows Apple Mail's accounts and smart mailboxes to programs with
    **Full Disk Access**. If you want the setup assistant to find them, give it
    to kuverta in **System Settings → Privacy & Security → Full Disk Access**.
    Everything else works without it.

## Linux

Pick one:

=== "AppImage"

    Runs on most distributions without installing anything:

    ```sh
    chmod +x kuverta_*.AppImage
    ./kuverta_*.AppImage
    ```

=== "Debian, Ubuntu"

    ```sh
    sudo apt install ./kuverta_*.deb
    ```

=== "Fedora, openSUSE"

    ```sh
    sudo dnf install ./kuverta-*.rpm
    ```

**A keyring has to be running.** kuverta keeps passwords in the Secret Service,
which is what GNOME Keyring and KWallet provide. On GNOME and KDE desktops it
already runs; on a minimal window manager, start `gnome-keyring-daemon` (or
another Secret Service provider) first, or kuverta cannot store a password.

The Linux packages are not signed. Only x86-64 is built; there is no ARM build
yet.

## Windows

Run the `-setup.exe` (or the `.msi`). The Windows build is compiled on every
change but has not been tried by anyone yet — reports are welcome.

SmartScreen warns about programs that are not signed: choose **More info →
Run anyway**. Passwords are kept in the **Windows Credential Manager**.

## The first run

On a new installation kuverta opens a **setup assistant** instead of an empty
window. Every step can be skipped and done later; it can be opened again from
**Settings → General → Open the setup assistant**.

1. **Local models.** Checks whether [Ollama](https://ollama.com) is installed and
   running, offers the download link or install command if it is not, starts it
   if it is installed but stopped, and downloads the models kuverta uses, with
   progress. See [Models](user-guide/models.md).
2. **Paper mail.** Looks for a Paperless-ngx at `localhost:8000` and
   `paperless.local:8000`. Connect to one you have — with your Paperless user
   name and password, which are exchanged for an API token, or with a token you
   paste — or let kuverta **install one with Docker**: it writes a compose file
   and an owner-only settings file into `paperless/` in its data directory and
   starts it. Docker itself is yours to install; the assistant says where. See
   [Paper post](user-guide/paper.md).
3. **Mail accounts.** Reads the accounts Thunderbird and Apple Mail know. For
   any other address it finds the servers from a built-in list of common
   providers, the provider's own autoconfig file, or Mozilla's directory — the
   way Thunderbird does. Thunderbird's saved passwords come across with its
   accounts (with a primary password set, kuverta asks for that one password);
   Apple Mail's stay in its keychain, so you type each one once. Every account
   is signed in to before the assistant moves on, and new accounts start
   syncing when it finishes.

The first sync downloads the whole mailbox and can take a while; the header
shows how far it has got.

## Updating

kuverta asks GitHub for the latest release twice a day. When there is a newer
one, the header says **"kuverta X.Y.Z is available"** with **Download**,
**What's new** and **Later**. **Settings → General → Check for updates** asks
straight away.

Updating is installing the new release over the old one — drag the new app
into Applications, install the new package, or run the new setup. Your mail,
settings and passwords stay where they are.

## Uninstalling

Removing kuverta completely is three things: the app, its data directory, and
its entries in the system keychain.

### 1. Remove the app

=== "macOS"

    Quit kuverta and move `/Applications/kuverta.app` to the Bin.

=== "Linux"

    Delete the AppImage, or remove the package:

    ```sh
    sudo apt remove kuverta     # .deb
    sudo dnf remove kuverta     # .rpm
    ```

=== "Windows"

    **Settings → Apps → Installed apps → kuverta → Uninstall.**

### 2. Remove the data directory

This holds the local copy of your mail, search index, settings, smart
mailboxes, scheduled mail, logs, and your OpenPGP keys (in `pgp/` — keep a copy
if you still need them). Your mail itself stays on your mail servers.

| System | Data directory |
| --- | --- |
| macOS | `~/.local/share/kuverta` |
| Linux | `~/.local/share/kuverta`, or `$XDG_DATA_HOME/kuverta` if that is set |
| Windows | `%APPDATA%\kuverta` |

(`KUVERTA_DATA_DIR`, if you set it, overrides all of these. A development
build uses `kuverta-dev` instead of `kuverta`.)

=== "macOS and Linux"

    ```sh
    rm -rf ~/.local/share/kuverta
    ```

=== "Windows (PowerShell)"

    ```powershell
    Remove-Item -Recurse -Force "$env:APPDATA\kuverta"
    ```

!!! warning "If the setup assistant installed Paperless for you"
    Your scanned post lives in Paperless's Docker volumes, not in the data
    directory. Stop Paperless **before** deleting the directory, from its
    folder:

    ```sh
    cd ~/.local/share/kuverta/paperless
    docker compose down        # stops it and keeps your documents
    ```

    `docker compose down -v` also deletes the volumes — **every scanned
    document** in that Paperless. Only do that if you have exported what you
    want to keep.

### 3. Remove the keychain entries

kuverta files every secret in the system keychain under the service name
**`kuverta`**, and OAuth2 sign-ins under **`kuverta-oauth`**. The account names
of the entries are:

| Entry | What it holds |
| --- | --- |
| your email address | the password or app password of that mail account |
| `<email> (google oauth client secret)` | a Google OAuth client secret |
| `paper:<key>` | the Paperless API token of a postal address |
| `ai:<key>` | the API key of a hosted model provider |
| `pgp:<fingerprint>` | the passphrase of an OpenPGP key |
| your email address, under `kuverta-oauth` | an OAuth2 refresh token (Gmail, Microsoft 365) |

**Remove account** in settings already deletes that account's password. To
remove everything that is left:

=== "macOS"

    Open **Keychain Access**, search for `kuverta`, and delete the entries whose
    *Where* is `kuverta` or `kuverta-oauth`. Or in Terminal:

    ```sh
    for service in kuverta kuverta-oauth; do
      while security delete-generic-password -s "$service" >/dev/null 2>&1; do :; done
    done
    ```

=== "Linux"

    With `secret-tool` (package `libsecret-tools` or `libsecret`):

    ```sh
    secret-tool clear service kuverta
    secret-tool clear service kuverta-oauth
    ```

    Or open *Passwords and Keys* (Seahorse) or KWalletManager and delete the
    entries labelled with `kuverta`.

=== "Windows"

    Open **Control Panel → Credential Manager → Windows Credentials**. Under
    *Generic Credentials*, kuverta's entries are named
    `<account>.kuverta` and `<account>.kuverta-oauth` — for example
    `erika@example.de.kuverta`. Remove each. From a command prompt:

    ```bat
    cmdkey /list | findstr kuverta
    cmdkey /delete:erika@example.de.kuverta
    ```

The development instance uses `kuverta-dev` and `kuverta-oauth-dev` instead;
see [dev and installed side by side](developers/index.md#dev-and-installed-side-by-side).

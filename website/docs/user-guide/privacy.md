---
description: What kuverta stores on your computer, what it keeps in the system keychain, and every place it sends anything.
---

# Privacy

kuverta is a program on your computer, not a service. There is no kuverta
account, no kuverta server, and no analytics or telemetry. This page lists what
it keeps and every place it connects to.

## On your computer

Everything kuverta knows is in its **data directory**:

| System | Data directory |
| --- | --- |
| macOS | `~/.local/share/kuverta` |
| Linux | `~/.local/share/kuverta` (or `$XDG_DATA_HOME/kuverta`) |
| Windows | `%APPDATA%\kuverta` |

Setting `KUVERTA_DATA_DIR` puts it somewhere else. In it:

- the **store** — a SQLite database with your accounts' settings (no
  passwords), a copy of your mail's headers and text, the search index,
  categories, your corrections, smart mailboxes, the queue of changes waiting
  to be sent, and scheduled mail;
- the **messages** themselves, as downloaded;
- `pgp/` — your [OpenPGP keys](encryption.md#where-keys-are-kept), secret keys
  protected by a passphrase and readable only by you;
- `logs/` — the [log](diagnostics.md), which holds addresses and subjects but
  no passwords, tokens or message text;
- `paperless/` — only if the setup assistant installed Paperless for you: its
  compose file and an owner-only settings file. The scanned documents live in
  Paperless's Docker volumes.

Small window preferences — the sort order, whether to ask about similar
messages — are kept by the window itself.

## In the system keychain

Every secret goes to the system's own keychain — the login keychain on macOS,
the Secret Service (GNOME Keyring, KWallet) on Linux, the Credential Manager on
Windows — and nowhere else:

- mail account passwords and app passwords,
- OAuth2 refresh tokens for Gmail and Microsoft 365, and a Google client secret,
- Paperless API tokens,
- API keys of hosted model providers,
- passphrases of your OpenPGP keys.

Secrets go one way: they are written to the keychain and never read back into a
settings page, which only says whether one is stored. The entries are named in
[Install → Uninstalling](../install.md#3-remove-the-keychain-entries).

## Where kuverta connects

| To | When | What is sent |
| --- | --- | --- |
| **Your mail servers** (IMAP, SMTP) | syncing, sending, changing mail | what any mail program sends them |
| **Your Paperless-ngx** | reading post | requests for your documents, with its API token |
| **Ollama on this computer** | reading scans, `kuverta classify` | scans and message excerpts — they do not leave your computer |
| **A model provider you added** | only for the jobs you chose it for | scans of letters, or the sender, subject and start of each message; see [Models](models.md#what-leaves-your-computer) |
| **Ollama's model library**, through your Ollama | when you download a model in the setup assistant | which model to download |
| **GitHub** (`api.github.com`) | twice a day, and on *Check for updates* | an anonymous request for the latest release |
| **Your provider's autoconfig, and Mozilla's directory** (`autoconfig.thunderbird.net`) | only in the setup assistant, when you add an address kuverta does not know | your domain; the provider's own autoconfig address is also given your email address, as Thunderbird does |
| **Google or Microsoft sign-in** | only for OAuth2 accounts | the sign-in itself |
| **A sender's unsubscribe address** | only when you unsubscribe | a one-click request or an unsubscribe mail; see [Cleanup](cleanup.md#unsubscribe) |

Web pages open in your browser only when you ask — an unsubscribe page, a
release, a download link. Messages are shown as text, so images, fonts and
tracking pixels in HTML mail are never loaded.

## What stays yours

- **Nothing is sent to a hosted model** unless you add a provider and choose it
  for a job, and the settings say what that job would send.
- **Scheduled mail is sent by kuverta**, not stored with a third party.
- **An assistant connected over MCP** reads your mail only if you set it up, is
  read-only unless you start it with `--allow-writes`, and can never send. See
  [the MCP server](../developers/mcp.md).

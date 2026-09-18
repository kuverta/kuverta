---
description: Every section of kuverta's settings — accounts, postal addresses, models, smart mailboxes, encryption, general and diagnostics.
---

# Settings

Open settings with <kbd>,</kbd> or the gear at the right of the header. They
open as a sheet over the window, so **Done** (or <kbd>Esc</kbd>) puts you back
exactly where you were. The sections are listed on the left:

| Section | |
| --- | --- |
| **Accounts** | each mail account; **+ Add** for a new one |
| **Postal addresses** | each [postal address](paper.md); **+ Add** for a new one |
| **Models** | **Model for each job**, and each model provider; **+ Add** for a new provider |
| **Mail** | **Smart mailboxes** and **Encryption** |
| **kuverta** | **General** and **Diagnostics** |

The version you are running is at the foot of the list.

## Accounts

![An account's settings](../assets/screens/settings-account-light.webp#only-light)
![An account's settings](../assets/screens/settings-account-dark.webp#only-dark)

- **Account** — a description, the email address, and the user name if it is
  not the address.
- **Incoming mail (IMAP)** — server, port and security: TLS, STARTTLS, or none
  (only for a server on this computer).
- **Outgoing mail (SMTP)** — the same for sending. Without it the account can
  read mail but not send it.
- **Sign-in** — *Password or app password*, or *OAuth2* for Microsoft 365 and
  Google (see below).
- **Folders not synced** — one per line: a mailbox name, or an attribute such as
  `\All`. On Gmail, leaving out `\All` (*All Mail*) is the difference between
  downloading your mail once and downloading it once per label.

**Verify** signs in with what the form says and reports whether it worked,
which extensions the server has, where sent, archived and deleted mail will be
filed, and how much a first sync would download — without downloading anything.
**Save** keeps it; **Remove account** removes the account from kuverta and
deletes its stored password. The mail on the server is not touched.

Passwords go to the system keychain and are never shown again: the field says
whether one is stored, and leaving it blank keeps it.

### Gmail and Microsoft 365

- **Gmail** takes an **app password** (turn on 2-Step Verification, then create
  one at `myaccount.google.com/apppasswords`) or OAuth2 with your own *Desktop
  app* OAuth client. Google is phasing app passwords out during 2026.
- **Microsoft 365** needs OAuth2 with an Azure app registration's client id.

OAuth2 sign-in itself happens in a terminal for now: `kuverta login --email …`
from the [command line](../developers/cli.md), which is not part of the
installers yet.

## Postal addresses

See [Paper post](paper.md#setting-up-a-postal-address).

## Models

See [Models](models.md).

## Smart mailboxes

The account's smart mailboxes, with **Edit** and **Delete**, **New smart
mailbox…**, and importing them from Thunderbird and Apple Mail. See
[Smart mailboxes](smart-mailboxes.md).

## Encryption

Your OpenPGP keys and other people's, creating a key, and importing keys. See
[Encryption](encryption.md).

## General

![Settings → General](../assets/screens/settings-general-light.webp#only-light)
![Settings → General](../assets/screens/settings-general-dark.webp#only-dark)

- **Setup** — **Open the setup assistant** again.
- **Deleting mail** — *After deleting a message, offer to delete messages like
  it too*: turns [similar messages](similar.md) on and off.
- **Updates** — which version this is, and **Check for updates**.

## Diagnostics

Detailed logging, the recent log, and exporting it for a bug report. See
[Diagnostics](diagnostics.md).

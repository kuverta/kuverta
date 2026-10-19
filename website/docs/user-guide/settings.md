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

### SPF, DKIM and DMARC

**Mail domain check** runs automatically for a new address or a selected import
card. For an existing account, choose **Check domain**. The check uses
[Google Public DNS over HTTPS](https://developers.google.com/speed/public-dns/docs/doh/json)
and sends only the domain and optional DKIM selector. It does not send the full
address, credentials or mail content, change DNS, or send a test message.

The results distinguish published records, common configuration problems,
missing records and failed lookups. Enter the **DKIM selector** from your mail
provider to check its public key. Without it, DKIM is **Not checked**; a domain
alone cannot identify the provider's selector.

This is an initial publication diagnostic, not a complete authentication test.
It does not evaluate SPF include/redirect chains or sending IPs, DKIM key strength
or signatures, or DMARC alignment and parent-domain inheritance. Those limits
are shown in the results. In particular, **Record found** does not mean that
outgoing mail passes authentication. A DMARC `p=none` policy is monitoring mode,
not a request to block unauthenticated mail. Domain warnings do not prevent
adding or receiving mail through the account.

The checks follow the publication formats in
[SPF (RFC 7208)](https://www.rfc-editor.org/rfc/rfc7208),
[DKIM (RFC 6376)](https://www.rfc-editor.org/rfc/rfc6376) and
[DMARC (RFC 9989)](https://www.rfc-editor.org/rfc/rfc9989).

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

### Token usage

How many tokens the models were sent and wrote back, counted on this computer
for every job — the assistant, sorting mail, judging urgency, tasks, reading
scans, and trying a model on the models page. Today, this month and since
counting began; the last thirty days as bars; and this month by model and by
job. A hosted service bills by tokens; a model on this computer costs time and
power instead. Only counts are kept, never what was asked or answered.
**Reset the statistics** starts counting again from nothing.

## Smart mailboxes

The account's smart mailboxes, with **Edit** and **Delete**, **New smart
mailbox…**, and importing them from Thunderbird and Apple Mail. See
[Smart mailboxes](smart-mailboxes.md).

## Encryption

Your OpenPGP keys and other people's, creating a key, and importing keys. See
[Encryption](encryption.md).

## Assistant memory

What the assistant remembers about each account: read a note, change it, add
one, or **Forget** it. See [Memory](assistant.md#memory) for how notes are
kept and synced to your other devices.

## General

The setup's first page offers **Email only** (the default) or **Email and paper
post**. Email-only setup skips Paperless and vision-model setup. This choice
does not remove existing postal addresses. On Windows, **Start with Windows**
is available during setup and in General settings; it is off until selected.

Message-list preferences include **Show categories** and **Open the latest message
when opening a thread**. Hiding categories removes their sidebar/list labels and
clears an active category filter, while leaving the independent unread filter
available. Categories remain stored; they are a single coarse classification per
message, not multiple synchronized IMAP tags. The latest-message option opens
only one message on an explicit thread click. Automatic refresh never advances
through unread mail; marking read still depends on **Mark read when opened**.

Each account can also connect an existing [attachment filing directory](filing.md).

![Settings → General](../assets/screens/settings-general-light.webp#only-light)
![Settings → General](../assets/screens/settings-general-dark.webp#only-dark)

- **Setup** — **Open the setup assistant** again.
- **Deleting mail** — *After deleting a message, offer to delete messages like
  it too*: turns [similar messages](similar.md) on and off.
- **Attachments** — **Safe preview**: see [Reading](reading.md#attachments).
- **Signature** — the picture the assistant signs PDFs with; see below.
- **Needs attention** — whether the sorting model also judges how soon new mail
  needs you; see [Needs attention](attention.md).
- **Updates** — which version this is, and **Check for updates**.

### Signature

A picture of your handwritten signature, for the [assistant](assistant.md#writing-and-signing-pdfs)
to put on a PDF when you ask it to. Sign on white paper, photograph or scan
it straight on, and **Choose a picture…** — a PNG or JPEG, at most 4000
pixels on a side. kuverta cuts the ink out of the paper: it measures how
bright the paper is across the picture, shadows and all, reads each pixel
against the paper under it, drops specks, and crops to the strokes. What is
kept is that cut-out, with the paper transparent, and the preview shows it on
paper with the date line beneath — exactly what goes on a page. A PNG that is
already cut out is only cropped. **Show on a page** opens a sample PDF signed
with it; **Remove** forgets it.

**Place** is written in front of the date beneath the signature wherever it
is placed, the way a letter is signed — *Musterstadt, 07.10.2026*; left
empty, the date stands alone.

It is a picture, not a certificate: what you would get from printing a page,
signing it and scanning it. The assistant signs only when you ask it to, shows
you the signed copy, and never sends it.

## Diagnostics

Detailed logging, the recent log, and exporting it for a bug report. See
[Diagnostics](diagnostics.md).

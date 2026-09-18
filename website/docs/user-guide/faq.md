---
description: Common problems with kuverta and what to do about them, and what is not there yet.
---

# Troubleshooting and FAQ

## macOS says kuverta "cannot be verified"

Releases are not notarised by Apple yet. Allow it once in **System Settings →
Privacy & Security → Open Anyway**, or run
`xattr -dr com.apple.quarantine /Applications/kuverta.app`. See
[Install](../install.md#macos).

## Windows SmartScreen will not start it

Choose **More info → Run anyway**. The program is not signed yet.

## On Linux, kuverta cannot store my password

kuverta keeps passwords in the Secret Service. GNOME Keyring or KWallet has to be
running and unlocked; on a minimal desktop, start `gnome-keyring-daemon` or
another Secret Service provider.

## No new mail arrives

kuverta fetches mail when you press <kbd>r</kbd>; it does not yet fetch it by
itself in the background. If a sync fails, the error is shown at the bottom of
the window and written to the [log](diagnostics.md).

## "Could not sign in" — but the password is right

- **Gmail** usually needs an **app password** rather than your Google password
  (2-Step Verification, then `myaccount.google.com/apppasswords`), or OAuth2.
- **Microsoft 365** does not accept passwords for IMAP at all; it needs OAuth2.
- **Verify** in the account's settings says what the server answered.

## Archive says the account has no Archive folder

Some servers come without one. Create a mailbox called `Archive` with the **+**
beside *Mailboxes*. See [Mailboxes](mailboxes.md).

## I archived or deleted the wrong message

Press <kbd>z</kbd>. Changes wait at least ten seconds, and until the next sync
after that, before they reach the server; <kbd>z</kbd> takes back the most
recent one each time you press it. Deleted mail is in the Trash in any case —
kuverta never empties it.

## My scheduled message did not go out

Scheduled mail is sent by kuverta, so it has to be running. If it was closed at
the time, the message goes the next time it opens. If it could not be sent,
**Scheduled** in the sidebar says so, with the reason; **Send now** or **Retry
at…** tries again. A send interrupted by kuverta closing is never retried by
itself — check your Sent mailbox first. See [Writing](writing.md#scheduled-mail).

## A mailbox cannot be deleted

kuverta only deletes an empty mailbox that has no mailboxes inside it, and never
the Inbox, Sent, Drafts, Archive, Junk or Trash. Move its messages out first.

## Scanned letters show nonsense text

That is Paperless's OCR on a difficult scan. Press **Read with the vision
model** above the text, or switch to the scan with **PDF** (<kbd>v</kbd>). If the
vision model fails, the note above the text says why — often that Ollama is not
running or the model is not downloaded. See [Paper post](paper.md).

## A postal address shows no post

Press **Verify** in its settings: it says how many documents the address
matches, and when it is none, which tags exist — a mistyped tag looks exactly
like an address that has had no post. Also check that a token is stored.

## The setup assistant does not find my Apple Mail accounts

macOS only shows them to programs with **Full Disk Access**: System Settings →
Privacy & Security → Full Disk Access → kuverta.

## Where is my data, and how do I remove it?

See [Privacy](privacy.md#on-your-computer) and
[Install → Uninstalling](../install.md#uninstalling).

## Can I use kuverta and another mail program at the same time?

Yes. kuverta works on the mail on your server like any IMAP client; it checks
every change against the server before writing it, and refuses rather than
guessing when another program got there first.

## What is not there yet

kuverta is usable, not finished. Not there yet:

- fetching new mail in the background (IDLE) — press <kbd>r</kbd>;
- attachments in compose, and HTML mail shown as HTML;
- OAuth2 sign-in from the window — it needs the `kuverta` command line;
- automatic updates — kuverta says when a release is out, and you install it;
- signed and notarised macOS builds, signed Windows and Linux builds, and Linux on ARM;
- for encryption: encrypted subjects, Autocrypt, key lookup, S/MIME
  ([more](encryption.md#not-there-yet)).

## Something else is wrong

[Report a bug](diagnostics.md#reporting-a-bug), with the exported log.

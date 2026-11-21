---
description: The message list, the six categories, picking several messages, attachments, searching, and how undo works.
---

# Reading and sorting

## The list

The list shows the sidebar choice: a mailbox, a smart mailbox or **All mail**,
optionally narrowed to one category and **Unread only**. The line above the
list names the filters, and **clear** drops them all.

Mail is sorted by date. **newest** and **oldest** at the top of the list set
which end it starts from (oldest first suits working through a folder from the
beginning). The choice is remembered.

Each row shows the sender, the subject, the start of the text, a paperclip for
an attachment, the category and the date (the time for today, the weekday for
this week, the date before that). A dot marks unread mail.

When you archive or delete the message under the cursor, the next one moves
into its place. The list does not jump back to the top.

### Picking several messages

| To | Do |
| --- | --- |
| Pick a block | <kbd>Shift</kbd>+<kbd>j</kbd> / <kbd>Shift</kbd>+<kbd>k</kbd>, <kbd>Shift</kbd>+arrow keys, or <kbd>Shift</kbd>+click |
| Pick one more, or drop one | <kbd>x</kbd>, or <kbd>⌘</kbd>/<kbd>Ctrl</kbd>+click |
| Drop the whole selection | **unpick** above the list, or <kbd>Esc</kbd> |

With more than one picked, the line above the list says how many, and archive,
delete and mark read act on all of them. Picking does not open a message.

## Categories

A rules classifier files every message into one of six categories. It reads
the headers (`List-Id`, `List-Unsubscribe`, `Precedence`, `Auto-Submitted`, the
sender, the subject, the number of recipients) in German and English:

| Category | Means |
| --- | --- |
| **personal** | A person wrote this to you and probably expects an answer. |
| **newsletter** | Mail you subscribed to: lists, digests, newsletters. |
| **marketing** | Promotional bulk mail, subscribed to or not. |
| **transactional** | Invoices, receipts, orders, bank statements (money and records). |
| **notification** | Machine-generated status: CI, cron jobs, alerts, delivery notices. |
| **unknown** | No signal strong enough to guess. This is a real answer. |

The **personal** category is for people. Mail from a department or a company
does not count, however it is signed: an address like `info@`, `service@` or
`kundenbetreuung@`, a sender named after its own domain (*Sparkasse
Musterstadt* from sparkasse-musterstadt.de), or a domain for sending bulk mail
(`news.…`, `email.…`). Mail a platform relays from a person, such as *Anna über
Kleinanzeigen*, still counts as personal.

When an update changes the rules, kuverta files your mail again after the next
sync, from the stored copy of each message. What you filed yourself stays put.

The categories in the sidebar are filters with counts. Click one to narrow the
list to it, and click it again to remove the filter. Inside a mailbox or smart
mailbox, the heading says *Categories in …* and the counts are for that
mailbox.

### Why a message is where it is

In [Cleanup](cleanup.md), opening a message shows the rules that gave it its
category, in words. If you filed it yourself and the rules would have said
something else, it says that.

### Correcting a category

In Cleanup, press <kbd>1</kbd> to <kbd>6</kbd> to file the message under a
category, in the order of the table above (<kbd>1</kbd> personal …
<kbd>6</kbd> unknown). A correction overrides the rules: a sender or list you
have filed once is filed that way from then on. Every correction is also kept,
because a record of how you sort your mail cannot be collected later.

Scanned post is filed the same way. See [Paper post](paper.md#filing-post).

## Attachments

An open message lists its attachments under its header, one button each with
the file's name and size. Click one to see it:

![An eSIM's QR code opened from its mail](../assets/screens/attachment-viewer-light.webp#only-light)
![An eSIM's QR code opened from its mail](../assets/screens/attachment-viewer-dark.webp#only-dark)

- **Pictures, PDFs and plain text** open in a viewer over the window. A QR code
  sent as a picture can be scanned from the screen with a phone.
- **Save to Downloads** writes it to your Downloads folder and shows it there.
  An existing file is never overwritten: a second copy is named `name (2)`.
- **Open in another app** hands it to your computer's program for that kind of
  file (Preview, a PDF reader, your office suite).
- **A PDF locked with a password**, such as a bank statement or a payslip, asks
  for it and then opens like any other. The password is not saved, not put in
  your keychain, and forgotten when you close the viewer. PDFs locked only
  against printing or copying open without asking.

Pictures inside a message's text (mostly logos) are left out of the list.
**N pictures in the text** shows them too.

Files that could run something are not opened from kuverta. Programs, scripts,
installers, disk images, Office files with macros and web pages are shown in
red, and **Open in another app** is switched off for them, because a web page
in a mail is how a fake sign-in form arrives. You can still save one and open
it yourself if you trust its source.

For [encrypted mail](encryption.md) the attachments are the ones inside the
encryption. They are decrypted when you open them and never stored decrypted.
Mail that cannot be decrypted shows none.

## Searching

Press <kbd>/</kbd> (or click the search box), type, and press <kbd>Enter</kbd>.
Search is full-text over the account's senders, subjects and text. Results are
ranked by relevance, and the date plays no part. <kbd>Esc</kbd> goes back to
the list you came from.

Search allows for different ways of writing:

- Each word matches the start of a word, so `rechnung` finds
  *Rechnungsnummer* and `steuer` finds *Steuererklärung*.
- The words can be in any order and anywhere in the message: `esim
  vodafone` finds mail that has both.
- Spellings are tried both ways: `esim` finds *eSIM*, *e-SIM* and *e SIM*;
  `email` finds *E-Mail*.
- Double quotes find only that exact phrase: `"Rust Weekly"`.

A word *inside* a longer one is not found: `steuer` does not find
*Einkommensteuer*. Search for the start of the long word, or ask the
[assistant](assistant.md#finding-a-message), which tries several words.

In a postal address, search uses Paperless's own full-text search over the
scanned text.

## Archive, delete, read and unread

| | Key | Button |
| --- | --- | --- |
| Archive | <kbd>e</kbd> | **Archive** |
| Move to Trash | <kbd>Backspace</kbd>, <kbd>Delete</kbd> or <kbd>#</kbd> | **Delete** |
| Read / unread | <kbd>u</kbd> | |
| Undo the last change | <kbd>z</kbd> | |

Nothing destroys mail. Delete moves a message to the Trash, and kuverta never
empties a Trash or expunges a folder. Archive needs an Archive folder. If the
account has none, kuverta says so, and you can [create one](mailboxes.md).

After deleting a single message kuverta may offer the
[messages that look like it](similar.md).

## Undo

Every change (archive, delete, move, read or unread) goes into a queue on your
computer and leaves the list at once. It is held for at least ten seconds and
reaches the server with the next sync after that. Until then, <kbd>z</kbd>
takes back the most recent change, and pressing it again takes back the one
before. The notice at the bottom of the window says what was undone.

Before writing a queued change, kuverta checks that the message is still where
it was and is still the same message. If not, for example because another mail
program moved it, the change is refused with a reason, so it never acts on the
wrong message.

If kuverta is closed with changes still waiting, they are sent with the next
sync after it opens again. It says how many are waiting when it starts.

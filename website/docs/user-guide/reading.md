---
description: The message list, the six categories, picking several messages, searching, and how undo works.
---

# Reading and sorting

## The list

The list shows whatever is chosen in the sidebar: a mailbox, a smart mailbox,
**All mail**, and optionally one category and **Unread only** on top. The line
above the list says exactly that, with **clear** to drop every filter at once.

Mail is sorted by date. **newest** and **oldest** at the top of the list choose
which end it starts from; oldest first is for working through a folder from the
beginning. The choice is remembered.

Each row shows the sender, the subject, the start of the text, a paperclip when
there is an attachment, the category, and the date — the time for today, the
weekday for this week, the date before that. A dot marks unread mail.

**Acting keeps your place.** Archive or delete the message under the cursor and
the next one moves up into its place; the list does not jump back to the top.

### Picking several messages

| To | Do |
| --- | --- |
| Pick a block | <kbd>Shift</kbd>+<kbd>j</kbd> / <kbd>Shift</kbd>+<kbd>k</kbd>, <kbd>Shift</kbd>+arrow keys, or <kbd>Shift</kbd>+click |
| Pick one more, or drop one | <kbd>x</kbd>, or <kbd>⌘</kbd>/<kbd>Ctrl</kbd>+click |
| Drop the whole selection | **unpick** above the list, or <kbd>Esc</kbd> |

While more than one is picked, the line above the list says how many, and
archive, delete and mark read act on all of them. Picking does not open a
message — picking several and reading one are different things.

## Categories

Every message is filed into one of six categories by a rules classifier that
reads the headers — `List-Id`, `List-Unsubscribe`, `Precedence`,
`Auto-Submitted`, the sender, the subject, the number of recipients — in German
and English:

| Category | Means |
| --- | --- |
| **personal** | A person wrote this to you and probably expects an answer. |
| **newsletter** | Mail you subscribed to: lists, digests, newsletters. |
| **marketing** | Promotional bulk mail, subscribed to or not. |
| **transactional** | Invoices, receipts, orders, bank statements — money and records. |
| **notification** | Machine-generated status: CI, cron jobs, alerts, delivery notices. |
| **unknown** | No signal strong enough to guess. A real answer, not a fallback. |

The categories in the sidebar are filters with counts. Click one to narrow the
list to it and click it again to let go; inside a mailbox or smart mailbox, the
heading says *Categories in …* and the counts are for that mailbox.

### Why a message is where it is

In [Cleanup](cleanup.md), opening a message shows why it has its category: the
rules that fired, in words. If you filed it yourself and the rules would have
said something else, it says that instead.

### Correcting a category

In Cleanup, press <kbd>1</kbd>–<kbd>6</kbd> to file the message under a category,
in the order of the table above (<kbd>1</kbd> personal … <kbd>6</kbd> unknown).
A correction overrides the rules outright: a sender or list you have filed once
is filed that way from then on. Every correction is kept, too — a record of how
you sort your own mail is not something that can be collected later.

Scanned post is filed the same way; see [Paper post](paper.md#filing-post).

## Searching

Press <kbd>/</kbd> (or click the search box), type, and press <kbd>Enter</kbd>.
Search is full-text over the account's mail — senders, subjects and text — and
results are ranked by relevance rather than date. <kbd>Esc</kbd> goes back to
the list you came from.

In a postal address, search uses Paperless's own full-text search over the
scanned text.

## Archive, delete, read and unread

| | Key | Button |
| --- | --- | --- |
| Archive | <kbd>e</kbd> | **Archive** |
| Move to Trash | <kbd>Backspace</kbd>, <kbd>Delete</kbd> or <kbd>#</kbd> | **Delete** |
| Read / unread | <kbd>u</kbd> | |
| Undo the last change | <kbd>z</kbd> | |

**Nothing destroys mail.** Delete moves a message to the Trash; kuverta never
empties a Trash or expunges a folder. Archive needs an Archive folder — if the
account has none, kuverta says so, and you can [create one](mailboxes.md).

After deleting a single message kuverta may offer the
[messages that look like it](similar.md).

## Undo

Every change — archive, delete, move, read or unread — first goes into a queue
on your computer and leaves the list at once. It is held there for at least ten
seconds and reaches the server with the next sync after that. Until then,
<kbd>z</kbd> takes back the most recent change, and pressing it again takes back
the one before. The notice at the bottom of the window says what was undone.

Before a queued change is written, kuverta checks that the message it is about
is still where it was and is still the same message. Anything that no longer
lines up — because another mail program moved it in the meantime, say — is
refused with a reason instead of acting on the wrong message.

If kuverta is closed with changes still waiting, they are sent with the next
sync after it opens again; it says how many are waiting when it starts.

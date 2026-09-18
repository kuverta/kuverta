---
description: A first tour of the kuverta window — the sidebar, the list, the reading pane and the header.
---

# Getting started

This guide is for people using kuverta. If you have not installed it yet, start
with [Install](../install.md); the setup assistant that opens on the first run
is described there too.

Screenshots in this guide were taken against a test mailbox, so the names and
messages in them are made up.

## The window

![The kuverta window](../assets/screens/main-light.webp#only-light)
![The kuverta window](../assets/screens/main-dark.webp#only-dark)

kuverta has three panes and a header.

**The header** holds, from left to right: the account being shown and what the
sync is doing (with a progress bar during a long sync), **New message**, the
search box, **Cleanup** with a count of the bulk mail still in your Inbox, and
the settings button. A notice appears here when a newer kuverta is out.

**The sidebar** on the left is the only place you choose what the list shows:

- the **profile** switcher — All, Private, a company — once you have
  [profiles](profiles.md);
- **Needs attention** — your Inbox [ranked by how soon it needs you](attention.md);
- your **accounts**, when there is more than one (or when you also have post);
- **Post** — each postal address, whose scanned letters come from
  [Paperless-ngx](paper.md);
- **Mailboxes** — the account's folders on the server, with unread and total
  counts, and **+** to [make a new one](mailboxes.md);
- **Smart mailboxes** — [saved searches](smart-mailboxes.md), with **+** to add one;
- **Scheduled** — mail waiting to be [sent later](writing.md#send-later), shown
  only while there is some;
- **Categories** — **Unread only**, and the categories the classifier sorts
  mail into, with counts. Inside a folder, the counts are for that folder.

**The list** in the middle shows what is selected in the sidebar. The line at
its top always says what you are looking at — "12 in INBOX · newsletter", for
example — with **clear** to go back to everything, and **newest** / **oldest**
to choose which end the list starts at. A filtered list that did not say so
would look exactly like mail going missing.

**The reading pane** on the right shows the open message with **Reply**,
**Reply all**, **Forward**, **Archive** and **Delete** above it. Messages are
shown as text — HTML mail as the text in it — so nothing in a message is loaded
from the internet and no tracking pixel learns that you opened it.

Along the bottom is a strip of the most useful keys. The complete list is in
[Keyboard shortcuts](shortcuts.md).

## The first few minutes

1. **Sync.** New mail is fetched when you press <kbd>r</kbd> — kuverta does not
   yet fetch it on its own in the background. A sync also sends the changes you
   have made — archiving, deleting, marking read — once they have left their
   undo window.
2. **Move through the list** with <kbd>j</kbd> and <kbd>k</kbd> (or the arrow
   keys) and open a message with <kbd>Enter</kbd>, or click it.
3. **Deal with it**: <kbd>e</kbd> archives, <kbd>Backspace</kbd> or <kbd>#</kbd>
   moves it to the Trash, <kbd>u</kbd> toggles read and unread, and
   <kbd>z</kbd> undoes the last change. Or use the buttons above the message.
4. **Answer it**: <kbd>R</kbd> replies, <kbd>A</kbd> replies to all,
   <kbd>f</kbd> forwards, <kbd>c</kbd> starts a new message.
5. **Clear out the rest**: open [Cleanup](cleanup.md) from the header to work
   through newsletters, marketing and notifications a whole kind at a time, and
   to unsubscribe from the ones you never read.

## Adding an account by hand

The setup assistant finds most accounts by itself. For any other, open
**Settings** (<kbd>,</kbd>), press **+ Add** beside *Accounts*, fill in the
servers, and press **Verify** before **Save**: Verify signs in and reports what
the server supports, where sent, archived and deleted mail will go, and how much
a first sync would download. See [Settings](settings.md#accounts).

## Where to go next

- [Reading and sorting](reading.md) — the list, categories, picking several, undo
- [Writing](writing.md) — replying, forwarding, and sending later
- [Cleanup and unsubscribing](cleanup.md)
- [Paper post](paper.md) — scanned letters beside your mail
- [Privacy](privacy.md) — what is stored where, and what is sent where

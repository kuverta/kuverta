---
description: Reading scanned paper post in kuverta — Paperless-ngx as a postal address's server, the vision model, filing letters, and the Raspberry Pi scanner.
---

# Paper post

kuverta treats a **postal address as an account**, and
[Paperless-ngx](https://docs.paperless-ngx.com/) as its server. Paperless does
the scanning intake, the OCR, the archive and the search; kuverta reads what it
holds and shows it like mail — the correspondent is the sender, the title is the
subject, the text of the scan is the body, and the date on the letter is when it
arrived. The same classifier sorts it.

kuverta **only reads** from Paperless. It never changes, moves or deletes a
document there.

## Setting up a postal address

The [setup assistant](../install.md#the-first-run) can connect to a Paperless
you have or install one for you with Docker. To add one by hand, open
**Settings** and press **+ Add** beside *Postal addresses*:

- **Description** — the name in the sidebar, such as *Home*.
- **Paperless-ngx address** — for example `http://paperless.local:8000`.
- **Which post belongs here** — *all of them*, or only documents *with the tag*,
  *from the correspondent*, or *in the storage path* you name. This is how one
  Paperless serves several addresses: give each its own tag.
- **API token** — from Paperless, under your name → *My Profile*. It is kept in
  the system keychain.

Press **Verify** before **Save**. It says how many documents the address
matches — and when it matches none, it names the tags that do exist, because a
mistyped tag looks exactly like an address that has had no post.

## Reading post

Each postal address appears under **Post** in the sidebar with its unread
count. Click it to see its letters, newest first.

- The line above the list switches between sorting by **letter date** (the date
  on the letter) and **scanned** (when it came through the letterbox).
- Opening a letter marks it read; <kbd>u</kbd> toggles it. Read state for post is
  kept by kuverta, since Paperless has none.
- **Text** and **PDF** above the letter — or <kbd>v</kbd> — switch between the
  text and the scan itself. The choice is kept from letter to letter.
- Search (<kbd>/</kbd>) uses Paperless's full-text search over the scanned text.
- The categories in the sidebar count and filter post as they do mail.
- Paperless does not tell anyone when a letter arrives, so kuverta asks: the
  open postal address every twenty seconds or so, and new letters appear on
  their own. <kbd>r</kbd> asks at once.

Post cannot be replied to, archived or deleted from kuverta — it belongs to
Paperless. Those keys say so rather than doing something to the account behind
it.

## Reading scans with the vision model

Paperless's OCR is often good and sometimes unreadable, especially for
photographed letters. kuverta can have a **vision model** — a model that can
look at images — read the scan instead.

New letters that nobody has opened yet are read this way as they arrive, one at
a time, so the list shows who a letter is from and what it is about, and sorts
it, from text a person could read. A note above the text says whose reading it
is:

- *This is Paperless's OCR.* — with **Read with the vision model** to have the
  scan read.
- *Read from the scan by …* — the model's reading, with **Read again**. Where
  the scan is unclear it can get a word wrong; the PDF tab has the page itself.

Which model reads scans, and where it runs, is chosen in
[Settings → Models](models.md). By default it is a small model run by Ollama on
your computer, and the scans do not leave it. If the model fails — Ollama is not
running, say — kuverta stops trying for new post and says why above the text.

## Filing post

In [Cleanup](cleanup.md), letters can be filed with <kbd>1</kbd>–<kbd>6</kbd>
like mail. Filing a letter **pins** it to that category, whatever the rules
make of it. When Paperless knows who sent it, the correspondent is learned too:
the next letter from the same sender is filed the same way without asking.

## Capturing post: the scanner

Paperless can take documents from any scanner. The project also has its own:
[`scannerd`](../developers/scanner.md), a small program for a Raspberry Pi with
a camera fixed above a spot on a table. It notices when a page is put down,
photographs it once it has stopped moving, and hands it to Paperless — tagged
for your postal address. A capture is kept on the Pi until Paperless has
confirmed it, so a dropped network or a flat battery costs a retry, not a
letter.

It has a setup page you open on your phone, and optionally a button to press
when a letter of several pages is complete. Building and installing it is
described for [developers](../developers/scanner.md).

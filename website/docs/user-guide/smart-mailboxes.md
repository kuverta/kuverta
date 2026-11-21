---
description: Smart mailboxes in kuverta, saved searches in the sidebar, every rule they can use, and importing them from Thunderbird and Apple Mail.
---

# Smart mailboxes

A smart mailbox gathers mail by rules (who sent it, what it says, how old it
is) without moving any of it. It sits in the sidebar under **Smart mailboxes**
with its unread and total counts, and works like any mailbox: click it to see
what it gathers, and narrow that by category.

A smart mailbox belongs to one account and looks at every message of that
account, whichever mailbox it is in.

## Making one

- Press **+** beside *Smart mailboxes* in the sidebar,
- **New smart mailbox…** in Settings → Smart mailboxes, or
- **Smart mailbox…** when kuverta offers [similar messages](similar.md) after a
  delete. The editor then opens with rules that would catch the next one.

![The smart mailbox editor with three rules and the preview below](../assets/screens/smart-editor.webp)

Give it a name, choose whether mail has to match **all** or **any** of the rules,
and add rules with **+ Add a rule** (**×** removes one). The box at the bottom
previews the result while you write: how many messages match, and the first
few. A rule that matches nothing shows before you save. Rules you are still
typing are left out of the preview and are not shown as errors.

**Save** creates it and opens it.

To change or delete one, right-click it in the sidebar and choose **Edit…** or
**Delete**, or use the list in Settings → Smart mailboxes. Deleting a smart
mailbox deletes no mail.

## Rules

| Field | Comparisons | Value |
| --- | --- | --- |
| **Sender** | contains, does not contain, is, is not, begins with, ends with | the sender's name and address, e.g. `@shop.example` or `Erika` |
| **Recipient** | contains, does not contain, is, is not, begins with, ends with | the To and Cc addresses |
| **Subject** | contains, does not contain, is, is not, begins with, ends with | text |
| **Message text** | contains, does not contain | words, searched in the text of the message |
| **Category** | is, is not | personal, newsletter, marketing, transactional, notification, unknown |
| **Mailing list** | contains, does not contain, is, is not, begins with, ends with | the list's id (`List-Id`), e.g. `news.example` |
| **Mailbox** | is, is not, contains | any mailbox the message has a copy in, e.g. `Archive`; names are offered as you type |
| **Unread** | is | yes or no |
| **Has an attachment** | is | yes or no |
| **Can be unsubscribed from** | is | yes or no: whether the mail carries `List-Unsubscribe` |
| **Older than (days)** | | sent more than this many days ago |
| **Received in the last (days)** | | sent within this many days |

Text comparisons ignore upper and lower case: `rechnung` finds *Rechnung*.

A few examples:

| Smart mailbox | Match | Rules |
| --- | --- | --- |
| Receipts | any | Subject contains `receipt` · Subject contains `Rechnung` |
| Unread from people | all | Category is personal · Unread is yes |
| Old newsletters | all | Category is newsletter · Older than (days) 30 |
| Things to stop | all | Can be unsubscribed from is yes · Received in the last (days) 7 |

## Bringing them over from Thunderbird and Apple Mail

**Settings → Smart mailboxes → Look for smart mailboxes** reads:

- **Thunderbird**'s saved searches, from `virtualFolders.dat` in each of its
  profiles, and
- **Apple Mail**'s smart mailboxes, from its `SmartMailboxes.plist`. macOS only
  lets a program read that with **Full Disk Access** (System Settings → Privacy &
  Security). Without it, kuverta says so.

![Settings → Smart mailboxes, with the import section below](../assets/screens/settings-smart-light.webp#only-light)
![Settings → Smart mailboxes](../assets/screens/settings-smart-dark.webp#only-dark)

Each one found is listed with its rules as kuverta understood them and the
account it will go to: the one it belonged to if kuverta can match it,
otherwise the one you are looking at. Tick the ones you want and press
**Import the chosen ones**.

The three programs' rules do not line up exactly. An import translates what it
can and lists the rest under the smart mailbox as *Left out: …*, because a
smart mailbox with a rule missing gathers more than the original. One with no
rules left cannot be ticked.

Importing twice makes no duplicates: a smart mailbox with the same name on
that account is shown as *already imported*. Imported smart mailboxes
are marked **Thunderbird** or **Apple Mail** in the settings list and are
ordinary smart mailboxes from then on.

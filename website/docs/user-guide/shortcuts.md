---
description: Every keyboard shortcut in kuverta's main window and in Cleanup.
---

# Keyboard shortcuts

Keys are case-sensitive: <kbd>R</kbd> (reply) is not <kbd>r</kbd> (sync). None of
them fire while you are typing in a field — typing an `e` into a subject line
never archives anything.

## Main window

### Moving and picking

| Key | |
| --- | --- |
| <kbd>j</kbd> or <kbd>↓</kbd> | next message |
| <kbd>k</kbd> or <kbd>↑</kbd> | previous message |
| <kbd>J</kbd> / <kbd>K</kbd>, or <kbd>Shift</kbd>+<kbd>↓</kbd> / <kbd>Shift</kbd>+<kbd>↑</kbd> | pick a block of messages |
| <kbd>x</kbd> | pick or unpick the message under the cursor |
| <kbd>Enter</kbd> or <kbd>o</kbd> | open the message |
| <kbd>Esc</kbd> | drop the selection and close the message |
| <kbd>/</kbd> | search; <kbd>Enter</kbd> runs it, <kbd>Esc</kbd> goes back |

With the mouse: <kbd>Shift</kbd>+click picks a block, <kbd>⌘</kbd>/<kbd>Ctrl</kbd>+click
picks one more.

### Acting on mail

| Key | |
| --- | --- |
| <kbd>e</kbd> | archive |
| <kbd>Backspace</kbd>, <kbd>Delete</kbd> or <kbd>#</kbd> | move to Trash |
| <kbd>u</kbd> | mark read / unread |
| <kbd>z</kbd> | undo the last change not yet sent |
| <kbd>r</kbd> | sync: send waiting changes, fetch new mail |

### Writing

| Key | |
| --- | --- |
| <kbd>c</kbd> | new message |
| <kbd>R</kbd> | reply |
| <kbd>A</kbd> | reply all |
| <kbd>f</kbd> | forward |
| <kbd>⌘</kbd>/<kbd>Ctrl</kbd>+<kbd>Enter</kbd> | send (in compose) |
| <kbd>Esc</kbd> | discard (in compose) |

### Post and settings

| Key | |
| --- | --- |
| <kbd>v</kbd> | switch a letter between its text and the scan |
| <kbd>i</kbd> | open or close the [assistant](assistant.md) |
| <kbd>Shift</kbd>+<kbd>I</kbd> | hand the message under the cursor — or the ones picked — to the assistant |
| Ask the assistant about the message under the cursor, or the ones picked |
| <kbd>,</kbd> | settings |
| <kbd>Esc</kbd> | close settings, a dialog or a menu |

In a postal address, the keys that change mail — archive, delete, undo, reply —
do nothing but say that post cannot be changed from here.

## Cleanup

Cleanup has the same keys for moving and acting, plus:

| Key | |
| --- | --- |
| <kbd>1</kbd> | file as personal |
| <kbd>2</kbd> | file as newsletter |
| <kbd>3</kbd> | file as marketing |
| <kbd>4</kbd> | file as transactional |
| <kbd>5</kbd> | file as notification |
| <kbd>6</kbd> | file as unknown |
| <kbd>?</kbd> | what Cleanup is for, and every key |
| <kbd>Esc</kbd> | close the message; with none open, clear the filter |

In Cleanup, <kbd>#</kbd> and <kbd>Delete</kbd> move to Trash (not
<kbd>Backspace</kbd>), and keys with <kbd>⌘</kbd>, <kbd>Ctrl</kbd> or
<kbd>Alt</kbd> are left to the system — <kbd>⌘</kbd>+<kbd>C</kbd> still copies.
While the Unsubscribe list is open, only its search box takes keys, and
<kbd>Esc</kbd> closes it.

The same keys work in the [Thunderbird extension](../developers/thunderbird.md).

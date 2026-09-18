---
description: kuverta's log, detailed logging, exporting the log, and how to report a bug.
---

# Diagnostics and reporting a bug

A bug report without a log is a description of a symptom. kuverta keeps a log
you can read, turn up, and send.

## The log

Everything kuverta records goes to a file in its
[data directory](privacy.md#on-your-computer):

```text
<data dir>/logs/kuverta.log
```

— on macOS and Linux usually `~/.local/share/kuverta/logs/kuverta.log`, on
Windows `%APPDATA%\kuverta\logs\kuverta.log`. When it grows past 4 MB it is
started afresh at the next launch, and the previous one is kept as
`kuverta.1.log`.

Errors the window shows you are written to the log too, so a log has both
halves of what went wrong.

!!! info "What is in the log"
    The log **never** holds passwords, tokens or the text of your messages. It
    **does** hold email addresses and subjects. Read it before you send it
    anywhere public, and remove anything you would rather not share.

## Settings → Diagnostics

![Settings → Diagnostics](../assets/screens/settings-diagnostics-light.webp#only-light)
![Settings → Diagnostics](../assets/screens/settings-diagnostics-dark.webp#only-dark)

- **Detailed logging** — *Record what kuverta does in detail.* Switches
  kuverta's own components to debug level. It takes effect immediately, without
  a restart, and stays on across restarts until you turn it off. The network
  libraries underneath are left at their normal level, because theirs would
  record protocol traffic.
- Below it, where the log is kept and how large it is.
- **Recent log** — the last few hundred lines, newest at the bottom, with
  **Refresh** and **Copy**.
- **Export log…** — writes the log, and the one before it, to your
  **Downloads** folder as `kuverta-log-<date and time>.txt` and shows it in your
  file manager. A short header says which kuverta and which system produced it,
  and whether detailed logging was on.

## Reporting a bug

1. In **Settings → Diagnostics**, turn on **Detailed logging**.
2. Do what went wrong again.
3. Press **Export log…**.
4. Open an issue at
   [github.com/kuverta/kuverta/issues](https://github.com/kuverta/kuverta/issues):
   what you did, what you expected, what happened instead, your system and the
   kuverta version (at the foot of the settings list) — and attach the exported
   log, after reading it.
5. Turn detailed logging off again; detailed logs grow faster.

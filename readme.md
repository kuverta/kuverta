# fuckmail

Working title. A **mail client** built around triage: it syncs your IMAP
accounts into a local store, classifies everything, and gives you one
keyboard-first surface to read, reply and file from. Scanned paper mail is
meant to land in the same place.

It started as a read-only triage layer alongside a normal client. That changed
— triage alone never replaced Apple Mail, because every reply meant leaving the
app. Write capability went in cheapest-and-safest first: **send**, which is
additive, then **mailbox mutation**, which is not. See §1a of
[docs/implementation-plan.md](docs/implementation-plan.md) for what each stage
cost and what replaced the safety the first one had for free.

Status: **usable, not finished.** The window syncs, reads, triages, replies and
sends against real providers. There is no local model yet, and no paper half.

## What works today

- **Sync** — IMAP into SQLite + FTS5, incremental via `HIGHESTMODSEQ`, flag
  changes via `CHANGEDSINCE`, server-side deletions reconciled
- **Message identity** — one message in several folders is stored once with
  several locations, which is what Gmail's labels-as-folders needs
- **Send** — compose, reply, reply-all, forward, SMTP submission, and a copy
  filed in Sent via `APPEND`
- **Triage** — archive, delete, move, mark read/unread, with a durable queue,
  an undo window, and conflict checks before anything is written
- **A desktop window** — three panes, a sidebar of accounts and folders,
  virtualized list (200k rows at 59.8 fps —
  [findings](docs/spike-tauri-list.md)), compose, search, and settings
- **Auth** — app passwords in the OS keychain, the OAuth2 device flow
  (RFC 8628) for Microsoft 365, and the loopback flow (RFC 8252, with PKCE)
  for Gmail, which does not accept the device grant
- **A rules classifier** — six categories, German and English, corrections
  override the heuristics
- **A Docker dev stack** — a seeded Dovecot, an SMTP sink, and a second Dovecot
  wearing Gmail's folder layout, so none of it needs a real mailbox

Not yet: the local model, IDLE, QRESYNC, attachments in compose, HTML
rendering, and the Paperless half.

## Quick start

```sh
make dev-up     # Dovecot with nine fixture messages, an SMTP sink, a Gmail shape
make e2e        # register the dev account, sync it, send one, show the result
./run.sh --dev  # open the window on that scratch store
```

Expected: **9 fixtures, 8 messages, 9 locations.** The newsletter appears in
both INBOX and Archive under one Message-ID and must collapse to a single
message. If that number is 9, dedup has regressed.

## Using it on a real account

Either from the window — `./run.sh`, then `,` for settings, fill in the form
and press **Verify** before saving — or from the CLI:

```sh
cargo run -p fuckmail-cli -- add-account \
    --email you@example.de --host imap.example.de --port 993 \
    --smtp-host smtp.example.de --smtp-port 587 --smtp-security starttls
cargo run -p fuckmail-cli -- set-password --email you@example.de   # reads stdin
cargo run -p fuckmail-cli -- check --measure   # connect, report, download nothing
cargo run -p fuckmail-cli -- sync
```

`check` is worth running first on any account you have not synced before: it
reports whether the credentials work, which extensions the server has, where
mail will be filed, and how much a first sync would fetch.

**Gmail** needs either an app password (2-Step Verification, then
`myaccount.google.com/apppasswords`) or an OAuth2 client of type *Desktop app*
— `--auth oauth2 --oauth-provider google --client-id …`, then `login`. Google
is phasing app passwords out during 2026. Its `[Gmail]/All Mail` holds a copy
of everything, so `check` will suggest `exclude '\All'` and say what that costs.

**Microsoft 365** needs an Azure app registration and
`--auth oauth2 --client-id …`, then `login`.

## Layout

| Path | What |
|---|---|
| `crates/core-store` | SQLite store, dedup, FTS5 search, blobs, the change queue |
| `crates/core-proto` | IMAP client, MIME parsing, folder sync, the mutation executor |
| `crates/core-accounts` | Credentials behind an auth trait; both OAuth2 flows |
| `crates/core-rules` | The deterministic classifier |
| `crates/core-smtp` | Composing RFC 5322 messages and submitting them |
| `crates/core-rpc` | The typed surface both front ends talk to |
| `apps/cli` | Development driver — not the product |
| `apps/desktop` | The triage window |
| `docker/` | Dev stack: Dovecot, an SMTP sink, a Gmail shape, Ollama, Paperless |
| `docs/` | Plan, evaluation, original brief |

`make help` lists the useful targets. `run.sh --help` explains the window.

## Design decisions worth knowing before you change anything

**Both front ends run the same code.** `core-rpc` decides which copy of a
message to archive, what to draw for a missing subject, whether a change can
still be undone. The window and the CLI are both thin over it, so a decision is
tested once rather than implemented twice and drifting.

**Nothing destroys mail.** There is no expunge operation — delete means move to
Trash — and a bare `EXPUNGE` is never sent, even as part of a move fallback,
because it would remove every `\Deleted` message in the folder including ones
another client marked.

**No UID is trusted on its own.** Before any write, the executor checks that
the folder still has the UIDVALIDITY the change was queued under, that there is
still a message at the UID, and that its `Message-ID` is the expected one.
Anything that does not line up is refused with a reason. See
`core-proto/src/mutate.rs`.

**Bcc reaches the envelope and never the message.** `Draft::build` computes the
serialised message and the SMTP envelope together and by hand, because
mail-send's own conversion puts Bcc in both. It is the one mistake here that
cannot be walked back.

**Dedup is per account, never global.** The same message on two of your
accounts really is two copies. See `core-store/src/dedup.rs`.

**A sync must be cheap when nothing changed.** Folders whose `HIGHESTMODSEQ`
matches the stored one are skipped without a fetch — on a quiet mailbox that is
every folder. Anything added to the per-folder path has to preserve that.

**Orphans are collected once, at the end of a sync.** A message being moved is
out of its old folder before it appears in the new one; collecting as you go
destroys the row in between, along with its classifier verdict, its queued
operations and the user's corrections. Whether that happened used to depend on
the order the server listed folders in.

**The list must page over the IPC bridge.** Tauri serialises commands as JSON
at roughly 78 MiB/s, so shipping a whole mailbox across it stalls visibly.
Request the visible slice, not everything.

**Getting a credential must never wait for a human.** `AuthProvider::credential`
runs inside sync, so an expired login fails with an error naming the command to
fix it. Only `login` is interactive.

**Passwords go one way.** They are written to the keychain and never read back
into a settings view, which carries `has_password` and no secret.

**Corrections are captured from day one.** Nothing consumes the `correction`
table until the model lands; it is being filled now so there is training data
when that happens.

## Licence

Not yet licensed. Intended to be AGPL-3.0 when first published — see decision 8
in the plan.

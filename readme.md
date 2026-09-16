# kuverta

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

## Two halves

The repository holds a Rust mail client and a JavaScript triage surface, and
they are not the same thing wearing two hats:

- **The Rust half** — `crates/` and `apps/` — is a mail client. IMAP, SMTP, the
  store, accounts, the deterministic classifier.
- **The JavaScript half** — [`kuverta-bird/`](kuverta-bird/) — is the triage surface and
  the classifier ported to run wherever the mail is. It mounts over this client
  *and* over Thunderbird as an extension, from the same code, behind a contract
  each host implements.

The second exists because of the add-on ecosystem: Thunderbird extensions need
Gecko, and Gecko cannot be embedded outside Mozilla's own applications, so
running them means *being* a Thunderbird rather than talking to one. Rather than
choose, the surface was written once and given two hosts. See
[docs/thunderbird-fork-brief.md](docs/thunderbird-fork-brief.md) for the
reasoning and [docs/decisions.md](docs/decisions.md) for what it has cost.

`make triage-ui` copies the surface into the desktop app; then click **triage**
in its header.

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

One command, if the scratch store already has mail in it:

```sh
make triage     # copy the surface in, build, open the window
```

Then click **triage** in the window's header. It opens on `.devdata`, a
gitignored scratch store that is not your real mail.

To fill that store from nothing, you need Docker running:

```sh
make dev-up     # Dovecot with nine fixture messages, an SMTP sink, a Gmail shape
make e2e        # register the dev account, sync it, send one, show the result
make triage     # open the window
```

Expected after `make e2e`: **9 fixtures, 8 messages, 9 locations.** The
newsletter appears in both INBOX and Archive under one Message-ID and must
collapse to a single message. If that number is 9, dedup has regressed.

Nine messages is enough to check that sync works and nowhere near enough to
feel like a mailbox — which is what triage needs in order to be worth looking
at. For that:

```sh
make fill-dev   # ~250 varied messages into the dev mailbox, then sync them
```

Both languages, every category the classifier knows, HTML-only marketing,
attachments, a threaded conversation, and dates spread over seven months so the
list has something realistic to sort.

Other useful commands:

```sh
make help       # every target, with a line each
make test       # both test suites: Rust, then the triage surface
./run.sh --help # what the window's options do
```

## First run

On a store with no accounts and no postal addresses, the window opens a setup
assistant instead of an empty list (again any time from settings, **Setup
assistant**). Every step can be skipped:

- **Local models** — whether Ollama is installed and running, with a download
  link or the install command if not, **Start Ollama** if it is installed but
  stopped, and a download button with progress for each model a job needs.
- **Paper mail** — looks for Paperless at `localhost:8000` and
  `paperless.local:8000`, then either connects to one (a Paperless user name and
  password are exchanged for that user's token; pasting a token works too) or
  installs one with Docker: a compose file and an owner-only env file in
  `<data dir>/paperless/`, `docker compose up -d`, and a wait until it answers.
  Docker itself is the person's to install; the assistant says where.
- **Mail accounts** — reads the IMAP accounts from Thunderbird's profiles and from
  Apple Mail (the system's Internet Accounts, which macOS only shows to a program
  with Full Disk Access; the assistant links to the setting). Any other address
  gets its servers from a built-in list of common providers, the provider's own
  autoconfig file, or Mozilla's directory, as Thunderbird does. Passwords are not
  imported — the other programs keep theirs encrypted — so each is entered once,
  stored in the keychain, and checked by signing in before the assistant moves on.
  New accounts start syncing when it finishes.

A marker file in the data directory stops it opening by itself again; a store
that already had accounts before the assistant existed never sees it.

## Using it on a real account

Either from the window — `./run.sh`, then `,` for settings, fill in the form
and press **Verify** before saving — or from the CLI:

```sh
cargo run -p kuverta-cli -- add-account \
    --email you@example.de --host imap.example.de --port 993 \
    --smtp-host smtp.example.de --smtp-port 587 --smtp-security starttls
cargo run -p kuverta-cli -- set-password --email you@example.de   # reads stdin
cargo run -p kuverta-cli -- check --measure   # connect, report, download nothing
cargo run -p kuverta-cli -- sync
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

## Paper

A physical address is an account, and [Paperless-ngx](https://docs.paperless-ngx.com/)
is its server. Post that arrives at it is mail: the correspondent wrote it, the
title is the subject, the OCR text is the body, and the date on the letter is
when it arrived. The same classifier files it — the rules read a sender, a
subject and some text, and a letter has all three.

It runs in the dev stack, so there is post to work with as well as mail:

```sh
make dev-up     # ...including Paperless-ngx on http://localhost:8000 (admin/admin)
```

Against that, or any existing instance:

```sh
export PAPERLESS_TOKEN=...          # Paperless: settings → users → create token
kuverta paper --check              # what is there, before anything depends on it
kuverta paper                      # list it, classified, like `kuverta list`
kuverta paper --tag home           # one address of several in one instance
kuverta paper --query rechnung     # full-text, over the OCR'd text
```

`--tag`, `--correspondent` and `--storage-path` are how one Paperless serves
several addresses. `--check` exists because a selector that matches nothing
looks exactly like an address that has had no post, which is the §3.6 lesson
applied to paper.

A physical address is set up like an account: **Settings → Add postal
address**, with the Paperless URL, which of its documents belong to this address
(all of them, or those with a tag, correspondent or storage path), and an API
token from Paperless's profile menu. **Verify** runs a preflight that says how
many documents the address matches — and names the tags that exist when it
matches none, because a mistyped tag looks exactly like an address with no post.

Post is filed like mail, with `1`–`6`. Filing a letter pins it, and if Paperless
knows who sent it, the next letter from that correspondent is filed the same way.

### Capturing it

[`scannerd`](apps/scannerd/) is the other end: a Pi with a camera over a fixed
spot, which notices when a page is put down, photographs it once it has stopped
moving, and hands it to Paperless. It is the only part of the paper pipeline
that is ours, and it stays small by refusing to do what upstream already does —
no deskew (ocrmypdf's job), no page-finding (the camera is bolted above a fixed
spot, so cropping is `--roi` rather than geometry), and no image library at all,
because raw YUV's first plane is already the greyscale image.

```sh
cargo test -p scannerd        # 26 tests, no camera needed
scannerd --drain-only         # upload whatever is spooled, and exit
```

A capture is written to the spool before any upload is attempted and removed
only once Paperless confirms it, so a flat battery between the two costs a
retry rather than a letter.

## The local model

A model can classify mail too, and is kept honest by never being trusted on its
own say-so. Its verdicts are recorded beside the rules' and compared; they never
change what the list shows.

```sh
kuverta classify --email you@example.com   # after a sync, never during one
kuverta disagreements                      # where it and the rules differ

python3 docker/fill-mailbox.py --dump 250 | node kuverta-bird/tools/dump-facts.js > labelled.jsonl
kuverta eval labelled.jsonl                # rules vs prompting vs embeddings
kuverta eval labelled.jsonl --split sender # the same, on senders never filed
```

Which model sorts mail, and which reads scanned post, is chosen in the desktop
app's settings under **Models**: the Ollama on this computer by default, another
Ollama, or a hosted service with an OpenAI-compatible API such as DeepSeek,
OpenAI or OpenRouter, its key kept in the keychain. The page lists each
provider's models, marks the ones that can see images where the provider says,
and says plainly when a job would send mail or scans off this computer.
`classify --model` overrides the chosen model; `--ollama` (or `OLLAMA_URL`) asks
that Ollama directly instead.

Measured on the generated corpus with an 8B model: prompting 100% on both
splits at ~620 ms a message, embeddings 100% on known senders and 79.5% on new
ones at 11 ms, the rules 86.4% where their known blind spot is scored. On stored
mail the model disagreed with the rules 30 times and was right in 29. The corpus
is templated, so read that as "not worse anywhere", not as a forecast — see
[docs/decisions.md](docs/decisions.md) §15.

`eval` also tries both together — the nearest filings when they are close and
agree, the model otherwise — at every threshold. At the default of 0.85 that is
100% on both splits, 11 ms for senders filed before, and the model for strangers
(§16).

## For an assistant

[`kuverta-mcp`](apps/mcp/) serves the mailbox — mail and scanned post — over
MCP on stdio. Read-only unless started with `--allow-writes`, and even then it
cannot send, cannot delete, and holds every change for five minutes before it
may reach the server, cancellable the whole time. An assistant's filing shows in
the list but does not teach the classifier; only yours do.

## Layout

| Path | What |
|---|---|
| `crates/core-store` | SQLite store, dedup, FTS5 search, blobs, the change queue |
| `crates/core-proto` | IMAP client, MIME parsing, folder sync, the mutation executor |
| `crates/core-accounts` | Credentials behind an auth trait; both OAuth2 flows |
| `crates/core-rules` | The deterministic classifier |
| `crates/core-ai` | The local model: prompting and embeddings over Ollama |
| `crates/core-smtp` | Composing RFC 5322 messages and submitting them |
| `crates/core-rpc` | The typed surface both front ends talk to |
| `apps/cli` | Development driver — not the product |
| `apps/desktop` | The triage window |
| `apps/scannerd` | Pi capture daemon: page detect → capture → spool → upload |
| `apps/mcp` | The mailbox as an MCP server, for an assistant |
| `crates/core-paper` | Paperless-ngx read as a mailbox: post, in the shape mail has |
| `kuverta-bird/core` | The shared triage surface and the classifier, in JavaScript |
| `kuverta-bird/hosts` | One adapter per host: this client, and Thunderbird |
| `docker/` | Dev stack: Dovecot, an SMTP sink, a Gmail shape, Ollama, Paperless |
| `docs/` | Plan, evaluation, briefs, and the decisions log |

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

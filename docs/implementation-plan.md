# kuverta: Implementation Plan

Working title. Solo project, ~10h/week. Revised 2026-09-10 after decision review.
Original scope evaluation: [`readme-evaluation.md`](readme-evaluation.md).

---

## 0. Decisions (locked)

| # | Decision | Choice | Why |
|---|---|---|---|
| 1 | Capacity | Solo, ~10h/week (~43h/month) | Given. Drives everything below. |
| 2 | **What is v1** | **Full client, send first** | *Revised 2026-09-10.* Triage alone does not replace Apple Mail: every reply meant leaving the app. Write is in, safest first: **send** (SMTP submission + `APPEND` to Sent), then **mailbox mutation** (archive/delete/move/mark-read). See §1a. |
| 3 | UI stack | Tauri v2, dev mode only | Native shell, signing deferred until distribution. **Validated:** 59.8 fps scrolling 200k rows ([spike](spike-tauri-list.md)). |
| 4 | Auth | App passwords now, pluggable trait | Google's restricted-scope verification (CASA) only applies when *distributing* an OAuth client. Personal use avoids it. |
| 5 | Providers | Custom domain → Gmail → M365 | Easiest first. M365 needs an Azure app registration + device-code flow, but only for your own tenant: hours, not weeks. |
| 6 | Classification | Local Ollama model from the start | User's call, against the heuristics-first recommendation. Mitigated by logging a rules baseline (§4). |
| 7 | Paper half | `scannerd` + Paperless-ngx | Saves ~4 months. You write capture only; OCR, tagging, archive and search exist. Cost: a Docker container on the Pi/NAS. |
| 8 | Licence | Private now, AGPL-3.0 at publish | A sole copyright holder can always dual-license, so AGPL costs nothing and protects the SaaS option. |
| 9 | Name | `kuverta` as working title | Private repo. Keep it renameable: never in crate names, bundle ID, or schema. |

Deferred, not decided: SaaS/multi-device sync, OpenPGP, mobile app, distribution. Revisit at month 9.

---

## 1. What v1 actually is

kuverta syncs your mail over IMAP, classifies it, and gives you one triage surface to read, reply, send and file from. Scanned paper lands in the same surface.

Still non-goals: HTML mail *composition*, OpenPGP, multi-user, mobile, anything signed or distributable. Rendering received HTML, threading UI and attachments are in scope but late, because they are UI work.

The original 18-month estimate came from the items on that list. Keep them out.

### 1a. Write capability, in two stages

Decision 2 changed after month 4. Write is split in two so most of the original safety property holds while it is built:

| Stage | What it is | What it costs | Safety |
|---|---|---|---|
| **Send** | Compose, reply, forward, SMTP submission, `APPEND` to Sent | Contained: a new crate, a schema migration, no change to the sync path | IMAP stays `EXAMINE`-only. Additive: a bug can misdeliver a message it was asked to send, but cannot touch stored mail |
| **Mailbox mutation** | Archive, delete, move, mark-read from inside kuverta | The expensive half: `SELECT`+`STORE`+`MOVE`, a durable operation queue, conflict resolution against server-side changes, an undo window | Gives up the structural guarantee. Intent is recorded before the server is touched, no UID is acted on without proving it still holds the expected message, and **delete means move-to-Trash**. Nothing destroys mail permanently |

Send ships and is dogfooded before mutation starts, so if time runs out mid-write the safe half is done.

Both stages have landed. These rules replace the structural guarantee and are covered by tests:

- The queue is written before anything is acted on. A crash between the user acting and the round trip completing loses nothing and repeats nothing.
- No UID is trusted on its own. The folder's UIDVALIDITY, the UID's occupancy and the message's `Message-ID` are all checked before a write. A mismatch is refused with a reason.
- Nothing is destroyed. There is no expunge operation: delete moves to Trash. A bare `EXPUNGE` is never sent, even in a move fallback, because it would remove every `\Deleted` message in the folder, including ones another client marked.
- The executor changes nothing locally. The sync pass observes the server afterwards, with the same code that reconciles changes from any other client.

Remaining risk: a bug in the queue can move mail somewhere you did not ask for. It cannot delete it.

---

## 2. Architecture

```
   Tauri v2 shell (dev mode, desktop only)
                │  typed RPC  ← also the future MCP/agent surface
   ┌────────────▼──────────────────────────────┐
   │              inbox-core (Rust)            │
   │  accounts · sync · store · rules · ai      │
   └────┬─────────────┬───────────────┬────────┘
     IMAP read-only  SQLite+FTS5   Ollama (local)
     (rustls)        + blobs        + rules baseline
                          ▲
                   Paperless-ngx API
                          ▲
                   scannerd (Pi + camera)
```

### Workspace

| Crate | Month | Responsibility |
|---|---|---|
| `core-store` | 1 | SQLite + FTS5, blobs on disk, **Message-ID dedup** |
| `core-proto` | 1 to 2 | IMAP: LIST, FETCH, CONDSTORE, IDLE, plus the writes behind §1a |
| `core-accounts` | 2 to 3 | Auth trait: app password \| OAuth device flow; autoconfig |
| `core-rules` | 4 | Deterministic baseline classifier |
| `core-smtp` | 5 to 6 | Compose, MIME construction, SMTP submission (§1a stage 1) |
| `core-ai` | 5 | Provider trait → Ollama; embeddings or prompting |
| `core-rpc` | 4 to 6 | Stable surface for the shell (and later, agents) |
| `scannerd` | 6 to 7 | Pi capture daemon (separate binary, cross-compiled) |

### Lean on, don't rewrite

- **`mail-parser`** (Stalwart): MIME parsing. Best in the ecosystem. Do not write your own.
- **`async-imap`** or **`imap-next`**: IMAP client.
- **`email-lib`** (pimalaya/Himalaya): read its account/backend abstraction before writing `core-accounts`.
- **`keyring`**: OS keychain. Credentials never touch a config file.
- **Paperless-ngx**: the entire paper pipeline.

### Two constraints that must be right from commit one

1. Message-ID dedup in the store. Gmail exposes labels as folders, so one message arrives via several paths. Retrofitting this means rebuilding the store.
2. Auth behind a trait. You need app passwords, and M365 needs OAuth device flow. If auth is not pluggable by month 3 you will rewrite `core-accounts`.

---

## 3. Timeline (~43h/month)

| Month | Work | Gate |
|---|---|---|
| **1** | Repo, workspace skeleton, CI. `core-store` schema + dedup. Spike: Tauri window renders 20k rows smoothly. | Store holds real messages |
| **2** | `core-proto` read-only sync against **custom domain**. Incremental via CONDSTORE. | One account syncs offline |
| **3** | Auth trait. Gmail app password + label dedup verified. M365 Azure app + device flow. | All three accounts sync |
| **4** | `core-rpc`. Triage UI in Tauri: list, filter, categorise, keyboard-first. `core-rules` baseline. | You open it daily |
| **5** | `core-ai` → Ollama. Correction logging. Measure model vs. baseline. | **SHIPPABLE: daily driver** |
| **5 to 6** | `core-smtp`: compose, reply, SMTP submission, `APPEND` to Sent (§1a stage 1). | You reply from kuverta |
| **6** | Mailbox mutation: operation queue, `STORE`/`MOVE`, conflict resolution, undo (§1a stage 2). | Apple Mail stays closed |
| **6 to 7** | Paperless-ngx in Docker. `scannerd` on the Pi: page detect → capture → deskew → crop → upload with offline spooling. | Paper is searchable |
| **8** | Unified inbox: documents and mail as one item type in one triage surface. | **The actual product** |
| **9+** | Optional: mobile, distribution, SaaS. Decide with 4 months of real usage behind you. | none |

### Progress

Month 1 is done ahead of the gate, and part of month 2 with it:

- Cargo workspace, CI (fmt + clippy + tests against a real IMAP server), Makefile
- `core-store`: schema, migrations, **Message-ID dedup**, FTS5 search, blob store
- `core-accounts`: **auth behind a trait**, OS keychain, env provider for CI
- `core-proto`: read-only IMAP (`EXAMINE`/`BODY.PEEK[]`), CONDSTORE, MIME parsing, folder sync
- `docker/`: seeded Dovecot dev server, Ollama and Paperless-ngx behind profiles
- 31 tests green; sync verified end to end against the dev server

- `apps/desktop`: the Tauri list spike, **PASS**, see [spike-tauri-list.md](spike-tauri-list.md)

Both "must be right from commit one" constraints from section 2 are in and
tested. A Tauri v2 webview holds 59.8 fps scrolling 200k rows, so decision 3
stands. The Rust/JS bridge costs ~78 MiB/s of JSON, so the list requests
windowed slices, never the whole mailbox.

Incremental sync landed. A folder with an unchanged `HIGHESTMODSEQ` is skipped
without fetching, flag changes arrive via `CHANGEDSINCE`, and messages
expunged on the server are removed locally. On the seeded account a cold sync
touches 6 folders and the next one 0.

Still open for month 2: IDLE for push, and QRESYNC (`VANISHED`) to replace the
`UID SEARCH ALL` reconciliation, which is proportional to folder size.

Month 3's OAuth2 device flow (RFC 8628) is implemented and tested against a
scripted endpoint: polling, `slow_down`, expired codes, refresh-token rotation,
and a rejected refresh token that is discarded, not retried forever.
`AuthProvider` became async for it.

It needs an Azure AD app registration (a public client with the
`IMAP.AccessAsUser.All` and `offline_access` scopes), with its application id
passed as `--client-id`. Not yet verified against a real tenant, the one thing
outstanding for M365.

Month 4's rules baseline landed. A scoring classifier (six categories) runs on
every message during sync and records its verdict, so the model can later be
measured against it across the whole mailbox. It is 8/8 correct on the seeded
fixtures, and its confidence tracks how clear the evidence was. A sender or
list you have filed is filed that way next time, overriding the heuristics.
`kuverta triage` shows the result.

Stage 1 of the write capability (§1a) is in. `core-smtp` composes RFC 5322
messages and submits them over SMTP. `kuverta send` sends, replies and
forwards, and files a copy in Sent via `APPEND`. `make e2e` runs sync, send,
and the sent copy coming back on the next sync against the dev stack.

Composer rules: Bcc reaches the SMTP envelope but never the message, addresses
and subjects are checked for line breaks that would forge headers, `Re:`/`AW:`
prefixes collapse instead of stacking, References is capped, reply-all
excludes the sender, and `Reply-To` wins over `From`. A Mailpit sink in the
compose stack records the envelope separately from the headers, so the Bcc
rule is tested end to end.

IMAP is still `EXAMINE`-only. `APPEND` is the single write, and it can only
create a message (see the note in `core-proto::client`).

Adding `mail-send` exposed a latent crash. With both aws-lc-rs (via reqwest)
and ring (via mail-send) linked, rustls cannot pick a process-wide crypto
provider, and `core-proto` would have panicked on the first sync of a real TLS
account. No test reached that path because the dev server is plaintext. Every
TLS config now names its provider, each with a server-free regression test.

Stage 2 landed too. `kuverta archive`, `delete`, `move`, `read` and `unread`
queue a change. `kuverta sync` sends everything whose undo window has elapsed,
then reconciles. `kuverta undo` cancels the last change that has not left the
machine, and `kuverta queue` shows what is waiting.

§1a lists what the executor guarantees. Nine integration tests against Dovecot
cover the move, the flag change, both conflict paths, the obsolete path,
cancellation, ordering, and mail with no `Message-ID`. The conflict check was
verified by disabling it: the executor then moves the wrong message and the
test fails.

Dogfooding found one bug the tests missed. Mail with no `Message-ID` header
is legal and is in the seeded fixtures. Reading "no `Message-ID` came back" as
"no message at this UID" marked the operation obsolete, so such mail could not
be filed. Absence is now compared in both directions.

Then a pass for real providers, since everything above had only met Dovecot
and nine synthetic fixtures:

- Fetching is batched. Sync used to pull `1:*` in one command and hold every
  body in memory before writing a row: about 1.5 GB for a 30k-message mailbox,
  an OOM kill on this machine. It now asks for UIDs and sizes first, then
  fetches in batches capped at 200 messages and ~16 MB. The fixtures fit in
  one batch, so no test would have caught this.
- Gmail can be archived. Gmail has no Archive folder. Archiving removes the
  INBOX label, which over IMAP is a move into the folder marked `\All`.
  Special-folder resolution tries attributes in priority order, falls back to
  names matched on their last path segment (`[Gmail]/All Mail`,
  `INBOX.Archive`), and treats `\All` as the last-resort archive. Gmail, M365
  and a bare Dovecot are covered as table-driven tests.
- STARTTLS works, for servers on port 143. It is hand-rolled, because
  `async-imap` neither performs the upgrade nor hands back its stream. No
  silent downgrade to cleartext, pre-TLS capabilities are discarded, and
  certificate verification matches the implicit-TLS path. The dev server
  advertises STARTTLS with a self-signed certificate, so the test proves both
  that the upgrade happens and that the certificate is rejected.
- `kuverta check` reports what a server offers before anything depends on it:
  extensions, folder layout, where sent/archived/deleted mail will go, and
  with `--measure` how much a first sync will download.

A real account is connected and sync, send, archive and undo run against it.
That found one bug. Expunge reconciliation collected orphaned messages as it
went, so a message crossing folders was destroyed and rebuilt, losing its
classifier verdict, queued operations and the user's corrections. It depended
on the order the server listed the two folders in, so the dev server never
showed it. Orphans are now collected once, after every folder has been seen.

Gmail is covered without a Google account. A second Dovecot in `docker/` uses
Gmail's layout: `[Gmail]/` hierarchy, `\All` instead of `\Archive`, and the
same message under several folders, which is how labels look over IMAP and
what Message-ID dedup exists for.

Folders can be excluded from sync, by name or by special-use attribute. Gmail's
All Mail holds a copy of every message, so a mailbox averaging two labels per
message is downloaded twice. Dedup keeps one row and one body but the bytes are
still transferred. `kuverta check` measures the difference and names the
command. Excluding All Mail hides archived mail, which lives only there, and
the output says so.

Google does **not** issue `https://mail.google.com/` to the limited-input
device grant, so the RFC 8628 flow built for M365 cannot serve Gmail. Gmail
uses the RFC 8252 loopback redirect, now implemented: PKCE, a `state` check,
and a listener bound to `127.0.0.1` only. Token handling is shared with the
device flow, so refresh and rotation are identical and only the first
authorisation differs. Four tests drive it against a scripted authorization
server that records the challenge and refuses the exchange unless the verifier
hashes to it.

Decision 4 still holds. The restricted scope needs Google's CASA assessment
only for a *published* client. One left in testing, with your own address as a
test user, avoids it.

`core-rpc` exists now: the typed surface the shell talks to. It has no Tauri
dependency, because section 2 reuses it as the MCP/agent surface later.

The list is windowed because of the bridge cost: `messages` takes an offset
and a limit and returns a total. `unread` is derived, not stored: a message is
unread when no copy of it carries `\Seen`, which gives one answer for a
message in several folders, as most are on Gmail.

`apps/desktop` is now the triage window, not the spike. It opens the store,
lists messages, filters by rules category and by unread, reads a message, and
archives, trashes, marks read and undoes. Keys: j/k, e, #, u, z and `/`. Every
command is a thin wrapper over `core-rpc`, so decisions such as which copy of
a message to archive, or what to draw for a missing subject, are made and
tested in the core, not in JavaScript.

The list fetches windows of 200 and recycles a fixed pool of DOM nodes, so the
node count tracks the viewport. It is the spike's virtualizer with the data
source changed from "all of it in memory" to "the pages actually on screen".

The spike code is removed (it is in the history, numbers in
[spike-tauri-list.md](spike-tauri-list.md)). `make spike` is now `make app`.

Sync moved behind `core-rpc` too, as in the section 2 diagram. `Session`
assembles the account's auth provider, sends the queued changes, then runs the
sync pass. The order matters, because the pass makes the changes visible by
observing the server. The CLI and the window run the same code; `r` syncs from
the window.

A sync opens its own connection, because a lock held for as long as the
network takes would freeze the window. SQLite is in WAL mode, so one writer
and any number of readers coexist. A test reads fifty times while a sync runs,
and the window stays responsive while the CLI syncs the same store.

A sync also runs on its own thread. It holds its `Store` across every await,
and `rusqlite`'s connection is `Send` but not `Sync`, so the future is not
`Send`, which Tauri's async commands require.

Compose works the same way. `Session::send` builds the draft, submits it and
files the copy in Sent. `Session::preview` builds the same draft and stops; the
CLI's `--dry-run` prints it and the compose pane requests it before drawing.
Reply-all rules (drop yourself from the recipients, honour `Reply-To`) are
applied once in the core. `c`, `R`, `A` and `f` open the pane.

The pane shows the SMTP envelope as you type, the only place a blind recipient
appears. The CLI's `send` and `sync` are now thin wrappers over `core-rpc`
too.

The window got a sidebar and follows Apple Mail, the reference the user named:
three panes side by side, a recessed sidebar with rounded selection pills and
drawn icons, and three-line list rows (sender and date, subject, preview) in
place of a dense single-line table. Dates are relative: the time today, the
weekday this week, the date beyond.

All navigation is in the sidebar: accounts, their folders with unread counts,
then the classifier's categories. Folders are in the conventional order
(INBOX, Drafts, Sent, Archive, the user's own folders, then Junk and Trash),
because people find them by position. Counts are of messages, not locations: a
message with three Gmail labels counts once in each folder, and the sidebar
never adds up to more than the mailbox holds.

Running the window showed two problems. The list preview needed a `snippet`
the summary did not carry, though the parse layer had recorded it since the
first commit. And `#compose { display: flex }` is an id selector, so it beat
the user agent's `[hidden]` rule and the compose pane was always open.

A filtered list says what it is showing and offers a way out, because a
silently narrowed list looks like missing mail. Acting on a message keeps the
cursor in place and the list does not jump to the top.

Accounts can be configured from the window as well as the CLI. The settings
sheet has the account list beside the form: IMAP and SMTP endpoints, the auth
method, the excluded folders, and a **Verify** button. Verify connects without
changing anything and reports credentials, extensions, where mail will be
filed, and what a first sync would fetch. It is the `kuverta check` answer as
a structure.

Two rules are in the core, not the form. A password is written to the keychain
and never read back, so `AccountSettings` carries `has_password` and no secret,
and the form cannot show or leak one. Plaintext is refused for anything but
localhost.

Two bugs were fixed on the way. `Core::open` never created its data directory,
so the very first run died before the window appeared. The exclusions textarea
had to *replace* the set, not add to it, or an excluded folder could never be
restored.

Still open: the model layer (local Ollama, section 4), IDLE for push, QRESYNC,
and attachments.

Month 5 and month 8 are the real milestones. Everything before month 5 is scaffolding. Motivation is most likely to fail in months 2 and 3, so keep those short and concrete.

---

## 4. Classification: making the local-model choice work

The risk with a local model is never finding out whether it beat a `HashMap` of sender→folder. Two cheap defences:

1. Log both verdicts. Run the rules baseline (sender, `List-Id`, DKIM domain, subject patterns) alongside the model and record where they disagree and who was right. An afternoon of work, and the only way to justify the latency.
2. Spike embeddings against prompting before committing. For "which category is this," embedding the mail and taking nearest neighbours among *your own past filings* is typically faster and cheaper, needs no prompt tuning, and improves as you correct it. Generative prompting is the obvious choice and often the worse one.

Model size: 4 to 8B instruct on Apple Silicon is ample. Classification is easy; do not go higher.

Every correction is training data. Capture it from day one, even though nothing consumes it yet.

---

## 5. Risks

| Risk | Mitigation |
|---|---|
| **Motivation dies in months 2 and 3** (top risk for a solo side project) | Keep the pre-month-5 stretch short and concrete; ship read-only early; no UI polish before the sync works |
| Scope creep back toward a full client | The non-goals list in §1 is the contract |
| Local model underperforms plain rules | Baseline logging (§4) |
| Pi Zero too weak | It only captures and uploads; OCR runs in Paperless-ngx upstream |
| M365 auth friction | Hit it in month 3, not month 1, after two accounts already work |
| Paperless-ngx proves limiting | Only ~600 lines of `scannerd` are yours; swapping the backend is contained |

---

## 6. Week one

1. `git init`; Cargo workspace with the crates from §2, all empty; CI running `fmt`, `clippy`, `test`.
2. Spike: Tauri v2 window rendering a 20k-row virtualized list. If this is unpleasant, better to know now.
3. Spike: connect to the custom-domain account with `async-imap` + app password from the keychain; print the last 50 subjects.
4. Write `core-store`'s schema, with Message-ID dedup in it from the start.
5. Do not write any UI chrome.

# kuverta — Implementation Plan

Working title. Solo project, ~10h/week. Revised 2026-09-10 after decision review.
Original scope evaluation: [`readme-evaluation.md`](readme-evaluation.md).

---

## 0. Decisions (locked)

| # | Decision | Choice | Why |
|---|---|---|---|
| 1 | Capacity | Solo, ~10h/week (~43h/month) | Given. Drives everything below. |
| 2 | **What is v1** | **Full client, send first** | *Revised 2026-09-10.* Triage alone does not replace Apple Mail: every reply meant leaving the app, which is exactly the friction the project exists to remove. Write capability is in, sequenced cheapest-and-safest first — **send** (SMTP submission + `APPEND` to Sent), then **mailbox mutation** (archive/delete/move/mark-read). See §1a. |
| 3 | UI stack | Tauri v2, dev mode only | Native shell + on-ramp to a real client, signing deferred until distribution. **Validated:** 59.8 fps scrolling 200k rows ([spike](spike-tauri-list.md)). |
| 4 | Auth | App passwords now, pluggable trait | Google's restricted-scope verification (CASA) only applies when *distributing* an OAuth client. Personal use sidesteps it entirely. |
| 5 | Providers | Custom domain → Gmail → M365 | Easiest-first. M365 needs an Azure app registration + device-code flow, but only for your own tenant: hours, not weeks. |
| 6 | Classification | Local Ollama model from the start | User's call, against the heuristics-first recommendation. Mitigated by logging a rules baseline alongside (§4). |
| 7 | Paper half | `scannerd` + Paperless-ngx | Saves ~4 months. You write capture only; OCR, tagging, archive and search already exist. Cost: a Docker container on the Pi/NAS. |
| 8 | Licence | Private now, AGPL-3.0 at publish | Sole copyright holder can always dual-license, so AGPL costs nothing and protects the SaaS option. |
| 9 | Name | `kuverta` as working title | Private repo. Keep it renameable: never in crate names, bundle ID, or schema. |

**Deferred, not decided:** SaaS/multi-device sync, OpenPGP, mobile app, distribution. Revisit at month 9.

---

## 1. What v1 actually is

kuverta syncs your mail over IMAP, classifies everything, and gives you one triage surface you can act from: read, reply, send, and file. Scanned paper lands in the same surface.

**Still non-goals:** HTML mail *composition*, OpenPGP, multi-user, mobile, anything signed or distributable. Rendering received HTML, threading UI and attachments are now in scope, but late — they are UI work, not core work.

That non-goals list is where the 18 months went. It is shorter than it was; guard what is left of it.

### 1a. Write capability, in two stages

Decision 2 changed after month 4. Splitting write into two stages keeps most of the original safety property while it is being built:

| Stage | What it is | What it costs | Safety |
|---|---|---|---|
| **Send** | Compose, reply, forward, SMTP submission, `APPEND` to Sent | Contained: a new crate, a schema migration, no change to the sync path | IMAP stays `EXAMINE`-only. Purely additive — a bug can misdeliver a message it was asked to send, but cannot touch stored mail |
| **Mailbox mutation** | Archive, delete, move, mark-read from inside kuverta | The expensive half: `SELECT`+`STORE`+`MOVE`, a durable operation queue, conflict resolution against server-side changes, and an undo window | Gives up the structural guarantee, and earns back what it can: intent is recorded before the server is touched, no UID is acted on without proving it still holds the expected message, and **delete means move-to-Trash** — nothing here destroys mail permanently |

Send ships and gets dogfooded before mutation starts. The point of the ordering is that if motivation or time runs out mid-write, it runs out with the safe half done rather than the dangerous half half-done.

Both stages have landed. What replaced the structural guarantee, and what the tests hold in place:

- **Intent is durable before it is acted on.** The queue is written first, so a crash between the user acting and the round trip completing loses nothing and repeats nothing.
- **No UID is trusted on its own.** A UID means nothing without the UIDVALIDITY it was issued under, and neither proves it still refers to the same mail. The folder's UIDVALIDITY, the UID's occupancy, and the message's `Message-ID` are all checked before a write. Anything that does not line up is refused with a reason, not applied.
- **Nothing is destroyed.** There is no expunge operation: delete moves to Trash. A bare `EXPUNGE` is never sent even as part of a move fallback, because it would remove every `\Deleted` message in the folder including ones another client marked.
- **The local store is never guessed at.** The executor changes nothing locally; the sync pass observes the server afterwards, using the same code that reconciles changes made from any other client.

The remaining risk is honest and unshrinkable: kuverta can now move your mail, and a bug in the queue can move it somewhere you did not ask for. It cannot delete it.

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
| `core-proto` | 1–2 | IMAP: LIST, FETCH, CONDSTORE, IDLE, plus the writes behind §1a |
| `core-accounts` | 2–3 | Auth trait: app password \| OAuth device flow; autoconfig |
| `core-rules` | 4 | Deterministic baseline classifier |
| `core-smtp` | 5–6 | Compose, MIME construction, SMTP submission (§1a stage 1) |
| `core-ai` | 5 | Provider trait → Ollama; embeddings or prompting |
| `core-rpc` | 4–6 | Stable surface for the shell (and later, agents) |
| `scannerd` | 6–7 | Pi capture daemon (separate binary, cross-compiled) |

### Lean on, don't rewrite

- **`mail-parser`** (Stalwart) — MIME parsing. Best in the ecosystem. Do not write your own.
- **`async-imap`** or **`imap-next`** — IMAP client.
- **`email-lib`** (pimalaya/Himalaya) — read its account/backend abstraction before writing `core-accounts`.
- **`keyring`** — OS keychain. Credentials never touch a config file.
- **Paperless-ngx** — the entire paper pipeline.

### Two constraints that must be right from commit one

1. **Message-ID dedup in the store.** Gmail exposes labels as folders, so one message arrives via several paths. Retrofitting this means rebuilding the store.
2. **Auth behind a trait.** You need app passwords, and M365 needs OAuth device flow. If auth is not pluggable by month 3 you will rewrite `core-accounts`.

---

## 3. Timeline (~43h/month)

| Month | Work | Gate |
|---|---|---|
| **1** | Repo, workspace skeleton, CI. `core-store` schema + dedup. Spike: Tauri window renders 20k rows smoothly. | Store holds real messages |
| **2** | `core-proto` read-only sync against **custom domain**. Incremental via CONDSTORE. | One account syncs offline |
| **3** | Auth trait. Gmail app password + label dedup verified. M365 Azure app + device flow. | All three accounts sync |
| **4** | `core-rpc`. Triage UI in Tauri: list, filter, categorise, keyboard-first. `core-rules` baseline. | You open it daily |
| **5** | `core-ai` → Ollama. Correction logging. Measure model vs. baseline. | **SHIPPABLE — daily driver** |
| **5–6** | `core-smtp`: compose, reply, SMTP submission, `APPEND` to Sent (§1a stage 1). | You reply from kuverta |
| **6** | Mailbox mutation: operation queue, `STORE`/`MOVE`, conflict resolution, undo (§1a stage 2). | Apple Mail stays closed |
| **6–7** | Paperless-ngx in Docker. `scannerd` on the Pi: page detect → capture → deskew → crop → upload with offline spooling. | Paper is searchable |
| **8** | Unified inbox: documents and mail as one item type in one triage surface. | **The actual product** |
| **9+** | Optional: mobile, distribution, SaaS. Decide with 4 months of real usage behind you. | — |

### Progress

Month 1 is done ahead of the gate, and part of month 2 with it:

- Cargo workspace, CI (fmt + clippy + tests against a real IMAP server), Makefile
- `core-store`: schema, migrations, **Message-ID dedup**, FTS5 search, blob store
- `core-accounts`: **auth behind a trait**, OS keychain, env provider for CI
- `core-proto`: read-only IMAP (`EXAMINE`/`BODY.PEEK[]`), CONDSTORE, MIME parsing, folder sync
- `docker/`: seeded Dovecot dev server, Ollama and Paperless-ngx behind profiles
- 31 tests green; sync verified end to end against the dev server

- `apps/desktop`: the Tauri list spike — **PASS**, see [spike-tauri-list.md](spike-tauri-list.md)

Both "must be right from commit one" constraints from section 2 are in and
covered by tests, and the UI-stack risk is retired: a Tauri v2 webview holds
59.8 fps scrolling 200k rows. The spike did change one design decision — the
Rust/JS bridge costs ~78 MiB/s of JSON, so the list must request windowed slices
rather than the whole mailbox (decision 3 stands; the UI just pages).

Incremental sync landed too: a folder whose `HIGHESTMODSEQ` is unchanged is
skipped without fetching, flag changes arrive via `CHANGEDSINCE`, and messages
expunged on the server are removed locally. On the seeded account a cold sync
touches 6 folders; the next one touches 0.

Still open for month 2: IDLE for push, and QRESYNC (`VANISHED`) to replace the
`UID SEARCH ALL` reconciliation, which is currently proportional to folder size.
Month 3's OAuth2 device flow (RFC 8628) is implemented and tested against a
scripted endpoint: polling, `slow_down`, expired codes, refresh-token rotation,
and a rejected refresh token that is discarded rather than retried forever.
`AuthProvider` became async to accommodate it — the reason the trait existed.

To use it you need an Azure AD app registration (a public client with the
`IMAP.AccessAsUser.All` and `offline_access` scopes); pass its application id as
`--client-id`. Nothing about it is verified against a real tenant yet, which is
the one thing still outstanding for M365.

Month 4's rules baseline landed. A scoring classifier (six categories) runs on
every message during sync and records its verdict, so the model — when it
arrives — has a baseline to be measured against across the whole mailbox rather
than from the day it is switched on. On the seeded fixtures it is 8/8 correct,
with confidence that tracks how clear-cut the evidence was. Corrections feed
back in: a sender or list you have filed is filed that way next time,
overriding the heuristics outright. `kuverta triage` shows the result.

Stage 1 of the write capability (§1a) is in: `core-smtp` composes RFC 5322
messages and submits them over SMTP, and `kuverta send` sends, replies and
forwards from the command line, filing a copy in Sent via `APPEND`. `make e2e`
now runs that loop end to end against the dev stack — sync, send, and the sent
copy coming back on the next sync.

The composer is deliberately the thick half: Bcc reaches the SMTP envelope but
never the message, addresses and subjects are checked for the line breaks that
would forge headers, `Re:`/`AW:` prefixes collapse instead of stacking,
References is capped, reply-all excludes the sender, and `Reply-To` wins over
`From`. A Mailpit sink in the compose stack is what makes the Bcc property
assertable end to end, because it records the envelope separately from the
headers.

IMAP is still `EXAMINE`-only. `APPEND` is the single write, and it can only
create a message — see the note in `core-proto::client`.

Adding `mail-send` also surfaced a latent crash: with both aws-lc-rs (via
reqwest) and ring (via mail-send) linked, rustls cannot pick a process-wide
crypto provider, and `core-proto` would have panicked on the first sync of a
real TLS account — a path no test reaches, because the dev server is plaintext.
Every TLS config now names its provider, with a server-free regression test on
each.

Stage 2 landed too. `kuverta archive`, `delete`, `move`, `read` and `unread`
queue a change rather than performing one; `kuverta sync` sends everything
whose undo window has elapsed and then reconciles, so one command does both.
`kuverta undo` cancels the last change that has not left the machine, and
`kuverta queue` shows what is waiting.

The executor is where the care went, and §1a lists what it guarantees. Nine
integration tests against Dovecot cover the move, the flag change, both
conflict paths, the obsolete path, cancellation, ordering, and mail with no
`Message-ID`. The conflict check was verified by disabling it: without it the
executor moves the wrong message and the test fails.

That last case is worth recording, because the tests did not catch it and
dogfooding did. A message with no `Message-ID` header is legal and is in the
seeded fixtures. Reading "no `Message-ID` came back" as "no message at this
UID" made such mail silently impossible to file — the operation was marked
obsolete and nothing happened. Absence is now compared as carefully as
presence, in both directions.

Then a pass to make all of that survive contact with a real provider, since
everything above had only ever met Dovecot and nine synthetic fixtures:

- **Fetching is batched.** Sync used to pull `1:*` in one command and hold every
  body in memory before writing a row — about 1.5 GB for a 30k-message mailbox,
  which on this machine is an OOM kill rather than a slow sync. It now asks for
  UIDs and sizes first, then fetches in batches capped at 200 messages and
  ~16 MB. No test would have caught this: the fixtures fit in one batch.
- **Gmail can be archived.** Gmail has no Archive folder — archiving is removing
  the INBOX label, which over IMAP is a move into the folder it marks `\All`.
  Special-folder resolution now tries attributes in priority order, falls back
  to names matched on their last path segment (`[Gmail]/All Mail`,
  `INBOX.Archive`), and treats `\All` as the last-resort archive. Gmail, M365
  and a bare Dovecot are covered as table-driven tests.
- **STARTTLS works**, for servers on port 143. Hand-rolled, because `async-imap`
  neither performs the upgrade nor hands back its stream: no silent downgrade to
  cleartext, pre-TLS capabilities discarded rather than cached, and the same
  certificate verification as the implicit-TLS path. Tested against the dev
  server, which advertises STARTTLS with a self-signed certificate — so the test
  proves both that the upgrade happens and that the certificate is rejected.
- **`kuverta check`** reports what a server actually offers before anything
  depends on it: extensions, folder layout, where sent/archived/deleted mail
  will go, and with `--measure` how much a first sync will download.

A real account is connected and the whole loop runs against it — sync, send,
archive, undo — which turned up one bug worth recording: expunge reconciliation
collected orphaned messages as it went, so a message crossing folders was
destroyed and rebuilt in between, taking its classifier verdict, its queued
operations and the user's corrections with it. Whether that happened depended
on the order the server listed the two folders in, which is why the dev server
never showed it. Orphans are now collected once, after every folder has been
seen.

Gmail is covered without a Google account: a second Dovecot in `docker/` wears
Gmail's layout — `[Gmail]/` hierarchy, `\All` instead of `\Archive`, and the
same message under several folders, which is what labels look like over IMAP.
That last one is the store's load-bearing invariant in its native habitat.

Folders can be excluded from sync, by name or by special-use attribute. Gmail
is the reason: its All Mail holds a copy of every message, so a mailbox whose
mail averages two labels crosses the wire twice over. Deduplication keeps one
row and one body, but it cannot give the bytes back. `kuverta check` measures
the difference and names the command; excluding costs visibility of archived
mail, which lives only there, and says so.

That last doubt is now settled, and badly: Google does **not** issue
`https://mail.google.com/` to the limited-input device grant at all, so the
RFC 8628 flow built for M365 can never serve Gmail. Gmail takes the RFC 8252
loopback redirect instead, and that is now implemented — PKCE, a `state` check,
and a listener bound to `127.0.0.1` and nothing else. It shares its token
handling with the device flow, so refreshing and rotation behave identically
and only the first authorisation differs. Four tests drive it against a
scripted endpoint that plays a conforming authorization server: it records the
challenge and refuses the exchange unless the verifier hashes to it, which is
what makes the PKCE test worth having rather than self-confirming.

Decision 4 still holds, and the loopback flow is why: the restricted scope
needs Google's CASA assessment only for a *published* client. One left in
testing, with your own address as a test user, sidesteps it.

`core-rpc` exists now — the typed surface the shell will talk to, and
deliberately ignorant of Tauri, since section 2 has it doubling as the
MCP/agent surface later and a layer that has taken a dependency on one shell
is no longer a surface.

Its one non-obvious shape is that the list is windowed: `messages` takes an
offset and a limit and returns a total. That is not a preference. The spike
measured the Rust/JS bridge at about 78 MiB/s of JSON, so handing a whole
mailbox across it is the single thing that would undo the 59.8 fps it
otherwise reaches. `unread` is derived rather than stored — a message is
unread when no copy of it anywhere carries `\Seen` — which keeps one answer
for a message that sits in several folders, as most of them do on Gmail.

And the triage window exists. `apps/desktop` is no longer the spike: it opens
the store, lists messages, filters by rules category and by unread, reads a
message, and archives, trashes, marks read and undoes — keyboard-first, with
j/k, e, #, u, z and `/`. Every command in it is a thin wrapper over `core-rpc`,
which is what keeps the decisions testable: which copy of a message to archive
and what to draw for a missing subject are answered in the core, not in
JavaScript.

The list fetches windows of 200 and recycles a fixed pool of DOM nodes, so the
node count tracks the viewport rather than the mailbox — the same virtualizer
the spike validated, with the data source changed from "all of it in memory" to
"the pages actually on screen".

The spike itself is gone, and its result stands: the numbers are in
[spike-tauri-list.md](spike-tauri-list.md) and the code is in the history.
`make spike` is now `make app`.

Sync moved behind `core-rpc` too, which is the facade the section 2 diagram
draws. `Session` assembles the auth provider an account is configured for,
sends the queued changes, then runs the sync pass — in that order, because the
pass is what makes the changes visible by observing the server rather than by
guessing what it did. The CLI and the window now run that same code instead of
two copies that drift; `r` syncs from the window.

A sync opens a connection of its own rather than borrowing the one the list is
served from. It runs for as long as the network takes, and a lock held across
that is a frozen window. SQLite is in WAL mode, so one writer and any number of
readers coexist — there is a test that reads fifty times while a sync runs, and
the real window stays responsive while the CLI syncs the same store underneath
it.

It also runs on a thread of its own, which is not a performance choice: a sync
holds its `Store` across every await and `rusqlite`'s connection is `Send` but
not `Sync`, so the future is not `Send` either — which is what Tauri's async
commands require. Giving it one thread it never leaves satisfies that without
pretending the store is something it is not.

Compose followed the same route. `Session::send` builds the draft, submits it
and files the copy in Sent; `Session::preview` builds the same draft and stops,
which is what the CLI's `--dry-run` prints and what the compose pane asks for
before drawing anything. That matters for reply-all in particular: dropping
yourself from the recipients and honouring `Reply-To` are rules, and they are
applied in one place rather than re-implemented in JavaScript. `c`, `R`, `A`
and `f` open it.

The pane shows the SMTP envelope as you type, because that is the only place a
blind recipient appears and the one thing worth checking before committing.

With that the CLI's `send` and `sync` are both thin: every decision either
front end makes now lives in `core-rpc`, which is the point of the layer.

The window then grew a sidebar and took its visual cues from Apple Mail, which
is the reference the user named. Concretely that meant three panes side by side
rather than stacked, a recessed sidebar with rounded selection pills and drawn
icons, and list rows of three lines — sender and date, subject, preview —
instead of a dense single-line table. Dates are relative at the resolution you
want at each distance: the time today, the weekday this week, the date beyond.

The sidebar is the only place navigation happens: accounts, then their folders
with unread counts, then the classifier's categories. Folders come back in the
conventional order rather than alphabetically — INBOX, Drafts, Sent, Archive,
the user's own folders, then Junk and Trash — because people find these by
position, not by name. Counts are of messages rather than locations, so a
message carrying three Gmail labels counts once in each folder and the sidebar
never adds up to more than the mailbox holds.

Two things came out of actually looking at the window rather than the code. The
list preview needed a `snippet` the summary had never carried, which the parse
layer had been recording since the first commit and nothing had ever read. And
`#compose { display: flex }` is an id selector, so it beat the user agent's
`[hidden]` rule and the compose pane was simply always open — invisible in the
source, obvious on screen.

A filtered list now says what it is showing and offers a way out, because a
list that quietly narrows looks exactly like mail going missing. Acting on a
message keeps the cursor where it was, so the next one slides under it instead
of the list jumping back to the top.

Accounts can now be configured from the window rather than only from the CLI:
a settings sheet with the account list beside the form it edits, IMAP and SMTP
endpoints, the auth method, the excluded folders, and a **Verify** button that
connects without changing anything and reports what it found — credentials,
extensions, where mail will be filed, and what a first sync would fetch. That
is the same answer `kuverta check` gives, as a structure rather than as
printed lines, so neither has to be learned separately.

Two rules live in the core rather than the form. A password goes one way: it
is written to the keychain and never read back, so `AccountSettings` carries
`has_password` and no secret — a form that cannot display a password cannot
leak one into a screenshot. And plaintext is refused for anything but
localhost, because a password sent in the clear is a password disclosed
whichever front end collected it.

Two bugs turned up in the doing. `Core::open` never created its data
directory, so the very first run — the one the settings pane exists to serve —
died before the window appeared. And the exclusions textarea had to *replace*
the set rather than add to it, or a folder could be excluded and never
restored.

Still open: the model layer (local Ollama, section 4), IDLE for push, QRESYNC,
and attachments.

**Month 5 and month 8 are the real milestones.** Everything before month 5 is scaffolding; if motivation is going to fail, it fails in months 2–3, so keep those two months as short and concrete as possible.

---

## 4. Classification: making the local-model choice work

The risk in going straight to a local model is not that it fails — it is that you never find out whether it beat a `HashMap` of sender→folder. Two cheap defences:

1. **Log both verdicts.** Run the rules baseline (sender, `List-Id`, DKIM domain, subject patterns) alongside the model and record where they disagree and who was right. An afternoon of work; the only way to justify the latency.
2. **Spike embeddings against prompting before committing.** For "which category is this," embedding the mail and taking nearest neighbours among *your own past filings* is typically faster, cheaper, needs no prompt tuning, and improves automatically as you correct it. Generative prompting is the more obvious choice and often the worse one.

Model size: 4–8B instruct on Apple Silicon is ample. Classification is easy; do not reach higher.

Every correction you make is training data — capture it from day one even though nothing consumes it until later.

---

## 5. Risks

| Risk | Mitigation |
|---|---|
| **Motivation dies in months 2–3** (top risk for a solo side project) | Keep the pre-month-5 stretch short and concrete; ship read-only early; no UI polish before the sync works |
| Scope creep back toward a full client | The non-goals list in §1 is the contract |
| Local model underperforms plain rules | Baseline logging (§4) |
| Pi Zero too weak | It only captures and uploads; OCR runs in Paperless-ngx upstream |
| M365 auth friction | Hit it in month 3, not month 1 — after two accounts already work |
| Paperless-ngx proves limiting | Only ~600 lines of `scannerd` are yours; swapping the backend is contained |

---

## 6. Week one

1. `git init`; Cargo workspace with the crates from §2, all empty; CI running `fmt`, `clippy`, `test`.
2. **Spike:** Tauri v2 window rendering a 20k-row virtualized list. If this is unpleasant, better to know now.
3. **Spike:** connect to the custom-domain account with `async-imap` + app password from the keychain; print the last 50 subjects.
4. Write `core-store`'s schema, with Message-ID dedup in it from the start.
5. Do not write any UI chrome.

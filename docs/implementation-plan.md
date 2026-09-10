# fuckmail — Implementation Plan

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
| 9 | Name | `fuckmail` as working title | Private repo. Keep it renameable: never in crate names, bundle ID, or schema. |

**Deferred, not decided:** SaaS/multi-device sync, OpenPGP, mobile app, distribution. Revisit at month 9.

---

## 1. What v1 actually is

fuckmail syncs your mail over IMAP, classifies everything, and gives you one triage surface you can act from: read, reply, send, and file. Scanned paper lands in the same surface.

**Still non-goals:** HTML mail *composition*, OpenPGP, multi-user, mobile, anything signed or distributable. Rendering received HTML, threading UI and attachments are now in scope, but late — they are UI work, not core work.

That non-goals list is where the 18 months went. It is shorter than it was; guard what is left of it.

### 1a. Write capability, in two stages

Decision 2 changed after month 4. Splitting write into two stages keeps most of the original safety property while it is being built:

| Stage | What it is | What it costs | Safety |
|---|---|---|---|
| **Send** | Compose, reply, forward, SMTP submission, `APPEND` to Sent | Contained: a new crate, a schema migration, no change to the sync path | IMAP stays `EXAMINE`-only. Purely additive — a bug can misdeliver a message it was asked to send, but cannot touch stored mail |
| **Mailbox mutation** | Archive, delete, move, mark-read from inside fuckmail | The expensive half: `SELECT`+`STORE`+`MOVE`+`EXPUNGE`, an offline operation queue, conflict resolution against server-side changes, and an undo window | Gives up the structural guarantee. This is the stage that needs the queue, the conflict rules, and the tests to earn it back |

Send ships and gets dogfooded before mutation starts. The point of the ordering is that if motivation or time runs out mid-write, it runs out with the safe half done rather than the dangerous half half-done.

Until stage 2 lands, read-only remains a real security posture: folders are opened with `EXAMINE`, so the server itself rejects any mutation, and a sync bug can lose cached data but never mail.

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
| `core-proto` | 1–2 | IMAP: LIST, FETCH, CONDSTORE, IDLE — read-only plus `APPEND` |
| `core-accounts` | 2–3 | Auth trait: app password \| OAuth device flow; autoconfig |
| `core-rules` | 4 | Deterministic baseline classifier |
| `core-smtp` | 5–6 | Compose, MIME construction, SMTP submission (§1a stage 1) |
| `core-ai` | 5 | Provider trait → Ollama; embeddings or prompting |
| `core-rpc` | 4 | Stable surface for the shell (and later, agents) |
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
| **5–6** | `core-smtp`: compose, reply, SMTP submission, `APPEND` to Sent (§1a stage 1). | You reply from fuckmail |
| **6** | Mailbox mutation: operation queue, `STORE`/`MOVE`/`EXPUNGE`, conflict resolution, undo (§1a stage 2). | Apple Mail stays closed |
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
overriding the heuristics outright. `fuckmail triage` shows the result.

Stage 1 of the write capability (§1a) is in: `core-smtp` composes RFC 5322
messages and submits them over SMTP, and `fuckmail send` sends, replies and
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

Still open: the model layer itself (local Ollama, plan section 4), the triage
UI, IDLE for push, QRESYNC, connecting the three real accounts, and stage 2 of
the write capability (mailbox mutation: operation queue, conflict resolution,
undo).

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

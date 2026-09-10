# fuckmail — Implementation Plan

Working title. Solo project, ~10h/week. Revised 2026-09-10 after decision review.
Original scope evaluation: [`readme-evaluation.md`](readme-evaluation.md).

---

## 0. Decisions (locked)

| # | Decision | Choice | Why |
|---|---|---|---|
| 1 | Capacity | Solo, ~10h/week (~43h/month) | Given. Drives everything below. |
| 2 | **What is v1** | **Triage layer, not a mail client** | A full client is ~18 months to beta at this pace. A read-only triage layer is usable in ~5 months and can be abandoned at any month with something left over. |
| 3 | UI stack | Tauri v2, dev mode only | Native shell + on-ramp to a real client, signing deferred until distribution. **Validated:** 59.8 fps scrolling 200k rows ([spike](spike-tauri-list.md)). |
| 4 | Auth | App passwords now, pluggable trait | Google's restricted-scope verification (CASA) only applies when *distributing* an OAuth client. Personal use sidesteps it entirely. |
| 5 | Providers | Custom domain → Gmail → M365 | Easiest-first. M365 needs an Azure app registration + device-code flow, but only for your own tenant: hours, not weeks. |
| 6 | Classification | Local Ollama model from the start | User's call, against the heuristics-first recommendation. Mitigated by logging a rules baseline alongside (§4). |
| 7 | Paper half | `scannerd` + Paperless-ngx | Saves ~4 months. You write capture only; OCR, tagging, archive and search already exist. Cost: a Docker container on the Pi/NAS. |
| 8 | Licence | Private now, AGPL-3.0 at publish | Sole copyright holder can always dual-license, so AGPL costs nothing and protects the SaaS option. |
| 9 | Name | `fuckmail` as working title | Private repo. Keep it renameable: never in crate names, bundle ID, or schema. |

**Deferred, not decided:** SaaS/multi-device sync, compose & send, OpenPGP, mobile app, distribution. Revisit at month 9.

---

## 1. What v1 actually is

You keep reading and sending mail in Apple Mail. fuckmail runs alongside, connects **read-only** over IMAP, classifies everything, and gives you one triage surface. Scanned paper lands in the same surface.

**Explicit non-goals for v1:** composing, sending, HTML mail rendering, threading UI, attachment handling, search UX, multi-user, anything signed or distributable.

That non-goals list is where the 18 months went. Guard it.

Read-only is also a security posture: the app cannot destroy mail, so a sync bug is an annoyance rather than a catastrophe. It removes the offline-op-queue, conflict-resolution and draft-loss problems that eat the most time in real clients.

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
| `core-proto` | 1–2 | IMAP read-only: LIST, FETCH, CONDSTORE, IDLE |
| `core-accounts` | 2–3 | Auth trait: app password \| OAuth device flow; autoconfig |
| `core-rules` | 4 | Deterministic baseline classifier |
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
| **6–7** | Paperless-ngx in Docker. `scannerd` on the Pi: page detect → capture → deskew → crop → upload with offline spooling. | Paper is searchable |
| **8** | Unified inbox: documents and mail as one item type in one triage surface. | **The actual product** |
| **9+** | Optional: compose/send, mobile, distribution, SaaS. Decide with 4 months of real usage behind you. | — |

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

Still open for month 2: incremental sync via `HIGHESTMODSEQ` (captured but not
yet used to skip unchanged folders), and IDLE.

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

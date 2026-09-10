# fuckmail

Working title. A read-only **mail triage layer**: it connects to your existing
IMAP accounts, classifies everything, and gives you one surface to triage from —
while you keep reading and sending mail in your normal client. Scanned paper mail
lands in the same place.

It is **not** a mail client and does not try to be one. See
[docs/implementation-plan.md](docs/implementation-plan.md) for why that scope was
chosen, and [docs/readme-evaluation.md](docs/readme-evaluation.md) for the
analysis behind it.

Status: **early.** Sync works end to end against a local dev server. There is no
UI yet.

## What works today

- Read-only IMAP sync (`EXAMINE`, `BODY.PEEK[]`) into SQLite + FTS5
- Message identity and deduplication — one message in several folders is stored
  once with several locations, which is what Gmail's labels-as-folders needs
- MIME parsing: RFC 2047 encoded headers, quoted-printable, charsets, attachments
- `List-Id` extraction, ready for the rules baseline
- Full-text search
- Credentials in the OS keychain, behind a pluggable auth trait
- A Docker dev stack with a seeded Dovecot, so nothing needs a real mailbox

## Quick start

```sh
make dev-up     # Dovecot with nine fixture messages
make e2e        # register the dev account, sync it, show the result
```

Expected: **9 fixtures, 8 messages, 9 locations.** The newsletter appears in both
INBOX and Archive under one Message-ID and must collapse to a single message.

Against a real account:

```sh
cargo run -p fuckmail-cli -- add-account \
    --email you@example.de --host imap.example.de --port 993
cargo run -p fuckmail-cli -- set-password --email you@example.de   # reads stdin
cargo run -p fuckmail-cli -- sync
cargo run -p fuckmail-cli -- list
cargo run -p fuckmail-cli -- search Rechnung
```

Gmail and Microsoft 365 need an app-specific password for now; OAuth2 arrives in
month 3 (`core-accounts::OAuth2Device`).

## Layout

| Path | What |
|---|---|
| `crates/core-store` | SQLite store, dedup, FTS5 search, blobs |
| `crates/core-proto` | IMAP client, MIME parsing, folder sync |
| `crates/core-accounts` | Credentials behind an auth trait |
| `apps/cli` | Development driver — not the product |
| `docker/` | Dev stack: Dovecot, Ollama, Paperless-ngx |
| `docs/` | Plan, evaluation, original brief |

`make help` lists the useful targets.

## Design decisions worth knowing before you change anything

**Read-only is structural.** Folders open with `EXAMINE`, not `SELECT`, so the
server rejects mutations. A sync bug can lose cached data but never mail. This is
the main reason v1 is a triage layer.

**Dedup is per account, never global.** The same message on two of your accounts
really is two copies. See `core-store/src/dedup.rs`.

**Both classifiers always record a verdict.** Rules and model verdicts coexist in
the `classification` table so `store.disagreements()` can answer "is the model
beating plain heuristics" — the model layer is not in yet, but the schema is.

**Corrections are captured from day one.** Nothing consumes the `correction`
table until the model lands; it is being filled now so there is training data
when that happens.

## Licence

Not yet licensed. Intended to be AGPL-3.0 when first published — see decision 8
in the plan.

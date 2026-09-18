---
description: How kuverta is put together — the crates and apps, the two halves, and the design rules worth knowing before changing anything.
---

# Architecture

## Two halves

- **The Rust half** — `crates/` and `apps/` — is a mail client: IMAP, SMTP, the
  store, accounts, the deterministic classifier, OpenPGP, Paperless.
- **The JavaScript half** — `kuverta-bird/` — is the triage surface and the
  classifier ported to run wherever the mail is. It mounts over this client
  *and* over Thunderbird as an [extension](thunderbird.md), from the same code,
  behind a contract each host implements.

The desktop window is Tauri: a Rust process and a web view talking over typed
commands. The window and the CLI are both thin over `core-rpc`, so a decision —
which copy of a message to archive, what to draw for a missing subject, whether
a change can still be undone — is tested once rather than implemented twice.

## Layout

| Path | What |
| --- | --- |
| `crates/core-store` | SQLite store, dedup, FTS5 search, blobs, the change queue, smart mailboxes, the outbox |
| `crates/core-proto` | IMAP client, MIME parsing, folder sync, the mutation executor |
| `crates/core-accounts` | credentials behind an auth trait; both OAuth2 flows; data directory and keychain naming |
| `crates/core-rules` | the deterministic classifier |
| `crates/core-ai` | models: prompting and embeddings, over Ollama or an OpenAI-compatible API |
| `crates/core-smtp` | composing RFC 5322 messages and submitting them |
| `crates/core-pgp` | OpenPGP on rPGP: the keyring, PGP/MIME out, reading what arrives |
| `crates/core-paper` | Paperless-ngx read as a mailbox: post, in the shape mail has |
| `crates/core-rpc` | the typed surface both front ends talk to |
| `apps/desktop` | the window (Tauri) |
| `apps/cli` | the [development driver](cli.md) — not the product |
| `apps/mcp` | the mailbox as an [MCP server](mcp.md), for an assistant |
| `apps/scannerd` | the Pi [capture daemon](scanner.md): page detect → capture → spool → upload |
| `kuverta-bird/core` | the shared triage surface and the classifier, in JavaScript |
| `kuverta-bird/hosts` | one adapter per host: this client, and Thunderbird |
| `docker/` | the [dev stack](dev-stack.md): Dovecot, an SMTP sink, a Gmail shape, Ollama, Paperless |
| `website/` | this site |
| `docs/` | plan, evaluation, briefs, and the decisions log |

## Rules worth knowing before you change anything

These are from the readme, where each is explained at more length.

**Nothing destroys mail.** There is no expunge operation — delete means move to
Trash — and a bare `EXPUNGE` is never sent, even in a move fallback, because it
would remove every `\Deleted` message in the folder, including ones another
client marked. Folders are deleted only when the server says they are empty.

**No UID is trusted on its own.** Before any write, the executor checks that the
folder still has the UIDVALIDITY the change was queued under, that there is
still a message at the UID, and that its `Message-ID` is the expected one.
Anything that does not line up is refused with a reason
(`core-proto/src/mutate.rs`).

**Bcc reaches the envelope and never the message.** The message and the SMTP
envelope are computed together, by hand. It is the one mistake that cannot be
walked back — which is also why encrypted mail cannot have Bcc: an OpenPGP
message names every key it is encrypted to.

**Dedup is per account, never global.** One message in several folders is one
message with several locations, which is what Gmail's labels need; the same
message on two of your accounts is two copies.

**A sync must be cheap when nothing changed.** Folders whose `HIGHESTMODSEQ`
matches the stored one are skipped without a fetch.

**Orphans are collected once, at the end of a sync**, because a message being
moved is out of its old folder before it is in the new one.

**The list pages over the IPC bridge.** Tauri serialises commands as JSON, so
the window asks for the visible slice, never the whole mailbox — 200,000 rows
scroll at 60 fps (`docs/spike-tauri-list.md`).

**Getting a credential never waits for a human.** Only `login` is interactive;
an expired login during sync fails with an error naming the command to fix it.

**Secrets go one way.** Passwords, tokens, API keys and passphrases are written
to the keychain and never read back into a settings view.

**Corrections are captured from day one**, because a labelled corpus of your own
mail is not something you can go back and collect later.

## The decisions log

`docs/decisions.md` records the decisions that shaped the project, with what
each cost — among them:

- why the classifier was ported to JavaScript instead of kept in Rust (§1);
- one core, two hosts, and a contract between them (§4);
- keyboard-first is not the same thing as undiscoverable (§8);
- a physical address is an account (§11), and filing post by hand (§17);
- why `scannerd` refuses to do most of the job (§13);
- what "more careful than for a person" means for an assistant (§14, §18);
- measuring the local model before trusting it (§15, §16, §19);
- choosing models and hosted providers (§25), and the setup assistant (§26).

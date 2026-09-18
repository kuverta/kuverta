---
description: kuverta's Docker development stack — Dovecot with fixtures, a Gmail-shaped Dovecot, Mailpit, Paperless-ngx and Ollama.
---

# The dev stack

Everything in `docker/` is for local development, so none of the work needs a
real mailbox. Nothing there is hardened; nothing should be exposed beyond
localhost.

```sh
make dev-up      # Dovecot + fixtures, a Gmail-shaped Dovecot, Mailpit, Paperless-ngx
make ai-up       # + Ollama (compose profile "ai")
make dev-reset   # destroy the mailbox and reseed it
make dev-down    # stop, keeping the data
```

| Service | Address | Sign in |
| --- | --- | --- |
| Dovecot (IMAP, no TLS) | `127.0.0.1:10143` | `dev@kuverta.test` / `devpass` |
| Dovecot with Gmail's folder layout | `127.0.0.1:10144` | |
| Mailpit (SMTP sink) | SMTP `127.0.0.1:1025`, web UI and API `http://localhost:8025` | accepts any login |
| Paperless-ngx 3.1.3 | `http://localhost:8000` | `admin` / `admin` |
| Ollama (profile `ai`) | `http://localhost:11434` | |

## The fixtures

Nine `.eml` files are loaded into Dovecot on first start, over IMAP `APPEND`
(which doubles as the readiness check). Each exercises something:

| Fixture | Why it exists |
| --- | --- |
| `01-plain` | baseline plain text |
| `02-newsletter` | `List-Id` and `List-Unsubscribe`, for the rules |
| `03-html-tracking` | HTML with a tracking pixel and a `<script>` |
| `04-attachment` | `multipart/mixed` with a real PDF |
| `05-utf8-subject` | RFC 2047 German subject, ISO-8859-1 quoted-printable body |
| `06-no-messageid` | **no `Message-ID`** — forces the synthetic dedup key |
| `07-invoice-de` | a realistic German invoice, a classification target |
| `archive/02-newsletter-relabelled` | **the same `Message-ID` as 02, in another folder** — Gmail's labels |
| `archive/08-old-thread` | `In-Reply-To` / `References` |

The two in bold catch the expensive regressions. Seeding is idempotent; a
mailbox that already holds mail is left alone.

## Paperless-ngx

Paperless is in the default stack rather than behind a profile: post is half of
what kuverta is for, and a dev environment with only mail in it makes the paper
half something you remember to test rather than something you use. The first
start is slow while it builds its index. OCR is set up for German and English,
with deskew and page rotation on for the scanner's photographs.

An API token:

```sh
curl -X POST http://localhost:8000/api/token/ -d username=admin -d password=admin
```

Then `PAPERLESS_TOKEN=… kuverta paper --check`. Drop PDFs or images into
`docker/paperless/consume/` and Paperless files them, or send them the way the
Pi does, without a camera:

```sh
# JPEGs named <unix-seconds>-<n>.jpg in a directory
scannerd --drain-only --spool <dir> --tag "<address>"
```

A postal address is usually a tag: create it in Paperless first, then add the
address in the app's settings with that tag and paste the token there.

## Ollama

On Apple Silicon the container has no GPU access, so it is much slower than a
native Ollama; use it for CI and parity, and run Ollama natively for real work.
Both listen on 11434, so run one or the other. `make ai-model` pulls the chat
model and `nomic-embed-text`, the embedding model `kuverta eval` compares
prompting against.

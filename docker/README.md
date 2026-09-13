# Dev stack

Everything here is for local development. Nothing is hardened, nothing should be
exposed beyond localhost.

```sh
make dev-up          # Dovecot + fixtures + Paperless-ngx    (default)
make ai-up           # + Ollama                              (profile: ai)
make dev-reset       # destroy the mailbox and reseed
```

Paperless-ngx is in the default stack rather than behind a profile. Post is not
a separate feature to switch on — it is half of what this client is for, and a
dev environment with only mail in it quietly makes the paper half something you
remember to test rather than something you use. It costs a Redis and a Django,
and the first start is slow while it builds its index.

## Paperless-ngx — test document archive

`http://localhost:8000`, user `admin`, password `admin`.

For an API token: **Settings → Users & Groups → admin → Create token**, or

```sh
curl -X POST http://localhost:8000/api/token/ \
  -d username=admin -d password=admin
```

Then `PAPERLESS_TOKEN=... fuckmail paper --check`. Drop PDFs or images into
`docker/paperless/consume/` and Paperless will OCR and file them; that is the
directory `scannerd` will write to from the Pi.

## Dovecot — test IMAP server

`127.0.0.1:10143`, user `dev@fuckmail.test`, password `devpass`, no TLS.

Nine `.eml` fixtures load on first start. They are not filler; each one exercises
something specific:

| Fixture | Why it exists |
|---|---|
| `01-plain` | Baseline plain-text mail |
| `02-newsletter` | `List-Id` + `List-Unsubscribe` for the rules baseline |
| `03-html-tracking` | HTML with a tracking pixel and a `<script>` — the sanitiser's target |
| `04-attachment` | `multipart/mixed` with a real PDF |
| `05-utf8-subject` | RFC 2047 encoded German subject, iso-8859-1 quoted-printable body |
| `06-no-messageid` | **No `Message-ID`** — forces the synthetic dedup key |
| `07-invoice-de` | Realistic German invoice, a classification target |
| `archive/02-newsletter-relabelled` | **Same `Message-ID` as `02`, in another folder** — the Gmail labels case |
| `archive/08-old-thread` | `In-Reply-To`/`References`, for threading later |

The two in bold are the ones that catch the expensive regressions.

### Why these versions

**Dovecot 2.4, not 2.3.** The 2.3 images are amd64-only and crash under Rosetta
on Apple Silicon (`unable to mmap ExecutableHeap`). 2.4 ships arm64 builds. It
runs rootless, so IMAP is on 31143 inside the container, mapped to 10143.

**Seeded over IMAP `APPEND`, not by writing Maildir files.** Dovecot 2.4 defaults
to an indexed mailbox layout rather than Maildir++, so the on-disk folder names
are not predictable. Seeding through the protocol is layout-agnostic and doubles
as a readiness check. The image is distroless with no shell tools, which is also
why readiness is handled by the seeder retrying instead of a compose healthcheck.

Seeding is idempotent — a mailbox that already holds messages is left alone. Use
`make dev-reset` to start over.

## Ollama

`http://localhost:11434`, profile `ai`.

**On Apple Silicon the container has no GPU access** — no Metal passthrough — so
it is much slower than a native install. Use the container for CI and parity, and
run Ollama natively for real development. `make ai-model` pulls a chat model and `nomic-embed-text`, the embedding model `fuckmail eval` compares prompting against.

## Paperless-ngx

`http://localhost:8000`, admin/admin, profile `paperless`. OCR is configured for
German and English.

This is the paper half of the product (month 6+): `scannerd` on the Pi will drop
captured pages into `docker/paperless/consume/`, Paperless does OCR and archival,
and fuckmail pulls documents into the same triage inbox as email over its API.
Not wired up yet.

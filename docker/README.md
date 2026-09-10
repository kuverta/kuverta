# Dev stack

Everything here is for local development. Nothing is hardened, nothing should be
exposed beyond localhost.

```sh
make dev-up          # Dovecot + fixtures        (default)
make ai-up           # + Ollama                  (profile: ai)
make paperless-up    # + Paperless-ngx           (profile: paperless)
make dev-reset       # destroy the mailbox and reseed
```

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
run Ollama natively for real development. `make ai-model` pulls a model.

## Paperless-ngx

`http://localhost:8000`, admin/admin, profile `paperless`. OCR is configured for
German and English.

This is the paper half of the product (month 6+): `scannerd` on the Pi will drop
captured pages into `docker/paperless/consume/`, Paperless does OCR and archival,
and fuckmail pulls documents into the same triage inbox as email over its API.
Not wired up yet.

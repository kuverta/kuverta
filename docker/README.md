# Dev stack

Everything here is for local development. Nothing is hardened. Do not expose
any of it beyond localhost.

```sh
make dev-up          # Dovecot + fixtures + Paperless-ngx    (default)
make ai-up           # + Ollama                              (profile: ai)
make dev-reset       # destroy the mailbox and reseed
```

Paperless-ngx is in the default stack, with no profile. Post is half of what
this client is for, and a mail-only dev environment means the paper half goes
untested. It costs a Redis and a Django, and the first start is slow while it
builds its index.

## Paperless-ngx: test document archive

`http://localhost:8000`, user `admin`, password `admin`.

For an API token: **Settings → Users & Groups → admin → Create token**, or

```sh
curl -X POST http://localhost:8000/api/token/ \
  -d username=admin -d password=admin
```

Then `PAPERLESS_TOKEN=... kuverta paper --check`. Drop PDFs or images into
`docker/paperless/consume/` and Paperless will OCR and file them. To send them
the way the Pi does, without a camera, put JPEGs named `<unix-seconds>-<n>.jpg`
in a directory and run `scannerd --drain-only --spool <dir> --tag "<address>"`.

A postal address is a tag. Create it in Paperless first (`scannerd` refuses a
tag Paperless does not have, and keeps the capture). Then add the address in the
app's settings sheet with that tag and paste the token there. The sheet files
the token in the keychain as the app. A token added with `security` is not
filed that way.

## Dovecot: test IMAP server

`127.0.0.1:10143`, user `dev@kuverta.test`, password `devpass`, no TLS.

Nine `.eml` fixtures load on first start. Each one exercises something
specific:

| Fixture | Why it exists |
|---|---|
| `01-plain` | Baseline plain-text mail |
| `02-newsletter` | `List-Id` + `List-Unsubscribe` for the rules baseline |
| `03-html-tracking` | HTML with a tracking pixel and a `<script>`, the sanitiser's target |
| `04-attachment` | `multipart/mixed` with a real PDF |
| `05-utf8-subject` | RFC 2047 encoded German subject, iso-8859-1 quoted-printable body |
| `06-no-messageid` | **No `Message-ID`**, forces the synthetic dedup key |
| `07-invoice-de` | Realistic German invoice, a classification target |
| `archive/02-newsletter-relabelled` | **Same `Message-ID` as `02`, in another folder**, the Gmail labels case |
| `archive/08-old-thread` | `In-Reply-To`/`References`, for threading later |

The two in bold catch the expensive regressions.

### Why these versions

Dovecot is 2.4 because the 2.3 images are amd64-only and crash under Rosetta
on Apple Silicon (`unable to mmap ExecutableHeap`). 2.4 ships arm64 builds. It
runs rootless, so IMAP is on 31143 inside the container, mapped to 10143.

Fixtures are seeded over IMAP `APPEND`. Dovecot 2.4 defaults to an indexed
mailbox layout, not Maildir++, so the on-disk folder names are not
predictable. Seeding through the protocol works with any layout and doubles
as a readiness check. The image is distroless with no shell tools, so
readiness is handled by the seeder retrying, with no compose healthcheck.

Seeding is idempotent: a mailbox that already holds messages is left alone. Use
`make dev-reset` to start over.

## Ollama

`http://localhost:11434`, profile `ai`.

On Apple Silicon the container has no GPU access (no Metal passthrough), so
it is much slower than a native install. Use the container for CI and parity, and
run Ollama natively for real development. `make ai-model` pulls a chat model and `nomic-embed-text`, the embedding model `kuverta eval` compares prompting against.

## Paperless-ngx

`http://localhost:8000`, admin/admin, in the default stack. OCR is configured for
German and English, with deskew and page rotation on for `scannerd`'s photographs.

The image is pinned (3.1.3) and the compose file sets a dev-only
`PAPERLESS_SECRET_KEY`, because `latest` once moved to a release that will not
start without one. Only bump the pin intentionally.

`scannerd` on the Pi uploads captured pages over the API, Paperless does OCR
and archival, and kuverta reads each address's documents into the same triage
surface as email (`docs/decisions.md` §17, §22).

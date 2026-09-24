# Reporting a vulnerability

Use GitHub's private vulnerability reporting: **Security → Report a
vulnerability** on <https://github.com/kuverta/kuverta>. It opens a thread
only you and the maintainers can see. Please do not open a public issue for
something exploitable.

There is no bounty and no service-level agreement. kuverta is one person's
project; you will get a real answer, not necessarily a fast one.

## What kuverta reads that somebody else wrote

Worth knowing before you go looking, because it is most of the attack
surface:

- **Mail, whole.** Raw MIME off an IMAP server, PGP packets inside it, and
  the HTML a letter arrives in. `crates/core-proto`, `crates/core-pgp`,
  `crates/core-rpc/src/html.rs`.
- **A model's replies.** Whatever the local Ollama sends back, treated as
  untrusted text. `crates/core-ai`.
- **Paperless-ngx.** Documents, their OCR text and their tags, over HTTP on
  the local network. `crates/core-paper`.
- **The scanner's own HTTP port.** `apps/scannerd` serves a setup page on the
  local network, behind a password. Anything on that network can reach it.

All of it is fuzzed — `make fuzz-list` — and the rule throughout is that a
parser refuses rather than guesses. A half-read message reads exactly like a
well-read one, which is the outcome worth avoiding.

## What is out of scope

- **The scanner's web page is not hardened against the local network.** It is
  a password over plain HTTP on a Raspberry Pi. Anyone who can watch that
  network can read the password. It is meant for a home network, and it says
  so in `apps/scannerd/readme.md`.
- **`SCANNERD_UI_PASSWORD` and the Paperless token live in a file on the Pi.**
  Anyone with a shell there has them.
- **kuverta does not display remote images or run scripts in mail**, so
  reports about tracking pixels are about a thing that does not happen.
- **Findings from a scanner with no proof they can be reached.** A crash needs
  an input; a dependency advisory needs a path from kuverta's own code to the
  affected function.

## How this is checked

Every push and pull request, and again nightly, in
`.github/workflows/security.yml`:

| | |
|---|---|
| `cargo deny` | advisories, licences, and crates from anywhere but crates.io |
| `npm audit` | the JavaScript dev dependencies |
| CodeQL | Rust, JavaScript and the workflows, with `security-extended` |
| zizmor | the workflows: pinning, permissions, script injection, cache poisoning |
| `cargo fuzz` | a minute a target on a push, ten a target nightly |

Every GitHub Action is pinned to a commit, not a tag: a tag is a label its
owner can move, and CI is where the signing keys are. The workflow that signs
and publishes the installers restores nothing from any cache.

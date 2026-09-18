---
description: The kuverta command-line tool — accounts, sync, reading, changing and sending mail, post, and the model tools.
---

# The command line

`kuverta` is the development driver for the same core the window uses. It is
not the product, and it is not in the installers yet — but it is the only way
to do an OAuth2 sign-in for now, and it is the quickest way to script or
inspect a store.

```sh
cargo install --path apps/cli     # installs `kuverta` into ~/.cargo/bin
# or, without installing:
cargo run -p kuverta-cli -- <command> …
```

Every command takes `--data-dir` (or `KUVERTA_DATA_DIR`) to choose the store,
and honours `KUVERTA_INSTANCE=dev` like the app. Commands that act on one
account take `--email`, which may be left out when only one account is
registered. `kuverta <command> --help` has the details of each.

## Accounts

```sh
kuverta add-account \
    --email erika@example.de --host imap.example.de --port 993 \
    --smtp-host smtp.example.de --smtp-port 587 --smtp-security starttls
kuverta set-password --email erika@example.de        # reads the password from stdin
kuverta check --measure                               # connect, report, download nothing
kuverta sync
```

| Command | |
| --- | --- |
| `add-account` | register an account: `--host`, `--port`, `--security tls\|starttls\|plaintext`, `--username`, `--label`, `--smtp-host/--smtp-port/--smtp-security`, and for OAuth2 `--auth oauth2 --oauth-provider microsoft\|google --client-id … [--tenant …]` |
| `set-password` | store an app password in the keychain, read from stdin |
| `login` | authorise an OAuth2 account. Microsoft uses the device flow (a code to enter at a URL); Google a browser redirect back to this process. Only the refresh token is kept, in the keychain. `--client-secret` for Google, the first time. |
| `logout` | forget a stored OAuth2 login |
| `accounts` | list registered accounts |
| `set-smtp --email …` | add or change an account's outgoing server (`--smtp-host/--smtp-port/--smtp-security`), or `--clear` it |
| `check [--measure]` | connect and report what the server supports, where sent, archived and deleted mail will go, and with `--measure` how much a first sync would fetch. Run it before the first sync of any real account. |
| `exclude <folder>`, `include <folder>` | stop or resume syncing a folder, by name or by attribute such as `'\All'` |
| `create-folder <name> [--use '\Archive']` | create a folder on the server, optionally marked with a special-use attribute |

**Gmail**: an app password, or an OAuth2 client of type *Desktop app* with
`--auth oauth2 --oauth-provider google --client-id …` and then `login`. Its
`[Gmail]/All Mail` holds a copy of everything, so `check` suggests
`exclude '\All'` and says what that costs. **Microsoft 365**: an Azure app
registration, `--auth oauth2 --client-id …`, then `login`.

## Reading

| Command | |
| --- | --- |
| `sync [--email …]` | fetch new mail into the store; every account by default. `--password-env VAR` reads the password from an environment variable instead of the keychain (for the dev stack and CI). |
| `list [-n 20]` | the most recent messages, numbered |
| `search <query>` | full-text search: each word as the start of a word, in any order, spellings like *eSIM*/*e-SIM* both ways; `'"exact phrase"'` for a phrase |
| `attachments --id <id> [--save <n>] [--out <dir>]` | a message's attachments, or save one (never overwriting) |
| `reclassify [--dry-run]` | file mail again with the current rules, from each message's stored copy; `--dry-run` says what would move where. Corrections and model verdicts are left alone. |
| `triage [--category …]` | each message's category as the rules filed it; optionally only one category |
| `status` | what is in the store |

## Changing mail

Each takes a message — its number from `list`, or its Message-ID — and queues a
change, exactly like the window. Nothing deletes mail: `delete` moves to Trash.

| Command | |
| --- | --- |
| `archive <message>` | move to the Archive folder |
| `delete <message>` | move to the Trash |
| `move <message> --to <folder>` | move to a named folder |
| `read <message>`, `unread <message>` | mark read or unread |
| `queue` | changes queued and not yet sent |
| `undo` | cancel the most recent change not yet sent |

`--undo-window <seconds>` (default 10) is how long a change is held at least
before a `sync` may send it.

## Sending

The body is read from stdin, so it composes with an editor or a heredoc:

```sh
kuverta send --to erika@example.de --subject "Unterlagen" <<'EOF'
Hallo Erika,

anbei die Unterlagen.
EOF
```

| Option | |
| --- | --- |
| `--to`, `--cc`, `--bcc` | repeat for several recipients |
| `--reply-to <Message-ID>` | reply to a stored message; `--reply-all` for everyone on it |
| `--forward <Message-ID>` | forward a stored message (needs a `--to`) |
| `--sign`, `--encrypt` | sign with your OpenPGP key; encrypt to every recipient and to you. `--encrypt` is refused with `--bcc`. See [Encryption](../user-guide/encryption.md). |
| `--dry-run` | print the message and its envelope instead of sending |
| `--no-save-to-sent` | do not file a copy in Sent |

## Post

A postal address's post, read from Paperless-ngx:

```sh
export PAPERLESS_TOKEN=...            # Paperless: your profile → API token
kuverta paper --check                 # what is there, before anything depends on it
kuverta paper                         # list it, classified, like `kuverta list`
kuverta paper --tag home              # one address of several in one instance
kuverta paper --query rechnung        # full-text, over the OCR'd text
```

`--url` (or `PAPERLESS_URL`, default `http://localhost:8000`), and `--tag`,
`--correspondent` or `--storage-path` to choose which documents are this
address's. `--check` exists because a selector that matches nothing looks
exactly like an address that has had no post.

## Cleanup

| Command | |
| --- | --- |
| `unsubscribable` | senders whose mail can be unsubscribed from, and how |
| `similar --id <id>` | messages that look like one — what the window offers to delete with it |
| `ask "…"` | ask the assistant, with the window's tools; prints what it does and its answer |
| `run-tasks [--model]` | run the account's tasks now |
| `urgent [--model] [--min N]` | Inbox mail that needs you soonest, as the urgency agent judges it; `--model` asks the sorting model too |

## Models

A model can classify mail too, and is kept honest by never being trusted on its
own say-so: its verdicts are recorded beside the rules' and compared, and never
change what the list shows.

```sh
kuverta classify --email erika@example.de   # after a sync, never during one
kuverta disagreements                       # where it and the rules differ

python3 docker/fill-mailbox.py --dump 250 | node kuverta-bird/tools/dump-facts.js > labelled.jsonl
kuverta eval labelled.jsonl                 # rules vs prompting vs embeddings
kuverta eval labelled.jsonl --split sender  # the same, on senders never filed
```

`classify` uses the model chosen for sorting mail in the app's settings
(`llama3.2:3b` until one is chosen); `--model` overrides it, and `--ollama` (or
`OLLAMA_URL`) asks that Ollama directly. `eval` scores the rules, a prompted
model (`--model`) and nearest-neighbour embeddings (`--embed-model`, default
`nomic-embed-text`, `--k 5`) on the same held-out half, and also both together
at every threshold. The measurements and what they decided are in
`docs/decisions.md` §15–16.

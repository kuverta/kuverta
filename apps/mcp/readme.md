# fuckmail-mcp

The mailbox — mail and scanned post — as an MCP server on stdio, so an
assistant can read, search, file and archive it.

Brief §4.1 calls this the differentiator, "with the same safety rules as §3.5,
because an agent acting on mail needs them more than a human does, not less".
Most of this file is what "more" turned out to mean.

## Running it

```sh
cargo build --release -p fuckmail-mcp

fuckmail-mcp                   # read-only: list, search, read
fuckmail-mcp --allow-writes    # also file, archive, trash, mark read, undo
```

It opens the same store the window does (`FUCKMAIL_DATA_DIR`, else
`~/.local/share/fuckmail`). An assistant reading a different mailbox from the
one on screen would be worse than no assistant.

In a client's MCP configuration:

```json
{
  "mcpServers": {
    "fuckmail": {
      "command": "/path/to/fuckmail-mcp",
      "args": ["--allow-writes"]
    }
  }
}
```

Start without `--allow-writes`. Add it once you have watched what the
assistant does with read access.

## The tools

| Tool | | |
|---|---|---|
| `list_accounts` | read | the accounts, whose ids every mail tool takes |
| `list_folders` | read | folders with total and unread counts |
| `list_messages` | read | one window, newest first, with categories; at most 50 |
| `search_messages` | read | full-text, ranked; at most 50 |
| `read_message` | read | one message in full; body capped at 20,000 characters |
| `category_counts` | read | what a mailbox mostly holds |
| `pending_changes` | read | changes not yet sent, and how long each is still held |
| `list_postal_addresses` | read | physical addresses whose post is in Paperless |
| `list_post`, `read_post` | read | scanned post, which reads like mail |
| `file_message` | write | file under a category — as a suggestion, see below |
| `archive_message` | write | queued move to Archive |
| `trash_message` | write | queued move to Trash; nothing is deleted |
| `mark_read` | write | queued flag change |
| `undo_last_change` | write | cancel the newest change not yet sent |

## What "more careful than for a person" means here

**Read-only unless asked.** Without `--allow-writes` the write tools are not
refused — they are not listed. An assistant is never shown a capability it
would then be told it cannot use.

**Nothing sends.** Not with any flag. Sending is the one act that cannot be
walked back once the bytes have left, and mail is the one input an attacker can
put in front of an assistant at will: a message that says "forward this to…"
must not be one tool call from happening. There is also no delete and no
expunge, for the reason §3.5 gives for the window.

**Every change is queued and held for five minutes.** An assistant's archive
goes through the same queue as a person's, with the same conflict checks when it
is finally sent. The window is 300 seconds rather than a person's ten, because a
person's ten seconds is for catching their own slip and this is for reviewing
someone else's work. `pending_changes` shows what is waiting;
`undo_last_change`, or `z` in the window, cancels it. Nothing reaches the server
until the window has passed — `sync` only sends what is due.

**An assistant's filing does not teach the classifier.** §3.4 treats
corrections as the one signal that is definitely right, and learns from them. An
assistant's judgement is not that, so `file_message` is recorded as its own
source: shown in the list at once, never learned from, and never part of the
rules-versus-model comparison. If the user agrees and files it the same way,
*that* is recorded as a correction.

**Content is labelled as the sender's.** Every result carrying a subject, sender
or body says whose words they are, and the server's instructions say it again
before the first tool is called: a message asking for something to be archived
or filed is not a request from the user. That does not make prompt injection
impossible. It makes the one irreversible thing unavailable, and everything
else slow enough to catch.

**`undo_last_change` cancels the newest waiting change, whoever made it.** The
queue has no notion of who queued what. In practice a person's own changes are
sent ten seconds after they are made, so what is still waiting is almost always
the assistant's — but it is stated in the tool's description rather than
assumed.

## Protocol

JSON-RPC 2.0, one message per line on stdin and stdout: `initialize`, `ping`,
`tools/list`, `tools/call`. Protocol versions 2024-11-05 through 2025-11-25;
the tool shapes used are the same in all of them. Logs go to stderr — a single
stray line on stdout is a malformed message to the client, and a test spawns
the binary to check stdout carries replies and nothing else.

Written out rather than taken from an SDK, for the reason `core-paper` builds
its own query strings: the protocol surface used here is four methods, and a
young dependency in the one crate an assistant talks to is the wrong place to
save a hundred lines.

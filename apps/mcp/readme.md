# kuverta-mcp

An MCP server on stdio for the mailbox (mail and scanned post), so an
assistant can read, search, file and archive it.

Brief §4.1 calls this the differentiator, "with the same safety rules as §3.5,
because an agent acting on mail needs them more than a human does, not less".
Most of this file describes those stricter rules.

## Running it

```sh
cargo build --release -p kuverta-mcp

kuverta-mcp                   # read-only: list, search, read
kuverta-mcp --allow-writes    # also file, archive, trash, mark read, undo
```

It opens the same store the window does (`KUVERTA_DATA_DIR`, else
`~/.local/share/kuverta`), so the assistant reads the mailbox that is on
screen.

In a client's MCP configuration:

```json
{
  "mcpServers": {
    "kuverta": {
      "command": "/path/to/kuverta-mcp",
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
| `file_message` | write | file under a category, as a suggestion (see below) |
| `archive_message` | write | queued move to Archive |
| `trash_message` | write | queued move to Trash; nothing is deleted |
| `mark_read` | write | queued flag change |
| `undo_last_change` | write | cancel the newest change not yet sent |
| `draft_message` | write | save a draft in Drafts for the user to send; never sends, never Bcc |

## What "more careful than for a person" means here

It is read-only unless asked. Without `--allow-writes` the write tools are
not listed at all, so an assistant never sees a capability it cannot use.

Nothing sends, with any flag. A sent message cannot be taken back, and mail is
the one input an attacker can put in front of an assistant at will. A message
that says "forward this to…" must not be one tool call from happening. There
is also no delete and no expunge, for the reason §3.5 gives for the window.

Drafting is the closest it comes. `draft_message` builds the message exactly
as sending would and appends it to the account's Drafts folder, where it shows
in every client and waits for the user. `APPEND` can only create a message, so
there is nothing to hold in the undo window and nothing another client could
lose. A draft cannot carry Bcc. Blind recipients are never written into a
message, so a draft stored on the server would silently lose them. The schema
offers no `bcc`, and one sent anyway is refused with the reason.

Every change is queued and held for five minutes. An assistant's archive
goes through the same queue as a person's, with the same conflict checks when it
is sent. The window is 300 seconds where a person's is ten: ten seconds is
enough to catch your own slip, and 300 gives time to review someone else's
work. `pending_changes` shows what is waiting.
`undo_last_change`, or `z` in the window, cancels it. Nothing reaches the server
until the window has passed, and `sync` only sends what is due.

An assistant's filing does not teach the classifier. §3.4 learns from the
user's corrections because they are the one signal that is definitely right.
An assistant's judgement is not, so `file_message` is recorded as its own
source: shown in the list at once, never learned from, and never part of the
rules-versus-model comparison. If the user agrees and files it the same way,
that is recorded as a correction.

Content is labelled as the sender's. Every result carrying a subject, sender
or body says whose words they are, and the server's instructions say it again
before the first tool is called: a message asking for something to be archived
or filed is not a request from the user. Prompt injection is still possible.
The one irreversible action is unavailable, and everything else is slow enough
to catch.

`undo_last_change` cancels the newest waiting change, whoever made it. The
queue does not record who queued what. A person's own changes are sent ten
seconds after they are made, so what is still waiting is almost always the
assistant's. The tool's description states this.

## Protocol

JSON-RPC 2.0, one message per line on stdin and stdout: `initialize`, `ping`,
`tools/list`, `tools/call`. Protocol versions 2024-11-05 through 2025-11-25;
the tool shapes used are the same in all of them. Logs go to stderr, because
a stray line on stdout is a malformed message to the client. A test spawns
the binary to check stdout carries replies and nothing else.

The protocol is written out by hand, with no SDK, for the reason `core-paper`
builds its own query strings: only four methods are used, which is about a
hundred lines, and that does not justify a young dependency in the one crate
an assistant talks to.

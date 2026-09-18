---
description: kuverta-mcp serves your mailbox — mail and scanned post — to an assistant over MCP, read-only by default and never able to send.
---

# The MCP server

`kuverta-mcp` serves the mailbox — mail and scanned post — over the
[Model Context Protocol](https://modelcontextprotocol.io) on stdio, so an
assistant can read, search, file and archive it. It is built to be **more
careful than it would be for a person**, because mail is the one input an
attacker can put in front of an assistant at will.

## Running it

```sh
cargo build --release -p kuverta-mcp

kuverta-mcp                   # read-only: list, search, read
kuverta-mcp --allow-writes    # also file, archive, trash, mark read, undo, draft
```

It opens the same store the window does (`KUVERTA_DATA_DIR`, otherwise the
default data directory). In a client's MCP configuration:

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

Start without `--allow-writes`. Add it once you have watched what the assistant
does with read access.

## The tools

| Tool | | |
| --- | --- | --- |
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
| `draft_message` | write | save a draft in Drafts for you to send; never sends, never Bcc |

## The safety rules

- **Read-only unless asked.** Without `--allow-writes` the write tools are not
  listed at all.
- **Nothing sends**, with any flag. `draft_message` builds the message exactly
  as sending would and puts it in the account's Drafts folder, where it waits
  for you in every mail program. A draft cannot carry Bcc, and an assistant's
  drafts are never signed or encrypted.
- **No delete, no expunge.** Trash is a move.
- **Every change is held for five minutes** in the same queue as yours, with the
  same checks when it is finally sent — 300 seconds rather than a person's ten,
  because this is for reviewing someone else's work. `pending_changes` shows
  what is waiting; `undo_last_change`, or <kbd>z</kbd> in the window, cancels it.
- **An assistant's filing does not teach the classifier.** It shows in the list
  at once, is recorded as the assistant's, and is never learned from. If you
  file it the same way, *that* is a correction.
- **Content is labelled as the sender's.** Every result with a subject, sender
  or body says whose words they are, and the server's instructions say again
  that a message asking for something is not a request from you.

That does not make prompt injection impossible. It makes the one irreversible
thing unavailable, and everything else slow enough to catch.

## Protocol

JSON-RPC 2.0, one message per line on stdin and stdout: `initialize`, `ping`,
`tools/list`, `tools/call`; protocol versions 2024-11-05 through 2025-11-25.
Logs go to stderr — a stray line on stdout would be a malformed message to the
client. More in `apps/mcp/readme.md` and `docs/decisions.md` §14 and §18.

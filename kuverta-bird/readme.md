# kuverta-bird

A keyboard-first triage surface for mail, with a classifier that files
everything into six categories and tells you why.

This directory is the JavaScript half: the classifier, the triage model, the
surface, and one adapter per host. The Rust half (IMAP, SMTP, the store, the
accounts) is the rest of the repository. They were two repositories for a day;
see [decisions.md §10](../docs/decisions.md).

The same code runs in two hosts:

- as a Thunderbird extension, which gives access to the add-on ecosystem;
- inside `kuverta`, the standalone client in this repository, through the Tauri
  commands its desktop app already speaks.

[The brief](../docs/thunderbird-fork-brief.md) allows for a Thunderbird fork at
stage 4, and its §7.1 accepts that it may never happen if stages 1 to 3 work as
a plain add-on. Two hosts keep that option cheap: the product is the surface.

## What it does

Every message is scored on a few headers (`List-Id`, `List-Unsubscribe`,
`Precedence`, `Auto-Submitted`, the sender, the subject, the recipient count)
and filed under one of:

**personal** · **newsletter** · **marketing** · **transactional** ·
**notification** · **unknown**

`unknown` is a real category. With no strong signal the classifier says so
instead of guessing the commonest one.

You triage from one list, with categories as the axis instead of folders:

```
j / k       move            e   archive        z   undo
enter       read            #   trash          /   search
1…6         file as         u   unread         ?   shortcuts
```

Two rules:

Acting keeps the cursor where it was. Archive a message and the next one slides
into its place.

A filtered list says what it is showing and offers a way out. Otherwise it
looks like mail going missing.

A correction overrides the heuristics: a sender or list filed once is filed
that way next time. Every correction is logged from the first day, because that
labelled corpus cannot be collected later.

## Layout

```
core/                the whole idea, and no host in it
  category.js        the six categories
  signals.js         the heuristics and their weights
  classify.js        scoring, confidence, learned overrides
  facts.js           headers → the facts the classifier reads
  corrections.js     the corrections log
  host.js            the port: what a host must do, and what it may not
  triage.js          the list: cursor, scopes, actions
  keys.js            the key map, written down once
  view/triage.js     the surface both hosts mount

hosts/thunderbird/   an adapter over the MailExtension API
hosts/kuverta/      an adapter over the client's Tauri commands
```

`core/` knows about neither host. `test/wiring.test.js` fails if anything under
`core/` mentions `messenger.`, `__TAURI__` or `browser.`, or if one host
imports the other.

What only one host can do is declared in a `capabilities` object, so the
surface hides what is unavailable. All three adapters now declare every
capability. A fourth reference host, `ReadOnlyMailbox`, declares almost nothing
and keeps the "refused, not quietly ignored" rule and the skip machinery
exercised. Most of its conformance run is skips.

### The contract

`test/support/conformance.js` is the host contract as a test suite. Three
adapters run it: Thunderbird against a fake `messenger`, `kuverta` against a
fake `invoke`, and an in-memory reference.

Capability gates report as skips with a reason, never as passes. A test that
did nothing and reported `ok` would hide a regression.

## Using it

### In Thunderbird

1. **Tools → Developer Tools → Debug Add-ons**
2. **Load Temporary Add-on…** → pick `manifest.json` at the repository root

No build step. The manifest sits at the root so that `core/` is inside the
extension root, and Thunderbird loads the same files the tests run.
`npm run package` produces an installable `kuverta-bird.xpi`.

New mail is filed as it arrives. For existing mail, right-click a folder
→ **kuverta-bird: classify this folder**. The toolbar button opens the triage tab.

Requires Thunderbird 128 or newer.

### In `kuverta`

See [hosts/kuverta/readme.md](hosts/kuverta/readme.md).

## Working on it

From this directory:

```sh
npm test       # the model, the adapters, the classifier, and the view in jsdom
npm run e2e    # the triage page and the settings sheet in real Chromium
npm run corpus # the classifier over 250 generated messages, with the rules that fired
```

## What is true, and what is not yet

**Verified.** The classifier is a line-for-line port of `kuverta`'s Rust
`core-rules`. Over the same 250 messages it gives identical verdicts and
identical confidences to six decimal places. The triage model's cursor
behaviour, the corrections log, the tag writing, and all three host adapters
are tested, including that trashing is always a move and that undo puts it
back.

**Checked against the documentation.** All twenty-five Thunderbird API calls
the adapter makes were checked against the Manifest V2 reference. That found
four missing permissions (`messagesMove`, `messagesTagsList`, `compose`,
`tabs`) and three wrong assumptions, all fixed. A fake cannot catch these. The
table is in `test/permissions.test.js`, which also fails if the manifest asks
for a permission nothing uses. See [decisions.md §6](../docs/decisions.md).

**Not verified.** Neither host has been run. Expect the first real load to
behave differently from the documentation in places.

**Known gaps.**
- No paperclip in the Thunderbird list. `MessageHeader` has no attachment
  information, and getting it costs a round trip per message or a second query
  per scope. The classifier still sees attachments, from the part tree when a
  message is opened.
- The Thunderbird adapter materialises a whole scope to answer an offset,
  because Thunderbird's lists are cursor-paged and unordered and the port wants
  an offset into a list sorted newest first. Capped at 20,000 messages. A large
  folder pauses the first time it is opened. Make it lazy once there is a real
  mailbox to test against.
- Manifest V2, for the reason in [decisions.md §3](../docs/decisions.md).
- Bulk marketing carrying `List-Id` and `Precedence: bulk` is filed as a
  newsletter, a third of that category on the test corpus (the brief's §3.2
  weakness). One correction fixes it for that sender.
- No compose in the shared surface. Each host has its own.
- The corrections log is recorded and consulted, but nothing reads it back for
  the model of brief §3.3, which does not exist yet.

## Where this is going

| Stage | | |
|---|---|---|
| 0 | Build unmodified `comm-central` ESR | not started |
| **1** | The classifier as a MailExtension | **done** |
| **2** | The triage surface, acting keeps the cursor | **done, in both hosts** |
| **3** | Corrections recorded, and the rules learning from them | done in Thunderbird; needs one command in `kuverta` |
| 4 | Rebrand and cut the first real build. The fork begins here | |
| 5 | The model, with both verdicts logged from the first message | |
| 6 | MCP surface | |
| 7 | Paper: `scannerd` + Paperless-ngx, unified list | |

Stages 1 to 3 need no fork. Take on the fork's costs at stage 4, and only for
something an add-on cannot do.

## Licence

Not yet decided (brief §5 and §7.4). Nothing here derives from Mozilla code, so
MPL-2.0 does not apply yet. That changes at stage 4.

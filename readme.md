# fuckbird

A keyboard-first triage surface for mail, with a classifier that files
everything into six categories and tells you why.

It runs in two places from the same code:

- **as a Thunderbird extension**, which is what makes the add-on ecosystem
  available;
- **inside [`fuckmail`](../fuckmail)**, the standalone client, through the Tauri
  commands its desktop app already speaks.

Eventually, per [the brief](docs/brief.md), possibly a Thunderbird fork. That
is stage 4 and it is not the point. The brief's §7.1 allows that "if stages 1–3
deliver the idea as a plain add-on, the honest answer may be no, and that would
be a good outcome" — and building for two hosts is what makes that answer
cheap, because neither host is the product. The surface is.

## What it does

Every message is scored against a handful of headers — `List-Id`,
`List-Unsubscribe`, `Precedence`, `Auto-Submitted`, the sender, the subject, the
recipient count — and filed under one of:

**personal** · **newsletter** · **marketing** · **transactional** ·
**notification** · **unknown**

`unknown` is a category, not a fallback to the commonest guess. No signal strong
enough to guess is better than a confident wrong answer; a classifier that is
never unsure teaches you to distrust all of it.

Then you triage from one list, with the categories as the axis rather than
folders:

```
j / k       move            e   archive        z   undo
enter       read            #   trash          /   search
1…6         file as         u   unread         ?   shortcuts
```

Two rules that are most of why it is worth using:

**Acting keeps the cursor where it was.** Archive the message under the cursor
and the next one slides into its place, rather than the list jumping to the top
and making you find your way again.

**A filtered list says what it is showing, and offers a way out.** A list that
quietly narrows looks exactly like mail going missing, and that is the one thing
a mail client may never look like it is doing.

Correcting a verdict overrides the heuristics outright — a sender or list you
have filed once is filed that way next time — and every correction is written to
a log from the first day, because a labelled corpus of your own mail is not
something you can go back and collect later.

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
hosts/fuckmail/      an adapter over the client's Tauri commands
```

`core/` knows about neither host. That is enforced rather than intended:
`test/wiring.test.js` fails if anything under `core/` so much as mentions
`messenger.`, `__TAURI__` or `browser.`, or if one host imports the other.

Everything a host can do that the other cannot is **declared** in a
`capabilities` object, so the surface hides what is unavailable instead of
offering it and failing. Today that matters in one place: `fuckmail` has no
command to file a message by category, so it says so and the category keys do
not appear there. See [hosts/fuckmail/readme.md](hosts/fuckmail/readme.md).

### The contract

`test/support/conformance.js` is the host contract, as a test suite. Three
adapters run it — Thunderbird against a fake `messenger`, `fuckmail` against a
fake `invoke`, and an in-memory reference that proves the contract is
satisfiable at all. That is what makes "it works in either host" a claim rather
than a hope.

Capability gates report as **skips with a reason**, never as passes: a test that
quietly did nothing and said `ok` is how a regression hides.

## Using it

### In Thunderbird

1. **Tools → Developer Tools → Debug Add-ons**
2. **Load Temporary Add-on…** → pick `manifest.json` at the repository root

No build step: the manifest sits at the root so that `core/` is inside the
extension root, and the files Thunderbird loads are byte for byte the files the
tests run. For something installable, `npm run package` produces `fuckbird.xpi`.

New mail is filed as it arrives. For mail you already have, right-click a folder
→ **fuckbird: classify this folder**. The toolbar button opens the triage tab.

Requires Thunderbird 128 or newer.

### In `fuckmail`

See [hosts/fuckmail/readme.md](hosts/fuckmail/readme.md).

## Working on it

```sh
npm test       # 153 tests, no mail client required
npm run corpus # the classifier over 250 generated messages, with the rules that fired
```

## What is true, and what is not yet

Being explicit, because the difference matters and is easy to lose.

**Verified.** The classifier is a line-for-line port of `fuckmail`'s Rust
`core-rules`: run against it over the same 250 messages it produces identical
verdicts *and* identical confidences to six decimal places. The triage model's
cursor behaviour, the corrections log, the tag writing, and all three host
adapters are tested — including that trashing is always a move and that undo
puts it back.

**Not verified.** Neither host has been run. The classifier and the model are
proven; the adapters are written against the documented APIs and tested against
fakes, which catches an adapter drifting but cannot catch an API that is not
what the documentation says. Expect the first real load to turn up at least one
wrong permission string and at least one Thunderbird API that behaves
differently from the fake.

**Known gaps.**

- **The Thunderbird adapter materialises a whole scope** to answer an offset,
  because Thunderbird's lists are cursor-paged and unordered where the port asks
  for an offset into a list sorted newest first. Capped at 20,000 messages. A
  large folder will pause the first time it is opened. Making it lazy is worth
  doing against a real mailbox and not before.
- **`fuckmail` cannot file by category** until it grows one command.
- **Manifest V2**, for the reason in [decisions.md §3](docs/decisions.md).
- Bulk marketing carrying `List-Id` and `Precedence: bulk` is filed as a
  newsletter — a third of that category on the test corpus. The brief's §3.2
  weakness, measured, and the best argument for the corrections log: one
  correction fixes all of it for that sender.
- No compose in the shared surface. Each host has its own and this does not
  rebuild them.
- The corrections log is recorded and consulted, but nothing reads it back for
  the model of brief §3.3, which does not exist yet.

## Where this is going

| Stage | | |
|---|---|---|
| 0 | Build unmodified `comm-central` ESR | not started |
| **1** | The classifier as a MailExtension | **done** |
| **2** | The triage surface, acting keeps the cursor | **done, in both hosts** |
| **3** | Corrections recorded, and the rules learning from them | done in Thunderbird; needs one command in `fuckmail` |
| 4 | Rebrand and cut the first real build — the fork begins here | |
| 5 | The model, with both verdicts logged from the first message | |
| 6 | MCP surface | |
| 7 | Paper: `scannerd` + Paperless-ngx, unified list | |

Stages 1–3 need no fork at all. Do not pay the fork's costs until stage 4, and
only for something an add-on genuinely cannot do.

## Licence

Not yet decided — brief §5 and §7.4. Nothing here is derived from Mozilla code,
so this repository is not yet constrained by MPL-2.0; that changes at stage 4.

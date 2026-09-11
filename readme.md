# fuckbird

A Thunderbird add-on that files mail into six categories and tells you why.

Eventually, per [the brief](docs/brief.md), a fork. Not yet, and possibly never
— the brief's own §7.1 allows that "if stages 1–3 deliver the idea as a plain
add-on, the honest answer may be no, and that would be a good outcome". This is
stage 1.

It is the classifier from [`fuckmail`](../fuckmail), ported to JavaScript and
wired into stock Thunderbird. Nothing is forked, nothing is patched, and there
is no second executable to install.

## What it does

Every message that arrives is scored against a handful of headers — `List-Id`,
`List-Unsubscribe`, `Precedence`, `Auto-Submitted`, the sender, the subject, the
recipient count — and filed under one of six categories as a Thunderbird tag:

**personal** · **newsletter** · **marketing** · **transactional** ·
**notification** · **unknown**

`unknown` is a category, not a fallback to the commonest guess. No signal strong
enough to guess is better than a confident wrong answer; a classifier that is
never unsure teaches you to distrust all of it.

Click the button in the message header to see the evidence, and to disagree:

```
  Newsletter                            78% confident
  ─────────────────────────────────────────────────────
  sent through the mailing list news.golem.example.de  +2.5
  marked Precedence: bulk                              +1
  offers a List-Unsubscribe link                       +1
  ─────────────────────────────────────────────────────
  File it as
  [Personal] [Newsletter] [Marketing] [Transactional] …
```

A correction overrides the heuristics outright — a sender or list you have filed
once is filed that way next time — and every correction is written to a log from
the first day, because a labelled corpus of exactly your own mail is not
something you can go back and collect later.

## Using it

Load it unpacked while you are working on it:

1. Thunderbird → **Tools → Developer Tools → Debug Add-ons**
2. **Load Temporary Add-on…** → pick `extension/manifest.json`

Or build something installable:

```sh
npm run package     # -> fuckbird.xpi
```

New mail is filed as it arrives. For mail you already have, right-click a folder
and choose **fuckbird: classify this folder**. To correct a verdict, either use
the button in the message header or right-click messages in the list and pick
**fuckbird: file as**.

The corrections log — everything it has learned, and the events behind it — is
on the add-on's options page, with a JSON export.

Requires Thunderbird 128 or newer.

## Working on it

```sh
npm test            # 49 tests, no Thunderbird required
```

The classifier is deliberately free of any Thunderbird API, which is what lets
it be tested in a plain Node process. The split is worth preserving:

| | |
|---|---|
| `extension/src/category.js` | the six categories |
| `extension/src/signals.js` | the heuristics and their weights |
| `extension/src/classify.js` | scoring, confidence, learned overrides |
| `extension/src/facts.js` | headers → the facts the classifier reads |
| `extension/src/corrections.js` | the corrections log |
| `extension/src/messages.js` | the only file that talks to Thunderbird |
| `extension/src/tags.js` | categories as tags |
| `extension/background.js` | wiring, and nothing else |

To check the port still matches the Rust it came from, or to see what the rules
do across a few hundred messages at once:

```sh
python3 tools/fill-mailbox.py --dump 250 | node tools/classify-corpus.js --reasons
```

`tools/fill-mailbox.py` is carried over from `fuckmail` unchanged apart from
that `--dump` mode; given an IMAP server it still fills a test mailbox with a
few hundred realistic messages instead.

## What is true, and what is not yet

Being explicit, because the difference matters and is easy to lose:

**Verified.** The classifier is a line-for-line port. Run against the Rust
`core-rules` over the same 250 messages it produces identical verdicts *and*
identical confidences to six decimal places. The twelve tests from the Rust
suite were carried across and pass, alongside 37 more covering the parts that
are new here. See [decisions.md](docs/decisions.md).

**Not verified.** None of this has been run inside Thunderbird yet. The
classifier is proven; the add-on around it — the manifest, the permission names,
the tag API, the menu contexts — is written from the documented API and has not
met a real client. Expect the first install to turn up at least one wrong
permission string.

**Known gaps.**

- Manifest V2, for the reason in [decisions.md §3](docs/decisions.md). It will
  have to move to V3.
- Bulk marketing that carries `List-Id` and `Precedence: bulk` is filed as a
  newsletter. On the test corpus that is a third of the newsletter category.
  This is the brief's §3.2 weakness, measured — and the best argument for the
  corrections log, since one correction fixes all of it for that sender.
- The corrections log is recorded and consulted, but nothing reads it back for
  the model of brief §3.3, which does not exist yet.
- No triage surface. That is stage 2, and it is the stage that makes this worth
  using daily rather than merely correct.

## Where this is going

From the brief's staged plan:

| Stage | | |
|---|---|---|
| 0 | Build unmodified `comm-central` ESR | not started |
| **1** | **The classifier as a MailExtension** | **this** |
| 2 | The triage tab: keyboard-first, acting keeps the cursor | next |
| 3 | Corrections recorded, and the rules learning from them | done early — it was cheap, and §3.4 wanted the log from day one |
| 4 | Rebrand and cut the first real build — the fork begins here | |
| 5 | The model, with both verdicts logged from the first message | |
| 6 | MCP surface | |
| 7 | Paper: `scannerd` + Paperless-ngx, unified list | |

Stages 1–3 need no fork at all. Do not pay the fork's costs until stage 4, and
only for something an add-on genuinely cannot do.

## Licence

Not yet decided — brief §5 and §7.4. Nothing here is derived from Mozilla code,
so this repository is not yet constrained by MPL-2.0; that changes at stage 4.

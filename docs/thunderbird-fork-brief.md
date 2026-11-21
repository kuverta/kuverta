# Brief: a triage-first Thunderbird fork

Written 2026-09-11.

Later changes are marked inline with **Since:** and recorded in
[decisions.md](decisions.md). The original reasoning is left as written.

> **Since (2026-09-11):** this brief assumes a new repository. There was one for
> a day, then it was merged back. The triage surface and the classifier live in
> [`kuverta-bird/`](../kuverta-bird/) beside the Rust client, as one product
> with two hosts. See [decisions.md](decisions.md) §4 and §10.

Starting brief for building the idea behind `kuverta` on Thunderbird. The reason
is the add-on ecosystem: extensions need Gecko, and Gecko cannot be embedded
outside Mozilla's own applications.

It lists what to carry across and why. The code is a few thousand lines and
Thunderbird has equivalents for most of it. The decisions took longer, and
several came out of bugs.

---

## 1. What kind of fork

A soft fork of ESR, the [Betterbird](https://www.betterbird.eu/faq/) model: a
patch series against the Extended Support Release, rebased roughly once a year,
plus security updates. A hard fork would mean tracking a trunk that changes
daily.

Keep the patch series small. Every line patched into `comm-central` is
re-applied at every rebase, so anything that can live in an add-on should.

### Where each thing should live

| Mechanism | Use it for | Rebase cost |
|---|---|---|
| **MailExtension** (WebExtension API) | Anything the [41 documented namespaces](https://developer.thunderbird.net/add-ons/mailextensions/supported-webextension-api) can do: reading messages, folders, compose, menus, toolbar buttons | None |
| **Experiment API** | Privileged JS reaching into internals the API does not expose | Low, but breaks on internal refactors |
| **Core patch** | Only what neither can do: new database columns, sync-path hooks, new UI surfaces | High, forever |

For each feature, use the cheapest mechanism that works. Most of what follows is
an add-on.

---

## 2. What Thunderbird already gives you

Do not rebuild these. `kuverta` has all of them:

- IMAP sync, including CONDSTORE/QRESYNC and IDLE
- A message store, full-text search, and offline copies
- MIME parsing, compose, SMTP submission, save-to-Sent
- Accounts, OAuth2 for Gmail and Microsoft 365, the OS keychain
- Address book, filters, tagging, threading, HTML rendering, attachments
- The add-on ecosystem

In `kuverta` that is `core-proto`, `core-store`, `core-smtp`, `core-accounts`
and most of `core-rpc`, around 11,000 of 15,500 lines. Dropping them only pays
off if the add-ons are worth it.

Thunderbird has no equivalent for classification, the triage surface, the agent
interface, and the paper half. Those are the project.

---

## 3. What to carry across

### 3.1 The triage surface

One keyboard-first list, with the classifier's categories as the primary axis
instead of folders.

- `j`/`k` move, `enter` read, `e` archive, `#` trash, `u` unread, `z` undo,
  `c` compose, `R` reply, `A` reply-all, `f` forward, `r` sync, `/` search,
  `,` settings
- Acting keeps the cursor where it was, so the next message slides under it.
  Easy to get wrong.
- A filtered list says what it is showing and offers a way out. Otherwise it
  looks like mail going missing.

*Where:* a MailExtension providing a tab, using `mailTabs`, `messages`,
`folders` and `menus`. No core patch.

> **Since (2026-09-11):** built once for two hosts. The surface is
> `core/view/triage.js`, driven by `core/triage.js`, over the port in
> `core/host.js`. Thunderbird and `kuverta` are both adapters behind it. See
> [decisions.md](decisions.md) §4.

### 3.2 The deterministic classifier

Six categories (personal, newsletter, marketing, transactional, notification,
and **unknown**), scored from `List-Id`, `List-Unsubscribe`, `Precedence`,
`Auto-Submitted`, sender, subject patterns and recipient count. Bilingual
(German and English), like the mailbox.

`Unknown` is a real category. With no strong signal the classifier says so
instead of guessing the commonest category, because confident wrong answers
make the user distrust all of it.

Corrections override the heuristics: a sender or list filed once is filed that
way next time.

*Carry the code.* `core-rules` is 793 lines with no dependencies in the tree,
and Thunderbird has nothing like it. Port it to JS or keep it as Rust behind a
native-messaging host (§3.7).

*Measured result on 251 realistic messages:* newsletter 77, transactional 62,
notification 61, personal 33, marketing 18. Bulk marketing carrying `List-Id`
and `Precedence: bulk` reads as a newsletter. That weakness is a good first
target for the model.

> **Since (2026-09-11):** the counts are correct but describe one mailbox, the
> one still in `kuverta`'s scratch store (251 messages, 77/62/61/33/18):
> `fill-mailbox.py` output from an earlier run plus one real message from the
> provider. A fresh `--dump 250` gives 79/62/59/32/18 because the draw differs.
> Every sender gets the same category in both. Do not cite the counts as a
> baseline. The weakness is larger than stated: a quarter of those newsletters
> are a marketing fixture whose subject has no keyword the rules know. See
> [decisions.md](decisions.md).

### 3.3 The local model, and how to avoid fooling yourself about it

`kuverta` never got this far. Keep its design:

1. Always log both verdicts. Run the rules baseline beside the model and record
   where they disagree and which was right. Otherwise you cannot tell whether
   the model beats a `HashMap` of sender→folder or just adds latency.
2. Spike embeddings against prompting first. For "which category is this",
   nearest neighbours among your own past filings are usually faster and
   cheaper than generative prompting, need no prompt tuning, and improve with
   each correction.
3. A 4 to 8B instruct model on Apple Silicon is ample.
4. Run it after sync, so a slow model cannot slow sync down.

*Where:* a native-messaging host talking to Ollama, or an add-on calling a local
HTTP endpoint. Never a core patch.

### 3.4 Corrections are training data

Record every reclassification from day one, even though nothing uses it until
the model lands. The model then starts with a labelled corpus of the mail it
will be asked about.

*Where:* add-on storage, or your own table. Thunderbird's tags hold only the
current state, and you need the correction event.

### 3.5 The safety rules for writes

These came out of bugs. Thunderbird's own write paths are mature, so they apply
mostly to code you add that mutates mail.

- Nothing destroys mail. Delete means move to Trash. Never send a bare
  `EXPUNGE`: it removes every `\Deleted` message in the folder, including ones
  another client marked.
- Never trust a UID alone. Before acting, check the folder still has the
  UIDVALIDITY the change was queued under, that a message is still at that UID,
  and that its `Message-ID` is the expected one. The server only guarantees it
  will not reissue a UID.
- Queue intent before acting. A crash between click and round trip must lose
  nothing and repeat nothing.
- Undo works by holding back the send. A change is cancellable while it is
  queued.
- Bcc goes in the envelope, never in the message. This cannot be undone once
  sent.

### 3.6 Things learned about real providers

Put these in the new repo's notes on day one:

- Gmail's `[Gmail]/All Mail` holds a copy of everything, so a naive sync
  fetches most mail twice. Exclude it.
- Gmail does not accept the OAuth2 device flow for `https://mail.google.com/`.
  It needs the loopback redirect (RFC 8252, with PKCE). Thunderbird already
  handles this.
- Dovecot advertises `SPECIAL-USE` but not `CREATE-SPECIAL-USE`, so a folder
  you create has no attribute and must be found by name.
- A message with no `Message-ID` is legal and occurs. Code that treats it as
  "no message" breaks without an error.
- Run a preflight before downloading a mailbox: connect, then report
  credentials, extensions, where mail will be filed, and what a first sync
  would fetch.

### 3.7 Keeping the Rust, if you want to

`core-rules` (and later the model layer) can stay Rust behind a **native
messaging host**: add-ons may launch a local binary and speak JSON to it. That
keeps the classifier, its tests, and the option to reuse it. The cost is a
second executable to ship and install.

Decide early. It gets awkward later.

---

## 4. New ground

### 4.1 The agent surface

Expose the mailbox over **MCP** so an assistant can read, search, classify,
draft and file, under the safety rules of §3.5. An agent needs those rules more
than a human does.

Thunderbird has no equivalent and no add-on does it well. This is what nobody
else has, so consider building it first.

### 4.2 The paper half

Scanned post lands in the same triage surface as mail. `scannerd` on a Pi
captures, deskews and uploads. [Paperless-ngx](https://docs.paperless-ngx.com/)
does OCR, tagging and archive. The fork shows documents and mail as one item
type in one list.

Only the capture daemon is yours, roughly 600 lines, which keeps this
affordable.

---

## 5. What you are signing up for

Plan for these. None is a reason to stop.

**Licence.** Thunderbird is MPL-2.0 and forked files stay MPL-2.0. Your own new
files can use any licence. Whether the combined work can ship under AGPL depends
on MPL-2.0 §3.3 and on no relevant files being marked "Incompatible With
Secondary Licenses". Check before promising a licence: the original plan's
decision 8 assumed AGPL.

**Trademark.** MPL grants no trademark rights, so the fork must be rebranded.
Betterbird carries [branding
patches](https://github.com/Betterbird/thunderbird-patches) for this. Budget for
a name, icons and an About screen before first distribution.

**Build times.** `comm-central` is a full Gecko build: tens of minutes from cold
on a laptop and a large object directory. On a 10h/week schedule this is the
biggest change. The fast edit-run loop only survives inside add-ons.

**The rebase.** One substantial ESR rebase per year plus security updates, for
as long as the project lives.

**Signing and notarisation.** Distributing a macOS application needs an Apple
Developer account and notarisation. This can wait while you are the only user.

---

## 6. A staged plan

Sized for ~10h/week. Each stage is useful alone.

| Stage | Work | Gate |
|---|---|---|
| **0** | Build unmodified `comm-central` ESR and run it. Nothing else. | It builds and launches, and you know how long that takes |
| **1** | The classifier as a MailExtension, categories shown as tags. No fork yet. | Your own mail is categorised in stock Thunderbird |
| **2** | The triage tab: keyboard-first list over `mailTabs`, acting keeps the cursor | You triage from it daily |
| **3** | Corrections recorded, and the rules learning from them | Filing once changes what happens next time |
| **4** | Rebrand and cut the first real build. The fork begins here | You run your own build |
| **5** | The model, with both verdicts logged from the first message | You can say whether it beats the rules, with numbers |
| **6** | MCP surface | An assistant can work your mail safely |
| **7** | Paper: `scannerd` + Paperless-ngx, unified list | Post is searchable beside mail |

Stages 1 to 3 need no fork. They run on stock Thunderbird and still work if the
fork never happens. Take on the fork's costs at stage 4, and only for something
an add-on cannot do.

---

## 7. Decisions still open

1. **Does the fork ever actually happen?** If stages 1 to 3 deliver the idea as
   a plain add-on, possibly not. That would be a good outcome.
2. ~~**Rust behind native messaging, or port the classifier to JS?** (§3.7)~~
   **Closed 2026-09-11: ported to JS.** Stages 1 to 3 must run on stock
   Thunderbird, and a second executable to install breaks that. The port is
   verified against the Rust line for line. See [decisions.md](decisions.md).
3. **Embeddings or prompting** for classification. Spike both, briefly. (§3.3)
4. **Licence**, once §5 has been checked properly.
5. **Name**, needed before stage 4.
6. ~~**What to do with `kuverta`.** It syncs, sends, triages and has 171
   passing tests. Options: archive it as a reference, keep it as the
   native-messaging backend, or keep running it alongside (it and Thunderbird
   coexist on the same IMAP account today).~~
   **Closed 2026-09-11: neither.** `kuverta` is a *host*, and so is
   Thunderbird. The product is the triage surface and the classifier, which now
   run on both from one core behind a tested contract. Decision 1 matters less
   as a result, since both run with or without the fork. See
   [decisions.md](decisions.md) §4.

---

## 8. Worth reading alongside this

- [`readme.md`](../readme.md), especially the design decisions section
- [`docs/implementation-plan.md`](implementation-plan.md): §1a on the two
  stages of write capability, §4 on the classifier, §5 on risks
- [`crates/core-rules/`](../crates/core-rules/): the classifier, portable as-is
- [`crates/core-proto/src/mutate.rs`](../crates/core-proto/src/mutate.rs): the
  conflict checks of §3.5, with the reasoning in the comments
- [`docker/fill-mailbox.py`](../docker/fill-mailbox.py): fills a test mailbox
  with a few hundred realistic messages against any IMAP server. Bring it
  across on day one

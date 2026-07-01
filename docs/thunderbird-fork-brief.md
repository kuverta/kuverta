# Brief: a triage-first Thunderbird fork

Written 2026-09-11.

Kept as written. What has changed since is marked inline with **Since:** rather
than edited into the reasoning, and recorded in [decisions.md](decisions.md).

> **Since (2026-09-11):** this brief assumes a new repository, and there was one
> for a day. It has been merged back: the triage surface and the classifier live
> in [`kuverta-bird/`](../kuverta-bird/) alongside the Rust client, because they are one
> product with two hosts rather than two projects. See
> [decisions.md](decisions.md) §4 and §10.

This is the starting brief for building the same idea as
`kuverta` on top of Thunderbird instead of from scratch. The
motivation is the add-on ecosystem: Thunderbird extensions need Gecko, Gecko
cannot be embedded outside Mozilla's own applications, and so running them means
*being* a Thunderbird rather than talking to one.

What follows is everything worth carrying across, with the reasoning attached.
The reasoning is the valuable part — the code is a few thousand lines and
Thunderbird already has most of its equivalents, but the decisions behind it
took real time to reach and several were paid for with bugs.

---

## 1. What kind of fork

**A soft fork of ESR, not a hard fork of trunk.** This is the
[Betterbird](https://www.betterbird.eu/faq/) model: keep a patch series against
the Extended Support Release and rebase it, rather than owning a diverging copy
of a twenty-year-old codebase.

It is the single decision that most determines whether the project survives
month six. A hard fork of trunk means tracking a codebase that changes daily; a
patch series against ESR means one substantial rebase roughly yearly, plus
security updates.

**Corollary: keep the patch series small.** Every line patched into
`comm-central` is a line to re-apply forever. Anything that can live in an
add-on should.

### Where each thing should live

| Mechanism | Use it for | Rebase cost |
|---|---|---|
| **MailExtension** (WebExtension API) | Anything the [41 documented namespaces](https://developer.thunderbird.net/add-ons/mailextensions/supported-webextension-api) can do — reading messages, folders, compose, menus, toolbar buttons | None |
| **Experiment API** | Privileged JS reaching into Thunderbird internals the API does not expose | Low, but breaks on internal refactors |
| **Core patch** | Only what neither can do — new database columns, sync-path hooks, UI surfaces that do not exist | High, forever |

Start every feature by asking which of the three it needs, and push it down the
list. Most of what follows is an add-on.

---

## 2. What Thunderbird already gives you

Do not rebuild these. Every one of them exists in `kuverta` and every one is
redundant here:

- IMAP sync, including CONDSTORE/QRESYNC and IDLE
- A message store, full-text search, and offline copies
- MIME parsing, compose, SMTP submission, save-to-Sent
- Accounts, OAuth2 for Gmail and Microsoft 365, the OS keychain
- Address book, filters, tagging, threading, HTML rendering, attachments
- The add-on ecosystem, which is the entire point

In `kuverta` terms that is `core-proto`, `core-store`, `core-smtp`,
`core-accounts` and most of `core-rpc` — around 11,000 of 15,500 lines. Deleting
them is the price of admission and it is the right trade *if* the add-ons are
worth it.

What has no Thunderbird equivalent, and is therefore the actual project:
classification, the triage surface, the agent interface, and the paper half.

---

## 3. What to carry across

### 3.1 The triage surface

One keyboard-first list you act from, with the classifier's categories as the
primary axis rather than folders.

- `j`/`k` move, `enter` read, `e` archive, `#` trash, `u` unread, `z` undo,
  `c` compose, `R` reply, `A` reply-all, `f` forward, `r` sync, `/` search,
  `,` settings
- **Acting keeps the cursor where it was**, so the next message slides under it
  rather than the list jumping to the top. This is most of what makes triage
  bearable and it is easy to get wrong.
- **A filtered list says what it is showing, and offers a way out.** A list
  that quietly narrows looks exactly like mail going missing.

*Where:* a MailExtension providing a tab, using `mailTabs`, `messages`,
`folders` and `menus`. No core patch needed.

> **Since (2026-09-11):** built, and built once for two hosts rather than for
> Thunderbird alone — the surface is `core/view/triage.js`, driven by
> `core/triage.js`, over the port in `core/host.js`. Thunderbird and `kuverta`
> are both adapters behind it. See [decisions.md](decisions.md) §4.

### 3.2 The deterministic classifier

Six categories — personal, newsletter, marketing, transactional, notification,
and **unknown** — scored from `List-Id`, `List-Unsubscribe`, `Precedence`,
`Auto-Submitted`, sender, subject patterns and recipient count. Bilingual
(German and English) because the mailbox is.

`Unknown` is a category rather than a fallback to the commonest guess, and that
is deliberate: *no signal strong enough to guess is better than a confident
wrong answer.* A classifier that is never unsure trains the user to distrust
all of it.

Corrections override the heuristics outright: a sender or list you have filed
once is filed that way next time.

*Carry the code.* `core-rules` is 793 lines with no dependency on anything else
in the tree, and it is the one crate here Thunderbird has no answer to. It can
be ported to JS or kept as Rust behind a native-messaging host (§3.7).

*Measured result on 251 realistic messages:* newsletter 77, transactional 62,
notification 61, personal 33, marketing 18. Note that bulk marketing carrying
`List-Id` and `Precedence: bulk` reads as a newsletter to the rules — a real
weakness, and a good first target for the model.

> **Since (2026-09-11):** these counts are exactly right, and they describe one
> mailbox rather than the generator. That mailbox is still in `kuverta`'s
> scratch store — 251 messages, 77/62/61/33/18 — and it is `fill-mailbox.py`
> output from an earlier run plus one real message from the provider. A fresh
> `--dump 250` gives 79/62/59/32/18 instead, because the draw differs; every
> sender is filed under the same category in both. So cite it as a fact about
> that mailbox, not as a baseline. The weakness the paragraph names is real and
> larger than it suggests: a quarter of those newsletters are a marketing
> fixture whose subject carries no keyword the rules know. See
> [decisions.md](decisions.md).

### 3.3 The local model, and how to avoid fooling yourself about it

The plan in `kuverta` never got to this, and its design should survive intact:

1. **Log both verdicts, always.** Run the rules baseline alongside the model and
   record where they disagree and which was right. Without this you never find
   out whether the model beat a `HashMap` of sender→folder, and you will have
   paid latency for nothing.
2. **Spike embeddings against prompting before committing.** For "which category
   is this", embedding the message and taking nearest neighbours among *your own
   past filings* is usually faster, cheaper, needs no prompt tuning, and
   improves as you correct it. Generative prompting is the obvious choice and
   often the worse one.
3. **4–8B instruct on Apple Silicon is ample.** Classification is easy.
4. **Run it after sync, not during.** A slow model must never be able to make
   sync slow.

*Where:* a native-messaging host talking to Ollama, or an add-on calling a local
HTTP endpoint. Never a core patch.

### 3.4 Corrections are training data

Every reclassification is recorded from the first day, even though nothing
consumes it until the model lands. By the time the model arrives there is a
labelled corpus of exactly the mail it will be asked about.

*Where:* add-on storage, or a table of your own. Do not rely on Thunderbird's
tags for this — you want the correction event, not the current state.

### 3.5 The safety rules for writes

These were paid for with bugs. Thunderbird's own write paths are mature, so most
apply only to code *you* add that mutates mail — but they are the rules:

- **Nothing destroys mail.** Delete means move to Trash. Never send a bare
  `EXPUNGE`: it removes every `\Deleted` message in the folder, including ones
  another client marked.
- **No UID is trusted on its own.** Before acting, check the folder still has
  the UIDVALIDITY the change was queued under, that a message is still at that
  UID, and that its `Message-ID` is the expected one. A UID is stable only in
  the sense that the server will not reissue it.
- **Queue intent before acting, not after.** A crash between the click and the
  round trip must lose nothing and repeat nothing.
- **An undo window, and it gates sending rather than cancelling.** A change stays
  cancellable for as long as it is queued.
- **Bcc reaches the envelope and never the message.** The one mistake that
  cannot be walked back once the bytes have left.

### 3.6 Things learned about real providers

Worth writing into the new repo's notes on day one:

- **Gmail's `[Gmail]/All Mail` holds a copy of everything**, so a naive sync
  fetches most mail twice. Folder exclusion is not a nicety.
- **Gmail does not accept the OAuth2 device flow** for `https://mail.google.com/`
  — it needs the loopback redirect (RFC 8252, with PKCE). Thunderbird already
  handles this; do not re-derive it.
- **Dovecot advertises `SPECIAL-USE` but not `CREATE-SPECIAL-USE`**, so a folder
  your software creates carries no attribute and must be found by name.
- **A message with no `Message-ID` is legal and does happen.** Code that treats
  "no Message-ID" as "no message" silently breaks on it.
- **A preflight check is worth more than it costs.** Connect, report credentials,
  extensions, where mail will be filed, and what a first sync would fetch —
  before downloading a mailbox.

### 3.7 Keeping the Rust, if you want to

`core-rules` (and later the model layer) can stay Rust behind a **native
messaging host**: Thunderbird add-ons may launch and speak JSON to a local
binary. That preserves the classifier, its tests, and the option of reusing it
elsewhere, at the cost of shipping a second executable and handling its
installation.

Decide this early. It is cheap at the start and awkward later.

---

## 4. New ground

### 4.1 The agent surface

The differentiator, and the reason this is not just Thunderbird with a nicer
list. Expose the mailbox over **MCP** so an assistant can read, search,
classify, draft and file — with the same safety rules as §3.5, because an agent
acting on mail needs them more than a human does, not less.

This has no Thunderbird equivalent and no add-on does it well. It is the part
worth building first if the goal is something nobody else has.

### 4.2 The paper half

Scanned post lands in the same triage surface as mail: `scannerd` on a Pi
captures, deskews and uploads; [Paperless-ngx](https://docs.paperless-ngx.com/)
does OCR, tagging and archive; the fork shows documents and mail as one item
type in one list.

Only the capture daemon is yours — roughly 600 lines — which is what makes the
paper half affordable at all.

---

## 5. What you are signing up for

Not reasons to stop; things to plan for.

**Licence.** Thunderbird is MPL-2.0. Forked files stay MPL-2.0 — you cannot
relicense Mozilla's code. Your own new files can be whatever you like. Whether
the *combined* work can ship under AGPL depends on MPL-2.0 §3.3 and on no
relevant files being marked "Incompatible With Secondary Licenses". **Check this
properly before promising a licence**, because the original plan's decision 8
assumed AGPL and that assumption no longer travels for free.

**Trademark.** MPL grants no trademark rights. The fork cannot be called
Thunderbird and must be rebranded — Betterbird carries [branding
patches](https://github.com/Betterbird/thunderbird-patches) for exactly this.
Budget for a name, icons and an About screen before first distribution.

**Build times.** `comm-central` is a full Gecko build. Expect tens of minutes
from cold on a laptop and a large object directory. This changes the shape of a
10h/week schedule more than anything else here: the fast edit-run loop you have
now does not survive the move, except inside add-ons.

**The rebase.** One substantial ESR rebase per year plus security updates, for
as long as the project lives. This is the recurring cost the soft-fork choice is
designed to bound.

**Signing and notarisation.** Distributing a macOS application means an Apple
Developer account and notarisation. Deferrable while it is only yours.

---

## 6. A staged plan

Sized for ~10h/week, and ordered so each stage is useful alone.

| Stage | Work | Gate |
|---|---|---|
| **0** | Build unmodified `comm-central` ESR and run it. Nothing else. | It builds and launches, and you know how long that takes |
| **1** | The classifier as a MailExtension, categories shown as tags. No fork yet. | Your own mail is categorised in stock Thunderbird |
| **2** | The triage tab: keyboard-first list over `mailTabs`, acting keeps the cursor | You triage from it daily |
| **3** | Corrections recorded, and the rules learning from them | Filing once changes what happens next time |
| **4** | Rebrand and cut the first real build — the fork begins here | You run your own build |
| **5** | The model, with both verdicts logged from the first message | You can say whether it beats the rules, with numbers |
| **6** | MCP surface | An assistant can work your mail safely |
| **7** | Paper: `scannerd` + Paperless-ngx, unified list | Post is searchable beside mail |

**Stages 1–3 need no fork at all.** That is deliberate: they are the whole idea,
they run on stock Thunderbird, and if the fork never happens they still work. Do
not pay the fork's costs until stage 4, and only pay them for something an
add-on genuinely cannot do.

---

## 7. Decisions still open

1. **Does the fork ever actually happen?** If stages 1–3 deliver the idea as a
   plain add-on, the honest answer may be no — and that would be a good
   outcome, not a failure.
2. ~~**Rust behind native messaging, or port the classifier to JS?** (§3.7)~~
   **Closed 2026-09-11: ported to JS.** Stages 1–3 are supposed to run on stock
   Thunderbird so they still stand if the fork never happens, and a second
   executable to install breaks exactly that. The port is verified against the
   Rust line for line — see [decisions.md](decisions.md).
3. **Embeddings or prompting** for classification — spike both, briefly. (§3.3)
4. **Licence**, once §5 has been checked properly.
5. **Name**, needed before stage 4.
6. ~~**What to do with `kuverta`.** It syncs, sends, triages and has 171
   passing tests. Options: archive it as a reference, keep it as the
   native-messaging backend, or keep running it alongside — it and Thunderbird
   coexist on the same IMAP account today.~~
   **Closed 2026-09-11: neither.** There was a fourth answer the list missed.
   `kuverta` is a *host*, and so is Thunderbird; the product is the triage
   surface and the classifier, which now run on both from one core behind a
   tested contract. That also takes the weight out of decision 1 above: whether
   the fork happens stops being load-bearing when the thing that matters runs
   either way. See [decisions.md](decisions.md) §4.

---

## 8. Worth reading alongside this

- [`readme.md`](../readme.md) — the design decisions section in particular
- [`docs/implementation-plan.md`](implementation-plan.md) — §1a on the two
  stages of write capability, §4 on the classifier, §5 on risks
- [`crates/core-rules/`](../crates/core-rules/) — the classifier, portable as-is
- [`crates/core-proto/src/mutate.rs`](../crates/core-proto/src/mutate.rs) — the
  conflict checks of §3.5, with the reasoning in the comments
- [`docker/fill-mailbox.py`](../docker/fill-mailbox.py) — fills a test mailbox
  with a few hundred realistic messages; works against any IMAP server and is
  worth bringing across on day one

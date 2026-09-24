# Decisions

What was decided, when, and why — in the order it was decided. The brief's §7
lists six open questions; this records the ones that have closed and what
closing them cost.

---

## 1. The classifier is ported to JS, not kept in Rust behind native messaging

**2026-09-11.** Closes brief §7.2.

The brief left this open and said to decide it early, "cheap at the start and
awkward later". The brief's own staging decides it: stages 1–3 are meant to run
on stock Thunderbird so that "if the fork never happens they still work". A
native-messaging host means shipping a second executable and owning its
installation, which is precisely the thing that stops an add-on from being
something you can hand to someone.

So `core-rules` is now `extension/src/{category,signals,classify}.js`, and it is
a plain ES module with no Thunderbird API in it — which is what lets the tests
run under `node --test` with no mail client in the loop.

**What this costs.** Two implementations of the same rules now exist. They will
drift unless something checks, which is what §2 below is for.

**What it keeps open.** The classifier is a pure function over headers. Moving
it back behind a native-messaging host later is a change of transport, not of
design. The rule ids (`header.list_id`, `subject.marketing`, …) are deliberately
identical to the Rust ones, so a corrections log collected under one is still
readable by the other.

### The port is verified, not assumed

The twelve tests in `core-rules/tests/classify.rs` were carried over one for
one. That is necessary and not sufficient — identical assertions pass for two
implementations that disagree everywhere the assertions do not look.

So both were run over the same 250 messages and diffed:

```
python3 docker/fill-mailbox.py --dump 250 > corpus.jsonl
node kuverta-bird/tools/dump-facts.js < corpus.jsonl > facts.jsonl
# and the same facts through core-rules
diff rust.tsv js.tsv
```

Identical on all 250 — the same category *and* the same confidence to six
decimal places. The facts are built once, in JS, and fed to both, so this
measures the classifier and not the header parsing.

Worth re-running after any change to `kuverta-bird/core/signals.js`. The harness for the Rust
half is throwaway (a few lines depending on `core-rules` by path); it is not
kept in this repository because keeping it would mean keeping a Rust toolchain
in the loop for a JS add-on.

---

## 2. The brief's measured distribution does not reproduce

**2026-09-11.** Amends brief §3.2.

The brief reports, on 251 messages: newsletter 77, transactional 62,
notification 61, personal 33, marketing 18.

`core-rules` itself, over the 250 messages `fill-mailbox.py --dump` produces
today, gives: **newsletter 79, transactional 62, notification 59, personal 32,
marketing 18**.

Transactional and marketing match exactly; newsletter and notification differ by
two in opposite directions. That cannot come from the one-message difference in
corpus size, and it does not come from the port — the Rust and the JS agree
message for message. Every sender in the corpus classifies with a clear margin,
so no message is near enough to a boundary to flip. The corpus behind the
quoted figures was therefore a different one.

Nothing is broken. The number in the brief is just not a baseline, and should
not be cited as one. The reproducible command is:

```
python3 docker/fill-mailbox.py --dump 250 | node kuverta-bird/tools/classify-corpus.js --reasons
```

### Resolved: the corpus was real, and it is still on disk

**Later the same day.** The mailbox behind the brief's figures turned up in
`kuverta`'s scratch store, as a second account with exactly 251 messages.
Running the store's own query over it gives, exactly:

```
newsletter 77 · transactional 62 · notification 61 · personal 33 · marketing 18
```

So the brief's number was a correct measurement and not a mis-transcription.
What it measured was one mailbox: `fill-mailbox.py` output uploaded over IMAP
by an earlier run, plus one real message from the provider — a
`no-reply@systemli.org` notice, which is the 251st.

The two-message difference is corpus composition and nothing else. Grouping
that mailbox by sender and comparing it with `--dump 250`:

| sender | in the brief's mailbox | in `--dump 250` |
|---|---|---|
| angebote@mediamarkt | 25 | 27 |
| sec@news.heise | 21 | 22 |
| hello@news.rustweekly | 19 | 18 |
| newsletter@golem | 12 | 12 |

**Every sender is filed under the same category in both.** The classifier
agrees with itself; a different random draw produced two fewer newsletters and
two more notifications. Nothing to fix.

What stands from the original entry is the advice, for a clearer reason than it
gave: the figure describes one particular mailbox, not what the generator
produces, so it is a fact about that mailbox rather than a baseline to measure
against. `npm run corpus` is the reproducible one.

### The weakness the brief names is bigger than it says

§3.2 notes that "bulk marketing carrying `List-Id` and `Precedence: bulk` reads
as a newsletter to the rules". On this corpus that is 27 of the 79 newsletters —
a third of the category, all from one sender whose subject line ("Nur heute: 20%
auf alles") contains no keyword the rules know. `--reasons` shows why: every one
of those 79 verdicts rests on `header.list_id` and `header.precedence` and
nothing else.

This is the best-evidenced target for the model of §3.3, and it is also the
strongest argument for the corrections log: one correction on that sender fixes
27 messages, which no amount of keyword tuning would.

---

## 3. MV2, not MV3

**2026-09-11.** Not in the brief.

The add-on is Manifest V2 with a background *page*. The reason is narrow: a
background page loads ES modules, which is what lets `extension/src/` be shared
verbatim between Thunderbird and the Node tests. Under MV3's background scripts
that sharing needs a build step, and a build step between the tested code and
the shipped code is exactly the seam the port verification above exists to
close.

Thunderbird 128 and 140 ESR both support MV2. This will have to move, and when
it does the thing to preserve is that the tests run the same files the add-on
does.

---

## 4. One core, two hosts, and a contract between them

**2026-09-11.** Not in the brief, and it changes the brief's shape.

The brief assumes a line of succession: `kuverta` is the thing that exists,
Thunderbird is the thing to move to, and §7.6 asks what to do with the old repo
— "archive it as a reference, keep it as the native-messaging backend, or keep
running it alongside".

There is a fourth answer, and it is better: **neither is the product**. The
product is the triage surface and the classifier. Both are hosts.

So the layout is:

```
core/     the classifier, the triage model, the key map, the surface
          — no Thunderbird, no Tauri, no DOM in the model
hosts/thunderbird/   an adapter over the MailExtension API
hosts/kuverta/      an adapter over the client's Tauri commands
```

`kuverta-bird/core/host.js` is the seam. Message ids are opaque, folders are opaque, and
anything one host can do that the other cannot is **declared** in a
`capabilities` object rather than sniffed for. A core that starts asking "am I
in Thunderbird?" has stopped being a core, and `kuverta-bird/test/wiring.test.js` fails if
it ever does — it greps for `messenger.`, `__TAURI__` and `browser.` in
`core/`, and for imports reaching from one host into the other.

**What this buys.** The §7.1 question — does the fork ever happen — stops being
load-bearing. If it never does, the surface still runs in both. If it does, the
fork inherits a surface that already works.

**What it costs.** A contract to keep, and the discipline not to reach through
it. Both are cheaper than two implementations of the same list, which is what
existed before this: `kuverta`'s `apps/desktop/ui/app.js` is 1,136 lines and
most of it is the list, the cursor and the key map written once already.

### The contract is a test, not a document

`kuverta-bird/test/support/conformance.js` is the host contract as a suite. Three adapters
run it: Thunderbird against a fake `messenger`, `kuverta` against a fake
`invoke`, and an in-memory reference that exists to prove the contract is
satisfiable at all — a contract no host can pass is a bug in the contract, and
without a reference there is nothing to notice that with.

Two things the contract caught that a document would not have:

**Message ids do not survive a move.** Thunderbird issues a new id for a moved
message, and archive and trash are both moves. The first draft of the contract
required a trashed message to still be readable by its id, which no Thunderbird
adapter could ever satisfy. The contract now says ids are good until the
message is acted on, the surface reloads after every action rather than
patching the row it holds, and the Thunderbird adapter finds a moved message
again by its `Message-ID` in order to undo — and refuses, out loud, when the
message has none, because §3.6 says that is legal and moving the wrong message
back is worse than refusing.

**A capability read too early reads as false.** `kuverta`'s host has to ask
which folders are Archive and Trash before it can say whether it can archive.
The suite read `capabilities` synchronously, got `false`, and silently skipped
its archive and trash tests — passing, green, testing nothing. The suite now
awaits the host factory, and every capability gate reports a **skip** with a
reason rather than a pass, so a capability regression cannot hide as an `ok`.

### The two undos are different promises

`capabilities.undo` is `'queued' | 'compensating' | false`, never `true`.

`kuverta` queues a change before sending it, so undo **cancels** something
that never happened — brief §3.5, "a change stays cancellable for as long as it
is queued". Thunderbird's write paths do not queue, so by the time undo is
offered the move has happened, another client may have seen it, and undo is a
second change that **puts it back**. The surface uses those words, in that
order of strength. Collapsing them into one boolean would have made the weaker
one look like the stronger.

---

## 5. `kuverta` cannot file by category, and the surface says so

**2026-09-11.** A gap found by building the adapter.

`core-rpc` registers twenty commands. None of them sets a category: the
classifier runs during sync and nothing exposes a correction. So the
`kuverta` host declares `setCategory: false`, the surface hides the category
keys there, and three conformance tests report as skipped rather than passing.

This is the capability object earning its keep on the first day it exists. The
alternative — an adapter that accepts the call and does nothing — is the worst
outcome available: the key is shown, the user presses it, and nothing happens
with no way to tell why. The contract has a test for exactly that
("a capability it does not have is refused, not quietly ignored").

[`kuverta-bird/hosts/kuverta/readme.md`](../kuverta-bird/hosts/kuverta/readme.md) has the command that closes it. Once it exists, one
boolean changes here and the keys appear.

---

## 6. What a documentation pass found before the first load

**2026-09-11.** Checked against the Manifest V2 API reference and the
MailExtensions supported-API list.

The previous entries all ended with the same caveat: the adapters are written
from the documented API and tested against fakes, and a fake answers whatever
it is asked. So before loading the add-on for the first time, every API call
the Thunderbird adapter makes — twenty-five of them — was checked against the
documentation. This is what that turned up.

### Four missing permissions

Each would have failed at runtime, and three of them silently enough to be
annoying to find.

| Missing | Needed by | What would have happened |
|---|---|---|
| `messagesMove` | `messages.move` | archive and trash fail — the two most-used keys |
| `messagesTagsList` | `messages.tags.list` | `ensureTags` throws on startup, so no tags are ever created |
| `compose` | `compose.beginNew` | `c` does nothing |
| `tabs` | `tabs.query({url})` | the query returns nothing, so a *second* triage tab opens every time |

`messagesTagsList` is the instructive one. `tags.create` needs `messagesTags`
and `tags.list` needs `messagesTagsList`; they are two permissions with almost
the same name guarding two halves of the same job, and having one made it look
as though the pair was covered.

`tabs` is the instructive failure mode: without it the call does not throw, it
returns an empty list. The code would have concluded no triage tab was open and
cheerfully opened another one on every click.

### Three assumptions that were wrong

**`MessageHeader` carries no attachment information.** `toRow` was reading
`header.attachments?.length` and `header.hasAttachments`; neither exists in any
version. The row's paperclip was always going to be false. Knowing properly
costs either a `listAttachments` per message — thousands of round trips for an
icon — or a second `query({attachment: true})` per scope, which doubles the
cost of the operation already named as this adapter's scaling limit. So it
stays false, with a comment saying why, and the list does not show a paperclip
in Thunderbird. The classifier is unaffected: it reads attachments off the part
tree when a message is opened, which is a path that does have the answer.

**Header keys were being matched by exact spelling.** `getFull` promises "the
header name as key" and Thunderbird lowercases in practice, but the lookup
asked for `list-id` and would have found nothing had the key been `List-Id` —
and a missing header does not fail, it silently switches off a signal.
`firstHeader` now matches case-insensitively.

**`menus.onClicked`'s `selectedFolder` is deprecated**, replaced by
`selectedFolders` in TB 128 — which is the minimum version this targets. Both
are read now.

### Three assumptions that were right

Worth recording, because they were guesses and are no longer:

- `QueryInfo.tags` really is `{mode: 'all' | 'any' | 'none', tags: {key: boolean}}`.
- `messages.query` resolves to a `MessageList` with `id` and `messages`, and
  `folderId` (TB 121) and `headerMessageId` (TB 85) are both accepted — both
  older than the 128 minimum.
- **Message ids do not survive a move.** The docs say an id "does not follow an
  email that has been moved to a different folder", which is what §4 above
  designed the contract around, from the Thunderbird behaviour rather than from
  a document. Good to have the document agree.

### The table is now a test

`kuverta-bird/test/permissions.test.js` holds the API-to-permission table and checks three
things: that the manifest grants everything the code needs, that it grants
nothing it does not — a mail client asking for more than it uses is asking to
be trusted for no reason — and that every `messenger.*` call the code makes has
an entry in the table at all, so using a new API forces the table to be
updated rather than letting it rot into a snapshot of what was once true.

It is the assumption made explicit, not a second source of truth. If
Thunderbird disagrees with it, Thunderbird is right and the table is what gets
corrected.

---

## 7. Stage 3 was already built in `kuverta`, and unreachable

**2026-09-11.** Supersedes §5.

§5 recorded that `kuverta` could not file a message by category and declared
`setCategory: false`. That was right about the symptom and wrong about the
cause, which turned out to be more interesting.

The machinery was all there. `correction` as a table, `record_correction`,
`learned_categories`, and — at the top of `sync_account` — `load_history`
folding those corrections into a `core_rules::Learned` and handing it to the
`Classifier`. Tested end to end, including against the dev IMAP server.

And `record_correction` was called by nothing except tests. No CLI command, no
RPC method, no path from any interface. Stage 3's gate — "filing once changes
what happens next time" — was satisfied by the code and unreachable by a user.

This is what building the second host was for. Nothing about writing the
Thunderbird adapter would have found it; it took writing an adapter for
`kuverta` against a contract that asks a host what it can do, and having to
answer "not that".

### What the exposure cost

Not just a command, because of where the list reads a category from.

**`ClassifierSource::User`.** The list joined `source = 'rules'`, so a
correction stored as a user decision would not have shown until the next sync
re-classified. The cheap fix was to record corrections as rules verdicts, one
line shorter and quietly corrosive: the rules-versus-model comparison of brief
§3.3 reads the `rules` rows, and filling them with the user's answers would
make the baseline unbeatable and meaningless. So corrections are a third
source, and `disagreements` still reads `rules` only.

**`CURRENT_CATEGORY_JOIN`.** One definition of "the category a message is shown
under" — the user's if there is one, else the most recent rules verdict, and
never the model's — shared by the three queries that have to agree on it. A
model that started changing what the list showed would take §3.3's measurement
with it, so it is excluded here by the same rule that includes corrections.

**A counting bug, found on the way past.** `category_counts` joined every
classification row for a message. `COUNT(DISTINCT m.id)` stopped that inflating
a single category, but a message reclassified between syncs was counted under
*every* category it had ever been given — so the sidebar could add up to more
messages than the mailbox held. The shared join fixes it by construction, and
there is now a test that the totals equal the message count.

### What it bought

All three adapters now declare every capability, which left the contract's
"a capability it does not have is refused, not quietly ignored" test running
against nothing at all, and the skip machinery itself untested. So there is now
a fourth reference host, `ReadOnlyMailbox`, which declares almost nothing and
refuses what it declared it could not do. Most of its conformance run is skips,
and that is the point: something has to exercise the path a future host will
take, and "an account with no Archive folder" is not a hypothetical.

---

## 8. Keyboard-first is not the same thing as undiscoverable

**2026-09-11.** The first time anyone opened the triage surface.

The report was: *"I opened triage but as a user I actually have no clue what I
am supposed to do."*

The list was working — eight messages, all classified, five categories. It
simply said nothing. Every action was a key, no key was written down anywhere,
and the only hint was a `?` that you had to already know about.

That is a misreading of the brief, and mine. §3.1 asks for a list you act on
from the keyboard; it does not ask for a window that withholds what the keys
are. The two rules in that section are not in tension — "a filtered list says
what it is showing" is the same instinct applied to state, and the surface was
failing that one too, from the other side: `drawScope` *hid* the scope bar
unless the list was filtered, so an unfiltered list announced nothing at all.
A list that only explains itself once it has been narrowed leaves you working
out where you are from the rows.

### What changed

- **A legend along the bottom, always on screen.** `j k` move, `↵` read, `e`
  archive, `#` trash, `1-6` file, `u` unread, `z` undo, `/` search, `?` more.
  `?` is now for the rest rather than for the basics.
- **The scope bar is always shown**, saying what the list holds and how many.
  The way out still appears only when there is something to get out of.
- **The empty list says why it is empty** and what to do — silence there is
  indistinguishable from mail having gone missing, which is the one thing a
  mail client may never look like.
- **The reading pane says `Press ↵ to read this message`** when a row is
  selected. A highlighted row beside a pane reading "select a message" looks
  like the selection did not take.
- **The surface says which mailbox it is.** `mountTriage` takes a `title` the
  host supplies, because the core has no idea which account it is looking at
  and the person reading it very much does.
- **An account picker**, in the `kuverta` mount rather than in `core/` —
  which account is a `kuverta` idea, not a mail idea. The store held two
  accounts and the surface silently opened the first, which was the
  eight-message test one.

### And a bug the picker exposed

`mountTriage` returned a teardown that removed only the key handler. Mounting
twice — which an account switcher does — would have left the previous surface
subscribed to its model and listening on `window`, and **every keystroke would
have acted twice**: once on a list nobody could see. With `e` and `#` bound to
archive and trash, that is a message disappearing that the user never looked
at. The teardown now gives back the resize listener and the subscription too.

Worth noting how that was found. It was not found by using the account picker;
it was found by writing one and asking what the old mount was still holding.
The first user test bought a design fix, and the design fix bought a data-loss
bug — neither of which any of the 181 tests was going to raise, because none of
them can open a window.

---

## 9. The surface has to say what it is for, not just how to drive it

**2026-09-11.** The second thing the first user said.

Once the keys were on screen: *"I think it's working, but it should be
explained what triage is actually doing."*

Which is a different complaint from the first one and a better one. §8 fixed
*how to drive it*. This is *what it is* — six words down the side that nobody
had defined, a list sorted by something the window never named, and no
indication anywhere that pressing `1` teaches it something rather than just
relabelling one message.

The explanation existed. It was in a readme, in a different repository, which
is the one place the question never gets asked.

### What the panel says

Shown the first time the surface opens, and on `?` afterwards. Three things,
in this order:

1. **What it does** — every message is read for a handful of headers and filed
   under one of six categories; nothing moves, no folder changes, the category
   is a label and the list is sorted by it rather than by where mail happens to
   live.
2. **What the six mean**, each in a line. `CATEGORY_MEANINGS` now sits beside
   the doc comments in `kuverta-bird/core/category.js`: the comments explain the taxonomy to
   whoever reads the source, and these explain it in the window, which is where
   it is actually asked.
3. **That correcting it teaches it** — that `1`–`6` remembers the sender or the
   list, so the next message like it is filed the same way unasked. This is the
   part with no other way of being discovered: nothing about pressing a key
   suggests the key has a memory.

`localStorage` remembers that it has been seen, wrapped in try/catch both ways
— storage can be unavailable or full, and failing to record a dismissal is not
worth taking the surface down for. It just means the panel opens again.

### And the per-message half, which the brief already asked for

§3.2 says the rules layer earns its keep by being able to say *why*, "which is
what makes a wrong answer correctable instead of infuriating". The Thunderbird
message-header popup did that from the first commit. The shared triage surface
did not do it at all — the one place you would actually be looking when you
disagreed with a verdict.

The reading pane now carries it: the category, and the signals that produced
it. The reasons are recomputed from the facts the host hands back rather than
stored, because they are a pure function of those facts and a cached
explanation can go stale against the verdict it explains.

It is deliberately computed **without** the corrections history, so that when
the filed category and the rules disagree the surface can say so: *"You filed
this as Transactional. On the headers alone the rules would have said
Marketing."* That sentence is only available because the two were kept apart.

Where a host cannot supply facts — `kuverta`'s `message` command returns a
body and no headers — it shows the category and what that category means, and
stops. A weaker answer to the same question beats inventing reasons.

---

## 10. One repository

**2026-09-11.** Reverses the split that §4 was written inside.

`kuverta-bird` was a separate repository for about a day. Looking at the two side by
side, the reaction was the obvious one: why are there two of these?

There was no good answer. §4 had already concluded that neither the Rust client
nor Thunderbird is the product — the triage surface and the classifier are, and
both are hosts. A repository boundary between the surface and one of its two
hosts does not express that; it just makes the surface look like a separate
project that happens to have an adapter.

The evidence it was wrong was already sitting in both trees. The brief existed
twice, in two states, with the second copy's links rewritten to stop pretending
they resolved. The install script existed to carry files across a boundary that
had no reason to be there. And the fix in §7 — exposing corrections — touched
the Rust store, the Rust RPC, the Tauri command *and* the JavaScript adapter,
which is one change described in two commit messages in two histories.

So `kuverta-bird` is now a directory, merged with `git subtree` rather than copied,
so its seven commits and their reasoning are still in the log and interleaved
with the Rust ones in the order things actually happened.

### What that tidied

- **One brief.** The annotated copy survives as
  `docs/thunderbird-fork-brief.md`; the links that had been flattened to code
  spans are links again, because now they resolve.
- **One decisions log**, this file, covering both halves. §7 is about the Rust
  store and §8 about a JavaScript view, and they belong in one place because
  they are the same project.
- **`make test-js`**, and `make test` now runs both suites. Two test commands in
  two directories was another way of saying two projects.

### What stays copied, and why

`make triage-ui` still copies the surface into `apps/desktop/ui/`. That is not
left over from the split: Tauri serves exactly one directory as the web root,
and nothing outside it is reachable from the page. The copy is verbatim and
verified with `diff -r`, so it stays a copy rather than becoming a build step —
which is the same line the Thunderbird side walks by putting the manifest where
`core/` is already inside the extension root.

### What this does not change

The contract. `kuverta-bird/core/` still may not name a host, and
`kuverta-bird/test/wiring.test.js` still fails if it does. Sharing a repository with
one of the two hosts is exactly the circumstance in which that rule stops being
obvious and starts being load-bearing: the Rust client is now a directory away,
and reaching into it would work.

---

## 11. A physical address is an account

**2026-09-11.** Brief §4.2, and a reframing of it.

The brief describes the paper half as a pipeline: `scannerd` captures,
Paperless-ngx does OCR and archiving, and "the fork shows documents and mail as
one item type in one list". True, but it describes the plumbing and leaves the
model unsaid. The model is simpler than the pipeline:

**A physical address is an account, and Paperless is its server.**

Post that arrives at it is mail. The correspondent wrote it, the title is the
subject, the OCR text is the body, the date on the letter is when it arrived.
Which makes `core-paper` the exact counterpart of `core-proto`: a client for a
protocol that happens to be REST instead of IMAP, for a mailbox that happens to
be made of paper.

### What that framing buys immediately

**The classifier needs no teaching.** `Document::facts()` returns the same
`MessageFacts` the IMAP path builds, so post is filed by the rules that file
mail — no paper-specific classifier and no second taxonomy. An invoice is
transactional whether it arrived as a PDF attachment or through a letterbox,
and nothing had to be written twice to make that true.

> **Correction (2026-09-12):** an earlier version of this entry said a
> correction made on a letter teaches the same table a correction on an email
> does. That is the intent and it is not yet true. Corrections made on *mail*
> do reach post — `paper_classifier` loads every account's learned overrides,
> so a correspondent already filed once on the mail side is filed the same way
> on paper. The reverse does not work: `correction` hangs off a message row and
> a document is not one, so filing post by hand needs a table of its own. Until
> it has one the surface refuses, out loud, rather than appearing to accept.

**Several addresses, one instance.** A selector — a Paperless tag,
correspondent or storage path — is what separates a home from an office. It is
a Paperless concept rather than one invented here, so an address is set up by
tagging in Paperless rather than by configuring the same thing in two places.

### Two mappings worth arguing about

**The correspondent doubles as the address.** Paper has no `From` to key on,
and the learned overrides need something stable to remember a correction
against. "Everything from the Stadtwerke is transactional" is exactly the rule
a person wants to teach, and the correspondent is what carries it.

**A document always counts as carrying an attachment**, because it is one.
There is a PDF, and a scanned invoice is no less an invoice than an emailed one.

### The client is in Rust, not in the shared surface

The triage surface is JavaScript and reaches its host through a port, so a
Paperless client in `kuverta-bird/core/` would have been the obvious symmetry. Two
things say otherwise. The desktop app's CSP is `default-src 'self'`, so its
webview cannot reach `localhost:8000` at all — the request has to go through
Rust regardless. And `core-rpc` is already the place "both front ends talk to",
built Tauri-free on purpose so it can double as the MCP surface: an agent
asking about post should ask the same layer a window does.

### What is not done

Post does not yet appear in the triage window beside mail, which is the brief's
actual promise. What exists is the client, the classification, and a CLI that
reads an address the way `kuverta list` reads a mailbox — verifiable against
the dev stack today. The shapes already match, which is what makes the merged
list a display problem rather than a modelling one.

The other open piece is where an address's configuration lives. The CLI takes
it as arguments and environment; the window will need it stored, which means a
table and a keychain entry for the token, alongside the accounts.

---

## 12. A row may allow less than its host

**2026-09-12.** Found by the host contract, the moment post joined the list.

`capabilities` answers "what can this host do". That was enough while every
row in a list was the same kind of thing. It stopped being enough the moment
post from Paperless sat beside mail: the host can archive, and that particular
row cannot be archived, because Paperless owns the document and this only reads
it.

The contract caught it immediately and in the right way. `undo unwinds one
change at a time` archives the first two rows of the list; one of them was now
a letter, and the adapter refused. The test was not wrong — the port was. It
had no way to express a row that allows less than its host.

So `Row.actions` is an optional list of what a row permits. Absent means
"whatever the host can do", which keeps every adapter written before this
correct without a line changing. Documents declare `[]`.

Three things follow, and all three are the point:

- **The model checks before it asks.** `Triage.act` refuses a disallowed row
  itself rather than making a round trip to be told no, so the answer is
  immediate and names the row rather than the host: *"this cannot be archived
  from here"*, not *"this host cannot archive"*.
- **The contract picks a row that allows the action** instead of whatever
  sorted first, and gained a rule of its own: a row that declares it allows
  nothing must refuse loudly rather than quietly accept.
- **The adapter says no in one place.** Paper ids are namespaced
  (`paper:<address>:<document>`), so one `parsePaperId` guard at the top of
  each write is the whole of it.

This is the second time the contract has paid for itself by being wrong in a
useful way — the first was message ids not surviving a move (§4). Both times
the fix was in the port, both times it was found by adding a host rather than
by thinking harder about the port, and both times a test that assumed too much
was the thing that said so.

---

## 13. `scannerd` is small because it refuses to do most of the job

**2026-09-12.** Brief §4.2, and the plan's month 6–7.

The plan lists the capture daemon as "page detect → capture → deskew → crop →
upload with offline spooling" and budgets ~600 lines. Three of those five steps
turned out not to be this program's work at all, and saying why is more useful
than the code.

**Deskew is `ocrmypdf`'s.** Paperless runs it on every document. A photograph of
a letter on a desk is never square, and straightening it here would be writing —
and then maintaining, and then failing to test — something that already happens
downstream. The dev stack now sets `PAPERLESS_OCR_DESKEW` explicitly rather than
inheriting a default, so the dependency is visible to whoever changes it next.

**Cropping is a setting.** A capture rig has the camera fixed above a fixed
spot, so the page is always in the same part of the frame. `rpicam-still --roi`
crops on the sensor. That turns a page of perspective geometry into one line of
configuration — and geometry that cannot be tested against real photographs is
geometry that is guessed at, which was the deciding argument.

**Page detection is arithmetic, not vision.** `--encoding yuv420` hands back raw
YUV whose first plane *is* the greyscale image, so there is no JPEG to decode
and no image library in the binary. "Is there a page" becomes "what fraction of
pixels differ from the empty desk by more than a threshold".

That last one is why the interesting part is testable at all. The detector is
driven by synthetic frames on a laptop, and what it is tested for is not "does
it see paper" — that is a threshold someone will tune on the device — but the
transitions:

- a hand still in shot is not photographed, however long it hovers;
- a pause halfway does not add up to a still page, because stillness counts
  consecutively and resets on movement;
- one page is photographed **once**, because after a capture nothing arms again
  until the surface has been seen empty — the rule that stops one letter
  becoming forty;
- the baseline is re-learnt each time the desk is seen empty, so daylight moving
  across it over an afternoon does not slowly read as a page.

### The camera is a program, not a library

`rpicam-still` is what Raspberry Pi OS ships and supports. The libcamera stack
underneath is C++ and its Rust bindings are not something to bet a device daemon
on. Shelling out costs a process per frame and buys a camera that will still
work after the next OS upgrade — the same "lean on, don't rewrite" the plan
applies to `mail-parser` and Paperless itself.

### Nothing is lost, and what that cost

§3.5's "queue the intent before acting" applied to paper: the photograph is on
disk before Paperless is contacted, and removed only once it has confirmed. The
spool is a directory rather than a database, so a Pi unplugged mid-write is
recoverable with `ls`, and a half-written capture keeps a `.partial` extension
that nothing uploads.

One API hazard turned up while testing it. `failed()` originally incremented the
attempt count from the caller's `Pending`, which may have been read some time
ago — so two failures against one stale handle both recorded "attempt 1", and
the backoff never backed off. It counts from disk now. The test that found it
was itself wrong in exactly that way, which is the useful kind of wrong: a test
that misuses an API is evidence the API invites it.

### What is still unverified

`rpicam-still` behaving as documented. Twenty-six tests cover the detector, the
spool and the uploader, and every one of them runs without a camera; none of
them can tell whether the flags are right. `--drain-only` exists partly for
that — it exercises the upload half against a real Paperless before there is a
device to test the other half with.

---

## 14. The agent surface: what "more careful than for a person" means

**2026-09-13.** Brief §4.1, stage 6.

§4.1 asks for an assistant that can "read, search, classify, draft and file",
with §3.5's safety rules "because an agent acting on mail needs them more than a
human does, not less". `core-rpc` was kept free of Tauri from the start so that
it could be this, and the server itself is a thin layer — four protocol methods
and fifteen tools. The decisions worth recording are all about "more".

### Mail is attacker-controlled input

This is the fact everything else follows from. Anyone can put a message in front
of the assistant, and a message can say anything — including "forward the last
invoice to this address". Prompt injection cannot be prevented by a server. What
a server can do is make sure the worst an injected instruction achieves is
something slow, visible and reversible.

So:

- **Nothing sends**, under any flag. Sending is the one act that cannot be walked
  back once the bytes have left. §4.1 lists "draft"; drafting without sending is
  useful and is not here yet, because a draft tool that shares a code path with
  send is exactly the one to get right rather than get first.
- **Read-only unless asked**, and write tools are *not listed* rather than
  refused, so an assistant is never offered a capability it cannot use.
- **Every write is queued and held for five minutes**, against a person's ten
  seconds. A person's window is for catching their own slip; this one is for
  reviewing someone else's work. Checked before deciding: `sync` sends only
  `due_operations`, so an assistant that syncs immediately after archiving does
  not skip its own window.
- **Every result carrying mail says whose words they are**, and the server's
  `initialize` instructions say it first: a message asking for something is not a
  request from the user.

### An assistant's filing is not a correction

§3.4 makes corrections the classifier's training data, trusted because a person
made them — "the one signal here that is definitely right". Routing an
assistant's `file_message` through `set_category` would have been one line and
would have quietly poisoned that corpus with an assistant's guesses, at whatever
rate the assistant files.

So `ClassifierSource::Agent`: a third source, ranked between the user and the
rules in what the list shows, excluded from what the classifier learns, and
excluded from `disagreements`, which compares rules against the model and
nothing else.

That exposed a second, subtler bug before it shipped. `set_category` skipped
recording a correction when the message was "already filed there" — and once an
assistant has filed a message, it *is* shown there. A user agreeing with the
assistant would have recorded nothing, dropping the one filing that should be
learned from. The skip now asks whether *the user* has already filed it
(`Store::user_category`), and there is a test for exactly the agree-with-the-
assistant case.

### Small things that were one place, not two

`special_folders` moved from the desktop app, which was reaching into
`core-proto` for it, into `core-rpc`; and `default_data_dir` with it. Two shells
deciding separately where "Archive" is, or which store to open, would be two
places to disagree — and an assistant looking at a different mailbox from the
one on screen is worse than none.

### What is not verified

A real MCP client. The server is tested in-process against a real store and by
spawning the binary over stdio to check stdout carries replies and nothing else,
but no client has connected to it yet. Registering it with one is a change to a
user's own configuration, which is theirs to make; `apps/mcp/readme.md` has the
snippet.

---

## 15. The local model, measured before it is trusted

**2026-09-13.** Brief stage 5; plan §4.

The plan's warning is that the risk of a local model is not that it fails — it
is never finding out whether it beat a `HashMap` of sender to folder, and paying
its latency for nothing. Brief §3.3 turns that into four rules. What each became:

1. **Log both verdicts.** A model's verdict is recorded as `source = 'model'`,
   keyed by the model's name, beside the rules' verdict for the same message.
   `kuverta disagreements` lists where they differ. Model verdicts are never
   shown — `CURRENT_CATEGORY_JOIN` has excluded them since §7 — so a model that
   is wrong for a month moves nobody's mail.
2. **Spike embeddings against prompting before committing.** Both exist:
   `PromptClassifier` asks a chat model for one word, `Neighbours` embeds a
   message and takes the nearest of the user's own filings. `kuverta eval`
   scores both, and the rules, on the same held-out messages.
3. **4–8B instruct is ample.** The model on this machine is an 8B Llama; the
   results below are its.
4. **After sync, not during.** The model pass is its own command and its own
   call, `Core::model_pass`, so a slow model can never make a sync slow. It stops
   after three failures in a row rather than grinding through a mailbox against
   a server that is down.

### A labelled corpus, for free

`fill-mailbox.py` draws every message from a named bucket, so it already knows
the answer. The dump now carries it as `label`, beside the facts and never among
them. The side channel is deliberate: labelling adds no draw to the random
sequence, and the corpus was checked to be the same 250 messages in the same
order as before.

One label overrules its bucket. Parcel-delivered mail sits in the generator's
transactional list, but the taxonomy defines notification as "machine-generated
status: builds, alerts, delivery notices". Scoring against the bucket name would
mark a classifier wrong for being right, so the 23 DHL messages are labelled
`notification`.

### The sender's words are fenced

A prompt carries mail, and mail is written by whoever sent it. The system prompt
says the message is data and never instructions; the message sits inside a
`<message>` fence that a subject cannot close, because the fence's own tags are
neutralised wherever they appear in content. The worst an injected instruction
can achieve is a verdict nobody is shown.

### Scoring honestly

Four choices, each of which would otherwise have produced a number that was
easier to like than to believe:

- **Every method is scored on the same held-out messages.** None is marked on
  mail it learned from.
- **The first model call is not timed.** It loads the model — 25 seconds for this
  8B model — and averaging a cold start into a hundred warm answers says nothing
  true about either.
- **A reply that names no single category is not guessed at.** "Not personal,
  it is marketing" names two, and a parser that picked one would be inventing
  accuracy the model does not have. Those are counted separately.
- **Two splits, because one would flatter embeddings.** Holding out every other
  message leaves most scored messages with a near-identical sibling from the same
  sender among the filings — realistic for a mailbox in use, and an upper bound
  on a generated corpus whose senders repeat templates. `--split sender` holds
  whole senders out within each category, so every scored message is from a
  sender never filed. The rules and prompting learn nothing from filings, so
  only embeddings move between the two.

### Results

Dolphin3 8B for prompting, `nomic-embed-text` with the five nearest filings for
embeddings, on the 250-message generated corpus:

| | every other message held out | whole senders held out |
|---|---|---|
| rules, no corrections | 86.4% | 99.1% |
| prompting | **100%** · p50 634 ms | **100%** · p50 619 ms |
| embeddings | 100% · p50 11 ms | **79.5%** · p50 11 ms |

And over the 251 stored messages in the scratch store, where the model pass
took 571 ms a message: 30 disagreements with the rules. 25 are the MediaMarkt
advert §3.2 already named, read by the rules as a newsletter and by the model as
marketing. 4 are "Rückfrage zum Angebot", a person asking about a quote, which
the rules call marketing on the word *Angebot* and the model calls personal. The
model is right in all 29. The last is a real provider's welcome message, which
the rules call notification and the model declines to call anything.

Three readings, and one warning about reading them:

**Embeddings learn senders, not meaning.** Perfect and sixty times faster on
senders it has filings for; poor on a sender it has never seen — all 23 DHL
delivery notices read as transactional, because their nearest neighbours are the
other no-reply senders, the Finanzamt and the Stadtwerke. Plan §4's claim that
nearest-neighbours "improves automatically as you correct it" is true, and says
nothing about the first message from anyone.

**Prompting holds on both.** The only method that does not care whether a sender
is new.

**The rules' two numbers cannot be compared with each other.** They are scored
on different messages: holding senders out put MediaMarkt in the training half,
so the rules' one blind spot was never marked. Every comparison here is valid
within a column and not across it.

The warning: this corpus is generated from a handful of templates, and 100% on it
is a statement about the templates, not a forecast for real mail. What it does
establish is narrower and still worth having — the model is not *worse* than the
rules anywhere they were tested, it is right about both of the rules' known
weaknesses, and it costs well under a second a message after a sync rather than
during one.

### What this decides

Not what the list shows. Model verdicts stay recorded and compared, exactly as
before; nothing in this entry moves a message. That decision wants real mail
behind it — `kuverta disagreements` on a real mailbox, and the user's own
corrections scored against both — rather than a generated corpus that both
methods find easy.

What it does settle is the shape the eventual answer will take, because the two
methods fail in complementary places: embeddings are fast and right wherever a
sender has been filed before, prompting is slower and right on senders nobody has
filed. A classifier that asks the nearest filings first and falls back to the
model when they are far away, or disagree, would take the cheap answer when there
is one. That is the next thing to build and measure, not something to assume.

Two loose ends, stated rather than tidied:

- **The CLI's default model is `Dolphin3:latest`** because it is the one these
  numbers are for. `make ai-model` pulls `qwen3:8b`, which has not been measured
  and is a thinking model — with the twelve-token answer limit it may never get
  past its reasoning to the word. Renaming the default to match the Makefile
  without measuring it would be the exact mistake §3.3 warns about.
- **The model sees less of stored mail than it was scored on.** The store keeps
  some of the headers the rules read and not others, so the model pass re-parses
  the message from disk where there is one and falls back to the summary where
  there is not.

---

## 16. Nearest filings first, the model when they are far away

**2026-09-14.** Follows §15.

§15 found the two methods failing in complementary places: embeddings perfect
and sixty times faster on senders filed before, poor on strangers; prompting
right on both, at six hundred milliseconds. The obvious combination is to ask the
nearest filings first and the model only when they are not good enough. The
question is what "good enough" means, and the answer was measured rather than
chosen.

`Neighbours::nearest` now reports how close the single closest filing is, and
`Hybrid` accepts the filings' answer only when that similarity clears a threshold
*and* the neighbours agree (a share of at least 0.8). Both conditions: a close
filing its neighbours contradict is a sender filed two ways, and a unanimous vote
among distant filings is a stranger who happens to resemble one kind of mail.

`kuverta eval` keeps the model's answer and the nearest filings for every scored
message, so the combination can be tried at every threshold without asking the
model again:

| similarity ≥ | known senders: accuracy · by filings · ms | new senders: accuracy · by filings · ms |
|---|---|---|
| 0.60 | 100% · 100% · 11 | 79.5% · 96% · 37 |
| 0.75 | 100% · 100% · 11 | 83.0% · 69% · 175 |
| 0.80 | 100% · 100% · 11 | **100%** · 52% · 277 |
| **0.85** | **100% · 100% · 11** | **100% · 25% · 452** |
| 0.90 | 100% · 100% · 11 | 100% · 18% · 499 |
| model only | 100% · 0% · 600 | 100% · 0% · 596 |

**The default is 0.85, and deliberately not 0.80.** 0.80 is exactly where accuracy
on new senders recovers, and a threshold placed on the knee of a curve measured on
one corpus is a threshold tuned to that corpus. 0.85 keeps a margin, still answers
every known sender from the filings at 11 ms, and still spares the model a quarter
of the strangers. A test pins the default above the knee so it cannot drift down
to it quietly.

**Real mail should move the error the safe way.** Messages from one real sender
vary far more than the generator's templates do, so real similarities will run
lower than these. That means more messages go to the model, not more wrong
answers from the filings — the combination degrades towards "model only", whose
accuracy it already matches.

### What is not done

The combination is measured, not running. In production the filings are the
user's own corrections, and there are almost none yet, so a combined pass today
would ask the model about nearly everything anyway. Wiring it in means embedding
corrected messages once and keeping the vectors — a table keyed by message and
embedding model — so the filings are not re-embedded on every pass. That is worth
building when there are corrections for it to use, and the decision about when a
model verdict may change what the list shows is still §15's, still open.

---

## 17. Filing post by hand

**2026-09-14.** Closes the gap §11 recorded.

§11 left post readable and unfilable: pressing `1`–`6` on a letter was refused,
because `correction` hangs off a message row and a document is not one. Post now
has its own append-only table, `paper_correction`, and a correction on a letter
does two things on purpose.

**It pins the letter.** The document shows the category it was filed under from
then on, whatever the rules make of it — a person's filing of one letter is not
evidence to weigh, it is the answer.

**It teaches the correspondent, when there is one.** A document's facts already
use its correspondent as the sender address, so the learning path mail uses
needed nothing new: the next letter from the same sender is filed the same way
unasked. Post corrections are loaded after mail's, so where a correspondent was
filed differently on paper and by email, the paper filing wins for paper.

A letter with no correspondent pins itself and teaches nothing. That is enforced
at the edge rather than trusted: the row shows a dash for "unknown", and a dash
learned as if it were a correspondent would file every anonymous letter alike.
So the command asks Paperless who sent the document instead of reading the row
the list happens to hold, and blank names are dropped before recording.

Filing a letter where it already is records nothing, as with mail. And in the
host contract, a letter's `actions` is now `[setCategory]` rather than `[]` —
which moved the conformance rule about rows refusing what they declared from
"a row that allows nothing" to "a row that does not allow archiving", since the
first no longer described any row there was.

> **Since (2026-09-14):** §9's gap is closed. `kuverta`'s message detail now
> carries the classifier's facts, re-parsed from the stored message, so the
> reading pane explains mail there the way it does in Thunderbird — and a
> letter's detail carries them too, so post is explained the same way. A message
> that is not on disk gets no facts rather than half of them: the stored row
> keeps too few of the headers the rules read, and an explanation from half the
> evidence would be confidently wrong.

---

## 18. An assistant may draft, and still nothing sends

**2026-09-14.** Brief §4.1's "draft", which §14 left out.

§14 kept sending off the agent surface under every flag, because mail is input
an attacker controls and a message saying "forward this to…" must not be one
tool call from happening. It left drafting out with it, since a draft tool that
shared a path with send was exactly the thing to get right rather than first.

The shape it takes is a real draft in the account's Drafts folder, not a preview
handed back to the assistant. A preview lives only in the assistant's context; a
draft is where the user already looks, in every client, and it goes nowhere until
they send it themselves. Three decisions made that safe:

- **Built exactly as send builds it.** `Session::save_draft` uses the same
  construction as `send` and `preview`, so a saved draft cannot differ from what
  sending it would put on the wire — threading and the quoted original included.
- **Appended, not queued.** `APPEND` can only create a message. Nothing is moved,
  flagged or lost, so there is nothing for the undo window to protect, and a
  draft the user does not want is deleted like any other.
- **No Bcc.** §3.5: blind recipients reach the envelope and never the message,
  so the built message has no Bcc line. A draft saved to the server would
  therefore silently lose them, and whoever sent it later, from any client,
  would send it without them and with nothing to say so. The tool's schema
  offers no `bcc`; a client that sends one is refused with that reason; and
  `save_draft` refuses before building anything — while a blank Bcc field, which
  is what an empty form sends, is not mistaken for a recipient.

`find_drafts` resolves the folder the way `find_sent` does — the server's
`\Drafts` attribute first, then the names people use, German included — and
refuses rather than guessing when there is none.

**Not verified here:** the append itself. It needs an IMAP server, and the tests
cover everything that happens before a connection is made. `send`'s filing into
Sent uses the same `append` and is covered by the dev-server tests; drafting has
not yet been run against one.

---

## 19. The default model: `llama3.2:3b`

**2026-09-14.** Resolves the loose end §15 left.

§15 measured an 8B model and left the default pointing at it, with the Makefile
pulling a different and unmeasured one — `qwen3:8b`, a thinking model that may
never get past its reasoning inside a twelve-token answer. Asked for a small,
good default, three small instruct models were measured the same way, on both
splits, with nothing else on the CPU:

| model | size | every other message | senders never seen | p50 |
|---|---|---|---|---|
| Dolphin3 | 8.0B | 100% | 100% | ~620 ms |
| **`llama3.2:3b`** | **3.2B** | **97.6%** | **100%** | **~267 ms** |
| `qwen2.5:3b` | 3.1B | 87.2% | 97.3% | ~285 ms |
| `gemma3:4b` | 4.3B | 87.2% | 76.8% | ~850 ms |

`gemma3:4b`, the largest candidate, finished downloading after the default was
set, and would only have changed it by being perfect on both splits and no
slower. It was the worst of the three and the slowest: it read every DHL "Ihre
Sendung wurde zugestellt" as transactional although the taxonomy names delivery
notices as notification, and filed people asking about a quote as transactional
too. That first mistake is the one embeddings made on senders they had never
filed — a no-reply address and a number pulled toward receipts — which suggests
it is a property of the message rather than of either method, and a good first
case for a correction to settle.

**The rule stated in advance was wrong, and is corrected here.** It was "the
smallest model that holds 100% on senders never seen", on the reasoning that that
split tests understanding. That reasoning belongs to embeddings, which learn from
filings. A prompted model learns nothing from filings, so both splits test it
equally, and a miss on either is a miss.

**What decided it was which messages were missed, not how many.** `eval
--show-wrong` now lists them. All three of `llama3.2:3b`'s are one template —
the Sparkasse's "Kontoauszug verfügbar", a portal notice that a statement is
ready — read as notification where the label says transactional. The taxonomy
files statements under transactional and machine-generated status under
notification, and this message is both. It misfiled no invoice, newsletter,
advert or person. `qwen2.5:3b`'s twelve were transactional mail it simply got
wrong, and it also missed on senders it had never seen.

So the default is the model under half the size and more than twice as fast as
the one measured first, whose only misses are on a message a careful person could
file either way. `make ai-model` pulls it, and `--model` still overrides it.


---

## 20. The view gets tests, and they find a bug on their first run

**2026-09-14.**

The triage surface was the one part of the JavaScript half with no tests — the
model, the adapters and the classifier all had them; the thing a person actually
touches had none. It had already produced two bugs found by hand: keys that
explained nothing, and a teardown that would have made every keystroke act twice
after switching accounts.

`kuverta-bird/test/view.test.js` mounts the real surface into jsdom over the
in-memory host and drives it through the keyboard and clicks: the legend and the
scope bar say what they should, `j`/`k` move the cursor, archiving keeps the
cursor where it was, typing into the search box never fires a shortcut, digits
file by category, a row that refuses an action says so on screen, unmounting lets
go of the keyboard, an empty filtered list says why and offers the way out,
Escape closes the panel before it touches the filter, the reading pane explains a
verdict and shows a user's filing beside what the rules said, and nothing a
sender wrote is ever parsed as markup.

**Its first run failed, on a real bug.** The explanation panel was written to
open by itself on first launch, and never did. The line that opened it was added
by a patch whose search text had already changed underneath it; that one
replacement had no assertion, so it matched nothing and said nothing. The person
it was written for — who reported not knowing what triage was for, twice — never
saw it unless they pressed `?`. The test was run against the unfixed view first
and failed exactly there, so it is known to catch the bug rather than merely to
pass once it is gone. The fix is one line, and the replacement that added it
asserts that it matched.

### What it cannot see

jsdom has no layout and no CSS cascade. These tests can say an element is
`hidden`; they cannot say it is off screen. The bug found earlier in the settings
sheet — `display: flex` beating the hidden attribute, which would have put both
forms on screen at once — is outside what they check. That wants a real engine:
Playwright against `hosts/kuverta/ui/triage.html` with a stand-in bridge would
catch it, at the cost of a browser download.

### The dependency

jsdom 24.1.3, pinned exactly, as the only dev dependency. Not the current
release: that one needs a newer Node than the 20.10 this was developed on, and
fails on import rather than at install. `make test-js` installs from the lock file
only when `node_modules` is missing, so an ordinary run stays offline.

---

## 21. Browser tests, for what a DOM without layout cannot see

**2026-09-14.** Follows §20.

§20 said plainly what jsdom could not check: an element marked hidden that is
still drawn, because a `display` rule beat the hidden attribute. Playwright now
loads the real pages into Chromium over HTTP, with the same fake Tauri bridge the
adapter's unit tests use — loaded into the page by URL, so the two cannot disagree
about what a command returns.

`e2e/triage.test.js` covers the shared surface as `kuverta` mounts it: the
first-launch panel actually on screen and gone after a reload, the legend inside
the window rather than below it, hidden things off screen and not merely marked,
the cursor staying in view through a 120-message list, archiving keeping the
cursor in a real engine, post interleaved with mail, a narrow window dropping the
reading pane but keeping the list, and a session that leaves nothing in the
console. `e2e/settings.test.js` covers the desktop window's settings sheet.

### What it found on its first real run

**A letter could not be read from the keyboard.** §12 made a row declare what it
allows, and `Triage.act` checked that before *every* action — including Open. A
letter allows filing and nothing else, so Enter on it was refused as though
reading were a change. A click opened it, because a click does not go through
`act`, which is why using the window had not turned it up and neither had the
model's unit tests. Reading is now never refused, and there is a unit test for the
case the browser found.

**The settings test is known to catch the bug it was written for.** With the rule
that fixed `display: flex` over the hidden attribute removed from the stylesheet,
three of its four tests failed with exactly "the address form must be off screen"
and "the account form must be off screen"; restored, all four pass. A test that has
never been seen to fail on the bug it names is a test of nothing.

### And one that caught the harness

The first run reported zero tests and exited green. `node --test` given a directory
only picks up files named like tests, and `triage.e2e.js` was not — so the suite
was skipped and called a pass. Renamed to `triage.test.js`. "0 tests, 0 failed" is
the worst kind of green, and it is now something to look for rather than accept.

### Cost

Playwright 1.63.0 and its headless Chromium (~94 MB), both pinned by the lock file
and installed only when missing. Kept out of `make test` because they take seconds
and need the download; `make test-e2e` runs them.

## 22. Drafting and post, against the real servers

Everything in §17 and §18 had been tested against fakes: a canned Paperless on a
loopback socket, and drafting only up to the point a connection is made. With the
Docker stack up, both now have tests against the real thing, which skip when it is
down and fail under `KUVERTA_REQUIRE_DEV_SERVER=1`:

- `core-rpc/tests/drafts_dev_server.rs` saves a draft through `Session`, syncs,
  and reads it back from the store — on the plain Dovecot (`Drafts`) and on the
  Gmail-shaped one (`[Gmail]/Drafts`, where the plain `Drafts` exists but is not
  the special-use folder), flagged `\Draft \Seen`.
- `core-paper/tests/dev_server.rs` holds on an empty instance or a used one, by
  asserting agreement — report against listing, listing against each document,
  every existing tag against the documents that carry it — rather than contents.

The rest was checked by hand, on the stack, and is listed so it can be repeated:
`scannerd --drain-only` with two generated letters; `kuverta paper` with
`--check`, `--tag` and `--query`; and a postal address registered in `.devdata`,
read through `core-rpc` exactly as the desktop commands read it — token filed, check,
listing, opening a letter, filing it.

### What the real servers found

**`scannerd` could never have uploaded a tagged letter.** Paperless's upload
endpoint takes tag *ids* and answers a name with `400 Expected pk value, received
str`. The unit file example says `--tag post --tag home`; every capture would have
failed and waited in the spool forever. The fake accepted anything, so nothing had
said so. Names are still what a person writes: `scannerd` now looks each one up
once (`name__iexact`), sends ids, and a tag Paperless does not have fails the upload
*by name* and keeps the capture. The fake answers lookups now, and a test says why.

**A blank Cc refused the whole draft.** `save_draft` already read a blank Bcc as
no recipient; `build_draft`, shared with preview and send, parsed a blank Cc or To
as the address `""`. A compose form sends exactly that for an empty field. Blank
fields are now skipped for all three, with a test that needs no server.

**The dev stack's Paperless did not start.** `latest` moved to a release that
refuses to run without `PAPERLESS_SECRET_KEY`. The compose file sets a dev-only key
and pins the image (3.1.3), so the next such change is a decision rather than a
broken `make dev-up`.

### And one the test got wrong

The tag test first upper-cased names with Unicode rules, and "Hauptstraße 12"
became "HAUPTSTRASSE 12", which Paperless does not match. That is another spelling,
not another case; an ASCII case change ("HAUPTSTRAßE 12") does match, on the SQLite
the dev instance runs. The test changes case the way a person mistyping would.

### Still open

- ~~**The keychain entry for a paper token is `paper:{id}`**~~ — fixed. Row ids
  start at 1 in every store, so `.devdata` and the real store shared `paper:1`.
  Schema v9 gives each address a random `token_key`, fixed when it is added and
  kept through edits; the token is filed as `paper:{token_key}`. An old
  `paper:{id}` entry moves on first read — claimed by whichever store asks first,
  since the old entry cannot say whose it was; if that is the wrong store, the
  check reports the token rejected and it is entered again. Checked against the
  real keychain: the entry moved, the old one was removed, Paperless accepted it.
- ~~**Post reads poorly until Paperless knows who sent it**~~ — fixed. Paperless's
  suggestions are no help for first letters: they only propose correspondents that
  already exist, and a new instance has none. So `core-paper` reads the letter
  itself, and only for what Paperless has not been told:
  - the **sender** is the correspondent, or else the letterhead — the first line of
    OCR text with words in it, cut before an address on the same line;
  - the **subject** is the title, unless the title is a scan's name (`Post
    <timestamp>`, `IMG_2044`, none), in which case it is the first line after the
    address block (the last postcode-and-place line near the top) that is not a
    date — DIN 5008 order.

  Rows, the CLI and the classifier use both, and filing a letter learns its sender,
  so it is no longer the correspondent or nothing. On the dev stack the three
  scanned letters list as *Finanzamt Muenchen — Bescheid fuer 2025 ueber
  Einkommensteuer* and *Stadtwerke Musterstadt GmbH — Rechnung Nr. 2026-48211* /
  *Ihr neuer Abschlag ab November 2026*; filing the September invoice as a
  notification through `core-rpc` filed October's letter the same way, and left the
  Finanzamt alone. `tests/letters.rs` holds the verbatim OCR of those letters, a
  multi-line DIN layout, and the cases where a person's title or correspondent wins.

  Learned against a letterhead, a rule stops matching once Paperless is given a
  correspondent for that sender, because the key changes; the letter is then filed
  by whatever is taught about the correspondent. And a two-word letterhead such as
  "Finanzamt Muenchen" passes the rules' "looks like a person" test, as a
  two-word correspondent always did — outweighed here by the transactional
  evidence, but a reason `ROLE_WORDS` (shared with the JS port) may want
  institution words.
- **A token filed with `security add-generic-password` cannot be read by the app
  without a prompt,** whatever `-A` says: the item's partition list admits Apple's
  tools only. Paste tokens into the settings sheet, which files them as the app.

## 23. The scanner, without a Pi: what reading it found, and letters

No Raspberry Pi here, so this round was what can be settled without one: reading
`scannerd` closely, driving its loop by hand, and running the whole binary on a
laptop with ffmpeg standing in for `rpicam-still`.

### Found by reading

- **A failed upload waited for the next letter.** The spool was drained at start
  and after a capture, never otherwise, while the readme promised a retry "within
  the minute". It now drains on a timer (`--drain-every-secs`, 30) too, including
  while the camera fails.
- **The backoff did not back off.** It was measured from the capture, so anything
  older than five minutes was always due. It is measured from the last attempt,
  which the sidecar now records.
- **Uploads had no timeout**, and the loop waits on them: a link that dropped
  mid-request would stop the daemon seeing the next page. Two minutes.
- **Detection ignored `--roi`.** The crop applied to the photograph only, so the
  thresholds were fractions of the whole desk.
- **The systemd unit passed no arguments.** `ExecStart` never mentioned
  `$SCANNERD_ARGS`, and `SCANNERD_ROI` was offered by the env example but read by
  nothing — so on a Pi the tags and the crop in `/etc/scannerd.env` would never
  have arrived. Tags (`SCANNERD_TAGS`, comma-separated, since a tag may hold a
  space), button and crop are environment variables now.
- **The readme never created the `scannerd` user** the unit runs as.
- **Two captures in one second shared a name** (seconds plus the process id), and
  the second rename replaced the first. Nanoseconds instead.

`Scanner::turn` is the loop as a library function with the clock passed in, and
`tests/run.rs` drives it with a scripted camera and a Paperless that fails on
request; with the timer drain removed, two of its three tests fail.

### Letters of more than one page

Asked, and decided: **a button**. Grouping by time would merge a quickly scanned
stack of one-page letters, which is how post gets scanned; a separator sheet is
paper to print and handle; one page per document leaves the merging to be done by
hand. With `--button`, pages collect in the spool's `open/` until it is pressed and
the letter goes as one PDF. Details that matter:

- **The button is a key.** The kernel's `gpio-key` overlay turns a push button on
  GPIO 17 into Enter on an input device and debounces it, so `scannerd` reads
  24-byte `input_event` records — no GPIO library, no libgpiod version to match, a
  USB keypad works too, and `--button stdin` is Enter in a terminal.
- **A press while the last page is settling waits for it.** Putting the page down
  and pressing is one movement, and the press arrives first.
- **A letter nobody closes closes itself** five minutes after its last page.
- **The PDF embeds the photographs unchanged** (`/DCTDecode`), with no image
  library: a JPEG's size is in its frame header. Written as `.partial` and renamed;
  pages removed only after, so a crash leaves the same PDF twice (refused by
  Paperless by checksum) rather than nothing. A page that is not a readable JPEG is
  queued on its own rather than blocking the letter.
- Checked beyond the unit tests: a PDF built from the three real letter photographs
  passes Ghostscript and renders each page with its proportions.

### Not verified

- **On a Pi:** since checked on the user's Pi, which is a **Zero W Rev 1.1** —
  ARMv6, single core, 427 MB, 32-bit bookworm, OV5647 camera. That made the first
  `make scannerd-pi` (an arm64 container) the wrong build: it now cross-compiles
  with cargo-zigbuild for `arm-unknown-linux-gnueabihf.2.36` in about a minute and
  a half. On the device the binary's libraries resolve, and an HTTPS upload
  completes its handshake — which matters because that runs the TLS provider's
  compiled code, where ARMv7-only instructions would have crashed. scannerd's
  reqwest now uses ring as the provider rather than aws-lc, whose ARM assembly
  has not been run on an ARMv6. A preview frame takes 1.1–1.7 s (the camera
  starts per call) and a capture 3.7 s, so a page is photographed eight or nine
  seconds after it is put down; a streaming preview (`rpicam-vid`) would cut that,
  if it proves worth it. Still open on the device: detection against a real
  surface (the camera was pointed at a ceiling), exposure, the `gpio-key`
  overlay's device name, and the unit file.
- **Against Paperless:** the laptop run of the whole binary and the `make
  scannerd-pi` build were stopped by the Mac's disk filling (302 MB free): Docker's
  storage went read-error, Paperless answered 500, and a frame from ffmpeg took 18
  seconds. Space was freed — build output, Docker's build cache, three benchmark
  models — but Docker needs restarting before either can run.

## 24. Post read by a vision model, unread, and sorted like mail

Scans reached the desktop app and read badly. Four things were asked for: post
arrives unread, the text is readable, post is sorted into categories, and the list
can go by the date on the letter or by when it was scanned.

### Why the OCR was bad, and why not fix the image

The photographs are soft. The OV5647's lens is fixed-focus, set for distance, and
the page is at desk distance: the last scan's sharpness measured 0.08 where a
sharp synthetic page measures 0.32 and a deliberately blurred one 0.03. Tesseract
(`deu+eng`, as Paperless runs it) found about 170 words on it at 68% mean
confidence, only 24 of them confident. Upscaling, flattening the light and
sharpening did not raise that: what the lens did not resolve is not in the file.

A made-up letter drawn at the camera's resolution (about 150 dpi) and blurred by
steps shows the cliff Tesseract falls off:

| Gaussian blur radius | 0 | 1.2 | 2.0 | 2.8 |
|---|---|---|---|---|
| Tesseract, character similarity | 100% | 100% | 86% | 5% |
| `qwen2.5vl:3b`, character similarity | 100% | 100% | 100% | 99% |
| `qwen2.5vl:3b`, seconds for the page (Apple M2, 24 GB) | 45 | 34 | 33 | 33 |

Asked, and decided: **a local vision model reads the scans.** Nothing leaves the
machine, as with the rest of the AI here.

The 1% it lost on the blurriest page is worth more attention than its size: all
three errors were digits in dates — "3. September" read as "1.", and "2026" as
"2025" twice — and none was marked `[?]` as the prompt asks. A blurred number
comes back as a confident, plausible wrong one. So the scan stays one tab away,
and a date or amount that matters is checked against it.

### A statement it looped on

On a real tax office account statement the model read the letterhead and the
bank details, then wrote the table's two column headings, "EUR" and "Ct", 116
times until it stopped. Measured on that page, counts only:

| Reading | prompt tokens | answer tokens | stopped because | most repeated line | seconds |
|---|---|---|---|---|---|
| as first shipped: Ollama's 4096 context | 3341 | 755 | out of room | 116× | 25 |
| 16k context | 3341 | 4096 | out of room | 951× | 184 |
| a prompt with rules for tables | 3375 | 1500\* | out of room | 302× | 84 |
| temperature 0.3 | 3341 | 1500\* | out of room | 302× | 52 |
| the image scaled to 1024 px | 1127 | 1500\* | out of room | 604× | 61 |
| repeat penalty 1.2 over the last 128 tokens | 3341 | 864 | finished | 2× | 30 |

\* capped at 1500 for the test.

The context was too small — the page alone is 3,341 of Ollama's default 4,096
tokens — but it was not the cause: given room, the model looped for longer. The
repeat penalty is what stopped it, and it is not free. On the drawn pages, which
read cleanly without it:

| Page | similarity, without / with | numbers kept, without / with |
|---|---|---|
| plain letter, sharp | 100% / 78% | 100% / 89% |
| plain letter, blur 1.2 | 100% / 100% | 100% / 89% |
| plain letter, blur 2.0 | 100% / 78% | 100% / 89% |
| plain letter, blur 2.8 | 99% / 86% | 83% / 78% |
| table letter, sharp | 95% / 86% | 95% / 77% |
| table letter, blur 2.0 | 95% / 83% | 95% / 73% |

So a page is read in a 16k context, plainly first. A page that loops or runs out
of room is read again with the penalty, and that reading is kept only if it
finishes, marked as a second, looser reading so its names and numbers get checked
against the scan. A loop that survives both readings is cut after two repeats and
marked, and a page cut short without a loop says so. Answers are capped at 2,048
tokens — the pages measured took 200 to 864 — so a loop costs well under a minute
before the second reading rather than three. Hosted providers get the cutting and
the marks but no second reading: their penalties are on another scale, and
untried here.

### How

- **The original, not Paperless's archive PDF.** The archive is re-rendered with an
  OCR layer; the original is what `scannerd` sent — JPEG pages inside a PDF. The
  pages are taken out by `core_paper::scan::jpeg_pages` (the `/DCTDecode` streams,
  whole), so no PDF renderer is needed. A document that is not like that is
  refused in words rather than sent to the model as something it cannot see.
- **One request per page**, to Ollama's chat endpoint with the image, temperature 0,
  and a prompt that asks for the text as printed, untranslated, with `[?]` for a
  word that cannot be read — a model filling a blur with a plausible word is worse
  than an OCR gap, because it reads as right.
- **The transcript replaces the OCR everywhere**: the reading pane, and the sender,
  subject and category, which come from the text. It is stored per Paperless
  instance and document (`paper_transcript`), with the model's name, which the
  reading pane shows next to the text, with a button to read it again.
- **New post is read as it arrives**, one letter at a time — two at once on a
  laptop is both slowly. Once the model fails (Ollama not running, model missing),
  automatic reading stops for the session and the reason is shown, rather than
  every new letter failing the same way.
- `qwen2.5vl:3b` by default, `KUVERTA_VISION_MODEL` to change it.

### Unread, categories and the two dates

- **Read state is kuverta's.** Paperless has no notion of it, and writing a tag
  per letter back into Paperless would change the archive from a reader that
  promises not to. `paper_read`, keyed like the transcripts; `u` toggles, opening
  reads, the postbox shows an unread count.
- **Categories are mail's**, from the same classifier, with the same counts and
  filter in the sidebar. Paperless cannot filter by a category it has never heard
  of, so a filtered list is built from all the postbox's rows (at most 5000) and
  paged here; unfiltered lists still page in Paperless.
- **Letter date or scan date**: Paperless's `created` (from the letter, or the
  upload when it found none) or `added`. The list sorts and dates by the choice,
  and keeps it.


## 25. Choosing models, and hosted providers beside Ollama

Asked for: a settings page to choose between models, see which are available, and
connect an external provider such as DeepSeek.

### Shape

- **Providers and jobs.** A provider is somewhere models run: the Ollama on this
  computer (in every store from schema v11; its address can change, it cannot be
  removed), another Ollama, or a hosted service with an OpenAI-compatible API. A
  job is something kuverta asks a model to do: reading scans, and sorting mail
  (`kuverta classify`). Each job has one provider and model. A job with none
  chosen runs where it did before: the local Ollama, with `qwen2.5vl:3b` (or
  `KUVERTA_VISION_MODEL`, which §24 named and is now only the default) and
  `llama3.2:3b`.
- **One client for hosted services.** DeepSeek, OpenAI, OpenRouter and most others
  take the same `/chat/completions` request and list `/models`; the page's presets
  fill in each one's address. A page goes to them as an `image_url` data URL.
- **What a model can do is shown only where the provider says.** Ollama reports
  each model's capabilities and OpenRouter its input modalities; most services,
  DeepSeek among them, list names only. There the page says "not said" rather
  than guessing, and **Try them** runs each job once on a sample, which is the only
  way to know: a drawn page reading "Rechnung 4711" for reading, a one-word
  question for sorting.
- **Keys are in the keychain**, filed under a random name per provider, as
  Paperless tokens are; nothing hands one back to the window. Removing a provider
  removes its key and sends its jobs back to their defaults.
- **Leaving the computer is said, not implied.** Nothing falls back to a hosted
  service. When a job's provider is not this computer (decided by the address
  being a loopback one, so an Ollama elsewhere on the network counts as elsewhere),
  the note under the job says what goes there: scans of letters, or senders,
  subjects and the start of each message. `kuverta classify` prints the same
  before it starts.
- `classify --ollama` (or `OLLAMA_URL` in the environment) still asks that Ollama
  directly, past the settings; `--model` overrides the chosen model.

### Not verified

- **Against a real hosted service.** The client is tested against servers that
  answer in the documented shapes, not against DeepSeek itself: there was no key
  to test with. Whether a given service's models accept images is what Try them is
  for.

## 26. A setup assistant for the first run

A new store opened on an empty list and a settings form, which is the least
helpful thing a mail client can show someone who has not yet told it anything.
The window now opens an assistant instead: local models, a Paperless for paper
post, then mail accounts, each skippable.

- **It checks and points; it installs only Paperless.** Ollama and Docker are
  installed from their makers' pages or a command the assistant shows, because
  installers that need administrator rights are not the window's to run.
  Paperless is different: with Docker running, it is a compose file and
  `docker compose up -d`, written into `<data dir>/paperless/` where it can be
  found and removed. The image is pinned to the dev stack's version. The admin
  password lives in an owner-only env file, because Paperless reads it only when
  it first creates the user and a reinstall should create the same one.
- **A Paperless password, not a token.** `/api/token/` exchanges a user name and
  password for the user's token, so nobody is sent looking for one. The password
  goes to the core and to Paperless, never back. The endpoint is throttled; a 429
  is reported as "wait a minute", not as a wrong password.
- **Finding Paperless** asks `/api/documents/` anonymously and takes a 401 with
  `WWW-Authenticate: Token` as a Paperless — Django REST framework's token
  scheme, which little else on port 8000 speaks.
- **Programs are looked for where they are, not only on `PATH`.** An app started
  from the Dock gets `/usr/bin:/bin:/usr/sbin:/sbin`, which has neither Homebrew
  nor Docker; the Docker CLI also needs its credential helpers on the `PATH` it
  runs with.
- **Thunderbird's passwords are imported with its accounts.** Its `prefs.js` has
  every server, and `logins.json` the passwords — encrypted with a key in
  `key4.db` beside it, protected by an empty password unless a primary password
  is set. That is NSS's scheme, and the profile is the secret: anything that can
  read the folder can read the passwords, which is what Thunderbird itself does
  at startup. So kuverta reads them the same way (PBKDF2-SHA256 and AES-256-CBC
  for the key, 3DES or AES for each login) rather than asking for nineteen
  passwords again. With a primary password, the assistant asks for that one and
  passes it down; it is never stored. A password found this way goes from the
  profile into the keychain inside one call — `save_account_with_found_password`
  — so it never crosses into the window, which is the rule the settings form
  follows too.
  Apple Mail's passwords are **not** imported: they are in the login keychain
  under Mail's own access rules, and reading one raises a system prompt per
  account. Those are typed once, as before.
  Every account is signed in to before the assistant moves on; one that fails
  stays on the page with the server's answer.
- **Apple Mail is the system's Internet Accounts**, `~/Library/Accounts/
  Accounts4.sqlite`, which macOS shows only to a program with Full Disk Access.
  The assistant says so and links to the setting. The database is read from a
  copy, since the live one is in WAL mode and in use.
- **Servers for any address** come from a built-in table of common providers
  first, then the places Thunderbird looks: `autoconfig.<domain>`, the domain's
  `.well-known` file, and Mozilla's ISPDB. Only encrypted servers are taken from
  those files. Microsoft addresses are marked OAuth2, because Microsoft takes no
  passwords over IMAP.
- **First run means no marker and nothing in the store.** People who set kuverta
  up before the assistant existed never see it open by itself.

### Verified

- Thunderbird import against a real profile with 19 IMAP accounts, and the
  parser against a canned `prefs.js` and `profiles.ini`.
- Reading that profile's 46 saved passwords, which needed both keys such a
  profile keeps: 24 bytes for the logins written when 3DES was used, 32 for the
  AES ones. The round trip is tested against a login this code encrypts itself.
- Server lookup through a provider's own autoconfig file and through ISPDB.
- Ollama status against Ollama 0.34.1; the download stream's parsing against
  Ollama's documented lines.
- Paperless detection and sign-in against the dev stack's Paperless 3.1.3.
- The assistant's pages in a browser, against a stand-in for the core.

### Not verified

- **Apple Mail against a real accounts database.** This machine does not give the
  terminal Full Disk Access, so the reader is tested against a database built to
  the documented layout. Only the address is relied on; a provider's account
  gets its servers the way a typed-in address does.
- **Installing Paperless end to end.** The compose file was written and
  `docker compose up` ran, but the Docker VM on this machine failed with I/O
  errors (its disk was full) before the web server started.
- **Downloading a model through the window.** Both default models were already
  present here.

## 27. The first sync goes newest first

A first sync fetched each folder from UID 1 upwards, which is the order the
server hands them over in and the wrong order to read them in: on a mailbox of
any size the window sat for an hour showing mail from years ago, and the mail
that arrived this morning was the last thing to land. Reported as "it only
shows messages up to Thursday" — which it did, because Thursday was as far up
as it had got.

- **Only a folder with nothing held is walked downwards.** Mail above what a
  folder already holds is still fetched upwards: there is rarely more than a
  batch of it, and a pass that only ever adds to the top can be interrupted
  anywhere without leaving a hole.
- **A walk downwards leaves a hole, so the hole is written down.** `folder.
  backfill_uid` is the UID the walk has come down to; everything below it is
  owed. It is written after every batch, not at the end of the folder, because
  the case it exists for is a laptop closed mid-sync: the next pass has to
  resume inside the hole, and `MAX(uid)` — which is what says where to fetch
  from — would by then be pointing at the top of the mailbox, above it.
- **A folder that owes history is never skipped.** CONDSTORE skips a folder
  whose `HIGHESTMODSEQ` has not moved, which without this guard would be every
  folder on the sync after an interrupted one: the backfill would be owed for
  ever.
- **Not: fetching the newest and leaving the rest.** A mail client that holds
  the last month and says nothing about the rest is a worse thing to search
  than one that is still filling. The whole folder is still fetched; only the
  order changed.

### Verified

Against the dev Dovecot, in `crates/core-proto/tests/dev_server.rs`: a folder
of 201 messages — one more than a batch holds — where the first batch to reach
the store holds the newest message and not the oldest; and a store wound into
the state an interrupted walk leaves, which the next sync fills without being
asked, ending with nothing owed. Both fail if the walk is put back the way it
was.

## 28. Who the message is with, and when a letter's address may be claimed

**2026-09-23.**

A sender line says a name. It does not say whether this is the fortieth letter
from a shop you never answer or the first from somebody you have been writing
to for three years, and that is usually what decides what to do with the
message. `crates/core-rpc/src/sender.rs` answers it from mail already in the
store: how much there is each way, since when, who spoke last, what their mail
usually is, where it gets filed, whether any of it is still waiting, and the
last few messages either way.

Three things were decided rather than fallen into.

**Asked by message, not by address.** The window has a row under the cursor,
not an address, and the other person in a message the account itself sent is
its first recipient rather than its sender. The store settles that with the
same `COUNTERPART` expression the conversation list groups by, so a card
opened from a letter and the conversation it belongs to are always about the
same person. An address may be passed instead, which is what the People list
has to hand.

**One column on the right, not a second sidebar.** The card first went under
the mailboxes on the left, which was wrong twice over: it is about the message
in front of you, not about where mail lives, and it pushed the navigation
around every time a message opened. It shares the fourth column with the
assistant instead — both are about what is open — and the column exists only
when something is in it. They are stacked in a flex column where the card is
capped at 45% and what does not fit scrolls inside it, because the assistant
is where the typing happens and a card must never push the input off screen.

**A postal address is claimed only where it cannot be the reader's own.**
Paperless keeps no address for a correspondent, so a scanned letter's can only
come off the page — and the difficulty is that the recipient's address block
sits right under the letterhead and looks exactly the same. The dev-stack
letters are that shape: `Stadtwerke Musterstadt GmbH`, then `Erika Mustermann,
Hauptstrasse 12, 80331 Muenchen`, which is where the letter went, not where it
came from. Naming that as the sender's would be wrong in the most confusing
way available. So `Document::postal_address` answers in two layouts and no
others: the return line on the letterhead itself, which DIN 5008 puts there
for this purpose — `Stadtwerke Musterstadt GmbH · Postfach 1234 · 80000
München` — and a letter carrying two address blocks near the top, where by the
same standard the first is the sender's and the second the recipient's.
Anything else is `None`. Most scans will show no address, which is the honest
answer; somebody else's address on a card is worse than none.

### Verified

`crates/core-store/tests/cleanup.rs` sums a correspondent up from both
directions and finds the same person from her letter and from the reply to it;
`crates/core-rpc/tests/surface.rs` checks the card is the same whichever way
it is asked for. `crates/core-paper/tests/letters.rs` reads the address out of
both layouts that allow it and returns nothing for the two dev-stack letters,
whose only address block is the reader's.

`kuverta-bird/test/desktop-sender.test.js` drives the card itself in jsdom —
counts of nothing left out, the exchange newest first with its direction, the
hover copy free of anything that could not be clicked. What jsdom cannot see
is the layout, which is the whole point of the column, so
`kuverta-bird/e2e/sender.test.js` asks a real engine: the column sits beside
the reading pane, the card and the assistant share it with the chat input
still on screen, the list gets its width back when the message closes, and the
hover card lands clear of the list and inside the window.

## 29. What the sensor leaves out is gone: four ways a letter lost its edges

**2026-09-24.**

A Finanzamt letter came out of the rig with the "F" missing from
`Finanzamt(Finanzkasse)` and the whole right-hand column — the one headed
**Mahnung** — cut off. Paperless then read the first line it could, `26603
Aurich`, and filed the letter as being from a postcode. Four separate things
were wrong, and each of the first three hid the next.

**The sensor crop was the marks' own bounding box.** `--roi` crops on the
sensor, and the crop was computed as exactly the box the four marked corners
span. So a letter lying further left than the marks was cut by the camera
itself, before a single line of this code ran: nothing downstream can recover
what was never photographed. Measured on the rig, the sheet's top left corner
sat 9% of the frame outside the marks. The crop now leaves 8% of the marked
area round it. That costs nothing, which is the part worth knowing:
`capture_size` asks for the crop's own share of the sensor, so a wider crop is
a bigger photograph rather than a coarser one, and the page keeps every pixel
it had.

**The marks were then used as the page.** Even with room in the photograph,
the first warp goes onto the marks, so anything outside them is thrown away
before the page is looked for. `paper_overhanging` now asks the photograph
where the paper is and, when it reaches more than 2% outside the marks, adds
its corners to theirs. Added, not substituted: the marks are exact and the
found page is only as good as the light on its edges, so `Corners::around`
takes whichever reaches further on each side, in the marks' own perspective so
the shape and the angle to the camera survive.

**The trim cut into the sheet.** After the warp, the picture is straightened
once more onto the page found inside it, to remove the strips of table beside
a letter lying a little off the marks. The page is found by its brightness,
and the far edge of a sheet under a camera on a stalk falls off into shadow,
so it is found short — and the trim then cut real page. It now looks just
beyond each side before cutting it: a side with something as bright as the
page beyond it is not the sheet's edge, and stays where the picture ends.

**And the sender was read off whatever line came first.** With the letterhead
cut, that was `26603 Aurich`. It would have been wrong again on the next
letter whose top line OCR happened to read as the address beside the
letterhead rather than the letterhead itself. `Document::letterhead` now takes
the first of the top four lines that could be a name — not a postcode and a
place, not a date, not mostly digits — and cuts a line at a column gap, so
`Finanzamt(Finanzkasse)   26603 Aurich   02.05.25` reads as the Finanzamt.

### Verified

On the rig, against the letter that started it: the same sheet, in the same
place, photographed before and after. Before, the letterhead began at
`inanzamt` and **Mahnung** ran off the right edge; after, the whole page is
there, barcode to signature. `apps/scannerd/examples/crop_check` is what that
took — it puts a photograph through each step separately and prints where the
paper lies in the marks' own frame — and it is kept because this class of bug
is invisible from the code and obvious from the pictures.

In `apps/scannerd/tests/straighten.rs`: a letter overhanging the marks keeps
its letterhead, marks that hold the whole letter are left alone, the trim
takes a strip of table but leaves a shadow on the page, and the crop holds
every corner with room round it. Each fails with its fix removed.
`crates/core-paper/tests/letters.rs` reads the sender out of the line order
the rig's own OCR produced.

## 30. The learn-table button that a ten-minute window took away

**2026-09-24.**

Filing the last letter by hand — **No folder** and **Throw away** — and
learning the empty table were sharing one row of the panel, and refiling won
it whenever there was a letter still inside its undo window. That window is
ten minutes. So for ten minutes after every letter there was no way to learn
the table from the rig at all, which is exactly the moment you want it: the
pile has gone through, the table is clear, and learning it is the next thing
you would do.

Learning the table now goes last but is not given up. It stands down only when
five rows are already taken, which is the most the card above the buttons can
spare on a 480×320 panel.

## 31. The card is asked for, the columns are yours, and a letter is not an envelope

**2026-09-24.**

Three things about the sender card, and one bug it caused.

**It opens on being asked, not on reading.** The card opened itself whenever a
message was opened, which took a quarter of the window every time to answer a
question nobody had put. Now a sender's name — under the subject, and in a
conversation's header — is the thing you click, and the card opens there.
Once open it follows the cursor: having asked who one message is with, you are
usually asking of the next one too. Its × closes it and stops it following.
The hover card is unchanged; it was always the cheap version of the question
and nobody complained about it.

**The columns are dragged.** The widths were written into the grid — a fixed
sidebar and a list between 300 and 390 — which is a guess about somebody
else's screen. They are CSS variables now, dragged by the seams between the
panes, double-clicked to put back, and kept in `localStorage`. The seams are
grid items placed in the same cells as the panes, which is why the panes are
now placed by hand: grid only lets items overlap where both were placed, and
a handle has to sit *on* the border it moves. The one column with no width of
its own is the reading pane, so it is the one that gives when the right-hand
column opens.

**A message can be thrown away from the card.** Each line of the exchange has
a bin. It is the same queued move the list makes — `z` takes it back — and the
card is asked for again afterwards by *address*, never by the message it was
opened from, because that message may be the one that has just gone.

### The envelope that was a letter

Widening the quad in §29 broke something a long way from it. `Straightened::
covered` is the share of the picture the paper covers, and it is how an
envelope is told from a page — an envelope is smaller. It was measured against
whatever had just been warped onto, which used to be the marks and now is
sometimes the marks widened to hold an overhanging letter. A page measured
against the wider quad comes out at an envelope's share, and an envelope
finishes the letter being collected and starts a new one: so a letter was cut
in two and its first page called the envelope of the second.

`covered` is now always against the marks, whatever was warped onto. A number
that answers "is this smaller than a page" only means something if its
yardstick never moves.

The same report turned up a second thing, older. With no page seen yet,
`page_covers` stood in as 1.0 — a page fills the corners — and the rig's own
pages cover about 0.70 of corners set generously round where letters land,
against a threshold of 0.72. Every restart reset that, so the first page after
one was a coin toss. Now nothing is an envelope until a page has been seen:
an envelope mistaken for a page becomes page one of the letter it holds, which
is where it belongs, while a page mistaken for an envelope cuts a letter in
two. And paper whose edges could not be measured teaches that a page fills the
corners, so a rig set up with tight corners still learns enough to tell an
envelope later.

### Verified

`apps/scannerd/tests/straighten.rs` has a page that overhangs the marks, sized
so the two yardsticks fall on opposite sides of the line: 0.83 of the marks,
0.63 of the marks widened to hold it. It reads as a page, an envelope on the
same marks still reads as an envelope, and the test fails with the measurement
put back the way it was.

`kuverta-bird/e2e/sender.test.js` drives the rest in a browser, because all of
it is layout and pointers: reading a message leaves the column shut and
clicking the sender opens it, it then keeps up with the cursor, the × shuts it
for good, the reading pane is what gives up the width, the seams drag and stop
at the limits and are remembered, and a message binned from the card leaves
both the card and the list with the card still open on the same person.

## 32. Saying where a letter goes while it is still in your hand

**2026-09-24.**

Four things the rig got wrong about the letter in front of you, from one
session of working a pile through.

**The panel said nothing about it.** The Pi has read each page since §28 and
the setup page shows the guessed folder, but the panel — the thing you are
looking at, with the paper in your hand — showed only where the *previous*
letter had gone, and only once Paperless had answered. The guess is on the
panel now, as an outline pill in the top right, from the first page read. An
outline rather than a filled one because it is a guess; the last letter's
folder, which is settled, keeps the filled pill and gets the corner back when
there is no guess to show.

**A letter from the police had nowhere to go.** The shelf had no law folder,
so a Strafbefehl notice was filed under nobody. The deeper problem was the
word lists: they were written for a scanner and are matched against what a
camera on a stalk and tesseract make of a page, which is bad text. That letter
came out with "Strafprozessordnung" as "Sraiomzessorsnung", "Swatprozessordnung"
and "Stra prazessortrugg" — three wrong spellings of the one word that would
have caught it. It was filed in the end by "Staatsanwaltschaft", which happened
to survive on page two.

So the lists the setup assistant offers are longer than they look as though
they need to be: every extra spelling of the same idea is another chance that
one lands. There is a **Recht & Behörden** folder now, with the broadest list
of the lot, because a letter from a prosecutor is the one kind of post where
finding it a fortnight later is not good enough. "Anzeige" and "Ladung" are
deliberately not in it — one is also an advertisement and the other a lorry's
load. There is a **Gesundheit** folder too, and the other five lists are about
half as long again.

Each list has to fit the 256 characters Paperless keeps of a tag's match.
Three of the new ones did not, and the rig refused them — which is the good
outcome, but only because the rig checked. `desktop-shelf.test.js` now checks
it where the lists are written, along with two things no length check would
catch: that no suggestion matches on a word every letter carries, and that no
word appears in two folders.

**The preview showed the text without the page.** A letter's pages are read
into text that stays on screen until the next letter starts — but the pages
themselves were deleted the moment the letter was closed into its PDF. So
after every letter the setup page showed what the letter said beside an empty
frame. The pages go to `spool/sent/` now instead of being deleted, are served
from there, and are cleared at exactly the moment their text is: one letter on
screen, whole, or none.

**And the folders were chips.** In preview mode they are what you act on, one
key press each, from across a table. They are the biggest thing in the column
now rather than set at the size of the running text beside them.

### Verified

`apps/scannerd/tests/display.rs` has the panel naming the folder from a single
open page, joining two of them, and saying nothing when there is no letter
open. `apps/scannerd/tests/letters.rs` closes a letter and finds its pages
still readable in `sent/`, then gone once the next letter starts.
`kuverta-bird/test/desktop-shelf.test.js` holds the word lists to the match
limit and to being distinctive.

Against the real letter: the Wasserschutzpolizei notice that started this now
reads as `Lawstuff` on the rig, on Staatsanwaltschaft, Strafbefehlsverfahren,
StPO and Beschuldigten — four of the sixteen words, which is what a list of
sixteen is for. The same shelf leaves the Finanzamt letter in Taxes and
Inkasso/Rechnungen and out of the law folder.

## 33. One shelf, in one place, and one folder per letter

**2026-09-24.**

Asked whether the folders were kept in step between the rig, Paperless and
kuverta. They were not, and measuring it was unpleasant: the `Lawstuff` on the
rig and the `Lawstuff` in Paperless had **three of sixteen words in common**,
and `House` and `Work` existed only in Paperless, so the rig could not guess
them at all.

The wiring was the cause. kuverta's assistant wrote folders to Paperless as
tags. The rig adopted them — but only once, and only if nobody had ever told
it any folders, which `nothing_says_which_folders` tested by asking whether
the setup page had ever saved a list. Save one folder on the rig's page and it
never looked at Paperless again. And it never wrote back.

What made that worse than untidy is which list actually files a letter.
Paperless's tags do: they match the text and put the document in a folder. The
rig's list only drives its own guess and the number keys in preview. So the
panel could say one folder with complete confidence while Paperless used
another — and the further the lists drifted, the more often it did.

**Paperless is the shelf now.** The rig reads its tags every five minutes and
keeps no list of its own; the env file and the page's old list survive only as
what a rig falls back on in front of a Paperless with no tags yet — a first
run, or a reinstall — and are replaced the moment Paperless has any. The rig's
page shows the shelf read-only and says where to change it. The one thing it
still decides is which folder is the bin, because a Paperless tag cannot say
that; adopting keeps it.

### And a letter is one piece of paper

The same look found `guess` returning *every* folder a letter matched, which
the panel and the preview then both showed. A document can carry two tags and
Paperless is right to give it two. The shelf cannot: the paper goes in one
place, so naming two is handing back the decision the rig exists to make.

`folders::rank` counts how many of each folder's words stand in the text and
orders by it. A letter carrying five of Taxes' words and one of Rechnungen's
is a tax letter. Ties keep shelf order, so the answer does not move about
between readings of the same page. Paperless's own answer is put through the
same ranking, so when it tags a document twice the display names the best fit
and mentions the rest underneath.

### Merging the two shelves

The drift had to be undone before the new rule could take over, and the union
of the two lists did not fit: Paperless keeps 256 characters of a tag's match,
and `Lawstuff` wanted 29 words. What made it fit was noticing the lists were
arguing. Four debt-enforcement words — Mahnbescheid, Inkasso,
Gerichtsvollzieher, Vollstreckungsbescheid — were in both `Lawstuff` and
`Inkasso/Rechnungen`, and five employment words in both `Personal` and `Work`.
Putting each where it belongs freed exactly the room `Lawstuff` needed for
both of its subjects, criminal procedure and lawyers/courts/inheritance, at
nineteen words.

### Verified

`apps/scannerd/tests/folders.rs` ranks a tax letter that is also a Mahnung
above Rechnungen on word count, and holds ties to shelf order. The scanner's
own suite covers the rest.

Against the real shelf: the Wasserschutzpolizei letter reads as `Lawstuff`,
the Finanzamt letter as `Taxes` first and `Inkasso/Rechnungen` second — which
is what it is. Both sides now hold the same twelve lists.

## 34. The folder is decided once, while the paper is in your hand

**2026-09-24.**

§33 made the shelf one list and had a letter land in one folder. It still had
the folder decided twice: the rig guessed from its own tesseract reading of a
photograph, and a minute later Paperless matched the same tags against its own,
much better OCR, and that answer replaced the guess.

Which is wrong, and the reason is not accuracy. **The paper is already in a
drawer by then.** Somebody read the folder off the panel, walked to the shelf
and filed the letter. A record that changes its mind afterwards does not
correct anything — it makes the record disagree with the room, and the whole
point of keeping the record is to find the paper again.

So the decision is made once, at the moment the letter is finished, from what
the rig has read: `Scanner::closed` settles it and writes it beside the letter
in the spool, as a `.folders` sidecar next to the `.attempts` one. Beside it
rather than in memory because a letter can wait overnight for a Paperless that
is down, and the paper was filed last night. The upload names those tags on
the document, and **Paperless no longer matches folder tags at all**: they are
saved with `matching_algorithm` set to none.

The words stay on the tags, which is the part worth being careful about. They
are not there for Paperless any more; they are what the rig reads a page
against, and the tag is where they are edited and where both halves read them
from. So "a folder" can no longer be recognised by Paperless matching it —
a tag with words of its own is a folder, and a tag without any is a label
somebody puts on by hand, the address among them.

The cost is real and worth stating: the decision is made on a photograph read
by tesseract, which is worse text than Paperless's OCR of the same page. A
letter will occasionally be filed somewhere a better reader would not have
filed it. That is a worse folder, once; the alternative is a record that is
reliably wrong about where the paper is, for ever.

### Finding the paper again

An open letter in kuverta now says which folder it is in, under its subject —
a pill with the folder's name, and who it is for beside it. The address label
is left out: it is on every letter of a postbox and says nothing about where
this one is. It is the most practical fact on the screen, because it is the
only one you can act on with your hands.

### Verified

`apps/scannerd/tests/folders.rs` checks the tag is written with matching off
and its words kept. `kuverta-bird/e2e/postbox.test.js` opens a letter in a
browser and finds one folder pill and the person it is for, read apart.

On the rig: the twelve tags were switched to no matching and it still took all
twelve — `folders taken from Paperless count=12` — because it now knows a
folder by its words. The bin and both people came through.

## 35. A parcel notice, and whose fault the mess was

**2026-09-24.**

A DHL parcel notice read as four hundred lines of `<table border="0"
cellpadding="0">`, Outlook conditional comments and a page of tracking URLs
with a sentence of German scattered through it. The obvious reading is that
kuverta's HTML rendering is bad.

It is not. The message was pulled out of the store and looked at: **that is
DHL's own `text/plain` part**, markup and all. Their converter gave up halfway
and kuverta was showing faithfully what arrived. Nothing in the client was
wrong; it was just too trusting.

So the rule is now: a plain part wins, unless it is visibly the wreckage of a
converter — two or more of `<table`, `<td`, `<!--[if`, `<div`, `style="`,
`cellpadding` — and then the HTML is read here instead. Deliberately narrow:
a plain part is usually written on purpose and taking the HTML over it is a
step down far more often than up. On the real notice: 605 lines to 33.

### What the reader is, and is not

It is not a browser and must never become one. It takes a string and returns a
string. `<script>`, `<style>`, `<noscript>` and `<head>` are skipped unread;
`src` and `href` are text and are never opened; no input can make it reach the
network or the disk. There is a test that a tracking pixel, an `onclick`, a
`javascript:` href and an iframe all leave no trace in the output.

Every limit is a **refusal**, not a truncation. Too big, too deep, an
unterminated comment, script or tag: `to_text` returns `Refused` and the
caller keeps the sender's own text. A half-parsed body is the one outcome
worth avoiding, because it is indistinguishable from a well-parsed one to
whoever is reading it — a message that can silently drop the second half of a
sentence is worse than one that admits it could not be read. There is no
recursion, so no input can exhaust the stack; depth is counted and refused at
a hundred.

Two things it got wrong before the real message was put through it, both of
the kind worth writing down:

- **A tag does not end at the next `>`.** DHL writes `alt="Ablageort<br>
  buchen"`, and stopping there ended the tag inside a quoted value and spilled
  the rest of the attribute — style declarations included — into the message
  as words. Quotes are tracked now.
- **An attribute's value is text, so markup in it is taken out**, and a
  picture whose words repeat the words beside it is read once. Every button in
  the notice is an icon and a label inside one link with the same words in
  both, so it came out as "[Ablageort buchen] Ablageort buchen" — over two
  lines, because the label is written with a break through the middle of it.

### The scanner's own reading

Measured rather than guessed, on two of the rig's real pages: tesseract's
default page segmentation against `--psm 4` (a column of text, which is what a
letter is) and `--psm 6` (one uniform block). `--psm 4` won both — fifteen
recognised German words against thirteen, with a quarter less of the
punctuation-soup that marks a misread line, and nine against eight on the
second page. `--psm 6` finds many more "words" and no more real ones: it is
reading the noise between the columns. Upscaling the page first made it worse,
which is worth knowing — tesseract already does its own.

So `--psm 4` is the default, and **Read it again** in the preview tries the
other one. That button is worth having only because it does something
different: tesseract is deterministic, and running the same reading twice
gives the same text.

### The border

The trim left seven per cent of one side as table. `keep_paper` was
all-or-nothing — it either cut to the edge it had found or kept everything out
to the frame — so a side whose found edge was short kept the lot. It walks
outward now, a step at a time, and stops at the first strip that is not paper.

What a strip is judged by matters: its **brightest quarter**, not its average.
A strip across a column of dense text is mostly ink and averages dark, and
judging by the average cut the right-hand column off a letter. What says a
strip is still paper is that the paper between its letters is as bright as the
page. The table has no bright quarter. That side is now half a per cent.

## 36. The paper is told it is white

A page on the rig's table comes out grey. Not badly: readable, in focus at
1:1, plainly a letter. But the paper photographs at about 186 of 255 with a
warm cast from the room, the ink at 54, and a fold across one corner where a
lamp leaves a gradient the camera cannot see past. It looks like a photograph
of a letter rather than a scan of one, which is what it is.

Tesseract reads it worse for the same reason a person does: there is less
between the ink and the paper than there should be. On the two most recent
pages off the rig — a baggage inspection form, creased, filled in by hand —
it found 97 and 74 German words of four letters or more. After the paper is
put at white it finds 214 and 211.

So `flatten` finds the paper and tells it it is white. In tiles about eighty
pixels across, because the light over an A4 sheet is never even and one white
point cannot take out a gradient; at the 92nd percentile of each tile rather
than its brightest pixel, which is a glint off a staple; and per channel,
because paper is neutral and the difference between its channels *is* the
colour of the light. Correcting each channel by its own paper level is a white
balance measured off the one thing in the frame whose colour is known. A blue
signature stays blue, and there is a test that says so.

It is a tone curve and nothing else. Nothing moves, nothing is sharpened,
nothing is denoised — every filter that would smooth the sensor's noise would
smooth the thin strokes of 8-point print too, which is the same reason the
camera is run with `--denoise cdn_off`. And every doubt leaves the picture
alone: a tile too dark to hold paper borrows the page's own level instead of
being multiplied by whatever its darkness suggests, a page with no ink on it
is not stretched (its darkest half percent is grain, and stretching against
that turns a clean sheet into static), and a picture too small to tile, or
whose claimed size does not match its pixels, comes back exactly as it went
in.

The field is smoothed, and past the grid's edges it is continued along its own
slope rather than repeated. Repeating averages the dim end of the page with
the brighter tiles inside it, so the field there reads brighter than the paper
is and the correction undershoots: a lamp at one side of a sheet still left
twenty levels between the two edges after it had supposedly been taken out.

### What it costs, and who has to be told

Flattening destroys the two numbers the rig steers its exposure by. That is
the point of it — every page that comes out has paper at white — and it would
have been the end of the rig ever correcting itself. `quality::exposure` reads
`paper` to decide whether the next page should be taken half a stop darker; a
rig judging its own flattened pages would see a perfect exposure every time,
walk the camera down to −4 EV one page at a time, and never learn otherwise.

So `Straightened` carries a `Tone`: what the page looked like before, and only
that. `Quality::exposed_as` puts it back and decides the two exposure problems
again — is there paper here, is it washed out — while leaving sharpness and
"is there any text" measured on the picture that will actually be sent, which
is the right picture to measure them on. A photograph with no paper in it is
still no paper whatever was done to it afterwards, so that verdict throws the
rest away rather than reporting on a stretched table.

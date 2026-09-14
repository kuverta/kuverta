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
node fuckbird/tools/dump-facts.js < corpus.jsonl > facts.jsonl
# and the same facts through core-rules
diff rust.tsv js.tsv
```

Identical on all 250 — the same category *and* the same confidence to six
decimal places. The facts are built once, in JS, and fed to both, so this
measures the classifier and not the header parsing.

Worth re-running after any change to `fuckbird/core/signals.js`. The harness for the Rust
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
python3 docker/fill-mailbox.py --dump 250 | node fuckbird/tools/classify-corpus.js --reasons
```

### Resolved: the corpus was real, and it is still on disk

**Later the same day.** The mailbox behind the brief's figures turned up in
`fuckmail`'s scratch store, as a second account with exactly 251 messages.
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

The brief assumes a line of succession: `fuckmail` is the thing that exists,
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
hosts/fuckmail/      an adapter over the client's Tauri commands
```

`fuckbird/core/host.js` is the seam. Message ids are opaque, folders are opaque, and
anything one host can do that the other cannot is **declared** in a
`capabilities` object rather than sniffed for. A core that starts asking "am I
in Thunderbird?" has stopped being a core, and `fuckbird/test/wiring.test.js` fails if
it ever does — it greps for `messenger.`, `__TAURI__` and `browser.` in
`core/`, and for imports reaching from one host into the other.

**What this buys.** The §7.1 question — does the fork ever happen — stops being
load-bearing. If it never does, the surface still runs in both. If it does, the
fork inherits a surface that already works.

**What it costs.** A contract to keep, and the discipline not to reach through
it. Both are cheaper than two implementations of the same list, which is what
existed before this: `fuckmail`'s `apps/desktop/ui/app.js` is 1,136 lines and
most of it is the list, the cursor and the key map written once already.

### The contract is a test, not a document

`fuckbird/test/support/conformance.js` is the host contract as a suite. Three adapters
run it: Thunderbird against a fake `messenger`, `fuckmail` against a fake
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

**A capability read too early reads as false.** `fuckmail`'s host has to ask
which folders are Archive and Trash before it can say whether it can archive.
The suite read `capabilities` synchronously, got `false`, and silently skipped
its archive and trash tests — passing, green, testing nothing. The suite now
awaits the host factory, and every capability gate reports a **skip** with a
reason rather than a pass, so a capability regression cannot hide as an `ok`.

### The two undos are different promises

`capabilities.undo` is `'queued' | 'compensating' | false`, never `true`.

`fuckmail` queues a change before sending it, so undo **cancels** something
that never happened — brief §3.5, "a change stays cancellable for as long as it
is queued". Thunderbird's write paths do not queue, so by the time undo is
offered the move has happened, another client may have seen it, and undo is a
second change that **puts it back**. The surface uses those words, in that
order of strength. Collapsing them into one boolean would have made the weaker
one look like the stronger.

---

## 5. `fuckmail` cannot file by category, and the surface says so

**2026-09-11.** A gap found by building the adapter.

`core-rpc` registers twenty commands. None of them sets a category: the
classifier runs during sync and nothing exposes a correction. So the
`fuckmail` host declares `setCategory: false`, the surface hides the category
keys there, and three conformance tests report as skipped rather than passing.

This is the capability object earning its keep on the first day it exists. The
alternative — an adapter that accepts the call and does nothing — is the worst
outcome available: the key is shown, the user presses it, and nothing happens
with no way to tell why. The contract has a test for exactly that
("a capability it does not have is refused, not quietly ignored").

[`fuckbird/hosts/fuckmail/readme.md`](../fuckbird/hosts/fuckmail/readme.md) has the command that closes it. Once it exists, one
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

`fuckbird/test/permissions.test.js` holds the API-to-permission table and checks three
things: that the manifest grants everything the code needs, that it grants
nothing it does not — a mail client asking for more than it uses is asking to
be trusted for no reason — and that every `messenger.*` call the code makes has
an entry in the table at all, so using a new API forces the table to be
updated rather than letting it rot into a snapshot of what was once true.

It is the assumption made explicit, not a second source of truth. If
Thunderbird disagrees with it, Thunderbird is right and the table is what gets
corrected.

---

## 7. Stage 3 was already built in `fuckmail`, and unreachable

**2026-09-11.** Supersedes §5.

§5 recorded that `fuckmail` could not file a message by category and declared
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
`fuckmail` against a contract that asks a host what it can do, and having to
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
- **An account picker**, in the `fuckmail` mount rather than in `core/` —
  which account is a `fuckmail` idea, not a mail idea. The store held two
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
   the doc comments in `fuckbird/core/category.js`: the comments explain the taxonomy to
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

Where a host cannot supply facts — `fuckmail`'s `message` command returns a
body and no headers — it shows the category and what that category means, and
stops. A weaker answer to the same question beats inventing reasons.

---

## 10. One repository

**2026-09-11.** Reverses the split that §4 was written inside.

`fuckbird` was a separate repository for about a day. Looking at the two side by
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

So `fuckbird` is now a directory, merged with `git subtree` rather than copied,
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

The contract. `fuckbird/core/` still may not name a host, and
`fuckbird/test/wiring.test.js` still fails if it does. Sharing a repository with
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
Paperless client in `fuckbird/core/` would have been the obvious symmetry. Two
things say otherwise. The desktop app's CSP is `default-src 'self'`, so its
webview cannot reach `localhost:8000` at all — the request has to go through
Rust regardless. And `core-rpc` is already the place "both front ends talk to",
built Tauri-free on purpose so it can double as the MCP surface: an agent
asking about post should ask the same layer a window does.

### What is not done

Post does not yet appear in the triage window beside mail, which is the brief's
actual promise. What exists is the client, the classification, and a CLI that
reads an address the way `fuckmail list` reads a mailbox — verifiable against
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
   `fuckmail disagreements` lists where they differ. Model verdicts are never
   shown — `CURRENT_CATEGORY_JOIN` has excluded them since §7 — so a model that
   is wrong for a month moves nobody's mail.
2. **Spike embeddings against prompting before committing.** Both exist:
   `PromptClassifier` asks a chat model for one word, `Neighbours` embeds a
   message and takes the nearest of the user's own filings. `fuckmail eval`
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
behind it — `fuckmail disagreements` on a real mailbox, and the user's own
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

`fuckmail eval` keeps the model's answer and the nearest filings for every scored
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

> **Since (2026-09-14):** §9's gap is closed. `fuckmail`'s message detail now
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

`fuckbird/test/view.test.js` mounts the real surface into jsdom over the
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
Playwright against `hosts/fuckmail/ui/triage.html` with a stand-in bridge would
catch it, at the cost of a browser download.

### The dependency

jsdom 24.1.3, pinned exactly, as the only dev dependency. Not the current
release: that one needs a newer Node than the 20.10 this was developed on, and
fails on import rather than at install. `make test-js` installs from the lock file
only when `node_modules` is missing, so an ordinary run stays offline.

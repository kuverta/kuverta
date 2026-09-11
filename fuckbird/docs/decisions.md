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
python3 tools/fill-mailbox.py --dump 250 > corpus.jsonl
node tools/dump-facts.js < corpus.jsonl > facts.jsonl
# and the same facts through core-rules
diff rust.tsv js.tsv
```

Identical on all 250 — the same category *and* the same confidence to six
decimal places. The facts are built once, in JS, and fed to both, so this
measures the classifier and not the header parsing.

Worth re-running after any change to `signals.js`. The harness for the Rust
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
python3 tools/fill-mailbox.py --dump 250 | node tools/classify-corpus.js --reasons
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

`core/host.js` is the seam. Message ids are opaque, folders are opaque, and
anything one host can do that the other cannot is **declared** in a
`capabilities` object rather than sniffed for. A core that starts asking "am I
in Thunderbird?" has stopped being a core, and `test/wiring.test.js` fails if
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

`test/support/conformance.js` is the host contract as a suite. Three adapters
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

`hosts/fuckmail/readme.md` has the command that closes it. Once it exists, one
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

`test/permissions.test.js` holds the API-to-permission table and checks three
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
   the doc comments in `core/category.js`: the comments explain the taxonomy to
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

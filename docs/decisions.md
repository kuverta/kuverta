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

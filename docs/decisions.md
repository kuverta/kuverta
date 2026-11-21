# Decisions

What was decided, when, and why, in order. The brief's §7 lists six open
questions. This records the ones that have closed and what closing them cost.

---

## 1. The classifier is ported to JS, not kept in Rust behind native messaging

**2026-09-11.** Closes brief §7.2.

The brief's staging decides this: stages 1 to 3 must run on stock Thunderbird
so that "if the fork never happens they still work". A native-messaging host
means shipping and installing a second executable, which makes the add-on hard
to hand to someone.

`core-rules` is now `extension/src/{category,signals,classify}.js`, a plain ES
module with no Thunderbird API in it, so the tests run under `node --test`
without a mail client.

Cost: two implementations of the same rules, which will drift unless something
checks (see §2 below).

Still open: the classifier is a pure function over headers, so moving it behind
a native-messaging host later only changes the transport. The rule ids
(`header.list_id`, `subject.marketing`, …) are identical to the Rust ones, so a
corrections log collected under one is readable by the other.

### The port is verified, not assumed

The twelve tests in `core-rules/tests/classify.rs` were carried over one for
one. Those only cover what the assertions look at, so both implementations were
also run over the same 250 messages and diffed:

```
python3 docker/fill-mailbox.py --dump 250 > corpus.jsonl
node kuverta-bird/tools/dump-facts.js < corpus.jsonl > facts.jsonl
# and the same facts through core-rules
diff rust.tsv js.tsv
```

Identical on all 250: same category, same confidence to six decimal places.
The facts are built once, in JS, and fed to both, so this measures the
classifier only, without header parsing.

Re-run after any change to `kuverta-bird/core/signals.js`. The Rust half of the
harness is throwaway (a few lines depending on `core-rules` by path) and is not
kept here, because that would put a Rust toolchain in the loop for a JS add-on.

---

## 2. The brief's measured distribution does not reproduce

**2026-09-11.** Amends brief §3.2.

The brief reports, on 251 messages: newsletter 77, transactional 62,
notification 61, personal 33, marketing 18.

`core-rules` over the 250 messages `fill-mailbox.py --dump` produces today
gives: newsletter 79, transactional 62, notification 59, personal 32,
marketing 18.

Newsletter and notification differ by two in opposite directions. One extra
message cannot explain that. The port is not the cause either: Rust and JS
agree message for message, and every sender classifies with a clear margin. So
the brief's figures came from a different corpus.

Nothing is broken, but do not cite the brief's number as a baseline. The
reproducible command is:

```
python3 docker/fill-mailbox.py --dump 250 | node kuverta-bird/tools/classify-corpus.js --reasons
```

### Resolved: the corpus was real, and it is still on disk

Later the same day. The mailbox behind the brief's figures is in `kuverta`'s
scratch store, as a second account with exactly 251 messages. The store's own
query over it gives:

```
newsletter 77 · transactional 62 · notification 61 · personal 33 · marketing 18
```

The brief's number was a correct measurement of one mailbox: `fill-mailbox.py`
output uploaded over IMAP by an earlier run, plus one real provider message, a
`no-reply@systemli.org` notice, which is the 251st.

The difference is corpus composition only. That mailbox by sender, compared
with `--dump 250`:

| sender | in the brief's mailbox | in `--dump 250` |
|---|---|---|
| angebote@mediamarkt | 25 | 27 |
| sec@news.heise | 21 | 22 |
| hello@news.rustweekly | 19 | 18 |
| newsletter@golem | 12 | 12 |

Every sender is filed under the same category in both. A different random draw
gave two fewer newsletters and two more notifications. Nothing to fix.

The figure describes one mailbox, so it is still no baseline.
`npm run corpus` is the reproducible one.

### The weakness the brief names is bigger than it says

§3.2 notes that "bulk marketing carrying `List-Id` and `Precedence: bulk` reads
as a newsletter to the rules". On this corpus that is 27 of the 79 newsletters,
all from one sender whose subject line ("Nur heute: 20% auf alles") contains no
keyword the rules know. `--reasons` shows that all 79 verdicts rest on
`header.list_id` and `header.precedence` alone.

This is the best-evidenced target for the model of §3.3 and the strongest
argument for the corrections log: one correction on that sender fixes 27
messages, which keyword tuning would not.

---

## 3. MV2, not MV3

**2026-09-11.** Not in the brief.

The add-on is Manifest V2 with a background page. A background page loads ES
modules, so `extension/src/` is shared verbatim between Thunderbird and the
Node tests. Under MV3's background scripts that sharing needs a build step
between the tested code and the shipped code.

Thunderbird 128 and 140 ESR both support MV2. This will have to move
eventually. When it does, the tests must still run the same files the add-on
does.

---

## 4. One core, two hosts, and a contract between them

**2026-09-11.** Not in the brief, and it changes the brief's shape.

The brief treats Thunderbird as the successor to `kuverta`, and §7.6 asks what
to do with the old repo: "archive it as a reference, keep it as the
native-messaging backend, or keep running it alongside".

Decision: the product is the triage surface and the classifier. `kuverta` and
Thunderbird are both hosts.

```
core/     the classifier, the triage model, the key map, the surface
          — no Thunderbird, no Tauri, no DOM in the model
hosts/thunderbird/   an adapter over the MailExtension API
hosts/kuverta/      an adapter over the client's Tauri commands
```

`kuverta-bird/core/host.js` is the seam. Message ids and folders are opaque.
Anything only one host can do is declared in a `capabilities` object, never
sniffed for. `kuverta-bird/test/wiring.test.js` enforces this: it greps for
`messenger.`, `__TAURI__` and `browser.` in `core/`, and for imports reaching
from one host into the other.

Benefit: the §7.1 question (does the fork ever happen) no longer matters. The
surface runs in both hosts either way, and a fork would inherit it.

Cost: a contract to keep. That is cheaper than two implementations of the same
list, which existed before: `kuverta`'s `apps/desktop/ui/app.js` is 1,136
lines, mostly the list, the cursor and the key map.

### The contract is a test, not a document

`kuverta-bird/test/support/conformance.js` is the host contract as a suite.
Three adapters run it: Thunderbird against a fake `messenger`, `kuverta`
against a fake `invoke`, and an in-memory reference that proves the contract
can be satisfied at all.

It caught two things:

Message ids do not survive a move. Thunderbird issues a new id for a moved
message, and archive and trash are both moves. The first draft required a
trashed message to stay readable by its id, which no Thunderbird adapter could
satisfy. Now ids are good until the message is acted on, and the surface
reloads after every action. To undo, the Thunderbird adapter finds a moved
message by its `Message-ID`. It refuses with an error when the message has
none (legal per §3.6), because moving the wrong message back is worse.

A capability read too early reads as false. `kuverta`'s host has to ask which
folders are Archive and Trash before it knows whether it can archive. The suite
read `capabilities` synchronously, got `false`, and skipped its archive and
trash tests while reporting green. The suite now awaits the host factory, and
every capability gate reports a skip with a reason, never `ok`.

### The two undos are different promises

`capabilities.undo` is `'queued' | 'compensating' | false`, never `true`.

`kuverta` queues a change before sending it, so undo cancels something that
never happened (brief §3.5, "a change stays cancellable for as long as it is
queued"). Thunderbird's write paths do not queue. The move has happened and
another client may have seen it, so undo is a second change that puts it back.
The surface names the two differently. One boolean would have made the weaker
undo look like the stronger.

---

## 5. `kuverta` cannot file by category, and the surface says so

**2026-09-11.** A gap found by building the adapter.

`core-rpc` registers twenty commands and none sets a category. The classifier
runs during sync and nothing exposes a correction. So the `kuverta` host
declares `setCategory: false`, the surface hides the category keys there, and
three conformance tests report as skipped.

An adapter that accepted the call and did nothing would be worse: the user
presses a visible key and nothing happens, with no explanation. The contract
tests for that ("a capability it does not have is refused, not quietly
ignored").

[`kuverta-bird/hosts/kuverta/readme.md`](../kuverta-bird/hosts/kuverta/readme.md)
has the command that closes the gap. Once it exists, one boolean changes here
and the keys appear.

---

## 6. What a documentation pass found before the first load

**2026-09-11.** Checked against the Manifest V2 API reference and the
MailExtensions supported-API list.

The adapters were written from the documented API and tested against fakes,
which answer whatever they are asked. So before the first load, all twenty-five
API calls the Thunderbird adapter makes were checked against the documentation.

### Four missing permissions

Each would have failed at runtime, three of them silently.

| Missing | Needed by | What would have happened |
|---|---|---|
| `messagesMove` | `messages.move` | archive and trash fail, the two most-used keys |
| `messagesTagsList` | `messages.tags.list` | `ensureTags` throws on startup, so no tags are ever created |
| `compose` | `compose.beginNew` | `c` does nothing |
| `tabs` | `tabs.query({url})` | the query returns nothing, so a second triage tab opens every time |

`tags.create` needs `messagesTags` and `tags.list` needs `messagesTagsList`.
Having one made it look as though both were covered.

Without `tabs` the call does not throw, it returns an empty list, so the code
would have opened another triage tab on every click.

### Three assumptions that were wrong

`MessageHeader` carries no attachment information. `toRow` was reading
`header.attachments?.length` and `header.hasAttachments`, and neither exists in
any version. The real answer costs either a `listAttachments` per message
(thousands of round trips for an icon) or a second `query({attachment: true})`
per scope, which doubles the cost of the operation already named as this
adapter's scaling limit. So the value stays false, with a comment saying why,
and the list shows no paperclip in Thunderbird. The classifier is unaffected:
it reads attachments off the part tree when a message is opened.

Header keys were matched by exact spelling. `getFull` promises "the header name
as key" and Thunderbird lowercases in practice, but the lookup asked for
`list-id` and would have missed `List-Id`. A missing header does not fail, it
silently switches off a signal. `firstHeader` now matches case-insensitively.

`menus.onClicked`'s `selectedFolder` is deprecated, replaced by
`selectedFolders` in TB 128, the minimum version targeted. Both are read now.

### Three assumptions that were right

These were guesses and are now confirmed:

- `QueryInfo.tags` really is `{mode: 'all' | 'any' | 'none', tags: {key: boolean}}`.
- `messages.query` resolves to a `MessageList` with `id` and `messages`, and
  `folderId` (TB 121) and `headerMessageId` (TB 85) are both accepted. Both are
  older than the 128 minimum.
- Message ids do not survive a move. The docs say an id "does not follow an
  email that has been moved to a different folder", which is the behaviour the
  contract in §4 above was designed around.

### The table is now a test

`kuverta-bird/test/permissions.test.js` holds the API-to-permission table and
checks three things: the manifest grants everything the code needs, it grants
nothing the code does not use, and every `messenger.*` call in the code has an
entry in the table. A new API call therefore forces a table update.

If Thunderbird disagrees with the table, correct the table.

---

## 7. Stage 3 was already built in `kuverta`, and unreachable

**2026-09-11.** Supersedes §5.

§5 declared `setCategory: false` because `kuverta` could not file a message by
category. The symptom was right, the cause was wrong.

The machinery existed: `correction` as a table, `record_correction`,
`learned_categories`, and at the top of `sync_account`, `load_history` folding
those corrections into a `core_rules::Learned` and handing it to the
`Classifier`. It was tested end to end, including against the dev IMAP server.

But `record_correction` was called only by tests. There was no CLI command, no
RPC method and no path from any interface. Stage 3's gate ("filing once changes
what happens next time") was met by the code and unreachable by a user.

Writing the `kuverta` adapter against a contract that asks a host what it can
do found this. The Thunderbird adapter alone would not have.

### What the exposure cost

More than a command, because of where the list reads a category from.

`ClassifierSource::User`. The list joined `source = 'rules'`, so a correction
stored as a user decision would not have shown until the next sync
re-classified. Recording corrections as rules verdicts was one line shorter,
but the rules-versus-model comparison of brief §3.3 reads the `rules` rows, and
the user's answers in them would make the baseline unbeatable and meaningless.
So corrections are a third source, and `disagreements` still reads `rules`
only.

`CURRENT_CATEGORY_JOIN`. One definition of "the category a message is shown
under": the user's if there is one, else the most recent rules verdict, never
the model's. The three queries that must agree on it share it. The model is
excluded because changing what the list shows would spoil §3.3's measurement.

A counting bug, found on the way. `category_counts` joined every classification
row for a message. `COUNT(DISTINCT m.id)` stopped that inflating a single
category, but a message reclassified between syncs was counted under every
category it had ever been given, so the sidebar could add up to more messages
than the mailbox held. The shared join fixes it, and a test now checks that the
totals equal the message count.

### What it bought

All three adapters now declare every capability. That left the contract's
"a capability it does not have is refused, not quietly ignored" test and the
skip machinery running against nothing. So there is a fourth reference host,
`ReadOnlyMailbox`, which declares almost nothing and refuses what it declared
it could not do. Most of its conformance run is skips. It exercises the path a
future host will take, for example an account with no Archive folder.

---

## 8. Keyboard-first is not the same thing as undiscoverable

**2026-09-11.** The first time anyone opened the triage surface.

The report was: *"I opened triage but as a user I actually have no clue what I
am supposed to do."*

The list was working: eight messages, all classified, five categories. But
every action was a key, no key was written down, and the only hint was a `?`
that you had to already know about.

That misread the brief. §3.1 asks for a list you act on from the keyboard and
does not ask to hide the keys. The surface also failed the section's other
rule, "a filtered list says what it is showing": `drawScope` hid the scope bar
unless the list was filtered, so an unfiltered list announced nothing.

### What changed

- A legend along the bottom, always on screen. `j k` move, `↵` read, `e`
  archive, `#` trash, `1-6` file, `u` unread, `z` undo, `/` search, `?` more.
  `?` now covers the rest.
- The scope bar is always shown, saying what the list holds and how many. The
  way out still appears only when the list is filtered.
- The empty list says why it is empty and what to do. A silent empty list looks
  like mail has gone missing.
- The reading pane says `Press ↵ to read this message` when a row is selected.
  A highlighted row beside a pane reading "select a message" looks like the
  selection did not take.
- The surface says which mailbox it is. `mountTriage` takes a `title` the host
  supplies, because the core does not know the account.
- An account picker, in the `kuverta` mount and not in `core/`, because
  accounts are a `kuverta` idea. The store held two accounts and the surface
  silently opened the first, the eight-message test one.

### And a bug the picker exposed

`mountTriage` returned a teardown that removed only the key handler. Mounting
twice, which an account switcher does, would have left the previous surface
subscribed to its model and listening on `window`, so every keystroke would
have acted twice, once on a list nobody could see. With `e` and `#` bound to
archive and trash, a message the user never looked at would disappear. The
teardown now also removes the resize listener and the subscription.

It was found while writing the picker, by asking what the old mount still held.
None of the 181 tests would have raised it, because none can open a window.

---

## 9. The surface has to say what it is for, not just how to drive it

**2026-09-11.** The first user's second comment, once the keys were on screen:
*"I think it's working, but it should be explained what triage is actually
doing."*

§8 fixed how to drive it. This is about what it is: six undefined words down the
side, a list sorted by something the window never named, and no hint that
pressing `1` teaches it something. The explanation existed only in a readme in
another repository.

### What the panel says

Shown the first time the surface opens, and on `?` afterwards. Three things, in
this order:

1. What it does. Every message is read for a handful of headers and filed under
   one of six categories. Nothing moves and no folder changes. The category is a
   label and the list is sorted by it.
2. What the six mean, one line each. `CATEGORY_MEANINGS` sits beside the doc
   comments in `kuverta-bird/core/category.js`.
3. That correcting it teaches it. `1` to `6` remembers the sender or the list,
   so the next message like it is filed the same way. Nothing else reveals this.

`localStorage` remembers that the panel has been seen, wrapped in try/catch both
ways. If storage is unavailable or full, the panel just opens again.

### And the per-message half, which the brief already asked for

§3.2 says the rules layer earns its keep by being able to say why, "which is
what makes a wrong answer correctable instead of infuriating". The Thunderbird
message-header popup did that from the first commit. The shared triage surface
did not.

The reading pane now shows the category and the signals that produced it. The
reasons are recomputed from the facts the host hands back. They are not stored,
because a cached explanation can go stale.

They are computed without the corrections history, so when the filed category
and the rules disagree the surface can say so: *"You filed this as
Transactional. On the headers alone the rules would have said Marketing."*

Where a host cannot supply facts (`kuverta`'s `message` command returns a body
and no headers), it shows the category and its meaning, and no reasons.

---

## 10. One repository

**2026-09-11.** Reverses the split that §4 was written inside.

`kuverta-bird` was a separate repository for about a day. §4 had already
concluded that the product is the triage surface and the classifier, and that
the Rust client and Thunderbird are both hosts. A repository boundary between
the surface and one host contradicts that.

The split had costs. The brief existed twice, in two states, with the second
copy's links rewritten because they did not resolve. The install script existed
only to carry files across the boundary. The fix in §7 (exposing corrections)
touched the Rust store, the Rust RPC, the Tauri command and the JavaScript
adapter: one change in two histories.

`kuverta-bird` is now a directory, merged with `git subtree`, so its seven
commits stay in the log, interleaved with the Rust ones in order.

### What that tidied

- One brief. The annotated copy survives as `docs/thunderbird-fork-brief.md`,
  and the links that had been flattened to code spans resolve again.
- One decisions log, this file, covering both halves.
- `make test-js`, and `make test` now runs both suites.

### What stays copied, and why

`make triage-ui` still copies the surface into `apps/desktop/ui/`. Tauri serves
exactly one directory as the web root and the page can reach nothing outside it.
The copy is verbatim and verified with `diff -r`, so it is not a build step. The
Thunderbird side does the equivalent by putting the manifest where `core/` is
already inside the extension root.

### What this does not change

The contract. `kuverta-bird/core/` still may not name a host, and
`kuverta-bird/test/wiring.test.js` still fails if it does. The rule matters more
now: the Rust client is a directory away, and reaching into it would work.

---

## 11. A physical address is an account

**2026-09-11.** Brief §4.2, and a reframing of it.

The brief describes the paper half as a pipeline: `scannerd` captures,
Paperless-ngx does OCR and archiving, and "the fork shows documents and mail as
one item type in one list". The model behind it:

**A physical address is an account, and Paperless is its server.**

Post that arrives at it is mail. The correspondent wrote it, the title is the
subject, the OCR text is the body, the date on the letter is when it arrived.
`core-paper` is the counterpart of `core-proto`: a client for REST where the
other speaks IMAP.

### What that framing buys immediately

The classifier needs no teaching. `Document::facts()` returns the same
`MessageFacts` the IMAP path builds, so post is filed by the rules that file
mail. There is no paper-specific classifier and no second taxonomy.

> **Correction (2026-09-12):** an earlier version of this entry said a
> correction made on a letter teaches the same table a correction on an email
> does. That is the intent and not yet true. Corrections made on mail do reach
> post: `paper_classifier` loads every account's learned overrides, so a
> correspondent filed once on the mail side is filed the same way on paper. The
> reverse does not work: `correction` hangs off a message row and a document is
> not one, so filing post by hand needs its own table. Until then the surface
> refuses with a message.

Several addresses, one instance. A selector (a Paperless tag, correspondent or
storage path) separates a home from an office. It is a Paperless concept, so an
address is set up by tagging in Paperless and is not configured twice.

### Two mappings worth arguing about

The correspondent doubles as the address. Paper has no `From`, and the learned
overrides need something stable to key a correction on. "Everything from the
Stadtwerke is transactional" is the rule a person wants to teach.

A document always counts as carrying an attachment, because it is one: a PDF.

### The client is in Rust, not in the shared surface

The triage surface is JavaScript and reaches its host through a port, so a
Paperless client in `kuverta-bird/core/` would have been symmetrical. Two
reasons against. The desktop app's CSP is `default-src 'self'`, so its webview
cannot reach `localhost:8000` and the request has to go through Rust anyway. And
`core-rpc` is already the layer both front ends talk to, built Tauri-free so it
can double as the MCP surface. An agent asking about post should ask the same
layer a window does.

### What is not done

Post does not yet appear in the triage window beside mail, which is what the
brief promises. What exists is the client, the classification, and a CLI that
reads an address the way `kuverta list` reads a mailbox, verifiable against the
dev stack today. The shapes already match, so the merged list is a display
problem.

Also open: where an address's configuration lives. The CLI takes it as arguments
and environment. The window needs it stored: a table and a keychain entry for
the token, alongside the accounts.

---

## 12. A row may allow less than its host

**2026-09-12.** Found by the host contract when post joined the list.

`capabilities` answers "what can this host do". That was enough while every row
was the same kind of thing. With post from Paperless beside mail, the host can
archive but that row cannot, because Paperless owns the document and this only
reads it.

The contract test `undo unwinds one change at a time` archives the first two
rows of the list. One was now a letter, and the adapter refused. The port could
not express a row that allows less than its host.

`Row.actions` is now an optional list of what a row permits. Absent means
"whatever the host can do", so earlier adapters stay correct unchanged.
Documents declare `[]`.

Three consequences:

- The model checks before it asks. `Triage.act` refuses a disallowed row itself,
  without a round trip, and the message names the row: *"this cannot be archived
  from here"*.
- The contract picks a row that allows the action (it used to take whatever
  sorted first). New rule: a row that declares it allows nothing must refuse
  loudly.
- The adapter says no in one place. Paper ids are namespaced
  (`paper:<address>:<document>`), so one `parsePaperId` guard at the top of each
  write is enough.

This is the second gap in the port the contract caught. The first was message
ids not surviving a move (§4). Both were found by adding a host and fixed in the
port.

---

## 13. `scannerd` is small because it refuses to do most of the job

**2026-09-12.** Brief §4.2, and the plan's month 6 to 7.

The plan lists the capture daemon as "page detect → capture → deskew → crop →
upload with offline spooling" and budgets ~600 lines. Three of those five steps
are not this program's work.

Deskew is `ocrmypdf`'s. Paperless runs it on every document. The dev stack now
sets `PAPERLESS_OCR_DESKEW` explicitly, so the dependency is visible.

Cropping is a setting. The rig's camera is fixed above a fixed spot, so the page
is always in the same part of the frame. `rpicam-still --roi` crops on the
sensor: one line of configuration. Perspective geometry could not be tested
against real photographs, which decided it.

Page detection is arithmetic. `--encoding yuv420` returns raw YUV whose first
plane is the greyscale image, so there is no JPEG to decode and no image library
in the binary. "Is there a page" is "what fraction of pixels differ from the
empty desk by more than a threshold".

So the detector is tested with synthetic frames on a laptop. The threshold gets
tuned on the device. The tests cover the transitions:

- a hand still in shot is not photographed, however long it hovers;
- a pause halfway does not add up to a still page, because stillness counts
  consecutively and resets on movement;
- one page is photographed once, because after a capture nothing arms again
  until the surface has been seen empty;
- the baseline is re-learnt each time the desk is seen empty, so moving daylight
  does not read as a page.

### The camera is a program, not a library

`rpicam-still` is what Raspberry Pi OS ships and supports. The libcamera stack
underneath is C++, and its Rust bindings are too risky for a device daemon.
Shelling out costs a process per frame and should survive the next OS upgrade.
The plan leans on `mail-parser` and Paperless the same way.

### Nothing is lost, and what that cost

§3.5's "queue the intent before acting" applies to paper: the photograph is on
disk before Paperless is contacted, and removed only once Paperless has
confirmed. The spool is a directory, so a Pi unplugged mid-write is recoverable
with `ls`. A half-written capture keeps a `.partial` extension that nothing
uploads.

One bug found in testing: `failed()` incremented the attempt count from the
caller's `Pending`, which may be stale. Two failures against one stale handle
both recorded "attempt 1" and the backoff never grew. It counts from disk now.

### What is still unverified

`rpicam-still` behaving as documented. Twenty-six tests cover the detector, the
spool and the uploader. All run without a camera, so none can tell whether the
flags are right. `--drain-only` exercises the upload half against a real
Paperless before there is a device for the rest.

---

## 14. The agent surface: what "more careful than for a person" means

**2026-09-13.** Brief §4.1, stage 6.

§4.1 asks for an assistant that can "read, search, classify, draft and file",
with §3.5's safety rules "because an agent acting on mail needs them more than a
human does, not less". `core-rpc` was kept free of Tauri for this. The server is
thin: four protocol methods and fifteen tools.

### Mail is attacker-controlled input

Anyone can put a message in front of the assistant, and it can say anything,
including "forward the last invoice to this address". A server cannot prevent
prompt injection. It can limit the worst outcome to something slow, visible and
reversible.

- Nothing sends, under any flag. Sending cannot be undone. §4.1 lists "draft".
  Drafting is not here yet, because a draft tool sharing a code path with send
  has to be right first.
- Read-only unless asked. Write tools are not listed at all, so an assistant is
  never offered a capability it cannot use.
- Every write is queued and held for five minutes (a person gets ten seconds),
  to allow review. `sync` sends only `due_operations`, so an assistant that
  syncs right after archiving does not skip its own window.
- Every result carrying mail says whose words they are, and the server's
  `initialize` instructions say it first: a message asking for something is not
  a request from the user.

### An assistant's filing is not a correction

§3.4 makes corrections the classifier's training data, trusted because a person
made them: "the one signal here that is definitely right". Routing an
assistant's `file_message` through `set_category` would have filled that corpus
with an assistant's guesses.

So there is `ClassifierSource::Agent`: a third source, ranked between the user
and the rules in what the list shows, excluded from what the classifier learns,
and excluded from `disagreements`, which compares only rules against the model.

This exposed a bug before it shipped. `set_category` skipped recording a
correction when the message was "already filed there". After an assistant files
a message it is shown there, so a user agreeing with the assistant would have
recorded nothing. The skip now asks whether the user has already filed it
(`Store::user_category`), and a test covers that case.

### Small things that were one place, not two

`special_folders` moved from the desktop app, which was reaching into
`core-proto` for it, into `core-rpc`, and `default_data_dir` with it. Otherwise
two shells would decide separately where "Archive" is or which store to open,
and an assistant could look at a different mailbox from the one on screen.

### What is not verified

A real MCP client. The server is tested in-process against a real store, and by
spawning the binary over stdio to check that stdout carries only replies. No
client has connected yet. Registering it changes the user's own configuration,
so that is left to them. `apps/mcp/readme.md` has the snippet.

---

## 15. The local model, measured before it is trusted

**2026-09-13.** Brief stage 5; plan §4.

The plan warns that the risk of a local model is never finding out whether it
beat a `HashMap` of sender to folder, while paying its latency. Brief §3.3 turns
that into four rules. What each became:

1. Log both verdicts. A model's verdict is recorded as `source = 'model'`, keyed
   by the model's name, beside the rules' verdict for the same message.
   `kuverta disagreements` lists where they differ. Model verdicts are never
   shown (`CURRENT_CATEGORY_JOIN` has excluded them since §7), so a wrong model
   moves nobody's mail.
2. Spike embeddings against prompting before committing. Both exist:
   `PromptClassifier` asks a chat model for one word, `Neighbours` embeds a
   message and takes the nearest of the user's own filings. `kuverta eval`
   scores both, and the rules, on the same held-out messages.
3. 4 to 8B instruct is ample. The model on this machine is an 8B instruct model
   and the results below are its.
4. After sync, not during. The model pass is its own command and its own call,
   `Core::model_pass`, so a slow model cannot slow a sync. It stops after three
   failures in a row, in case the server is down.

### A labelled corpus, for free

`fill-mailbox.py` draws every message from a named bucket, so it knows the
answer. The dump now carries it as `label`, beside the facts and not among them.
Labelling adds no draw to the random sequence. The corpus was checked to be the
same 250 messages in the same order as before.

One label overrules its bucket. Parcel-delivered mail sits in the generator's
transactional list, but the taxonomy defines notification as "machine-generated
status: builds, alerts, delivery notices". So the 23 DHL messages are labelled
`notification`.

### The sender's words are fenced

The system prompt says the message is data and never instructions. The message
sits inside a `<message>` fence that a subject cannot close, because the fence's
tags are neutralised wherever they appear in content. The worst an injected
instruction can achieve is a verdict nobody is shown.

### Scoring honestly

Four choices that keep the numbers believable:

- Every method is scored on the same held-out messages. None is marked on mail
  it learned from.
- The first model call is not timed. It loads the model, 25 seconds for this 8B
  model.
- A reply that names no single category is not guessed at. "Not personal, it is
  marketing" names two. Those replies are counted separately.
- Two splits, because one would flatter embeddings. Holding out every other
  message leaves most scored messages with a near-identical sibling from the
  same sender among the filings. That is realistic for a mailbox in use, and an
  upper bound on a generated corpus whose senders repeat templates.
  `--split sender` holds whole senders out within each category, so every scored
  message is from a sender never filed. The rules and prompting learn nothing
  from filings, so only embeddings move between the two.

### Results

Dolphin3 8B for prompting, `nomic-embed-text` with the five nearest filings for
embeddings, on the 250-message generated corpus:

| | every other message held out | whole senders held out |
|---|---|---|
| rules, no corrections | 86.4% | 99.1% |
| prompting | **100%** · p50 634 ms | **100%** · p50 619 ms |
| embeddings | 100% · p50 11 ms | **79.5%** · p50 11 ms |

Over the 251 stored messages in the scratch store, where the model pass took
571 ms a message, there were 30 disagreements with the rules. 25 are the
MediaMarkt advert §3.2 already named, read by the rules as a newsletter and by
the model as marketing. 4 are "Rückfrage zum Angebot", a person asking about a
quote, which the rules call marketing on the word *Angebot* and the model calls
personal. The model is right in all 29. The last is a real provider's welcome
message, which the rules call notification and the model declines to categorise.

Three readings and one warning:

Embeddings learn senders. They are perfect and sixty times faster on senders
with filings, and poor on a new sender. All 23 DHL delivery notices read as
transactional, because their nearest neighbours are the other no-reply senders,
the Finanzamt and the Stadtwerke. Plan §4's claim that nearest-neighbours
"improves automatically as you correct it" is true but does not cover the first
message from anyone.

Prompting holds on both splits. It is the only method unaffected by a new
sender.

The rules' two numbers cannot be compared with each other. Holding senders out
put MediaMarkt in the training half, so the rules' one blind spot was never
marked. Comparisons are valid within a column only.

The warning: the corpus is generated from a handful of templates, so 100% on it
is no forecast for real mail. It does show that the model is no worse than the
rules anywhere tested, is right about both of the rules' known weaknesses, and
costs well under a second a message, after a sync.

### What this decides

It does not change what the list shows. Model verdicts stay recorded and
compared, and nothing here moves a message. That decision needs real mail:
`kuverta disagreements` on a real mailbox, and the user's own corrections scored
against both.

It does settle the shape of the eventual answer. Embeddings are fast and right
where a sender has been filed before. Prompting is slower and right on unfiled
senders. A classifier that asks the nearest filings first and falls back to the
model when they are far away, or disagree, would take the cheap answer when
there is one. That is the next thing to build and measure.

Two loose ends:

- The CLI's default model is `Dolphin3:latest` because these numbers are for it.
  `make ai-model` pulls `qwen3:8b`, which has not been measured and is a
  thinking model. With the twelve-token answer limit it may never get past its
  reasoning to the word. The default stays until `qwen3:8b` is measured, per
  §3.3.
- The model sees less of stored mail than it was scored on. The store keeps only
  some of the headers the rules read, so the model pass re-parses the message
  from disk where there is one and falls back to the summary otherwise.

---

## 16. Nearest filings first, the model when they are far away

**2026-09-14.** Follows §15.

§15 found embeddings perfect and sixty times faster on senders filed before and
poor on strangers, and prompting right on both at six hundred milliseconds. The
combination asks the nearest filings first and the model only when they are not
good enough. "Good enough" was measured.

`Neighbours::nearest` now reports how close the single closest filing is.
`Hybrid` accepts the filings' answer only when that similarity clears a
threshold and the neighbours agree (a share of at least 0.8). Both are needed: a
close filing its neighbours contradict is a sender filed two ways, and a
unanimous vote among distant filings is a stranger who resembles one kind of
mail.

`kuverta eval` keeps the model's answer and the nearest filings for every scored
message, so every threshold can be tried without asking the model again:

| similarity ≥ | known senders: accuracy · by filings · ms | new senders: accuracy · by filings · ms |
|---|---|---|
| 0.60 | 100% · 100% · 11 | 79.5% · 96% · 37 |
| 0.75 | 100% · 100% · 11 | 83.0% · 69% · 175 |
| 0.80 | 100% · 100% · 11 | **100%** · 52% · 277 |
| **0.85** | **100% · 100% · 11** | **100% · 25% · 452** |
| 0.90 | 100% · 100% · 11 | 100% · 18% · 499 |
| model only | 100% · 0% · 600 | 100% · 0% · 596 |

The default is 0.85. 0.80 is exactly where accuracy on new senders recovers, and
a threshold on the knee of a curve from one corpus is tuned to that corpus. 0.85
keeps a margin, still answers every known sender from the filings at 11 ms, and
still spares the model a quarter of the strangers. A test pins the default above
the knee.

Real mail should err on the safe side. Messages from one real sender vary more
than the generator's templates, so real similarities will run lower. More
messages then go to the model, and the combination degrades towards "model
only", whose accuracy it already matches.

### What is not done

The combination is measured but not running. In production the filings are the
user's own corrections, and there are almost none yet, so a combined pass would
ask the model about nearly everything. Wiring it in means embedding corrected
messages once and keeping the vectors in a table keyed by message and embedding
model. That is worth building once there are corrections to use. When a model
verdict may change what the list shows is still §15's open decision.

---

## 17. Filing post by hand

**2026-09-14.** Closes the gap §11 recorded.

§11 refused `1` to `6` on a letter, because `correction` hangs off a message
row. Post now has its own append-only table, `paper_correction`. A correction
on a letter does two things:

- It pins the letter to that category, whatever the rules say.
- It teaches the correspondent, if there is one. A document's facts use the
  correspondent as sender address, so mail's learning path applies unchanged
  and the next letter from that sender is filed the same way. Post corrections
  load after mail's, so the paper filing wins for paper.

A letter with no correspondent pins itself and teaches nothing. The row shows a
dash for "unknown", and learning it would file every anonymous letter alike. So
the command asks Paperless for the sender and drops blank names.

Filing a letter where it already is records nothing, as with mail. In the host
contract a letter's `actions` is now `[setCategory]`, not `[]`, so the
conformance rule on refusing undeclared actions now tests "a row that does not
allow archiving".

> **Since (2026-09-14):** §9's gap is closed. `kuverta`'s message detail now
> carries the classifier's facts, re-parsed from the stored message, so the
> reading pane explains mail as in Thunderbird. A letter's detail carries them
> too. A message not on disk gets no facts: the stored row keeps too few
> headers, and a partial explanation would be wrong.

---

## 18. An assistant may draft, and still nothing sends

**2026-09-14.** Brief §4.1's "draft", which §14 left out.

§14 kept sending off the agent surface under every flag, because mail is
attacker-controlled input and "forward this to…" must not be one tool call
away. Drafting was left out too because it shares a path with send.

The tool saves a real draft in the account's Drafts folder, where the user sees
it in every client. It goes nowhere until they send it. What makes it safe:

- `Session::save_draft` uses the same construction as `send` and `preview`, so
  the draft matches what sending would put on the wire, threading and quoted
  original included.
- `APPEND` can only create a message. Nothing is moved, flagged or lost, so
  the undo window is not involved, and an unwanted draft is deleted like any
  other.
- No Bcc. Per §3.5 blind recipients reach the envelope and never the message,
  so a server-side draft would lose them silently for whoever sends it later.
  The schema offers no `bcc`, a client that sends one is refused with that
  reason, and `save_draft` refuses before building. A blank Bcc field (an empty
  form) is not a recipient.

`find_drafts` resolves the folder like `find_sent`: the server's `\Drafts`
attribute, then common names (German included). It refuses if there is none.

Not verified here: the append itself, which needs an IMAP server. Tests cover
everything before the connection, and `send`'s filing into Sent uses the same
`append` under the dev-server tests.

---

## 19. The default model: `qwen3:4b-instruct`

**2026-09-14, default changed 2026-11-21.** Resolves the loose end §15 left.

§15 left an 8B model as default while the Makefile pulled the unmeasured
`qwen3:8b`, a thinking model that may not finish reasoning within a
twelve-token answer. Small instruct models were then measured the same way, on
both splits, on an idle CPU. The default must be Apache-2.0 or MIT, because the
app promises an open model on the user's machine. Measured 2026-11-21 (Apple
M2, 24 GB, Ollama 0.40), 250 generated messages, 125 scored:

| model | licence | size | every other message | senders never seen | p50 |
|---|---|---|---|---|---|
| **`qwen3:4b-instruct`** | **Apache-2.0** | **4.0B** | **92.0%** | **100%** | **~360 ms** |
| `phi4-mini` | MIT | 3.8B | 93.6% | 100% | ~273 ms |
| `granite3.3:2b` | Apache-2.0 | 2.5B | 76.0% | n/a | ~273 ms |
| `qwen3:4b` (thinking) | Apache-2.0 | 4.0B | 0%, no usable answer | n/a | ~740 ms |
| `qwen2.5:3b` | Qwen research licence | 3.1B | 87.2% | 97.3% | ~285 ms |
| `gemma3:4b` | Gemma licence | 4.3B | 87.2% | 76.8% | ~850 ms |
| Dolphin3 | n/a | 8.0B | 100% | 100% | ~620 ms |

`gemma3:4b` read every DHL "Ihre Sendung wurde zugestellt" and people asking
about a quote as transactional, though the taxonomy names delivery notices as
notification. Embeddings made the DHL mistake too on unseen senders (a no-reply
address and a number pull toward receipts), so it is a good case for a
correction. `granite3.3:2b` read two thirds of the notifications as
transactional. `qwen3:4b` with thinking on spends the whole answer thinking.
Its `-instruct` build does not think and is the one measured.

The rule set in advance, "the smallest model that holds 100% on senders never
seen", was wrong. That split matters for embeddings, which learn from filings.
A prompted model does not, so a miss on either split counts.

The kind of miss decided it (`eval --show-wrong` lists them). Every miss of
`qwen3:4b-instruct` and `phi4-mini` is one of two portal notices that a
document is ready: the Sparkasse's "Kontoauszug verfügbar" and the Finanzamt's
"Bescheid über Einkommensteuer". They were read as notification where the label
says transactional, and the taxonomy supports both. Neither model misfiled an
invoice, newsletter, advert or person, and both were perfect on unseen senders.
`qwen2.5:3b`'s twelve misses were plain errors on transactional mail, and it
also missed on unseen senders.

`phi4-mini` is 1.6 points better and a third faster at sorting.
`qwen3:4b-instruct` was chosen because the same default drives the assistant
(§25), which calls tools in German as often as English. Qwen3 is stronger there
and plainly Apache-2.0. In the app, nearest filings also answer first when they
agree (§15), and that combination scored 100% on this set with either model.

`make ai-model` pulls the default, and `--model` overrides it.

---

## 20. The view gets tests, and they find a bug on their first run

**2026-09-14.**

The triage surface was the only untested part of the JavaScript half, and two
bugs had been found in it by hand: keys that explained nothing, and a teardown
that made every keystroke act twice after switching accounts.

`kuverta-bird/test/view.test.js` mounts the real surface into jsdom over the
in-memory host and drives it by keyboard and clicks. It checks:

- the legend and scope bar text
- `j`/`k` move the cursor, and archiving keeps it in place
- typing in the search box never fires a shortcut
- digits file by category
- a row that refuses an action says so on screen
- unmounting releases the keyboard
- an empty filtered list says why and offers a way out
- Escape closes the panel before touching the filter
- the reading pane explains a verdict and shows a user's filing beside the
  rules' result
- nothing a sender wrote is parsed as markup

The first run failed on a real bug: the explanation panel never opened on first
launch, only on `?`. The patch that added the opening line matched nothing and
had no assertion. The test failed against the unfixed view, so it is known to
catch the bug. The fix is one line, and the replacement now asserts a match.

### What it cannot see

jsdom has no layout and no CSS cascade. It can tell that an element is `hidden`
but not that it is off screen, so it misses the settings sheet bug where
`display: flex` beat the hidden attribute and showed both forms. Playwright
against `hosts/kuverta/ui/triage.html` with a stand-in bridge would catch it,
at the cost of a browser download.

### The dependency

jsdom 24.1.3, pinned exactly, is the only dev dependency. The current release
needs a newer Node than 20.10 and fails on import, not at install.
`make test-js` installs from the lock file only when `node_modules` is missing,
so normal runs stay offline.

---

## 21. Browser tests, for what a DOM without layout cannot see

**2026-09-14.** Follows §20.

Playwright loads the real pages into Chromium over HTTP to catch what jsdom
cannot: an element marked hidden but still drawn because a `display` rule won.
It uses the same fake Tauri bridge as the adapter's unit tests, loaded by URL,
so both agree on what a command returns.

`e2e/triage.test.js` covers the shared surface as `kuverta` mounts it:

- the first-launch panel is on screen, and gone after a reload
- the legend is inside the window
- hidden things are off screen, not only marked
- the cursor stays in view through a 120-message list
- archiving keeps the cursor
- post is interleaved with mail
- a narrow window drops the reading pane but keeps the list
- a session leaves nothing in the console

`e2e/settings.test.js` covers the desktop window's settings sheet.

### What it found on its first real run

A letter could not be opened from the keyboard. `Triage.act` checked a row's
declared actions (§12) before every action, including Open, and a letter allows
only filing. A click worked because it bypasses `act`. Reading is now never
refused, and a unit test covers it.

The settings test catches the bug it was written for. With the rule that fixed
`display: flex` over the hidden attribute removed, three of its four tests
failed with "the address form must be off screen" and "the account form must be
off screen". With it restored, all four pass.

### And one that caught the harness

The first run reported zero tests and exited green. `node --test` on a
directory only picks up files named like tests, and `triage.e2e.js` was not.
Renamed to `triage.test.js`. Treat "0 tests, 0 failed" as a failure.

### Cost

Playwright 1.63.0 and its headless Chromium (~94 MB), pinned by the lock file
and installed only when missing. They take seconds and need the download, so
they run under `make test-e2e`, not `make test`.

## 22. Drafting and post, against the real servers

§17 and §18 were tested only against fakes: a canned Paperless on a loopback
socket, and drafting up to the connection. Both now have tests against the
Docker stack, which skip when it is down and fail under
`KUVERTA_REQUIRE_DEV_SERVER=1`:

- `core-rpc/tests/drafts_dev_server.rs` saves a draft through `Session`, syncs,
  and reads it back from the store, flagged `\Draft \Seen`. It runs on the plain
  Dovecot (`Drafts`) and the Gmail-shaped one (`[Gmail]/Drafts`, where a plain
  `Drafts` exists but is not the special-use folder).
- `core-paper/tests/dev_server.rs` asserts agreement, not contents, so it works
  on an empty or a used instance: report against listing, listing against each
  document, each tag against the documents that carry it.

Checked by hand on the stack: `scannerd --drain-only` with two generated
letters; `kuverta paper` with `--check`, `--tag` and `--query`; and a postal
address registered in `.devdata`, read through `core-rpc` as the desktop
commands do (token filed, check, listing, opening a letter, filing it).

### What the real servers found

`scannerd` could not upload a tagged letter. Paperless's upload endpoint takes
tag ids and answers a name with `400 Expected pk value, received str`, so the
unit file example (`--tag post --tag home`) would have left every capture in
the spool. The fake accepted anything. `scannerd` now looks each name up once
(`name__iexact`) and sends ids. An unknown tag fails the upload by name and
keeps the capture. The fake now answers lookups, with a test.

A blank Cc refused the whole draft. `save_draft` treated a blank Bcc as no
recipient, but `build_draft`, shared with preview and send, parsed a blank Cc
or To as the address `""`, which is what an empty form field sends. Blank
fields are now skipped for all three, with a test that needs no server.

The dev stack's Paperless did not start, which broke `make dev-up`. `latest`
moved to a release that requires `PAPERLESS_SECRET_KEY`. The compose file now
sets a dev-only key and pins the image (3.1.3).

### And one the test got wrong

The tag test upper-cased names with Unicode rules, turning "Hauptstraße 12"
into "HAUPTSTRASSE 12", which Paperless does not match because the spelling
differs. An ASCII case change ("HAUPTSTRAßE 12") does match on the dev
instance's SQLite, and the test now uses that.

### Still open

- ~~**The keychain entry for a paper token is `paper:{id}`**~~ (fixed). Row ids
  start at 1 in every store, so `.devdata` and the real store shared `paper:1`.
  Schema v9 gives each address a random `token_key`, set when added and kept
  through edits, and the token is filed as `paper:{token_key}`. An old
  `paper:{id}` entry moves on first read to whichever store asks first. If
  that is the wrong store, the check reports the token rejected and it must be
  entered again. Checked against the real keychain: the entry moved, the old
  one was removed, Paperless accepted it.
- ~~**Post reads poorly until Paperless knows who sent it**~~ (fixed).
  Paperless only suggests existing correspondents, and a new instance has none.
  So `core-paper` reads the letter for what Paperless has not been told:
  - the sender is the correspondent, or else the letterhead: the first line of
    OCR text with words in it, cut before an address on the same line;
  - the subject is the title, unless the title is a scan's name (`Post
    <timestamp>`, `IMG_2044`, none). Then it is the first non-date line after
    the address block (the last postcode-and-place line near the top), per
    DIN 5008.

  Rows, the CLI and the classifier use both, and filing a letter learns its
  sender even without a correspondent. On the dev stack the three scanned
  letters list as *Finanzamt Muenchen: Bescheid fuer 2025 ueber
  Einkommensteuer* and *Stadtwerke Musterstadt GmbH: Rechnung Nr. 2026-48211* /
  *Ihr neuer Abschlag ab November 2026*. Filing the September invoice as a
  notification through `core-rpc` filed October's letter the same way and left
  the Finanzamt alone. `tests/letters.rs` holds the verbatim OCR of those
  letters, a multi-line DIN layout, and cases where a person's title or
  correspondent wins.

  A rule learned against a letterhead stops matching once Paperless has a
  correspondent for that sender, because the key changes. The letter is then
  filed by what is taught about the correspondent. A two-word letterhead such
  as "Finanzamt Muenchen" passes the rules' "looks like a person" test, as
  two-word correspondents always did. The transactional evidence outweighs it
  here, but `ROLE_WORDS` (shared with the JS port) may need institution words.
- A token filed with `security add-generic-password` cannot be read by the app
  without a prompt, whatever `-A` says: the item's partition list admits only
  Apple's tools. Paste tokens into the settings sheet, which files them as the
  app.

## 23. The scanner, without a Pi: what reading it found, and letters

Done without a Raspberry Pi: reading `scannerd`, driving its loop by hand, and
running the binary on a laptop with ffmpeg standing in for `rpicam-still`.

### Found by reading

- A failed upload waited for the next letter. The spool drained only at start
  and after a capture, though the readme promised a retry "within the minute".
  It now also drains on a timer (`--drain-every-secs`, 30), including while the
  camera fails.
- The backoff was measured from the capture, so anything older than five
  minutes was always due. It is now measured from the last attempt, which the
  sidecar records.
- Uploads had no timeout and the loop waits on them, so a dropped link would
  stop the daemon seeing the next page. Now two minutes.
- Detection ignored `--roi`. The crop applied only to the photograph, so the
  thresholds were fractions of the whole desk.
- The systemd unit passed no arguments. `ExecStart` never mentioned
  `$SCANNERD_ARGS`, and nothing read `SCANNERD_ROI` from the env example, so
  tags and crop in `/etc/scannerd.env` never arrived on a Pi. Tags
  (`SCANNERD_TAGS`, comma-separated, since a tag may hold a space), button and
  crop are now environment variables.
- The readme never created the `scannerd` user the unit runs as.
- Two captures in one second shared a name (seconds plus process id), and the
  second rename replaced the first. Names now use nanoseconds.

`Scanner::turn` is the loop as a library function with the clock passed in.
`tests/run.rs` drives it with a scripted camera and a Paperless that fails on
request. Without the timer drain, two of its three tests fail.

### Letters of more than one page

Decided: a button. Grouping by time would merge a quickly scanned stack of
one-page letters, a separator sheet must be printed and handled, and one page
per document leaves merging to the user. With `--button`, pages collect in the
spool's `open/` until the press, then go as one PDF.

- The button is a key. The kernel's `gpio-key` overlay turns a push button on
  GPIO 17 into a debounced Enter on an input device, and `scannerd` reads
  24-byte `input_event` records. No GPIO library or libgpiod version is
  needed. A USB keypad works too, and `--button stdin` is Enter in a terminal.
- A press while the last page is settling waits for it, since the press often
  arrives before the page is detected.
- An unclosed letter closes itself five minutes after its last page.
- The PDF embeds the photographs unchanged (`/DCTDecode`) without an image
  library, since a JPEG's size is in its frame header. It is written as
  `.partial` and renamed, and pages are removed only afterwards. A crash can
  leave the same PDF twice (Paperless refuses it by checksum) but never
  nothing. A page that is not a readable JPEG is queued on its own.
- A PDF built from the three real letter photographs passes Ghostscript and
  renders each page with its proportions.

### Not verified

- On a Pi: since checked on the user's Pi, a **Zero W Rev 1.1** (ARMv6, single
  core, 427 MB, 32-bit bookworm, OV5647 camera). The first `make scannerd-pi`
  (an arm64 container) was the wrong build. It now cross-compiles with
  cargo-zigbuild for `arm-unknown-linux-gnueabihf.2.36` in about a minute and a
  half. On the device the libraries resolve and an HTTPS upload completes its
  handshake, which exercises the TLS provider's compiled code, where ARMv7-only
  instructions would crash. scannerd's reqwest uses ring instead of aws-lc,
  whose ARM assembly has not been run on ARMv6. A preview frame takes 1.1 to
  1.7 s (the camera starts per call) and a capture 3.7 s, so a page is
  photographed eight or nine seconds after it is put down. A streaming preview
  (`rpicam-vid`) would cut that. Still open on the device: detection against a
  real surface (the camera pointed at a ceiling), exposure, the `gpio-key`
  overlay's device name, and the unit file.
- Against Paperless: the laptop run of the binary and the `make scannerd-pi`
  build were stopped by the Mac's disk filling (302 MB free). Docker's storage
  gave read errors, Paperless answered 500, and an ffmpeg frame took 18
  seconds. Space was freed (build output, Docker's build cache, three benchmark
  models), but Docker needs a restart before either can run.

## 24. Post read by a vision model, unread, and sorted like mail

Scans read badly in the desktop app. Requested: post arrives unread, the text
is readable, post is sorted into categories, and the list can go by letter date
or scan date.

### Why the OCR was bad, and why not fix the image

The photographs are soft. The OV5647's fixed-focus lens is set for distance and
the page is at desk distance. The last scan's sharpness measured 0.08, against
0.32 for a sharp synthetic page and 0.03 for a blurred one. Tesseract
(`deu+eng`, as Paperless runs it) found about 170 words at 68% mean confidence,
24 of them confident. Upscaling, flattening the light and sharpening did not
help.

A made-up letter drawn at the camera's resolution (about 150 dpi) and blurred
in steps:

| Gaussian blur radius | 0 | 1.2 | 2.0 | 2.8 |
|---|---|---|---|---|
| Tesseract, character similarity | 100% | 100% | 86% | 5% |
| `qwen2.5vl:3b`, character similarity | 100% | 100% | 100% | 99% |
| `qwen2.5vl:3b`, seconds for the page (Apple M2, 24 GB) | 45 | 34 | 33 | 33 |

Decided: a local vision model reads the scans. Nothing leaves the machine.

The 1% lost on the blurriest page was three errors, all digits in dates: "3.
September" read as "1.", and "2026" as "2025" twice. None was marked `[?]` as
the prompt asks. A blurred number comes back confident and wrong, so the scan
stays one tab away, and dates and amounts that matter must be checked against
it.

### A statement it looped on

On a real tax office account statement the model read the letterhead and bank
details, then wrote the table's two column headings, "EUR" and "Ct", 116 times.
Measured on that page, counts only:

| Reading | prompt tokens | answer tokens | stopped because | most repeated line | seconds |
|---|---|---|---|---|---|
| as first shipped: Ollama's 4096 context | 3341 | 755 | out of room | 116× | 25 |
| 16k context | 3341 | 4096 | out of room | 951× | 184 |
| a prompt with rules for tables | 3375 | 1500\* | out of room | 302× | 84 |
| temperature 0.3 | 3341 | 1500\* | out of room | 302× | 52 |
| the image scaled to 1024 px | 1127 | 1500\* | out of room | 604× | 61 |
| repeat penalty 1.2 over the last 128 tokens | 3341 | 864 | finished | 2× | 30 |

\* capped at 1500 for the test.

The context was too small (the page alone is 3,341 of Ollama's default 4,096
tokens), but with more room the model only looped longer. The repeat penalty
stopped it, at a cost on the drawn pages, which read cleanly without it:

| Page | similarity, without / with | numbers kept, without / with |
|---|---|---|
| plain letter, sharp | 100% / 78% | 100% / 89% |
| plain letter, blur 1.2 | 100% / 100% | 100% / 89% |
| plain letter, blur 2.0 | 100% / 78% | 100% / 89% |
| plain letter, blur 2.8 | 99% / 86% | 83% / 78% |
| table letter, sharp | 95% / 86% | 95% / 77% |
| table letter, blur 2.0 | 95% / 83% | 95% / 73% |

So a page is first read plainly in a 16k context. If it loops or runs out of
room, it is read again with the penalty. The second reading is kept only if it
finishes, and is marked as looser so names and numbers get checked against the
scan. A loop that survives both is cut after two repeats and marked, and a page
cut short without a loop says so. Answers are capped at 2,048 tokens (measured
pages took 200 to 864), so a loop costs well under a minute, not three. Hosted
providers get the cutting and the marks but no second reading, because their
penalties are on another scale and untested.

### How

- The model reads the original, not Paperless's archive PDF, which is
  re-rendered with an OCR layer. The original is what `scannerd` sent: JPEG
  pages inside a PDF. `core_paper::scan::jpeg_pages` extracts them (the
  `/DCTDecode` streams, whole), so no PDF renderer is needed. Any other kind of
  document is refused with a message.
- One request per page to Ollama's chat endpoint, with the image, temperature
  0, and a prompt asking for the text as printed, untranslated, with `[?]` for
  unreadable words. A plausible invented word is worse than an OCR gap because
  it reads as right.
- The transcript replaces the OCR everywhere: the reading pane, and the
  sender, subject and category derived from the text. It is stored per
  Paperless instance and document (`paper_transcript`) with the model's name,
  which the reading pane shows next to the text, with a button to read again.
- New post is read as it arrives, one letter at a time, since two at once are
  both slow on a laptop. Once the model fails (Ollama not running, model
  missing), automatic reading stops for the session and the reason is shown.
- `qwen2.5vl:3b` by default, `KUVERTA_VISION_MODEL` to change it.

### Unread, categories and the two dates

- Read state is kuverta's. Paperless has none, and writing a tag per letter
  would change the archive from a reader that promises not to. It lives in
  `paper_read`, keyed like the transcripts. `u` toggles, opening marks as
  read, and the postbox shows an unread count.
- Categories are mail's, from the same classifier, with the same counts and
  sidebar filter. Paperless cannot filter by them, so a filtered list is built
  from all the postbox's rows (at most 5000) and paged here. Unfiltered lists
  still page in Paperless.
- Letter date or scan date: Paperless's `created` (from the letter, or the
  upload when it found none) or `added`. The list sorts and dates by the choice
  and keeps it.

## 25. Choosing models, and hosted providers beside Ollama

Asked for: a settings page to choose models, see which are available, and
connect an external provider such as DeepSeek.

### Shape

- A provider is where models run: the Ollama on this computer (in every store
  from schema v11; its address can change, it cannot be removed), another
  Ollama, or a hosted service with an OpenAI-compatible API. A job is a task
  for a model: reading scans, or sorting mail (`kuverta classify`). Each job
  has one provider and model. With none chosen it runs on the local Ollama with
  `qwen2.5vl:3b` (or `KUVERTA_VISION_MODEL`, which §24 named and is now only
  the default) and `qwen3:4b-instruct`.
- Hosted services share one client. DeepSeek, OpenAI, OpenRouter and most
  others take the same `/chat/completions` request and list `/models`. Presets
  fill in each address. A page is sent as an `image_url` data URL.
- Model capabilities are shown only where the provider reports them: Ollama
  per model, OpenRouter as input modalities. Most services, DeepSeek included,
  list names only, and the page says "not said". **Try them** runs each job
  once on a sample: a drawn page reading "Rechnung 4711" for reading, a
  one-word question for sorting.
- Keys are in the keychain under a random name per provider, like Paperless
  tokens, and never return to the window. Removing a provider removes its key
  and resets its jobs to defaults.
- Nothing falls back to a hosted service. A provider is local only if its
  address is loopback, so an Ollama on the network is not. For a non-local
  provider the note under the job says what is sent: scans of letters, or
  senders, subjects and the start of each message. `kuverta classify` prints
  the same before it starts.
- `classify --ollama` (or `OLLAMA_URL` in the environment) still asks that
  Ollama directly, past the settings. `--model` overrides the chosen model.

### Not verified

- A real hosted service. There was no key, so the client is tested against
  servers answering in the documented shapes, not DeepSeek. Try them shows
  whether a service's models accept images.

## 26. A setup assistant for the first run

A new store used to open on an empty list and a settings form. The window now
opens an assistant: local models, a Paperless for paper post, then mail
accounts, each skippable.

- It installs only Paperless. For Ollama and Docker it links to the makers'
  pages or shows a command, because the window should not run installers that
  need administrator rights. With Docker running, Paperless is a compose file
  and `docker compose up -d` in `<data dir>/paperless/`, pinned to the dev
  stack's image version. The admin password is in an owner-only env file:
  Paperless reads it only when it first creates the user, and a reinstall
  should create the same one.
- It asks for a Paperless password, not a token. `/api/token/` exchanges user
  name and password for the token. The password goes to the core and to
  Paperless, never back. The endpoint is throttled, and a 429 is reported as
  "wait a minute", not as a wrong password.
- Paperless is found by asking `/api/documents/` anonymously. A 401 with
  `WWW-Authenticate: Token` (Django REST framework's token scheme) counts as a
  Paperless, since little else on port 8000 speaks it.
- Programs are searched for beyond `PATH`. An app started from the Dock gets
  `/usr/bin:/bin:/usr/sbin:/sbin`, which has neither Homebrew nor Docker. The
  Docker CLI also needs its credential helpers on the `PATH` it runs with.
- Thunderbird's passwords are imported with its accounts. `prefs.js` has the
  servers and `logins.json` the passwords, encrypted with a key in `key4.db`,
  which has an empty password unless a primary password is set. This is NSS's
  scheme: whoever can read the profile can read the passwords. kuverta
  decrypts them the same way (PBKDF2-SHA256 and AES-256-CBC for the key, 3DES
  or AES for each login). A primary password is asked for, passed down and
  never stored. A found password goes into the keychain inside one call,
  `save_account_with_found_password`, and never reaches the window, as with
  the settings form.
  Apple Mail's passwords are not imported. They are in the login keychain
  under Mail's access rules, and reading one raises a system prompt per
  account. They are typed once.
  Every account is signed in to before the assistant moves on. A failed one
  stays on the page with the server's answer.
- Apple Mail accounts are the system's Internet Accounts, `~/Library/Accounts/
  Accounts4.sqlite`, readable only with Full Disk Access. The assistant says so
  and links to the setting. The database is read from a copy, since the live
  one is in WAL mode and in use.
- Servers for an address come from a built-in table of common providers, then
  from where Thunderbird looks: `autoconfig.<domain>`, the domain's
  `.well-known` file, and Mozilla's ISPDB. Only encrypted servers are taken
  from those files. Microsoft addresses are marked OAuth2, because Microsoft
  takes no passwords over IMAP.
- First run means no marker and nothing in the store. Existing installs never
  see the assistant open by itself.

### Verified

- Thunderbird import against a real profile with 19 IMAP accounts, and the
  parser against a canned `prefs.js` and `profiles.ini`.
- Reading that profile's 46 saved passwords, which needed both keys: 24 bytes
  for the 3DES logins, 32 for the AES ones. The round trip is tested against a
  login this code encrypts itself.
- Server lookup through a provider's own autoconfig file and through ISPDB.
- Ollama status against Ollama 0.34.1, and download stream parsing against
  Ollama's documented lines.
- Paperless detection and sign-in against the dev stack's Paperless 3.1.3.
- The assistant's pages in a browser, against a stand-in for the core.

### Not verified

- Apple Mail against a real accounts database. The terminal here has no Full
  Disk Access, so the reader is tested against a database built to the
  documented layout. Only the address is relied on, and servers are looked up
  as for a typed-in address.
- Installing Paperless end to end. The compose file was written and
  `docker compose up` ran, but the Docker VM failed with I/O errors (its disk
  was full) before the web server started.
- Downloading a model through the window. Both default models were already
  present.

## 27. The first sync goes newest first

A first sync fetched each folder from UID 1 upwards, so on a large mailbox the
window showed years-old mail for an hour and today's mail arrived last.
Reported as "it only shows messages up to Thursday".

- Only an empty folder is walked downwards. Mail above what a folder holds is
  still fetched upwards: it is rarely more than a batch, and adding to the top
  can be interrupted without leaving a hole.
- A downward walk leaves a hole, so `folder.
  backfill_uid` records the UID the walk has reached. Everything below it is
  owed. It is written after every batch, so an interrupted sync (a laptop
  closed) resumes inside the hole. `MAX(uid)`, which says where to fetch from,
  would point above the hole by then.
- A folder that owes history is never skipped. CONDSTORE skips a folder whose
  `HIGHESTMODSEQ` has not moved, which after an interrupted sync would be every
  folder, and the backfill would never finish.
- Rejected: fetching only the newest mail. A client that holds the last month
  and says nothing about the rest is worse to search than one still filling.
  The whole folder is fetched, only the order changed.

### Verified

Against the dev Dovecot, in `crates/core-proto/tests/dev_server.rs`: in a
folder of 201 messages (one more than a batch) the first batch stored holds the
newest message, and a store left as an interrupted walk leaves it is filled by
the next sync with nothing owed. Both fail with the old walk.

## 28. Who the message is with, and when a letter's address may be claimed

**2026-09-23.**

A sender line gives a name but no history.
`crates/core-rpc/src/sender.rs` builds a card from mail in the store: how much
there is each way, since when, who spoke last, what their mail usually is,
where it gets filed, whether any is still waiting, and the last few messages
either way.

Three decisions:

The card is asked for by message, because the window has a row under the
cursor. For a message the account sent, the other person is its first
recipient. The store uses the same `COUNTERPART` expression the conversation
list groups by, so card and conversation agree on the person. An address may be
passed instead, as the People list does.

The card sits in a column on the right. Under the mailboxes on the left it
pushed the navigation around whenever a message opened. It shares the fourth
column with the assistant, and the column exists only when something is in it.
The two are stacked in a flex column. The card is capped at 45% and scrolls
inside, so the assistant's input stays on screen.

A postal address is claimed only where it cannot be the reader's own.
Paperless keeps no address for a correspondent, so it must come off the page,
where the recipient's address block sits under the letterhead and looks the
same. The dev-stack letters show it: `Stadtwerke Musterstadt GmbH`, then
`Erika Mustermann,
Hauptstrasse 12, 80331 Muenchen`, the recipient. `Document::postal_address`
therefore answers in two layouts only. One is the DIN 5008 return line on the
letterhead: `Stadtwerke Musterstadt GmbH · Postfach 1234 · 80000
München`. The other is a letter with two address blocks near the top, where by
the same standard the first is the sender's. Anything else is `None`. Most
scans show no address, which is better than a wrong one.

### Verified

`crates/core-store/tests/cleanup.rs` sums a correspondent up from both
directions and finds the same person from her letter and from the reply.
`crates/core-rpc/tests/surface.rs` checks the card is the same whichever way
it is asked for. `crates/core-paper/tests/letters.rs` reads the address from
both allowed layouts and returns nothing for the two dev-stack letters, whose
only address block is the reader's.

`kuverta-bird/test/desktop-sender.test.js` drives the card in jsdom: complete
counts, the exchange newest first with its direction, hover copy with nothing
unclickable. `kuverta-bird/e2e/sender.test.js` checks the layout in a real
engine: the column sits beside the reading pane, card and assistant share it
with the chat input on screen, the list gets its width back when the message
closes, and the hover card lands clear of the list and inside the window.

## 29. What the sensor leaves out is gone: four ways a letter lost its edges

**2026-09-24.**

A Finanzamt letter came out of the rig with the "F" missing from
`Finanzamt(Finanzkasse)` and the right-hand column, headed **Mahnung**, cut
off. Paperless read the first line it could, `26603
Aurich`, and filed the letter under a postcode. Four things were wrong.

The sensor crop was the marks' bounding box. `--roi` crops on the sensor to
exactly the box the four marked corners span, so a letter lying outside the
marks was cut by the camera and could not be recovered. On the rig the sheet's
top left corner sat 9% of the frame outside the marks. The crop now leaves 8%
of the marked area round it. Resolution is unchanged: `capture_size` asks for
the crop's share of the sensor, so a wider crop is a bigger photograph.

The marks were used as the page. The first warp goes onto the marks, so
anything outside them was discarded before the page was looked for.
`paper_overhanging` now finds the paper in the photograph and, when it reaches
more than 2% outside the marks, adds its corners to theirs. The marks are exact
and the found page depends on the light, so `Corners::around` takes whichever
reaches further on each side, in the marks' perspective.

The trim cut into the sheet. After the warp the picture is straightened again
onto the page found inside it, to remove strips of table. The page is found by
brightness, and the far edge of a sheet falls into shadow, so it was found
short and real page was cut. The trim now looks just beyond each side first. A
side with something as bright as the page beyond it is not the sheet's edge and
stays where the picture ends.

The sender was read off the first line. With the letterhead cut, that was
`26603 Aurich`. The same happens when OCR reads the address beside the
letterhead first. `Document::letterhead` now takes the first of the top four
lines that could be a name (not a postcode and a place, not a date, not mostly
digits) and cuts a line at a column gap, so
`Finanzamt(Finanzkasse)   26603 Aurich   02.05.25` reads as the Finanzamt.

### Verified

On the rig, the same letter in the same place, before and after. Before, the
letterhead began at `inanzamt` and **Mahnung** ran off the right edge. After,
the whole page is there, barcode to signature.
`apps/scannerd/examples/crop_check` puts a photograph through each step
separately and prints where the paper lies in the marks' frame. It is kept
because these bugs show in the pictures and not in the code.

In `apps/scannerd/tests/straighten.rs`: a letter overhanging the marks keeps
its letterhead, marks that hold the whole letter are left alone, the trim
takes a strip of table but leaves a shadow on the page, and the crop holds
every corner with room round it. Each fails with its fix removed.
`crates/core-paper/tests/letters.rs` reads the sender from the line order the
rig's OCR produced.

## 30. The learn-table button that a ten-minute window took away

**2026-09-24.**

Refiling the last letter (**No folder** and **Throw away**) and learning the
empty table shared one panel row, and refiling won while a letter was inside
its ten-minute undo window. So the table could not be learned from the rig
right after a letter, which is when it is wanted.

Learning the table now goes last and is dropped only when five rows are already
taken, the most the card above the buttons can spare on a 480×320 panel.

## 31. The card is asked for, the columns are yours, and a letter is not an envelope

**2026-09-24.**

Three changes to the sender card, and one bug.

The card opens on request. It used to open with every message and took a
quarter of the window. Now the sender's name, under the subject and in a
conversation's header, is clicked to open it. Once open it follows the cursor.
Its × closes it and stops it following. The hover card is unchanged.

The columns are dragged. Widths were fixed in the grid: a fixed sidebar and a
list between 300 and 390. They are now CSS variables, dragged by the seams
between panes, reset by double-click, and kept in `localStorage`. The seams
are grid items in the same cells as the panes, so the panes are placed by hand:
grid lets items overlap only where both were placed. The reading pane has no
width of its own, so it gives way when the right-hand column opens.

A message can be thrown away from the card. Each line of the exchange has a
bin. It is the same queued move the list makes, and `z` undoes it. The card is
then reloaded by address, because the message it was opened from may be the
one binned.

### The envelope that was a letter

Widening the quad in §29 broke envelope detection. `Straightened::
covered` is the share of the picture the paper covers, and an envelope is told
from a page by being smaller. It was measured against whatever was warped onto,
which is now sometimes the marks widened for an overhanging letter. A page
measured against the wider quad gets an envelope's share. An envelope ends the
current letter and starts a new one, so a letter was cut in two and its first
page called the envelope of the second.

`covered` is now always measured against the marks.

A second, older bug: with no page seen yet, `page_covers` defaulted to 1.0.
The rig's pages cover about 0.70 of corners set generously round where letters
land, against a threshold of 0.72, so the first page after every restart was a
coin toss. Now nothing is an envelope until a page has been seen: an envelope
taken for a page only becomes page one of its letter, while a page taken for
an envelope cuts a letter in two. Paper whose edges could not be
measured teaches that a page fills the corners, so a rig with tight corners
can still tell an envelope later.

### Verified

`apps/scannerd/tests/straighten.rs` has a page overhanging the marks, sized so
the two measurements fall either side of the threshold: 0.83 of the marks, 0.63
of the widened marks. It reads as a page, an envelope on the same marks still
reads as an envelope, and the test fails with the old measurement.

`kuverta-bird/e2e/sender.test.js` covers the rest in a browser: reading a
message leaves the column shut, clicking the sender opens it, it follows the
cursor, the × shuts it for good, the reading pane gives up the width, the seams
drag, stop at the limits and are remembered, and a message binned from the card
leaves card and list with the card still open on the same person.

## 32. Saying where a letter goes while it is still in your hand

**2026-09-24.**

Four things the rig got wrong about the current letter, found in one session.

The panel did not show the guess. The Pi has read each page since §28 and the
setup page shows the guessed folder, but the panel showed only the previous
letter's folder, once Paperless had answered. The guess is now on the panel
from the first page read, as an outline pill in the top right. The last
letter's settled folder keeps the filled pill and takes the corner when there
is no guess.

A letter from the police had no folder. The shelf had no law folder, so a
Strafbefehl notice was filed under nobody. The word lists were written for a
scanner but are matched against camera and tesseract output, which is bad text:
"Strafprozessordnung" came out as "Sraiomzessorsnung", "Swatprozessordnung"
and "Stra prazessortrugg". The letter was filed by "Staatsanwaltschaft" on
page two.

The setup assistant's lists are therefore long, since each extra spelling is
another chance to match. There is now a **Recht & Behörden** folder with the
broadest list, because a prosecutor's letter must not be found a fortnight
late. "Anzeige" and "Ladung" are left out: one is also an advertisement, the
other a lorry's load. There is a **Gesundheit** folder too, and the other five
lists are about half as long again.

Each list must fit the 256 characters Paperless keeps of a tag's match. Three
new ones did not, and the rig refused them. `desktop-shelf.test.js` now checks
the length, that no suggestion matches a word every letter carries, and that no
word appears in two folders.

The preview showed the text without the page. A letter's text stays on screen
until the next letter starts, but its pages were deleted when the letter was
closed into its PDF, leaving an empty frame. Pages now go to `spool/sent/`,
are served from there, and are cleared together with their text.

The folders were chips. In preview mode they are picked with one key press
each, from across a table, so they are now the biggest thing in the column.

### Verified

`apps/scannerd/tests/display.rs` has the panel naming the folder from a single
open page, joining two of them, and saying nothing with no letter open.
`apps/scannerd/tests/letters.rs` closes a letter and finds its pages readable
in `sent/`, then gone once the next letter starts.
`kuverta-bird/test/desktop-shelf.test.js` holds the word lists to the match
limit and to being distinctive.

The Wasserschutzpolizei notice that started this now reads as `Lawstuff` on
the rig, on Staatsanwaltschaft, Strafbefehlsverfahren, StPO and Beschuldigten,
four of the sixteen words. The same shelf leaves the Finanzamt letter in Taxes
and Inkasso/Rechnungen and out of the law folder.

## 33. One shelf, in one place, and one folder per letter

**2026-09-24.**

The folders were not kept in step between the rig, Paperless and kuverta. The
`Lawstuff` on the rig and the `Lawstuff` in Paperless had three of sixteen
words in common. `House` and `Work` existed only in Paperless, so the rig
could not guess them.

Cause: kuverta's assistant wrote folders to Paperless as tags. The rig adopted
them only once, and only if `nothing_says_which_folders` found that the setup
page had never saved a list. It never wrote back.

Paperless's tags are what file a letter. The rig's list only drives its own
guess and the number keys in preview, so the panel could name one folder while
Paperless used another.

Paperless is the shelf now. The rig reads its tags every five minutes and
keeps no list of its own. The env file and the page's old list are only a
fallback for a Paperless with no tags yet (first run or reinstall). The rig's
page shows the shelf read-only and says where to change it. The rig still
decides which folder is the bin, because a Paperless tag cannot say that.
Adopting keeps it.

### And a letter is one piece of paper

`guess` returned every folder a letter matched, and the panel and the preview
showed them all. The paper goes in one place, so the rig has to name one.

`folders::rank` counts how many of each folder's words stand in the text and
orders by that count. A letter with five of Taxes' words and one of
Rechnungen's is a tax letter. Ties keep shelf order, so the answer is stable.
Paperless's own answer goes through the same ranking: when it tags a document
twice, the display names the best fit and lists the rest underneath.

### Merging the two shelves

The union of the two lists did not fit: Paperless keeps 256 characters of a
tag's match, and `Lawstuff` wanted 29 words. The lists also overlapped. Four
debt-enforcement words (Mahnbescheid,
Inkasso, Gerichtsvollzieher, Vollstreckungsbescheid) were in both `Lawstuff`
and `Inkasso/Rechnungen`, and five employment words were in both `Personal`
and `Work`. Putting each word in one folder left `Lawstuff` at nineteen words,
enough for both of its subjects: criminal procedure and
lawyers/courts/inheritance.

### Verified

`apps/scannerd/tests/folders.rs` ranks a tax letter that is also a Mahnung
above Rechnungen on word count, and holds ties to shelf order. The scanner's
own suite covers the rest.

Against the real shelf: the Wasserschutzpolizei letter reads as `Lawstuff`,
the Finanzamt letter as `Taxes` first and `Inkasso/Rechnungen` second. Both
sides now hold the same twelve lists.

## 34. The folder is decided once, while the paper is in your hand

**2026-09-24.**

After §33 the folder was still decided twice: the rig guessed from its own
tesseract reading, and a minute later Paperless matched the same tags against
its own, much better OCR and replaced the guess.

By then somebody has read the folder off the panel and filed the paper. A
record that changes afterwards no longer says where the paper is.

So the decision is made once, when the letter is finished, from what the rig
has read. `Scanner::closed` settles it and writes it beside the letter in the
spool as a `.folders` sidecar, next to the `.attempts` one. It is on disk
because a letter can wait overnight for a Paperless that is down. The upload
names those tags on the document. Paperless no longer matches folder tags:
they are saved with `matching_algorithm` set to none.

The words stay on the tags. The rig reads a page against them, and the tag is
where they are edited and where both halves read them from. A tag with words
of its own is a folder. A tag without any is a label put on by hand (the
address is one).

Cost: tesseract on a photograph gives worse text than Paperless's OCR of the
same page, so a letter will occasionally get a worse folder. The alternative
is a record that is wrong about where the paper is.

### Finding the paper again

An open letter in kuverta shows its folder under the subject as a pill with
the folder's name, and who it is for beside it. The address label is left out
because it is on every letter of a postbox and says nothing about where this
one is.

### Verified

`apps/scannerd/tests/folders.rs` checks the tag is written with matching off
and its words kept. `kuverta-bird/e2e/postbox.test.js` opens a letter in a
browser and finds one folder pill and the person it is for, read apart.

On the rig: the twelve tags were switched to no matching and it still took all
twelve (`folders taken from Paperless count=12`), because it now knows a
folder by its words. The bin and both people came through.

## 35. A parcel notice, and whose fault the mess was

**2026-09-24.**

A DHL parcel notice showed as four hundred lines of `<table border="0"
cellpadding="0">`, Outlook conditional comments and tracking URLs, with a
sentence of German scattered through it.

kuverta's HTML rendering was not at fault. That text is DHL's own `text/plain`
part, markup included, and kuverta showed what arrived.

The rule now: a plain part wins, unless it contains two or more of `<table`,
`<td`, `<!--[if`, `<div`, `style="`, `cellpadding`. Then the HTML is read
here instead. The test is narrow because a plain part is usually written on
purpose. On the real notice: 605 lines to 33.

### What the reader is, and is not

It is not a browser: string in, string out. `<script>`,
`<style>`, `<noscript>` and `<head>` are skipped unread. `src` and `href` are
text and are never opened. No input can make it reach the network or the disk.
A test checks that a tracking pixel, an `onclick`, a `javascript:` href and an
iframe leave no trace in the output.

Every limit is a refusal, never a truncation. Too big, too deep, an
unterminated comment, script or tag: `to_text` returns `Refused` and the
caller keeps the sender's own text. A half-parsed body would look complete to
the reader. There is no recursion, so no input can exhaust the stack. Depth is
counted and refused at a hundred.

Two bugs found before the real message went through:

- A tag does not end at the next `>`. DHL writes `alt="Ablageort<br>
  buchen"`. Stopping there ended the tag inside a quoted value and spilled the
  rest of the attribute, style declarations included, into the message.
  Quotes are tracked now.
- An attribute's value is text, so markup in it is removed, and a picture
  whose words repeat the words beside it is read once. Every button in the
  notice is an icon and a label inside one link with the same words in both,
  so it came out as "[Ablageort buchen] Ablageort buchen", over two lines
  because the label has a break in the middle.

### The scanner's own reading

Measured on two of the rig's real pages: tesseract's default page segmentation
against `--psm 4` (a column of text) and `--psm 6` (one uniform block).
`--psm 4` won both: fifteen recognised German words against thirteen, with a
quarter less of the punctuation noise that marks a misread line, and nine
against eight on the second page. `--psm 6` finds many more "words" and no
more real ones, because it reads the noise between the columns. Upscaling the
page first made it worse, since tesseract already upscales.

So `--psm 4` is the default, and **Read it again** in the preview tries the
other one, because tesseract is deterministic and the same reading would give
the same text.

### The border

The trim left seven per cent of one side as table. `keep_paper` either cut to
the edge it had found or kept everything out to the frame. It now walks
outward a step at a time and stops at the first strip that is not paper.

A strip is judged by its brightest quarter. Judging by the average cut the
right-hand column off a letter, because a strip across dense text averages
dark. The paper between the letters is still as bright as the page, and the
table has no bright quarter. That side is now half a per cent.

## 36. The paper is told it is white

A page on the rig's table comes out grey. It is readable and in focus at 1:1,
but the paper photographs at about 186 of 255 with a warm cast, the ink at 54,
and a lamp leaves a gradient across a fold in one corner.

Tesseract reads low contrast worse. On the two most recent pages off the rig
(a baggage inspection form,
creased, filled in by hand) it found 97 and 74 German words of four letters or
more. With the paper set to white it finds 214 and 211.

So `flatten` finds the paper and sets it to white:

- In tiles about eighty pixels across, because the light over an A4 sheet is
  never even and one white point cannot remove a gradient.
- At the 92nd percentile of each tile. The brightest pixel is a glint off a
  staple.
- Per channel. Paper is neutral, so the difference between its channels is the
  colour of the light, and correcting each channel by its own paper level is a
  white balance. A blue signature stays blue, and a test checks it.

It is only a tone curve: no moving, sharpening or denoising. A filter that
smooths sensor noise would also smooth the thin strokes of 8-point print,
which is why the camera runs with `--denoise cdn_off`. In doubtful cases the
picture is left alone:

- A tile too dark to hold paper borrows the page's own level.
- A page with no ink on it is not stretched. Its darkest half percent is
  grain, and stretching against that turns a clean sheet into static.
- A picture too small to tile, or whose claimed size does not match its
  pixels, comes back unchanged.

The field is smoothed, and past the grid's edges it is continued along its own
slope. Repeating the edge value made the correction undershoot: a lamp at one
side of a sheet still left twenty levels between the two edges.

### What it costs, and who has to be told

Flattening destroys the two numbers the rig steers its exposure by, because
every page comes out with paper at white. `quality::exposure` reads `paper` to
decide whether the next page should be taken half a stop darker. Judging
flattened pages, it would walk the camera down to −4 EV one page at a time.

So `Straightened` carries a `Tone`: what the page looked like before.
`Quality::exposed_as` puts it back and decides the two exposure problems again
(is there paper here, is it washed out). Sharpness and "is there any text"
stay measured on the picture that will be sent. A "no paper" verdict discards
the other measurements.

## 37. Letting the letters say where they belong

A folder's words are typed once, before any letter has arrived, and they are
guesses. `Taxes` was given `Finanzamt` and `Steuerbescheid` but not
`Kassenzeichen`, which is on every letter the tax office sends.

The letters already filed are labelled data: a page of text with a folder
chosen by somebody holding the paper. `core-paper::learn` counts words that
stand on several letters in one folder and on none in any other. The folders
page offers them, one row per folder, as buttons that put the word in that
folder's own words box, where it can be edited or removed. Nothing is written
until the folders are saved.

### Four things it will not say, and why each one had to be found out

- Nothing until two folders have letters in them. With only `Taxes` filed
  into, `Kassenzeichen` and `Sehr geehrter Herr` both stand on every letter
  and on none elsewhere. The first tests proposed filing tax
  letters on `freundlichen` and on the household's own street. This is a
  hard rule with no threshold to tune. It is also why the repository has no
  list of German filler words: the second folder filters them, along with the
  street name and the sender's sign-off.
- Never fewer than two letters. One is a coincidence, and it is what a misread
  word looks like. OCR errors rarely repeat.
- Not a misreading of a commoner word. Against a real archive this proposed
  `Hambura`: the same smudge on the same letterhead four times. The check is
  that `Hamburg` stands on more letters and is one character away. A real
  word one character from a commoner one is lost too, which is acceptable
  because the page has other words.
- Never for a person. A letter is for somebody because their name stands on
  it, and no other word means "this is for Nicolas". The first real run
  proposed `Hambura` for a person.

On the current archive (twenty-nine letters over twelve folders, most with two
or three) it proposes nothing and says so: not enough filed yet. That is
intended, because a suggestion made from three letters would be acted on.

## 38. Auditing the things that read what other people wrote

The security pipeline already had `cargo deny` for advisories and licences,
`npm audit`, CodeQL over Rust, JavaScript and the workflows, and five fuzz
targets with a growing corpus. These were the gaps.

### The HTML reader was not fuzzed, and it had a bug

`core-rpc` was not in the fuzz crate, so §35's HTML reader had only
hand-written examples. It is the newest parser and the most exposed: anyone
who can send mail can feed it.

Two minutes of fuzzing found `&\n\0Ľ!Ľ&6\0Ľ<`. Entities are looked for in a
window of twelve bytes after an `&`, and twelve bytes into that string is the
middle of an `Ľ`. Slicing a `str` there panics, which would have taken down
the thread rendering the letter. The window now runs over the bytes. That is
sound because `;` is ASCII and never appears inside a longer character.

The target asserts four things the module promises: it returns, its output is
bounded by its own limit and not by the input, a refusal is never a half-read
message, and nothing executable (a `<script`, an `onerror=`, a `javascript:`)
survives into the text. 2.8 million further runs found nothing.

The target's first assertion was wrong, and CI found it. It forbade the bytes
`<script`, `onerror=` and `javascript:` anywhere in the output. But
`&lt;script&gt;` correctly unescapes to the text `<script>`. The first run
against the seed corpus (real mail) failed within a minute on a message
quoting HTML.

The check now applies only to inputs with no character reference in them.
Whether a script's content is dropped is tested on particular
documents: one test holds the same bytes quoted and the same bytes sent as a
real script.

With the assertion fixed the fuzzer got further and found two more bugs.
libFuzzer stops at the first crash, so a clean run only means "nothing before
that point", including the first seeded run's two and a quarter million runs.

First, an empty link shows its address. That printed
`<a href="javascript:alert(1)"></a>` into the message word for word. Nothing
ran, because the window renders text, but it would be a hazard if anything
ever turned addresses in a message back into links. An address is now shown
only for
`http`, `https`, `mailto` and `tel`, after leading whitespace and control
characters are removed, so `\0java\0script:` is handled too.

Second, the text is not append-only. A block element inside a link runs
`newline`, which pops the trailing spaces, so by the closing `</a>` the text
can be shorter than at the opening one. Reading the link's words back by the
offset noted at the `<a>` panicked: index 104 into a string of 101 bytes. A
link wrapped round a paragraph does that, which is malformed and common. It is
now read as an empty link.

Both bugs were in the released code. Five million seeded runs after the fixes
were clean.

A second target covers the scanner's arithmetic over raw pixels, where width
and height are taken on trust and every offset is computed from them. Nothing
panicked. The one failure was the target's own post-condition: `trim_to_page`
hands back a buffer it does not recognise unchanged, dimensions included, and
the assertion now checks that.

### Every action is pinned, because CI is where the keys are

`uses: some/action@v4` follows a tag, and the owner can move a tag. There were
seventeen such references, four from outside GitHub,
and two of those ran in the workflow that holds the Apple signing
certificates. All are pinned to commit hashes now, with the version in a
comment. `zizmor` runs in CI so a new one cannot be added on a tag.

`zizmor` found three more things:

- Permissions granted to every job. `contents: write` on the release workflow
  and `pages: write` on the website one were declared at the top, where every
  job inherits them. Both now sit on the jobs that publish. In the release
  workflow that is currently every job, but a job added later has to ask.
- The workflow token left behind. `actions/checkout` writes it into
  `.git/config` and leaves it for every later step and action. All nine
  checkouts are now `persist-credentials: false`.
- A cache restored into the job that signs. The release build restored
  `target/`: unverified compiled objects, linked into the installer and
  signed with this project's certificate. Caching only the registry does not
  fix it: Cargo checks a `.crate` archive against Cargo.lock when unpacking,
  but does not hash `registry/src/` again on a later build. So the release
  workflow restores nothing, at a cost of a few minutes of downloading per
  release. CI still caches its builds, because CI signs nothing.

### What was looked at and left alone

The scanner's HTTP surface is the one thing on a network. Its path-traversal
guard is an allow-list, its password comparison is constant-time, its bodies
are length-limited, and it demands a header a browser will not attach to
somebody else's form. `tests/web.rs` checks each of those, including
percent-encoded traversal. Its base64 decoder is hand-rolled, but it
accumulates into a `u32` six bits at a time with at most thirteen live, so it
cannot overflow, and it rejects every byte outside the alphabet.

Nothing token-shaped has ever been committed to this repository. The history
was searched.

## 39. A letter is also the person it went to

Searching a real mailbox for `cadus` found a hundred and eighty letters.
Thunderbird found two hundred and six over the same mail. The twenty-six
missing ones were all sent letters, to `er-accounting@cadus.org`, by the
person searching.

The index held the subject, the sender and the body. The recipients were never
indexed, although they were on `message` as `recipients`, and in Sent the
recipient is what people search for. The sender's display name was missing
too, because the `sender` column held the address alone.

### The migration had to carry the old index, not rebuild it

The searchable body text is not a column on `message`. It is passed in as
`search_text` when a letter is stored, written straight into the index, and
kept nowhere else. The blob on disk is the original message. Dropping the
index and regenerating it from the message table would have left every letter
searchable by subject only, with no error or log line. So the migration
carries the rows across and takes only the recipients from `message`.

Against a copy of the real database: a hundred and eighty found became two
hundred and six, and all 12,674 bodies were still there afterwards. A test
rolls a store back to v17 and fails if the bodies are lost.

## 40. The box and the list have to say the same thing

Typing `cadus` and then clicking Sent showed the whole of Sent, with `cadus`
still in the search box.

Every reload leaves a search (it clears `state.searching`), so the box
described a list that was not on screen. The box is now emptied in `reload`,
the one place every folder, category, smart mailbox and account change goes
through, instead of in each of the dozen handlers.

People is exempt. There the box is a filter the page reads when it loads, so
clearing it would undo the filter. The
desktop window has no harness that boots it, so the test is structural and
pins two things: that the clearing lives in `reload`, and that People is left
out of it.

## 41. What a fuzzer cannot know, and three tries to admit it

The HTML fuzz target used to assert that nothing executable survives a read.
It cannot assert that. Three versions failed:

1. Forbidding `<script` in the output. `&lt;script&gt;` correctly unescapes
   to `<script>`, so any mail that quotes HTML trips it.
2. Excluding inputs with a character reference. It failed on
   `<a href="javascript:alert(1)"></a>`. That was a real reader bug, fixed in
   §38, but the assertion was still wrong: the sender had written those
   bytes.
3. Also excluding bytes the sender wrote. CI found this counter-example:

```text
From: Tom Fisher!vbscript<>:m
```

`<>` is an empty tag. Dropping it leaves `vbscript` next to `:m`, so the text
says `vbscript:` although the sender's bytes never did. This is correct:
`<b>java</b>script:` has to read as `javascript:`, because the words either
side of markup are one word.

Joining text across removed markup can produce any bytes, so an input-only
check cannot work and the assertion is removed. The target now asserts that
it returns, that its output is bounded by its own limit and the input's
length, and that it is deterministic. Script content,
event handlers and link addresses are tested in
`crates/core-rpc/tests/html.rs`, on documents with known input and output.

For callers: `to_text` returns text, which is not safe markup. It can hold
`javascript:` whatever the markup was. Anything that turns message text back
into links or HTML has to validate schemes itself.

### Reading a failure you are not allowed to download

Downloading a CI artifact or reading a job's log needs admin on the
repository. Listing artifacts needs no permission. So a small failing input
now travels in the artifact names, in hex, across three of them.

Before that the job was only a red tick. Locally, seventeen million fuzz
runs, eight million random builds of the fragments this parser has special
cases for, seven point nine million exhaustive orderings, six runs from the
seed mail and four two-stage runs mimicking CI exactly were all clean. CI had
the failing hundred and fifteen bytes the whole time.

## 42. A letter in Sent is about the person it went to

The reading pane named only the sender, so every letter in Sent showed your
own address. It now names the recipients too, in every folder: three by name,
then a count, with the rest in the title attribute. Each opens the same card
as the sender.

### Where the names come from when the letter is not on disk

`to` and `cc` were read off the parsed message, so they existed only for
letters whose body had been fetched. A first pass down a folder keeps headers
only (§17's backfill). For those letters the row's `recipients` column is
used, the same one §39 put in the search index.

The column is stored as the header had it, comma separated, and a display name
may hold a comma: `"Zemke, Nicolas" <nic@example.de>`. Joining any piece
without an `@` onto the piece before it fails when the quoted name is first
on the line. It now splits on commas outside quotes. An unclosed quote takes
the rest of the line: one recipient too few beats a name torn in half.

## 43. Files go out with a message, and the assistant reads them

A file dropped anywhere on compose, or on the reply box under a conversation,
now goes out with the message. Answer (✨, or
`a`) has the assistant write the reply from the message, what has been typed,
and the files.

### The bytes are in the draft

A `DraftInput` carries its files as base64 in JSON, with no paths. A scheduled
message is stored as its draft, and the file may have moved or changed by send
time. The window reads a dropped file with a `FileReader`, so the core never
opens a path the window names. The envelope previews that run on every change
of recipient leave the files out.

Files over 25 MB together are refused before anything is built. Most
providers refuse larger messages, base64 adds a third, and the server would
only say so after a long upload. The name and type end up in a header, so
`core-smtp` writes the name encoded and without control characters, and a
type that is not a plain `type/subtype` goes out as
`application/octet-stream`. With no type the name decides, since some Linux
desktops have none for a `.docx`.

Dropping needs Tauri's own file-drop handling off (`dragDropEnabled: false`).
With it on, a drop arrives as paths on a Tauri event and never reaches the
page. The page refuses drops wherever files are not taken, or a dropped file
would replace the window.

### Reading a file is turning it into text

`readable.rs` reads:

- Text as it is (UTF-8, or Windows-1252 for an old German CSV).
- HTML through §35's converter.
- A PDF by its text layer.
- Word, OpenDocument, Excel and PowerPoint by the XML inside them, with
  headings, lists and tables as Markdown, which models read best.
- A forwarded message by its headers and text.
- A photograph, or a PDF with no text layer, through the vision model that
  reads letters (§24), page by page, five pages at most.

Everything else is reported as unreadable.

Two crates were added, both MIT: `pdf-extract`, and `zip`, deflate only, whose
`flate2` with `zlib-rs` was already in the tree. A zip entry inflates to 32 MB
at most and a sheet to 300 rows. pdf-extract's panics on malformed files are
caught, so only that file goes unread.

The chat assistant has the same reader as a tool, `read_attachment`.

### Answer looks, and writes, and does nothing else

Answer uses the assistant's model with only the read tools: search, list, read
a message, a conversation, an attachment. It cannot move or trash anything,
and a request for another tool by name is refused. It returns text for
compose. Sending stays with the person.

What was typed is its brief: followed when it reads as instructions, finished
when it reads as a start. It writes `[Datum]` when it does not know a date.
The reply replaces the body through the field's own editing (`insertText`), so
⌘Z brings the notes back.

How much of a file it is shown depends on where the model runs. A local model
often has only a few thousand tokens.

## 44. Safe preview: an attachment cannot reach the system

Pictures and PDFs used to go to the system's readers, ImageIO and PDFKit.
Those are written in C and Objective-C and are where zero-click attacks have
lived: FORCEDENTRY was a JBIG2 image inside a PDF, BLASTPASS a WebP. With safe
preview, on unless turned off in Settings → General, nothing the sender wrote
reaches them. There are four layers (`core-rpc/src/preview.rs`):

1. **Decoders written in Rust.** hayro draws PDFs and forbids unsafe code.
   The `image` crate's codecs draw pictures. Word, Excel, web pages and
   forwarded mail are read as text by §43's reader. A malformed file causes
   an error or a panic, never a write past a buffer.
2. **In a process of its own, locked down before it looks.** The app starts
   itself again with `--preview-worker`, before any window, store or log. The
   worker reads the file from standard input and locks itself down before
   parsing a byte. On macOS it enters the system sandbox with a profile that
   denies everything: no file, no network, no lookup from another program. On
   every Unix it sets resource limits: no new descriptors (so no file, socket
   or pipe), no new processes, nothing written to disk, no core file, a CPU
   budget, and on Linux an address-space limit. A worker that cannot lock
   itself down draws nothing. Its environment is empty.
3. **Bounded, and killed if it is not done.** Thirty seconds on the wall
   clock, 64 MB in, 96 MB out. A picture may claim at most 20000 pixels a
   side and 512 MB of decoding. Pages come two, then four, at a time.
4. **Only pixels come back.** The worker answers with raw RGB and each page's
   size. The app checks that the sizes add up and writes the PNG itself, so
   the window's image decoder only sees a PNG kuverta wrote, even if the
   worker was taken over.

A test has the real, locked-down worker try to write a file, open a socket,
start a program and read a file. Each must fail. With the lockdown removed,
the test fails at the first step.

Since 2026-11-12 the macOS profile is written out in full. It used to be
`kSBXProfilePureComputation`, a named system profile. macOS 27 removed the
named profiles, and a program built against its SDK that asks for one gets
SIGKILL inside `sandbox_init`, before the worker can fail closed and say why.
Every preview then failed with "the preview worker stopped without an
answer". The profile is now two lines of SBPL in `preview.rs`, passed as a
profile, the same call `sandbox-exec -p` makes. It works on every macOS
kuverta runs on.

### What it cannot cover

A file opened in another program is read by that program, which now takes a
second click. Every file kuverta writes, saved to Downloads or copied to be
opened, carries the quarantine attribute on macOS or the Mark of the Web on
Windows, so Gatekeeper checks a program before it runs and Office opens a
document in Protected View. A file that cannot be marked is not kept.

Linux has only the resource limits (next step: a syscall filter). Windows has
only the separate process and the timeout (next: a job object and a
restricted token). The message's own HTML and pictures are still drawn by the
window (see §35).

### Why hayro 0.5

The toolchain is pinned to 1.90, and hayro 0.6 and later need 1.92, through
vello_cpu. 0.5 draws real-world PDFs faithfully, fonts and colour included.
Its only unsafe code is one lifetime transmute in its parser. Move hayro when
the toolchain moves.

The assistant reads attachments through the same worker when the app has one.
The CLI starts no worker and reads in its own process.

## 45. A smart mailbox can take its mail out of the Inbox

A smart mailbox was a saved search, as in Apple Mail, so its mail stayed in
the Inbox. New mail also showed in the Inbox before any rule saw it, because
after a sync the list was drawn before the tasks ran.

A smart mailbox can now be given a folder. It then moves what it gathers out
of the Inbox into that folder on the server, when it is saved and after every
sync, so the phone sees it too. This is chosen per mailbox and on by default
for a new one. Existing searches, and those imported from Thunderbird and
Apple Mail, still move nothing. Saving creates a missing folder, named after
the mailbox without an address in angle brackets and without the characters a
server reads as a path.

### The moving is a task's

A task already does this: rules, a move, after each sync, as a queued change
with Undo, each message once. So the mailbox gets its own task
(`task.smart_mailbox_id`), made, changed and deleted with it, and not listed
among the person's tasks, the assistant's included. Because each message is
handled once, mail put back in the Inbox by hand, or a move undone, stays
there at the next sync.

It acts on the Inbox only, since mail in Archive was filed there on purpose.
This is a scope on the task's search: a mailbox that matches any of its rules
cannot have "and it is in the Inbox" added as another.

### Sorted before it is shown

The window's sync now runs the tasks that need no model before it returns, for
the account that was synced. `runTasks` used the account that was open. A
queued move takes a message out of its list at once, while it waits out its
undo window, so new mail is already sorted when the list is drawn. The model's
tasks still run afterwards in the background (seconds a message).

A mailbox that takes its mail lists its folder instead of its rules, mail
moved there by hand included. The sidebar lists that folder once, as the
mailbox. Deleting the mailbox leaves the folder and its mail on the server,
and the folder is then listed again.

## 46. The To field suggests from the mail there is

To, Cc and Bcc now suggest as they are typed, from the account's own mail:
everyone it has written to (To and Cc) and everyone who has written to it.
There is no separate address book and nothing to import.

Left out: the account itself, senders nobody answers (`noreply`, bounces),
senders of newsletters, marketing and notifications, and mail that is only in
the Trash or the Junk. Ranking is by the amount of mail either way. Sent mail
counts three times as much as received, and each message counts half as much
for every half year of its age. A name is the one the person last signed
with. Someone only ever written to has none, since the store keeps recipients
as addresses.

The whole book, at most 5000 people, is loaded once when compose opens, and
the window filters it on each key. Building it takes 35 ms for twenty thousand
messages, so no index is kept. A new book is fetched after anything was sent,
after mail arrived, and otherwise every five minutes.

A match is the start of the name, of a word in it, of the address, of a part
of its local part, or of the domain: "eri", "muster", "example.de" all find
Erika Mustermann. ↓ ↑ choose, Enter or Tab take, and Escape closes the list
without discarding the message. A name with a comma goes in quoted,
`"Mustermann, Erika" <erika@example.de>`, and the field splits on commas
outside quotes, so it stays one recipient.

## 47. Questions asked in the window, and what Sent shows

### window.confirm never asked

Every action behind a `confirm()` did nothing: deleting a mailbox, a smart
mailbox, a task, a profile, a key or an account, cancelling a scheduled
message, sending what the assistant drafted. Tauri's webview on macOS (wry's
WKWebView) has no handler for JavaScript's dialogs, so WebKit answers
`confirm()` "no" at once and shows nothing. The browser tests ran in Chromium,
which shows the dialog and was told to accept it.

Questions are now asked in the window's own sheet, `ask()`, over whatever is
open, a settings sheet included. The answer names the action (Delete, Remove,
Send). Enter is yes, Escape is no, and no does not close the sheet the
question came from. Every browser test now fails on a native dialog.

### A sent message is in Sent when the send returns

The copy filed in Sent reached the store only with the next sync, minutes
later. The server does not report the UID of an APPEND (without UIDPLUS, which
is not relied on), so the copy cannot be stored directly. The Sent folder is
now synced alone, on the same connection, straight after the append, unless
it is excluded from syncing. Orphans are left alone, as in any sync that has
not seen every folder. A failure there is only logged: the message is filed,
and the next sync finds it.

### Sent names who it went to

A row of mail the account sent now says who it went to, for example "To Erika
Mustermann, bob@example.com and 2 more", with all recipients in its tooltip,
in Sent and wherever else sent mail is listed. The store keeps recipients as
addresses, so names are looked up by address in the sender index and are the
ones those people signed their own mail with. Someone only ever written to is
shown by address. So is anyone who writes their address in capitals, which the
index does not match.

## 48. The assistant on the web, and the skills it is handed

The assistant can now research online when asked, and take on skills (files
of instructions for one kind of job). Each has a switch in the settings.

### Mail could only lie; the web can also listen

The rule so far: text in a message is data, never an instruction. The worst
an injected message could do was move mail (queued, undoable, shown as a
card) or draft something the person sees before sending. Nothing could leave
the computer.

With web tools it can. A model that opens `https://evil.example/?q=` followed
by a bank statement's contents has posted the statement to a stranger, and a
message is enough to tell it to. Three rules prevent this.

Only addresses that were given. The assistant may open an address the person
wrote, an address a search returned, or a link on a page it read. Each is
taken from the structured part of a tool's result, never from page text or a
message. The allow-list is rebuilt from the conversation each time, so it
needs no state. A URL in a snippet's text, a URL in a mail's body, and an
allowed URL with one more query parameter are all refused. To open a link from
a message, the person pastes it into the chat. The search query is the one
address a model composes itself. It is capped at two hundred characters and
the model is told never to put mail in one. What remains, that the search
engine learns what was asked, is stated beside the switch.

Nothing on this computer or its network. Ollama, Paperless and the scanner all
answer HTTP on the local network without a password. The fetcher resolves a
name itself, refuses loopback, private and link-local addresses, and pins the
request to the addresses it checked, so a name that resolves differently the
second time cannot redirect it inside. Each redirect is followed by hand and
checked the same way. The person's own search engine is the one exception,
because they typed its address. It is sent only the query.

Nothing read from the web leads to a change. A page can say "now trash the
inbox", and a model can be talked into it. Once anything from the web has
been read in a question, that question can only look and draft: no moves, no
signing, no note. Once anything from the web has been read in a conversation,
that conversation can never keep a note, create a task or take on a skill.
The person is told in the chat and can ask again in a new question or a new
chat.

### A skill is instructions on purpose, so a person reads it

A skill is somebody else's instructions, meant to be followed, so a person
has to read it first. The assistant, or the settings page, can download a
skill, only from an address the person gave. Nothing in it reaches a model
until the person has had the whole text on screen in the settings and pressed
Accept. Accept sits beneath
the text, after a list of what stood out in it: addresses it names, talk of
hiding, ignoring rules, secrets, or sending something somewhere. No tool can
press it. The list is labelled as words that stood out and is no verdict.

An accepted skill goes into the system prompt fenced, after the rules. The
model is told that a skill adds a way of working and overrides neither the
rule that mail and pages are data nor the tools it has. A skill
fetched again after acceptance waits beside the accepted one, so an update is
read like the first version. One fetched again before anyone read it replaces
the earlier download.

### Off until switched on, and enforced where it counts

Both switches live in the store, because the core enforces them. A tool that
is not switched on is not in the list a model is offered, and a model that
asks for it by name is refused. The window's checkboxes only report what the
core will do. The search engine is DuckDuckGo's HTML page read for its
results, which needs no account and no key, or a SearXNG of the person's own.
The page may change shape or ask for a captcha, and then the assistant says
so.

## 49. A reply goes with its message, and ⌘A means all of them

Three list bugs. ⌘A did nothing, or selected the text of the whole window.
Opening another message left an open reply under a message it did not
answer. A click on a row opened the message twice (`select`, then the click
handler), which fetched everything twice and marked post read twice.

### A reply belongs to what it answers

The pane shows the message above and the answer below, so opening another
message takes a reply or a forward away. With nothing typed it goes
silently: recipients and quoting come from the core and cost nothing to
lose. With anything typed, including what the assistant wrote, the window's
own sheet asks, and "keep writing" puts the cursor back on the message the
reply is for. Escape in the reply asks the same question; it used to
discard the text. Reply pressed again, New message, and a scheduled draft
reopened from the outbox go through the same question. Before, each emptied
compose on its own.

A new message is about nothing on the list, so it stays under whatever is
read.

### ⌘A is every message, and every message is acted on

⌘A picks every row, including rows not fetched yet. The list is sparse, so
the pick is by index, and any action first fetches the missing pages:
"archived 300 messages" means 300. Exception: after the pointer last went
down in the open message, ⌘A selects that message's text. The window
remembers which side the pointer went down on, because neither a paragraph
nor the list can take focus. Marking read with `u` now acts on the pick
too, like every other key.

### A search is in the order the list is in

Search results came back in FTS5 rank order under a toggle that still said
"newest". §12 was right that a search is a different set and wrong about
the ordering: with a list sorted by date, the matches should be by date
too. The store's search now takes the order, by date in either direction
with rank as the tie-break. The window passes the toggle's value with every
search, and flipping the toggle re-runs the search. Rank stays the default
for the assistant and the MCP tool, which want the best match. Post was
already searched through Paperless in date order; the window had not been
passing which date.

## 50. A job with several steps is done to the end

"Put all the application mail in a new mailbox and tell me whether any of
the replies are positive" is five steps for the assistant: search with
alternatives, make the folder, move what was found, read it, report. It
stopped twice. The prompt said to create a folder "if the person agrees",
so it asked. And ten tool rounds ran out while it read one message per
round.

The prompt now says:

- a mailbox or a label is a folder;
- a folder the person asks for is created without asking;
- a job with several steps is done to the end before answering;
- a move of more than thirty messages is still announced first.

`read_messages` reads up to twenty-five messages in one round, each cut to
2,500 characters. The loop allows twenty-five rounds instead of ten. A
folder created a moment ago can be a move target because `create_folder`
writes it to the store as well as the server; a test covers this.

## 51. All inboxes

The window showed one account at a time, and a smart mailbox belonged to
one account. Two accounts meant looking twice, and "Bewerbungen" on both
meant two mailboxes with one name.

*All inboxes* is one list over every account the profile shows, newest
first. It has no account of its own. Each row carries its account, and
every action reads it off the row: opening, archiving, trashing, marking,
moving, replying. The desktop commands that take a message id therefore
take the account as optional and find it from the message. This works
because the ids are the store's own and unique across accounts. Archive and
Trash are per account, looked up once when the list opens. A row whose
account has no such folder says so. Undo is still per account, so z undoes
on the account the last change was queued on.

Smart mailboxes are listed merged by name with counts added up. A merged
one over All inboxes runs its rules over every account; its folder belongs
to one account and is not used. The merge is in the window only. In the
store a smart mailbox still belongs to one account, so tasks, folders and
imports are unchanged. A new one made from here is kept on the first
account shown and listed over all of them. Search asks every account and
sorts the answers together in the list's order. People and Needs attention
stay per account, since both are built on one account's index; the toggle
is not offered here. The choice is kept across restarts, like the profile.

## 52. A head that is never closed

An HTML-only appointment notice opened to its headers and nothing else. Its
HTML went `<head><title>…</title><meta …><body>` with no `</head>`. The
parser's own conversion produced nothing. kuverta's converter, which skips
a `<head>` to its end tag, found none and refused the whole message as
unterminated. The empty result was taken as the sender's text.

The converter now ends the head where a browser does: a `<head>` runs to
`</head>` or to `<body`, whichever comes first. Only a head with neither is
unterminated. And when the parser's conversion is empty and there is HTML,
the HTML is read. Known gap: the list's snippet is made at sync time from
the parser's conversion alone, so such a message still shows no first line
in the list. Its body opens.

## 53. HTML mail as a page

The reading pane showed only text: `html.rs` read an HTML message into
words. That is safe but wrong for a parcel notice whose buttons are
pictures, a ticket whose barcode is a picture, or a newsletter whose layout
is the message. Any message that has both now gets an HTML tab beside Text.
Text stays the default because it cannot be designed to mislead.

The tab does not show the sender's HTML. `html_view.rs` writes a new
document from an allow-list: text, layout elements, their presentation
attributes, and style with everything that fetches or runs removed from the
CSS. Scripts, frames, objects, forms, `<meta>` (a refresh is a navigation),
`<link>`, `<base>`, SVG and every `on…` attribute are not on the list. An
unknown element is dropped and its text kept. As in the text reader,
anything too big, too deep, unterminated or empty is refused whole.

The page goes into an `<iframe sandbox>` with no flags, loaded from the
app's own scheme, `kuverta-mail://`, which `page.rs` answers. `srcdoc` and
`blob:` were ruled out: such a document inherits the window's
Content-Security-Policy, which forbids inline style, and mail is all inline
style. A document from its own scheme gets its own policy: nothing loads
but style and the pictures the policy names, and `sandbox` is repeated in
the header. A WebKit probe confirmed three things first: a header from a
scheme handler is honoured, the parent's policy does not reach into the
frame, and a frame told `204 No Content` stays where it was.

The 204 behaviour makes links work. A link's address is rewritten to
`/open/<secret>?to=…&text=…` on the app's scheme, carrying the real address
and the link's text. A click navigates the frame there, the app answers 204
and tells the window, and the window asks. It shows the real address in
full, what the link said, and a sentence for each oddity:

- not encrypted;
- letters that look like other letters;
- a site before an `@` that is not the site;
- link text that names one site while the address names another.

The secret is made once per start, so a message cannot put that address in
a picture and have it fetched without a click. A `mailto:` opens compose. A
`tel:` is shown to copy. Anything else (`javascript:`, `data:`, `file:`) is
not a link.

Remote pictures tell the sender that the message was opened, and when. The
page is made without them, says how many it left out, and is made again
with them when the person asks, per message. A picture inside the message
(`cid:`) is fetched from the app by its index in the attachments list, and
only if it is a picture. An inline one (`data:image/…`) is kept, since it
cannot reach anywhere.

The page maker is fuzzed like the text reader (`html_page`). Because the
page is written and never copied, the fuzzer also checks for every input
that no element that could run or fetch is named in it. Claims about
particular documents (which attribute survives, where a link goes) are in
`crates/core-rpc/tests/html_view.rs`.

0.5.44: the window hears nothing without a capability. A click on a link in
the page did nothing. The frame navigated to `/open/<secret>` and the
scheme handler answered 204 and told the window, but the window was not
listening. `listen` is a command of Tauri's event plugin, and plugin
commands are checked against the app's capabilities even when the app has
none; the app's own commands pass without any. The window had no
capabilities, so every `listen` was refused, and the refusal went into a
`.catch(() => {})`. The same bug hid the pictures bar and the outbox's
"sent" report. `apps/desktop/capabilities/main.json` now grants
`core:event:allow-listen` and `allow-unlisten` to the main window and
nothing else. A test in `page.rs` keeps it there, and a refused `listen` is
now reported in the window. The jsdom harness cannot see this, because its
stubbed bridge answers every `listen`.

## 54. Claude as a provider of its own

Claude can be added under Settings → Models as a provider, beside Ollama
and the OpenAI-compatible services. It has its own kind, `anthropic`, and
uses Anthropic's Messages API directly (`crates/core-ai/src/anthropic.rs`),
with no OpenAI-compatible shim. The key goes in `x-api-key`, the system
prompt is its own field, a tool call and its result are content blocks
matched by id, and a page is an `image` block. The model list says which
models see images, so a vision job is checked. The key is kept in the
keychain as for any hosted service. It must be an API key from
console.anthropic.com; a Claude.ai subscription does not provide one.

Three differences from the other providers:

- No sampling settings, and room to think. Current Claude models refuse
  `temperature` and think before answering (some cannot be told not to),
  and thinking counts against `max_tokens`. A one-word judgement is
  therefore asked at low effort with at least 2,048 tokens of room, billed
  by what is used. A model that refuses an effort level (older ones) is
  asked again without one and not sent one from then on, as the OpenAI
  client does with `reasoning_effort`.
- Thinking is never sent back. Claude checks that the conversation behind a
  thinking block is unchanged, and the assistant changes its history: mail
  withheld after a refusal, a cut-off answer joined to its continuation.
  Each turn goes back as its text and tool calls only, which every Claude
  model accepts. A round offered no tools sends earlier calls and results
  as text, so no request carries tool blocks without tools.
- A refusal is an error. Claude's safety classifiers answer with
  `stop_reason: "refusal"` and a success status. That becomes
  `AiError::Refused`, which the assistant treats as a content filter: the
  question is asked once more without the mail.

`ai_provider`'s CHECK now allows `anthropic`. SQLite cannot alter a CHECK,
so migration 24 rebuilds the table with foreign keys off for its duration
and keeps every job that points at a provider.

---

## 55. A pull request is one change

**2026-10-20.**

A pull request (or merge request) carries one feature or one bug fix and
nothing else. It can be described in one sentence without an "and", and it
is merged or sent back as one thing.

Why: "Let a closing letter wait for its pages to be read, and run Paperless
commands as its user" (`9b5d5a4`) is a scanner race fix plus a
container-user fix in the rig setup. "Set the rig up from the repo, with
Paperless on it, and say when a letter starts" (`b3fe792`) is three
changes. Each landed as one, so each can only be reviewed, bisected or
reverted as one. A regression in the Paperless user handling cannot be
undone without undoing the race fix, and the diff for the rig move is
mostly about something else.

The rules:

- One concern per pull request: a feature, a bug fix, a refactor that
  changes no behaviour, or a dependency bump. No combinations and no
  drive-bys.
- Anything else found on the way gets a second pull request. If it is
  needed first, it goes first on its own and the feature branch rebases on
  it.
- The title says the one thing. If it needs an "and", split the branch.
  Commit titles here already say what the change does for the person.
- Size does not matter. A feature that touches twenty files is one pull
  request. Two unrelated one-line fixes are two.

Renames and reformatting that a change requires go with it; nearby ones do
not. The same goes for tests: only a test for the thing being changed
belongs to the change.

Cost: more and smaller pull requests, sometimes chained. A fix that depends
on a refactor waits for it to merge or is stacked on its branch. No tooling
enforces this. The author and the merger do, by reading the title.

Benefit: a commit on `main` is one change, so `git log` is readable,
`git bisect` names one cause and a revert removes one thing. Reviews are
small. A release note is the list of titles.

Releases already fit the rule: `make release` commits "Release x.y.z" with
the version bump alone (see `docs/releasing.md`).

---

## 56. A thing found in mail is offered, never applied

**2026-10-31.**

An invitation now has a card above the message: what the meeting is, when,
where, who else was asked, and, where it applies, why the card has no
buttons. Until now a `text/calendar` part was a 📅 in the attachment row,
and the `.ics` had to be opened in another program.

The decision is the shape, because three more kinds of thing will use it: a
todo taken from a sentence, a line off a bank statement, a document that
belongs in Paperless. `core-offers` holds only what they share: where it
was found, what identifies the thing, whether its sender is a stranger, and
whether it may be answered. What only an invitation needs (method,
attendees, repetition) sits in a typed payload beside that. `core-paper`
does the same by giving a document the shapes a message has (§7): unify on
the data, and the surface above learns it once.

### Three rules, each of which was cheaper to keep now

An offer is offered, never applied. Reading mail creates nothing outside
kuverta, so syncing a folder twice is as harmless as once. The same
invitation arrives more than once (forwarded, re-synced, or filed under two
Gmail labels), each time as a different message row. The `handoff` ledger
(migration 26) is keyed by the event's `UID`, so the second copy says "you
have already answered this one". `SEQUENCE` is in the key's row because an
organiser who moves the meeting sends the same `UID` with a higher
revision, and that is a new question. Mail arrives out of order, so a row
is never replaced by a lower revision.

No instant is invented. `DTSTART;TZID=Europe/Berlin:20261104T140000` is a
wall clock, and resolving it needs a zone database that `core-offers` does
not carry. An epoch second is filled in only where the sender gave a time
that needs no lookup. Otherwise the card shows the digits from the line
with the zone named beside them, because a wrong hour is worse than none.
The core decides this. The fuzz target (`read_invite`) asserts for every
input: if an instant is set, no zone was named and it was not a whole day.

A stranger's invitation gets no answer. Answering confirms that the address
is read, which is what an unsolicited invitation wants. Inbound volume is
no test, since the sender controls it: a newsletter that has written fifty
times is still a stranger. The test is whether this mailbox has ever
written to them, which nobody outside can fake. The core decides it, as §48
requires, and the card reports the result.

### What it has not got, on purpose

There is no sink. Nothing writes to a calendar, a reminder list or an
archive, and nothing sends a `METHOD:REPLY` yet. Those are two separate
decisions: sending mail as the person, and reaching outside the app. The
card says that answering is not built; it has no Accept that would fail.

This keeps the built half the same on every platform. macOS has one system
calendar behind one framework. Windows has nothing comparable that a
desktop program may use, and on Linux it is Evolution's data server or
nothing. The portable half is the card plus an `.ics` handed to whatever
the system opens it with. An EventKit adapter, if it comes, is optional and
nothing may depend on it.

Scope: kuverta reads invitations and will answer them. It is not a
calendar: no event store, no calendar view, no expanding an `RRULE` beyond
showing that the event repeats. This guards the non-goals list in the
plan's §1.

### The parser is one that reads what strangers wrote

`calcard` reads the iCalendar. It is from Stalwart, like `mail-parser`, and
fits the plan's "lean on, don't rewrite" list. It parses untrusted input,
so it falls under §38 and §44 and went into `fuzz/` the day it landed: it
must return, say the same thing twice, and never produce more than it was
given.

## 57. Letters on paper go through LetterXpress, and only when asked

**2026-11-03.**

A PDF document can be posted on paper: LetterXpress prints it, puts it in a
window envelope and hands it to Deutsche Post. It was chosen over Binect
and Pingen because it is the cheapest of the three for German post (about
0.80 € a standard letter, postage included), has no contract or minimum,
and its API is one JSON request with the PDF in it. Pingen has the nicer
API (webhooks, other countries) and is the one to revisit if letters often
need to leave Germany. `core-letter` is the client; nothing else in the
tree knows the service's name.

The address is on the page. LetterXpress reads it from the window position
of a DIN envelope, form B of DIN 5008: 20 mm from the left, 45 mm from the
top, 85 mm wide. `write_pdf` takes a recipient and a return address and
sets them there, with the text starting below. What LetterXpress read comes
back with the job and is shown on the document's card. That is the only
check that it read the address right.

A PDF with no address there (a form, a scan, someone else's document) gets
one typed into the send sheet. It is put either on white over the first
page's address field or on a cover sheet in front. With double-sided
printing the cover sheet gets an empty back, so the letter still starts on
its own sheet. This is done to the posted copy, never to the document, and
"Show the letter" opens exactly what will be sent. The return address is
remembered for the next letter.

Test mode is on until the person turns it off. In test mode a letter goes
to the Postbox on the LetterXpress site and is neither printed nor charged,
so the first letter can be checked before paying. Going live is a setting,
confirmed once. The send button names the price LetterXpress quoted for the
chosen options.

The letter is written down before it goes. A letter costs money and cannot
be unposted, so it follows the outbox's rule: the row is written as
`sending` before LetterXpress is asked. After a crash in between, the
letter says it may have gone. Nothing resends it automatically.

The assistant cannot post. It can write a letter ready for the envelope,
but posting is a button on the card behind a sheet that shows the price,
and no tool offers it to a model. A letter is an action in the world with a
bill attached, which is the line §56 draws for everything found in mail.

## 58. A PDF locked with a password is asked about, not refused

**2026-11-12.**

Safe preview reported a PDF it could not draw (password-protected or
damaged) and stopped, which left bank statements, payslips and insurance
policies unreadable. The password is now asked for.

Most locked PDFs never ask. The standard security handler has two
passwords: the owner's, which restricts printing, copying and changing, and
the user's, which locks reading. Most locked PDFs set only the owner's, and
the empty user password opens them. hayro gets the empty password on the
first look, so those draw as before. Only a real user password leads to a
question.

The worker asks, and the answer goes back to it. The worker answers
`Password { wrong }` instead of pages and the viewer shows a field. What is
typed goes along in the next request and unlocks the file inside the
worker, where decrypting is the same Rust code in the same locked-down
process that does the drawing. Nothing is decrypted in the app's own
process.

The password is not kept. It lives in the viewer's state while the viewer
is open, because paging needs it again, and is gone when the viewer closes.
It is not written to the store, not put in the keychain and not logged. A
wrong password is dropped and not retried. Remembering it would keep the
key to someone's bank statements beside the statements, which the store is
designed to avoid.

Still refused: a PDF locked with an algorithm hayro does not implement says
so and does not ask for a password. The assistant reading an attachment has
nobody to ask, so a locked PDF stays unreadable to it. Signing or posting
one still needs the unlocked file.

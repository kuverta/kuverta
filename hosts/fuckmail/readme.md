# The `fuckmail` host

Runs the shared triage surface inside the standalone client, against the same
Tauri commands its own UI already uses.

## Mounting it

The desktop app loads `apps/desktop/ui/index.html` from its own tree, so the
shared files have to be where that page can reach them:

```sh
# from this repository
cp -R core            ../fuckmail/apps/desktop/ui/fuckbird-core
cp -R hosts/fuckmail  ../fuckmail/apps/desktop/ui/fuckbird-host
```

Then point a page at `fuckbird-host/ui/triage.html`, or import the surface from
the existing `app.js`:

```js
import { Triage } from './fuckbird-core/triage.js';
import { mountTriage } from './fuckbird-core/view/triage.js';
import { FuckmailHost } from './fuckbird-host/host.js';

const host = await new FuckmailHost(invoke, account).open();
const triage = new Triage(host);
mountTriage({ root: document.getElementById('triage'), triage });
await triage.start();
```

Copying rather than importing across repositories is deliberate for now: the
alternative is a package step, and there is no version of this worth publishing
yet. The copy is verbatim, so `diff -r` says whether it has gone stale.

## What this host can and cannot do

Declared in `capabilities`, and the surface reads it rather than assuming:

| | |
|---|---|
| archive, trash | yes, when the account has those folders |
| read state | yes |
| search | yes |
| sync | yes, and it reports refused changes |
| undo | **`queued`** — a change waits in the undo window, so undoing cancels something that never happened |
| file by category | yes, since `set_category` |

Filing is the one change that is *not* queued, and deliberately: a category
lives in the store and never reaches a server, so there is no round trip to
hold back and nothing for the undo window to protect. Changing your mind is
another correction — an event, not an edit, which is what the log wants anyway.

### How filing got here

`fuckmail` had the whole of stage 3 built and unreachable. The `correction`
table, `record_correction`, `learned_categories`, and the `Learned` →
`Classifier` loop at the top of every sync had all been there since early on,
with end-to-end tests — and `record_correction` was called by nothing but those
tests. There was no CLI command, no RPC method and no UI path to it.

What was added is the exposure, plus one thing the exposure needed:

- `ClassifierSource::User`, so a correction is not recorded as a rules verdict.
  Filing it as `rules` would have been a line shorter and would have put the
  user's answers into the baseline the model of brief §3.3 is supposed to be
  measured against.
- `CURRENT_CATEGORY_JOIN` in `core-store`, one definition of "the category a
  message is shown under" — a user verdict if there is one, else the most
  recent rules verdict — shared by the three queries that need to agree, while
  `disagreements` deliberately still reads `rules` only.
- `Core::set_category`, which validates against `core-rules`, records the
  correction and the verdict, and does nothing at all if the message is already
  filed there.
- The `set_category` Tauri command.

That join also fixed a counting bug on the way past: `category_counts` joined
every classification row, so a message reclassified between syncs was counted
under every category it had ever been given, and the sidebar added up to more
messages than the mailbox held.

## Keeping the adapter honest

`test/fuckmail-host.test.js` runs the host contract against a fake `invoke`
whose shapes are taken from `core-rpc`: `date_utc` in seconds, snake_case keys,
`category_counts` answering with pairs rather than an object, `messages` taking
a folder id rather than a name, and an unknown command throwing rather than
returning null. A command that drifts in `core-rpc` will not be caught by that
fake — only by running it — so the fake is a guard against this adapter
drifting, not against the client changing underneath it.

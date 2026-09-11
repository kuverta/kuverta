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
| **file by category** | **no** |

The last one is the gap. `core-rules` classifies during sync and no command
exposes a correction, so `setCategory` is declared false, the surface hides the
category keys, and the corrections log of brief §3.4 collects nothing on this
host.

Closing it is one command in `fuckmail`:

```rust
#[tauri::command]
fn set_category(app: State<'_, App>, account: i64, id: i64, category: String)
    -> Result<(), String>
```

writing the category to the message row and appending the correction to a
table. Once it exists, flip `setCategory` to true here and the keys appear —
nothing else in the surface changes, which is the point of the capability
object.

## Keeping the adapter honest

`test/fuckmail-host.test.js` runs the host contract against a fake `invoke`
whose shapes are taken from `core-rpc`: `date_utc` in seconds, snake_case keys,
`category_counts` answering with pairs rather than an object, `messages` taking
a folder id rather than a name, and an unknown command throwing rather than
returning null. A command that drifts in `core-rpc` will not be caught by that
fake — only by running it — so the fake is a guard against this adapter
drifting, not against the client changing underneath it.

# The `kuverta` host

Runs the shared triage surface inside the standalone client, on the Tauri
commands its own UI already uses.

## Mounting it

From the repository root:

```sh
make triage-ui
```

That copies `kuverta-bird/core/` and this directory into
`apps/desktop/ui/kuverta-bird/`. Then open the app and click **triage** in the
header.

Tauri serves one directory as the web root, so the page cannot reach files
outside `apps/desktop/ui/`. The copy mirrors this directory exactly:

```
apps/desktop/ui/kuverta-bird/core/...
apps/desktop/ui/kuverta-bird/hosts/kuverta/...
```

Relative imports resolve unchanged and the app loads the same files the tests
run. The install checks this with `diff -r`. Paths are not rewritten, because
that would be a build step between tested and running code. The Thunderbird
manifest sits at `kuverta-bird/manifest.json` for the same reason.

The copy is gitignored. Re-run `make triage-ui` after changing anything under
`kuverta-bird/core/`.

## What this host can and cannot do

Declared in `capabilities`, which the surface reads:

| | |
|---|---|
| archive, trash | yes, when the account has those folders |
| read state | yes |
| search | yes |
| sync | yes, and it reports refused changes |
| undo | **`queued`**: a change waits in the undo window, so undoing cancels something that never happened |
| file by category | yes, since `set_category` |

Filing is the one change that is not queued. A category lives in the store and
never reaches a server, so the undo window has nothing to hold back. To change
your mind, file again: the log records it as another correction.

### How filing got here

`kuverta` already had all of stage 3, unreachable. The `correction` table,
`record_correction`, `learned_categories`, and the `Learned` → `Classifier`
loop at the top of every sync existed with end-to-end tests, but only the tests
called `record_correction`. There was no CLI command, RPC method or UI path.

Added:

- `ClassifierSource::User`, so a correction is not recorded as a rules verdict.
  Recording it as `rules` would put the user's answers into the baseline the
  model of brief §3.3 is measured against.
- `CURRENT_CATEGORY_JOIN` in `core-store`, the single definition of "the
  category a message is shown under": a user verdict if there is one, else the
  latest rules verdict. Three queries share it. `disagreements` still reads
  `rules` only, on purpose.
- `Core::set_category`, which validates against `core-rules`, records the
  correction and the verdict, and does nothing if the message is already filed
  there.
- The `set_category` Tauri command.

The join also fixed a counting bug. `category_counts` joined every
classification row, so a message reclassified between syncs was counted under
every category it ever had, and the sidebar total exceeded the mailbox.

## Keeping the adapter honest

`test/kuverta-host.test.js` runs the host contract against a fake `invoke`
shaped like `core-rpc`: `date_utc` in seconds, snake_case keys,
`category_counts` answering with pairs instead of an object, `messages` taking
a folder id instead of a name, and an unknown command throwing instead of
returning null. The fake catches drift in this adapter. Drift in `core-rpc`
only shows when the client is run.

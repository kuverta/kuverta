---
description: kuverta-bird — the triage surface and classifier as a Thunderbird extension, from the same code the desktop app mounts.
---

# Thunderbird extension

`kuverta-bird/` is the JavaScript half of the project: the classifier, the
triage model, the surface you know as [Cleanup](../user-guide/cleanup.md), and
one adapter per host. The same code runs in two places:

- **inside kuverta**, through the Tauri commands the desktop app speaks — `make
  triage-ui` copies it into `apps/desktop/ui`;
- **as a Thunderbird extension**, which is what makes Thunderbird's add-on
  ecosystem available alongside it.

Thunderbird extensions need Gecko, and Gecko cannot be embedded outside
Mozilla's own applications, so rather than choose, the surface was written once
and given two hosts. `docs/thunderbird-fork-brief.md` has the reasoning, and
`docs/decisions.md` what it has cost.

## Trying it in Thunderbird

Requires **Thunderbird 128 or newer**.

1. **Tools → Developer Tools → Debug Add-ons**
2. **Load Temporary Add-on…** and pick `kuverta-bird/manifest.json`.

There is no build step: the files Thunderbird loads are byte for byte the files
the tests run. For something installable, `npm run package` in `kuverta-bird/`
produces `kuverta-bird.xpi`.

New mail is filed as it arrives. For mail you already have, right-click a folder
→ **kuverta-bird: classify this folder**. The toolbar button opens the triage
tab. It uses the same [keys](../user-guide/shortcuts.md#cleanup) as Cleanup in
kuverta, and files categories as Thunderbird tags.

Neither host has been used day to day in Thunderbird yet; expect the first real
load to turn up behaviour that differs from what the documentation promised,
and please [report it](../user-guide/diagnostics.md#reporting-a-bug).

## Layout

```text
kuverta-bird/
  core/                the whole idea, and no host in it
    category.js        the six categories
    signals.js         the heuristics and their weights
    classify.js        scoring, confidence, learned overrides
    facts.js           headers → the facts the classifier reads
    corrections.js     the corrections log
    host.js            the port: what a host must do, and what it may not
    triage.js          the list: cursor, scopes, actions
    keys.js            the key map, written down once
    view/triage.js     the surface both hosts mount
  hosts/thunderbird/   an adapter over the MailExtension API
  hosts/kuverta/       an adapter over the client's Tauri commands
```

`core/` knows about neither host, and that is enforced by a test: it fails if
anything under `core/` mentions `messenger.`, `__TAURI__` or `browser.`, or if
one host imports the other. What a host can do that another cannot is declared
in a `capabilities` object, so the surface hides what is unavailable instead of
offering it and failing.

The **host contract** is a test suite, `test/support/conformance.js`, run
against the Thunderbird adapter (over a fake `messenger`), the kuverta adapter
(over a fake `invoke`) and an in-memory reference host. Capability gates report
as skips with a reason, never as passes.

The classifier is a line-for-line port of the Rust `core-rules`: over the same
250 generated messages it produces identical verdicts and identical confidences
to six decimal places.

## Working on it

```sh
cd kuverta-bird
npm test        # the model, the adapters, the classifier, the view in jsdom
npm run e2e     # the triage page and the settings sheet in real Chromium
npm run corpus  # the classifier over 250 generated messages, with the rules that fired
```

The extension uses Manifest V2, for the reasons in `docs/decisions.md` §3.

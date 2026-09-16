/**
 * That the manifest asks for what the code actually uses.
 *
 * Written after four permissions were found missing by reading the docs rather
 * than by running anything: `messagesMove` (so archive and trash would have
 * failed), `messagesTagsList` (so creating the tags would have failed at the
 * first `tags.list`), `compose`, and `tabs` — which `tabs.query({url})` needs
 * because it filters on a URL.
 *
 * None of the fakes could catch these: a fake answers whatever it is asked,
 * permissions and all. The only things that can are Thunderbird itself and a
 * table like this one.
 *
 * The table is from the Manifest V2 API documentation, checked on 2026-09-11.
 * It is the assumption made explicit, not a second source of truth — if
 * Thunderbird disagrees, Thunderbird is right and this table is what gets
 * corrected.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const HOST = join(ROOT, 'hosts', 'thunderbird');

/** `messenger.<path>` -> the permissions it needs. */
const REQUIRES = {
  'accounts.list': ['accountsRead'],
  'browserAction.onClicked': [], // needs the browser_action manifest key, not a permission
  'compose.beginNew': ['compose'],
  'menus.create': ['menus'],
  'menus.onClicked': ['menus'],
  'menus.removeAll': ['menus'],
  'messageDisplay.getDisplayedMessage': ['messagesRead'],
  'messages.continueList': ['messagesRead'],
  'messages.get': ['messagesRead'],
  'messages.getFull': ['messagesRead'],
  'messages.list': ['accountsRead', 'messagesRead'],
  'messages.move': ['accountsRead', 'messagesMove', 'messagesRead'],
  'messages.onNewMailReceived': ['accountsRead', 'messagesRead'],
  'messages.query': ['messagesRead'],
  'messages.tags.create': ['messagesRead', 'messagesTags'],
  'messages.tags.list': ['messagesRead', 'messagesTagsList'],
  'messages.update': ['messagesRead', 'messagesUpdate'],
  'notifications.create': ['notifications'],
  'runtime.getURL': [],
  'runtime.onMessage': [],
  'runtime.sendMessage': [],
  'storage.local': ['storage'],
  'tabs.create': [],
  'tabs.update': [],
  // Filtering by URL is what pulls the `tabs` permission in; without it the
  // query silently returns nothing and a second triage tab opens each time.
  'tabs.query': ['tabs'],
};

/**
 * Reading `MessageHeader.folder` needs `accountsRead` even though no call
 * names it — an easy one to lose, because without the permission the property
 * is simply absent rather than an error, and the adapter needs it to know
 * where a message came from before moving it.
 */
const PROPERTY_PERMISSIONS = ['accountsRead'];

function hostSources(dir = HOST, found = []) {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) hostSources(path, found);
    else if (entry.endsWith('.js')) found.push(path);
  }
  return found;
}

function callsUsed() {
  const used = new Set();
  for (const file of hostSources()) {
    const text = readFileSync(file, 'utf8');
    for (const [, path] of text.matchAll(/messenger\.((?:[a-zA-Z]+\.){1,2}[a-zA-Z]+)/g)) {
      // `messages.tags.list` is three segments; `messages.get` is two. Prefer
      // the longest entry in the table that matches.
      const parts = path.split('.');
      const three = parts.slice(0, 3).join('.');
      const two = parts.slice(0, 2).join('.');
      used.add(three in REQUIRES ? three : two);
    }
  }
  return used;
}

const manifest = JSON.parse(readFileSync(join(ROOT, 'manifest.json'), 'utf8'));
const granted = new Set(manifest.permissions);

test('every Thunderbird API the code calls is in the table', () => {
  // Forces the table to be updated when a new API is used, which is the only
  // way it stays a check rather than a snapshot of what was true once.
  const unknown = [...callsUsed()].filter((call) => !(call in REQUIRES));
  assert.deepEqual(
    unknown,
    [],
    `these calls have no documented permission recorded:\n${unknown.join('\n')}`,
  );
});

test('the manifest requests every permission those APIs need', () => {
  const needed = new Set(PROPERTY_PERMISSIONS);
  for (const call of callsUsed()) {
    for (const permission of REQUIRES[call] ?? []) needed.add(permission);
  }

  const missing = [...needed].filter((permission) => !granted.has(permission)).sort();
  assert.deepEqual(missing, [], `the manifest is missing:\n${missing.join('\n')}`);
});

test('the manifest requests nothing it does not need', () => {
  // A mail client asking for more than it uses is asking the user to trust it
  // for no reason, and an unused permission is usually a leftover from code
  // that has gone.
  const needed = new Set(PROPERTY_PERMISSIONS);
  for (const call of callsUsed()) {
    for (const permission of REQUIRES[call] ?? []) needed.add(permission);
  }

  const extra = [...granted].filter((permission) => !needed.has(permission)).sort();
  assert.deepEqual(extra, [], `requested but unused:\n${extra.join('\n')}`);
});

test('the browser_action key is present, since no permission grants it', () => {
  assert.ok(manifest.browser_action, 'browserAction is used but not declared');
});

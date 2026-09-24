/**
 * The folders the setup assistant offers, against what Paperless will take.
 *
 * Each is a Paperless tag whose words go in its match field, and Paperless
 * keeps 256 characters of that. A suggestion longer than the limit is offered
 * cheerfully and then refused on save, which is the worst way to find out —
 * so the lengths are checked here rather than on a rig.
 *
 * It lives here because this is the repository's JavaScript test runner; the
 * file it reads is the desktop app's.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const UI = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', 'apps', 'desktop', 'ui');
/// `folders::MATCH_LIMIT` in apps/scannerd.
const MATCH_LIMIT = 256;

/** The suggestions, read out of the file that defines them. */
function suggestions() {
  const source = readFileSync(`${UI}/setup.js`, 'utf8');
  const block = source.slice(source.indexOf('const SHELF_SUGGESTIONS = ['));
  return [...block.slice(0, block.indexOf('\n];')).matchAll(/\{ name: "([^"]+)", words: "([^"]+)" \}/g)]
    .map(([, name, words]) => ({ name, words }));
}

test('every suggested folder fits in the match field Paperless keeps', () => {
  const tooLong = suggestions()
    .filter((folder) => folder.words.length > MATCH_LIMIT)
    .map((folder) => `${folder.name}: ${folder.words.length}`);
  assert.deepEqual(tooLong, [], `Paperless keeps ${MATCH_LIMIT} characters of a tag's words`);
});

test('a suggested folder has a name and words that are words', () => {
  const offered = suggestions();
  assert.ok(offered.length >= 6, `there are ${offered.length} suggestions`);
  for (const { name, words } of offered) {
    assert.ok(name.trim(), 'a folder with no name');
    const list = words.split(',').map((word) => word.trim());
    assert.ok(list.length >= 5, `${name} has only ${list.length} words`);
    assert.ok(!list.some((word) => !word), `${name} has an empty word`);
    // A word every official letter carries would put every letter in that
    // folder, which is the one mistake these lists must not make.
    for (const common of ['Datenschutz', 'Betreff', 'Sehr geehrte', 'Anlage', 'Seite']) {
      assert.ok(
        !list.some((word) => word.toLowerCase() === common.toLowerCase()),
        `${name} matches on "${common}", which every letter has`,
      );
    }
  }
});

test('no two folders are offered the same distinctive word', () => {
  const seen = new Map();
  const shared = [];
  for (const { name, words } of suggestions()) {
    for (const word of words.split(',').map((w) => w.trim().toLowerCase()).filter(Boolean)) {
      if (seen.has(word) && seen.get(word) !== name) shared.push(`${word}: ${seen.get(word)} and ${name}`);
      seen.set(word, name);
    }
  }
  // A word in two folders puts a letter in both, which is not wrong but is
  // never what these lists mean.
  assert.deepEqual(shared, []);
});

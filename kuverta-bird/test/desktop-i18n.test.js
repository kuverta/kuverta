/**
 * Every string the desktop window asks for has a German translation.
 *
 * The catalogue is keyed by the English text, so an untranslated string is
 * not an error anywhere — it simply shows in English, which is why nothing
 * else would notice. This does: it reads every `t("…")` in the window's own
 * files and asks the catalogue for each one.
 *
 * It lives here because this is the repository's JavaScript test runner; the
 * files it reads are the desktop app's.
 *
 * What it cannot see: a key handed to `t` as a variable — the urgency levels,
 * a smart mailbox's fields and operators, which are table values. Those are
 * kept together in their own section of the catalogue, where they can be read
 * against the tables by eye.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const UI = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', 'apps', 'desktop', 'ui');

/** Every key passed to `t` as a plain string, with the file it came from. */
function keysInUse() {
  const found = new Map();
  for (const name of readdirSync(UI).filter((f) => f.endsWith('.js') && f !== 'i18n.js')) {
    const source = readFileSync(`${UI}/${name}`, 'utf8');
    for (const match of source.matchAll(/\bt\(\s*"((?:[^"\\]|\\.)*)"/g)) {
      found.set(JSON.parse(`"${match[1]}"`), name);
    }
  }
  return found;
}

/** The catalogue, as the window would read it. */
async function german() {
  const source = readFileSync(`${UI}/i18n.js`, 'utf8');
  // The file is a classic script, not a module: it defines globals for the
  // window rather than exporting. Evaluating it in a function that hands back
  // what the test needs is enough, and keeps the file itself free of test
  // scaffolding.
  const load = new Function(`${source}\nreturn { GERMAN, t, CATALOGUES };`);
  return load();
}

test('every string the window asks for is in the German catalogue', async () => {
  const { GERMAN } = await german();
  const missing = [...keysInUse()]
    .filter(([key]) => !Object.hasOwn(GERMAN, key))
    .map(([key, file]) => `${file}: ${JSON.stringify(key)}`);

  assert.deepEqual(missing, [], `untranslated strings:\n${missing.join('\n')}`);
});

test('no translation loses a placeholder the English has', async () => {
  const { GERMAN } = await german();
  const holes = (text) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

  const wrong = [];
  for (const [english, deutsch] of Object.entries(GERMAN)) {
    const want = holes(english);
    const got = holes(deutsch);
    // A dropped `{count}` is a sentence with a hole where its number was, and
    // a renamed one prints the braces at whoever is reading.
    if (want.join() !== got.join()) wrong.push(`${JSON.stringify(english)} → ${JSON.stringify(deutsch)}`);
  }

  assert.deepEqual(wrong, [], `placeholders do not match:\n${wrong.join('\n')}`);
});

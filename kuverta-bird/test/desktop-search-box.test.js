/**
 * The search box against the list it describes.
 *
 * Typing `cadus` and then clicking Sent showed the whole of Sent with `cadus`
 * still in the box, as though that were what had been asked for. Every reload
 * leaves a search behind — it clears `state.searching` — so a query left in
 * the box describes a list that is not on screen.
 *
 * This is a structural check, not a behavioural one, and that is a real
 * limitation: the desktop window has no harness that boots it, so what is
 * pinned here is that the clearing is inside `reload` (one place, rather than
 * repeated in each of the dozen nav handlers that could forget it) and that
 * People is exempt. People is the subtle half — the box there is not a search
 * but a filter `people.js` reads for itself, so clearing it would empty the
 * page somebody had just narrowed.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const UI = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', 'apps', 'desktop', 'ui');
const app = () => readFileSync(`${UI}/app.js`, 'utf8');

/** The body of `async function reload(...)`, to its closing brace. */
function reloadBody(source) {
  const from = source.indexOf('async function reload(');
  assert.ok(from > 0, 'reload() moved');
  // Past the parameter list, which is itself destructured with braces —
  // starting at the first `{` counts `({ keepPosition = false } = {})` as the
  // whole body and finds nothing in it.
  const opens = source.indexOf(') {', from);
  assert.ok(opens > from, 'reload() has no body');
  let depth = 0;
  for (let at = opens + 2; at < source.length; at += 1) {
    if (source[at] === '{') depth += 1;
    else if (source[at] === '}') {
      depth -= 1;
      if (depth === 0) return source.slice(from, at + 1);
    }
  }
  throw new Error('reload() has no end');
}

test('a reload empties the search box, so the box and the list agree', () => {
  const body = reloadBody(app());
  assert.match(
    body,
    /searchBox\.value = ""/,
    'reload() leaves the search behind but does not clear the box',
  );
  assert.match(
    body,
    /state\.searching = false/,
    'reload() should still be the place a search ends',
  );
});

test('People keeps what was typed, because there it is a filter and not a search', () => {
  const body = reloadBody(app());
  const clearing = body.slice(0, body.indexOf('searchBox.value = ""'));
  const guard = clearing.slice(clearing.lastIndexOf('\n', clearing.length - 2));
  assert.match(
    guard,
    /state\.view !== "people"/,
    'the box is cleared in People too, which empties the list somebody just narrowed',
  );

  // And People really does read the box, which is what makes the guard
  // necessary rather than decorative.
  const people = readFileSync(`${UI}/people.js`, 'utf8');
  assert.match(people, /searchBox\.value/, 'people.js no longer reads the box; the guard is stale');
});

test('no nav handler clears the box on its own', () => {
  // One place, or the next folder added forgets to do it.
  const source = app();
  const inReload = reloadBody(source);
  const everywhere = [...source.matchAll(/searchBox\.value = ""/g)].length;
  const inside = [...inReload.matchAll(/searchBox\.value = ""/g)].length;
  // `reload`, the "clear" button beside the scope, and Escape in the box.
  assert.ok(inside >= 1, 'reload() does not clear the box');
  assert.ok(
    everywhere - inside <= 2,
    `${everywhere - inside} places outside reload() clear the box; it belongs in one`,
  );
});

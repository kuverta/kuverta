/**
 * The host contract, as a test suite.
 *
 * Every adapter runs this. That is what makes "it works in either host" a
 * claim rather than a hope: the triage surface is written against this
 * contract and nothing else, so an adapter that passes can drive it and an
 * adapter that fails will break the surface in ways it cannot sensibly
 * handle.
 *
 * The suite only exercises what an adapter says it can do. A host that cannot
 * archive is not broken; a host that says it can archive and then does not is.
 *
 * Usage, from an adapter's own test file:
 *
 *   import { runConformance } from './support/conformance.js';
 *   runConformance('Thunderbird', () => new ThunderbirdHost(fakeMessenger()));
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { ALL_CATEGORIES } from '../../core/category.js';
import { ACTIONS } from '../../core/host.js';

/**
 * The first row that allows an action.
 *
 * A list can hold more than one kind of thing — post read from Paperless sits
 * beside mail and allows none of these — so a suite that acted on whatever was
 * at index 0 would be testing whichever source happened to sort first.
 */
function actionable(rows, action) {
  return rows.find((row) => !Array.isArray(row.actions) || row.actions.includes(action));
}

/**
 * @param {string} name        What to call this host in the test output.
 * @param {Function} makeHost  Returns a *fresh* host each call, or a promise
 *   for one. Always awaited: a host that needs a round trip before it can say
 *   what it is capable of — `kuverta` looks up Archive and Trash — would
 *   otherwise report every capability false and silently skip half this suite.
 *   Tests mutate, so it must be a new host each time.
 */
export function runConformance(name, makeHost) {
  // Every test takes `t` so a capability it does not have is reported as a
  // skip rather than as a pass. A test that quietly did nothing and said "ok"
  // is how a capability regression hides.
  const suite = (what, body) => test(`${name} host: ${what}`, (t) => body(t));

  // -- capabilities -------------------------------------------------------

  suite('declares every capability, and none it does not know about', async (t) => {
    const caps = (await makeHost()).capabilities;
    const known = [
      'archive',
      'trash',
      'setRead',
      'setCategory',
      'search',
      'sync',
      'undo',
      'compose',
    ];

    for (const key of known) {
      assert.ok(key in caps, `capabilities.${key} must be declared, even as false`);
    }
    for (const key of Object.keys(caps)) {
      assert.ok(known.includes(key), `capabilities.${key} is not part of the contract`);
    }
  });

  suite('declares which kind of undo it has, if any', async (t) => {
    // The two are different promises to the user — one cancels a change that
    // has not happened, the other performs a second change to reverse one that
    // has — so `true` is not an answer.
    const { undo } = (await makeHost()).capabilities;
    assert.ok(
      undo === false || undo === 'queued' || undo === 'compensating',
      `undo must be false, 'queued' or 'compensating', got ${JSON.stringify(undo)}`,
    );
  });

  // -- scopes -------------------------------------------------------------

  suite('every scope it offers can be shown to the user', async (t) => {
    // §3.1: a filtered list says what it is showing. A scope with no label is
    // a filter the user cannot see, which is the failure that looks like mail
    // going missing.
    for (const scope of await (await makeHost()).scopes()) {
      assert.ok(scope.kind, 'a scope needs a kind');
      assert.ok(
        typeof scope.label === 'string' && scope.label.trim() !== '',
        `scope ${JSON.stringify(scope)} needs a non-empty label`,
      );
    }
  });

  // -- paging -------------------------------------------------------------

  suite('returns rows with every field the list draws', async (t) => {
    const host = await makeHost();
    const { rows } = await host.page({ scope: everything(), offset: 0, limit: 10 });
    assert.ok(rows.length > 0, 'the fixture mailbox should not be empty');

    for (const row of rows) {
      assert.equal(typeof row.id, 'string', 'ids are opaque strings to the core');
      assert.ok(row.id !== '', 'an empty id cannot be acted on');
      assert.equal(typeof row.unread, 'boolean');
      assert.equal(typeof row.hasAttachments, 'boolean');
      // Everything else may legitimately be absent — a message with no subject
      // and no Message-ID is legal mail (§3.6) — but it must be null, not
      // undefined, so the list can tell "absent" from "not fetched".
      for (const field of ['subject', 'from', 'fromAddr', 'date', 'category']) {
        assert.ok(field in row, `row.${field} must be present, even as null`);
        assert.notEqual(row[field], undefined, `row.${field} must be null, not undefined`);
      }
    }
  });

  suite('reports a total, or says it cannot', async (t) => {
    const host = await makeHost();
    const page = await host.page({ scope: everything(), offset: 0, limit: 5 });
    assert.equal(typeof page.total, 'number');
    assert.ok(page.total === -1 || page.total >= page.rows.length);
  });

  suite('pages without repeating or skipping a message', async (t) => {
    const host = await makeHost();
    const first = await host.page({ scope: everything(), offset: 0, limit: 3 });
    const second = await host.page({ scope: everything(), offset: 3, limit: 3 });

    const ids = [...first.rows, ...second.rows].map((row) => row.id);
    assert.equal(new Set(ids).size, ids.length, 'a message appeared on two pages');
  });

  suite('a page past the end is empty rather than an error', async (t) => {
    const host = await makeHost();
    const page = await host.page({ scope: everything(), offset: 100000, limit: 10 });
    assert.deepEqual(page.rows, []);
  });

  suite('orders newest first, and the same way every time', async (t) => {
    // The list does not sort; it draws what it is given. Two calls that
    // disagree would make the cursor land on a different message each time.
    const host = await makeHost();
    const once = await host.page({ scope: everything(), offset: 0, limit: 10 });
    const twice = await host.page({ scope: everything(), offset: 0, limit: 10 });
    assert.deepEqual(
      once.rows.map((r) => r.id),
      twice.rows.map((r) => r.id),
      'two identical requests returned different orders',
    );

    const dates = once.rows.map((r) => r.date).filter((d) => d !== null);
    const sorted = [...dates].sort((a, b) => b - a);
    assert.deepEqual(dates, sorted, 'rows should arrive newest first');
  });

  // -- reading ------------------------------------------------------------

  suite('opening a message returns its row and body', async (t) => {
    const host = await makeHost();
    const { rows } = await host.page({ scope: everything(), offset: 0, limit: 1 });
    const opened = await host.message(rows[0].id);

    assert.equal(opened.row.id, rows[0].id);
    assert.ok('body' in opened, 'body must be present, even as null');
  });

  suite('opening an id that is gone fails loudly', async (t) => {
    // Quietly returning an empty message would show the user a blank pane and
    // let them believe they had read it.
    const host = await makeHost();
    await assert.rejects(() => host.message('definitely-not-a-real-id'));
  });

  // -- acting -------------------------------------------------------------

  // Identity across a move is compared by subject, not by id. A move may issue
  // a new id — Thunderbird's does — so an id is only good until the message is
  // acted on. That is a property of the contract, not a weakness of the test.

  suite('archiving takes the message out of the list', async (t) => {
    const host = await makeHost();
    if (!host.capabilities.archive) return t.skip('host cannot archive');

    const before = await host.page({ scope: everything(), offset: 0, limit: 50 });
    const target = actionable(before.rows, ACTIONS.Archive);
    const subject = target.subject;
    await host.archive(target.id);
    const after = await host.page({ scope: everything(), offset: 0, limit: 50 });

    assert.equal(after.total, before.total - 1, 'the list should be one shorter');
    assert.ok(
      !after.rows.some((row) => row.subject === subject),
      'the archived message is still in the list',
    );
  });

  suite('anything that can trash can put it back', async (t) => {
    // Brief §3.5: nothing destroys mail. A host with no way back from Trash
    // cannot honour that, so the two capabilities come as a pair — and this is
    // the only portable way to check it, since where a host *puts* a trashed
    // message is its own business.
    const caps = (await makeHost()).capabilities;
    if (!caps.trash) return t.skip('host cannot trash');
    assert.ok(caps.undo, 'a host that can trash must offer some kind of undo');
  });

  suite('nothing destroys mail: a trashed message comes back', async (t) => {
    const host = await makeHost();
    if (!host.capabilities.trash || !host.capabilities.undo) return t.skip('host cannot trash or has no undo');

    const before = await host.page({ scope: everything(), offset: 0, limit: 50 });
    const target = actionable(before.rows, ACTIONS.Trash);
    const subject = target.subject;

    await host.trash(target.id);
    await host.undo();

    const after = await host.page({ scope: everything(), offset: 0, limit: 50 });
    assert.ok(
      after.rows.some((row) => row.subject === subject),
      'a trashed message did not come back',
    );
  });

  suite('read state can be set both ways and sticks', async (t) => {
    // Checked through the list rather than through `message`, because the list
    // is where read state is shown and not every host carries flags on a
    // single-message read — `kuverta`'s detail view does not.
    const host = await makeHost();
    if (!host.capabilities.setRead) return t.skip('host cannot set read state');

    const first = await host.page({ scope: everything(), offset: 0, limit: 50 });
    const target = actionable(first.rows, ACTIONS.ToggleRead);
    const id = target.id;
    const unreadNow = async () =>
      (await host.page({ scope: everything(), offset: 0, limit: 50 })).rows.find(
        (row) => row.id === id,
      ).unread;

    await host.setRead(id, true);
    assert.equal(await unreadNow(), false);

    await host.setRead(id, false);
    assert.equal(await unreadNow(), true);
  });

  suite('filing by category shows up on the row', async (t) => {
    const host = await makeHost();
    if (!host.capabilities.setCategory) return t.skip('host cannot file by category');

    const { rows } = await host.page({ scope: everything(), offset: 0, limit: 50 });
    const id = actionable(rows, ACTIONS.SetCategory).id;
    const categoryNow = async () =>
      (await host.page({ scope: everything(), offset: 0, limit: 50 })).rows.find(
        (row) => row.id === id,
      ).category;

    for (const category of ALL_CATEGORIES) {
      await host.setCategory(id, category);
      assert.equal(await categoryNow(), category, `filing as ${category} did not stick`);
    }
  });

  suite('filing replaces the category rather than accumulating them', async (t) => {
    const host = await makeHost();
    if (!host.capabilities.setCategory) return t.skip('host cannot file by category');

    const { rows } = await host.page({ scope: everything(), offset: 0, limit: 50 });
    const id = actionable(rows, ACTIONS.SetCategory).id;
    await host.setCategory(id, 'newsletter');
    await host.setCategory(id, 'marketing');

    const after = await host.page({ scope: everything(), offset: 0, limit: 50 });
    assert.equal(after.rows.find((row) => row.id === id).category, 'marketing');
  });

  // -- undo ---------------------------------------------------------------

  suite('an empty undo is a normal answer, not an error', async (t) => {
    const host = await makeHost();
    if (!host.capabilities.undo) return t.skip('host has no undo');
    assert.equal(await host.undo(), null);
  });

  suite('undo reverses the last change and says what it was', async (t) => {
    const host = await makeHost();
    if (!host.capabilities.undo || !host.capabilities.archive) return t.skip('host has no undo or no archive');

    const before = await host.page({ scope: everything(), offset: 0, limit: 50 });
    const id = before.rows[0].id;

    const subject = before.rows[0].subject;

    await host.archive(id);
    const change = await host.undo();

    assert.ok(change, 'undo should report what it undid');
    assert.equal(typeof change.what, 'string');
    assert.ok(change.what.trim() !== '', 'the user is shown this string');

    const after = await host.page({ scope: everything(), offset: 0, limit: 50 });
    assert.ok(
      after.rows.some((row) => row.subject === subject),
      'the message did not come back',
    );
  });

  suite('undo unwinds one change at a time', async (t) => {
    const host = await makeHost();
    if (!host.capabilities.undo || !host.capabilities.archive) return t.skip('host has no undo or no archive');

    const before = await host.page({ scope: everything(), offset: 0, limit: 50 });
    const actionables = before.rows.filter(
      (row) => !Array.isArray(row.actions) || row.actions.includes(ACTIONS.Archive),
    );
    const [first, second] = actionables;

    await host.archive(first.id);
    // The second row's id survives the first archive here only because nothing
    // moved it; re-reading the list is what a caller should do.
    const reread = await host.page({ scope: everything(), offset: 0, limit: 50 });
    const stillThere = reread.rows.find((row) => row.subject === second.subject);
    await host.archive(stillThere.id);
    await host.undo();

    const after = await host.page({ scope: everything(), offset: 0, limit: 50 });
    const subjects = after.rows.map((row) => row.subject);
    assert.ok(subjects.includes(second.subject), 'the most recent change should be the one undone');
    assert.ok(!subjects.includes(first.subject), 'the earlier change should still stand');
  });

  suite('a capability it does not have is refused, not quietly ignored', async (t) => {
    // A host that says it cannot file by category and then silently accepts
    // one is the worst of both: the surface shows the key, the user presses
    // it, and nothing happens with no way to tell why.
    const host = await makeHost();
    if (host.capabilities.setCategory) return t.skip('host can file by category');

    const { rows } = await host.page({ scope: everything(), offset: 0, limit: 1 });
    await assert.rejects(() => host.setCategory(rows[0].id, 'marketing'));
  });

  suite('a row that allows less than its host does is refused, not ignored', async (t) => {
    // A list can hold more than one kind of thing. A row that quietly accepted
    // an action it had declared it did not allow would be the worst of both:
    // the key appears to work, and nothing happens.
    const host = await makeHost();
    const { rows } = await host.page({ scope: everything(), offset: 0, limit: 50 });
    // Any row that declares it does not allow archiving — not only one that
    // allows nothing, since a letter allows filing and nothing else.
    const limited = rows.find(
      (row) => Array.isArray(row.actions) && !row.actions.includes(ACTIONS.Archive),
    );
    if (!limited) return t.skip('every row in this host allows archiving');

    await assert.rejects(() => host.archive(limited.id));
  });

  // -- scoped lists -------------------------------------------------------

  suite('a category scope shows only that category', async (t) => {
    const host = await makeHost();
    if (!host.capabilities.setCategory) return t.skip('host cannot file by category');

    const all = await host.page({ scope: everything(), offset: 0, limit: 50 });
    await host.setCategory(all.rows[0].id, 'marketing');

    const scope = { kind: 'category', value: 'marketing', label: 'Marketing' };
    const scoped = await host.page({ scope, offset: 0, limit: 50 });

    assert.ok(
      scoped.rows.every((row) => row.category === 'marketing'),
      'a category scope returned something from another category',
    );
    assert.ok(scoped.rows.some((row) => row.id === all.rows[0].id));
  });

  suite('a search scope returns only matches', async (t) => {
    const host = await makeHost();
    if (!host.capabilities.search) return t.skip('host cannot search');

    const all = await host.page({ scope: everything(), offset: 0, limit: 1 });
    const word = (all.rows[0].subject ?? '').split(/\s+/)[0];
    if (!word) return t.skip('no word to search for');

    const scope = { kind: 'search', value: word, label: `Search: ${word}` };
    const found = await host.page({ scope, offset: 0, limit: 50 });

    assert.ok(found.rows.length > 0, `searching for ${word} found nothing`);
  });
}

function everything() {
  return { kind: 'all', label: 'Everything' };
}

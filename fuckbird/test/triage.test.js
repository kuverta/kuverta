/**
 * The triage model.
 *
 * Mostly about the cursor. "Acting keeps the cursor where it was" is one line
 * of the brief and most of what makes triage bearable, and it is the kind of
 * rule that survives a rewrite only if something checks it.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { Triage } from '../core/triage.js';
import { ACTIONS } from '../core/host.js';
import { MemoryMailbox } from './support/mailbox.js';

/** A triage over ten messages, subjects `Message 00` … `Message 09`. */
async function started(host = new MemoryMailbox()) {
  const triage = new Triage(host);
  await triage.start();
  return triage;
}

const subjectAt = (triage, index) => triage.rowAt(index)?.subject ?? null;

// -- the cursor ----------------------------------------------------------

test('starts on the first message', async () => {
  const triage = await started();
  assert.equal(triage.selected, 0);
  assert.equal(triage.total, 10);
});

test('j and k move by one and clamp at the ends', async () => {
  const triage = await started();

  triage.move(1);
  assert.equal(triage.selected, 1);
  triage.move(-1);
  assert.equal(triage.selected, 0);

  // Clamping rather than wrapping: going up from the first message should not
  // land on the last one, which is never what pressing k once more meant.
  triage.move(-1);
  assert.equal(triage.selected, 0);

  triage.move(100);
  assert.equal(triage.selected, 9);
  triage.move(1);
  assert.equal(triage.selected, 9);
});

test('archiving keeps the cursor, so the next message slides under it', async () => {
  // The rule. Without it the list jumps back to the top after every action and
  // you spend the session finding your place again.
  const triage = await started();
  triage.move(3);

  const acted = subjectAt(triage, 3);
  const next = subjectAt(triage, 4);

  await triage.act(ACTIONS.Archive);

  assert.equal(triage.selected, 3, 'the cursor should not have moved');
  assert.equal(subjectAt(triage, 3), next, 'the next message should be under it');
  assert.notEqual(subjectAt(triage, 3), acted);
  assert.equal(triage.total, 9);
});

test('archiving the last message moves the cursor up rather than off the end', async () => {
  const triage = await started();
  triage.move(9);

  await triage.act(ACTIONS.Archive);

  assert.equal(triage.selected, 8);
  assert.ok(triage.selectedRow, 'the cursor should be on a real row');
});

test('emptying the list leaves the cursor nowhere, not on row zero', async () => {
  // Selecting index 0 of an empty list is how a list ends up acting on a row
  // that is not there.
  const triage = await started();
  for (let i = 0; i < 10; i += 1) await triage.act(ACTIONS.Archive);

  assert.equal(triage.total, 0);
  assert.equal(triage.selected, -1);
  assert.equal(triage.selectedRow, null);
});

test('acting on an empty list does nothing rather than throwing', async () => {
  const triage = await started(new MemoryMailbox([]));
  await triage.act(ACTIONS.Archive);
  await triage.act(ACTIONS.Trash);
  assert.equal(triage.selected, -1);
});

test('changing scope resets the cursor, because the old position means nothing', async () => {
  const triage = await started();
  triage.move(5);

  await triage.setScope({ kind: 'search', value: 'Message 0', label: 'Search' });

  assert.equal(triage.selected, 0);
});

// -- saying what is being shown ------------------------------------------

test('a scoped list knows it is filtered and can say what by', async () => {
  // §3.1: a filtered list says what it is showing, and offers a way out. A
  // list that quietly narrows looks exactly like mail going missing.
  const triage = await started();
  assert.equal(triage.isFiltered, false);
  assert.equal(triage.scopeLabel, 'Everything');

  await triage.setScope({ kind: 'category', value: 'marketing', label: 'Marketing' });
  assert.equal(triage.isFiltered, true);
  assert.equal(triage.scopeLabel, 'Marketing');

  await triage.clearScope();
  assert.equal(triage.isFiltered, false);
});

test('a scope with no label still says something rather than nothing', async () => {
  const triage = await started();
  await triage.setScope({ kind: 'category', value: 'newsletter' });
  assert.ok(triage.scopeLabel.trim() !== '');
  assert.match(triage.scopeLabel, /newsletter/);
});

// -- undo ----------------------------------------------------------------

test('undo puts the message back and says which kind of undo it was', async () => {
  const triage = await started();
  const first = subjectAt(triage, 0);

  await triage.act(ACTIONS.Archive);
  assert.match(triage.notice.text, /archived/);
  // The memory host is compensating, so the wording is "put it back" rather
  // than "cancel" — the change already happened.
  assert.match(triage.notice.text, /put it back/);

  await triage.undo();
  assert.equal(triage.total, 10);
  assert.equal(subjectAt(triage, 0), first);
  assert.match(triage.notice.text, /put back/);
});

test('a queued host is offered in the words of a cancellation', async () => {
  // The two undos are different promises: one cancels a change that has not
  // happened, the other reverses one that has. Saying "cancel" for a change
  // another client may already have seen would be a lie.
  const queued = new MemoryMailbox();
  Object.defineProperty(queued, 'capabilities', { get: () => capabilitiesWith('queued') });

  const triage = new Triage(queued);
  await triage.start();
  await triage.act(ACTIONS.Archive);

  assert.match(triage.notice.text, /cancel/);
});

// -- capabilities --------------------------------------------------------

test('a host that cannot archive says so instead of failing silently', async () => {
  const limited = new MemoryMailbox();
  Object.defineProperty(limited, 'capabilities', {
    get: () => capabilitiesWith(false, { archive: false }),
  });

  const triage = new Triage(limited);
  await triage.start();
  await triage.act(ACTIONS.Archive);

  assert.equal(triage.total, 10, 'nothing should have happened');
  assert.match(triage.notice.text, /cannot archive/);
  assert.equal(triage.notice.tone, 'error');
});

// -- reading -------------------------------------------------------------

test('opening a message marks it read and updates its row', async () => {
  const triage = await started();
  assert.equal(triage.selectedRow.unread, true);

  await triage.openSelected();

  assert.ok(triage.open, 'the message should be open');
  assert.equal(triage.selectedRow.unread, false, 'the row should agree');
});

test('moving the cursor closes what was open', async () => {
  // Otherwise the reading pane shows one message while the cursor is on
  // another, which is the fastest way to act on the wrong thing.
  const triage = await started();
  await triage.openSelected();
  assert.ok(triage.open);

  triage.move(1);
  assert.equal(triage.open, null);
});

// -- notifications -------------------------------------------------------

test('subscribers hear about every change', async () => {
  const triage = await started();
  let heard = 0;
  const stop = triage.subscribe(() => {
    heard += 1;
  });

  triage.move(1);
  assert.ok(heard > 0);

  const was = heard;
  stop();
  triage.move(1);
  assert.equal(heard, was, 'unsubscribing should stop the calls');
});

test('a host that throws on load says so rather than showing an empty list', async () => {
  // An empty list and a failed load look identical to a reader, and one of
  // them means their mail is missing.
  const broken = new MemoryMailbox();
  broken.page = async () => {
    throw new Error('the store is locked');
  };

  const triage = new Triage(broken);
  await triage.start();

  assert.match(triage.notice.text, /could not load messages/);
  assert.match(triage.notice.text, /the store is locked/);
  assert.equal(triage.notice.tone, 'error');
});

function capabilitiesWith(undo, overrides = {}) {
  return {
    archive: true,
    trash: true,
    setRead: true,
    setCategory: true,
    search: true,
    sync: true,
    undo,
    compose: true,
    ...overrides,
  };
}

// -- rows that allow less than their host --------------------------------

test('a row that cannot be archived says so without a round trip', async () => {
  // A list can hold more than one kind of thing: post read from Paperless sits
  // beside mail and cannot be archived, because Paperless owns it. The answer
  // has to be immediate and specific, not a request that comes back refused.
  const host = new MemoryMailbox();
  let asked = false;
  host.archive = async () => {
    asked = true;
  };

  const triage = new Triage(host);
  await triage.start();
  // The row under the cursor declares that nothing may be done to it.
  const row = triage.selectedRow;
  Object.assign(row, { actions: [] });

  await triage.act(ACTIONS.Archive);

  assert.equal(asked, false, 'the host should not have been asked');
  assert.match(triage.notice.text, /cannot be archived from here/);
  assert.equal(triage.notice.tone, 'error');
});

test('a row that says nothing allows whatever its host can do', async () => {
  // Every adapter written before rows could refuse stays correct.
  const triage = await started();
  assert.equal(triage.selectedRow.actions, undefined);

  await triage.act(ACTIONS.Archive);
  assert.equal(triage.total, 9);
});

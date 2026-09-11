/**
 * The corrections log.
 *
 * §3.4 of the brief: the thing being kept is the correction *event*, not the
 * current state. These tests are mostly about that distinction — a store that
 * quietly collapsed reversals into a final answer would pass a naive reading
 * of the feature and throw away the interesting half of the corpus.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { Corrections, learnedFrom } from '../extension/src/corrections.js';
import { Classifier } from '../extension/src/classify.js';
import { Category } from '../extension/src/category.js';
import { EMPTY_FACTS } from '../extension/src/facts.js';

/** A `storage.local`-shaped area backed by an object. */
function fakeStorage(initial = {}) {
  const data = structuredClone(initial);
  return {
    data,
    get: async (key) => (key in data ? { [key]: structuredClone(data[key]) } : {}),
    set: async (patch) => Object.assign(data, structuredClone(patch)),
  };
}

test('an empty store yields an empty log', async () => {
  const corrections = new Corrections(fakeStorage());
  assert.deepEqual(await corrections.all(), []);
  assert.ok((await corrections.learned()).isEmpty);
});

test('a correction is stamped and kept', async () => {
  const corrections = new Corrections(fakeStorage());
  await corrections.record({
    messageId: 'abc@example.com',
    fromAddr: 'deals@shop.example.com',
    listId: null,
    subject: '20% off',
    was: Category.Marketing,
    now: Category.Transactional,
    wasRule: 'subject.marketing',
  });

  const [event] = await corrections.all();
  assert.equal(event.now, Category.Transactional);
  assert.equal(event.was, Category.Marketing);
  assert.equal(event.wasRule, 'subject.marketing');
  assert.ok(!Number.isNaN(Date.parse(event.at)), 'should carry a timestamp');
});

test('reversing a correction keeps both events', async () => {
  // The state is recoverable from the events; the events are not recoverable
  // from the state. A sender you filed twice, differently, is the most
  // interesting row in the log and must survive.
  const corrections = new Corrections(fakeStorage());
  await corrections.record({ fromAddr: 'a@example.com', now: Category.Newsletter });
  await corrections.record({ fromAddr: 'a@example.com', now: Category.Marketing });

  const events = await corrections.all();
  assert.equal(events.length, 2);
  assert.deepEqual(
    events.map((e) => e.now),
    [Category.Newsletter, Category.Marketing],
  );
});

test('the last correction for a sender is the one that binds', async () => {
  const learned = learnedFrom([
    { fromAddr: 'a@example.com', now: Category.Newsletter },
    { fromAddr: 'a@example.com', now: Category.Marketing },
  ]);
  const verdict = new Classifier(learned).classify({
    ...EMPTY_FACTS,
    fromAddr: 'a@example.com',
  });
  assert.equal(verdict.category, Category.Marketing);
});

test('one correction teaches both the sender and the list', async () => {
  // They are separate overrides: later mail may match either on its own.
  const learned = learnedFrom([
    {
      fromAddr: 'hello@news.example',
      listId: 'news.example',
      now: Category.Marketing,
    },
  ]);

  const bySender = new Classifier(learned).classify({
    ...EMPTY_FACTS,
    fromAddr: 'hello@news.example',
  });
  const byList = new Classifier(learned).classify({
    ...EMPTY_FACTS,
    fromAddr: 'someone-else@news.example',
    listId: 'news.example',
  });

  assert.equal(bySender.category, Category.Marketing);
  assert.equal(byList.category, Category.Marketing);
});

test('a corrupt or unknown category in the log is skipped, not fatal', async () => {
  // Storage is not schema-checked, and a log that throws on read would take
  // the whole classifier down with it.
  const learned = learnedFrom([
    { fromAddr: 'a@example.com', now: 'invoices' },
    null,
    { now: Category.Personal },
    { fromAddr: 'b@example.com', now: Category.Personal },
  ]);
  const { senders } = learned.entries();
  assert.deepEqual(senders, [['b@example.com', Category.Personal]]);
});

test('a log left in storage by an older version does not throw', async () => {
  const corrections = new Corrections(fakeStorage({ corrections: 'not an array' }));
  assert.deepEqual(await corrections.all(), []);
});

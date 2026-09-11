/**
 * Classifier behaviour on messages shaped like the seeded fixtures and the
 * conflict cases the scoring model exists to handle.
 *
 * Ported one-for-one from `fuckmail`'s `core-rules/tests/classify.rs`. They are
 * kept identical on purpose: if the JS port drifts from the Rust classifier,
 * the corrections collected under one are not comparable with the other, and
 * the rules-vs-model measurement of §3.3 loses its baseline.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { Classifier, Learned } from '../extension/src/classify.js';
import { Category } from '../extension/src/category.js';
import { EMPTY_FACTS } from '../extension/src/facts.js';

/** The Rust tests use `..Default::default()`; this is that. */
const facts = (overrides) => ({ ...EMPTY_FACTS, ...overrides });

const classify = (f) => {
  const c = Classifier.withoutHistory().classify(f);
  return [c.category, c.confidence];
};

test('a personal reply is personal', () => {
  const f = facts({
    fromAddr: 'anna.weber@example.de',
    fromName: 'Anna Weber',
    subject: 'Re: Termin nächste Woche',
    inReplyTo: '<prev@example.de>',
    recipientCount: 1,
  });
  assert.equal(classify(f)[0], Category.Personal);
});

test('a mailing list is a newsletter', () => {
  const f = facts({
    fromAddr: 'hello@news.rustweekly.example',
    fromName: 'Rust Weekly',
    subject: 'Rust Weekly #612 — async traits everywhere',
    listId: 'news.rustweekly.example',
    precedence: 'bulk',
    listUnsubscribe: '<https://news.rustweekly.example/u/8837221>',
    recipientCount: 1,
  });
  // A newsletter from a two-word "name" to a single recipient must not be
  // mistaken for personal mail — the bulk headers suppress that.
  assert.equal(classify(f)[0], Category.Newsletter);
});

test('a promotional blast is marketing', () => {
  const f = facts({
    fromAddr: 'deals@shop.example.com',
    fromName: 'Shop Deals',
    subject: '20% off everything this week only',
    listId: 'deals.shop.example.com',
    listUnsubscribe: '<https://shop.example.com/u>',
  });
  // It arrives as a list, but the marketing subject outweighs that.
  assert.equal(classify(f)[0], Category.Marketing);
});

test('a German invoice is transactional', () => {
  const f = facts({
    fromAddr: 'rechnung@hosting.example.de',
    fromName: 'Hosting AG',
    subject: 'Ihre Rechnung 4471182 für September 2026',
    recipientCount: 1,
  });
  assert.equal(classify(f)[0], Category.Transactional);
});

test('an invoice with a pdf is transactional', () => {
  const f = facts({
    fromAddr: 'buchhaltung@lieferant.example.de',
    fromName: 'Buchhaltung',
    subject: 'Rechnung RE-2026-0912',
    hasAttachments: true,
    recipientCount: 1,
  });
  const result = Classifier.withoutHistory().classify(f);
  assert.equal(result.category, Category.Transactional);
  assert.ok(
    result.reasons.some((r) => r.rule === 'attachment.with_transactional_subject'),
    'the attachment should be part of the reasoning',
  );
});

test('a cron notice from a noreply is a notification', () => {
  const f = facts({
    fromAddr: 'noreply@legacy-system.example.org',
    fromName: 'Legacy Cron',
    subject: 'nightly backup completed',
    recipientCount: 1,
  });
  assert.equal(classify(f)[0], Category.Notification);
});

test('nothing to go on yields unknown, not a confident guess', () => {
  const f = facts({
    fromAddr: 'x@example.com',
    subject: 'hi',
    recipientCount: 3,
  });
  const result = Classifier.withoutHistory().classify(f);
  assert.equal(result.category, Category.Unknown);
  assert.equal(result.confidence, 0.0);
});

test('confidence is lower when the evidence is split', () => {
  // An invoice sent through a mailing list: transactional subject vs. list
  // headers. It should still decide, but not claim certainty.
  const conflicted = facts({
    fromAddr: 'billing@service.example.com',
    subject: 'Ihre Rechnung für März',
    listId: 'service.example.com',
    listUnsubscribe: '<https://service.example.com/u>',
  });
  const clean = facts({
    fromAddr: 'rechnung@hosting.example.de',
    subject: 'Ihre Rechnung 4471182',
    recipientCount: 1,
  });

  const split = Classifier.withoutHistory().classify(conflicted).confidence;
  const unanimous = Classifier.withoutHistory().classify(clean).confidence;
  assert.ok(
    split < unanimous,
    `split evidence (${split}) should be less confident than unanimous (${unanimous})`,
  );
});

test('a learned sender overrides the heuristics', () => {
  // The user has decided this shop's mail is transactional (order updates),
  // not marketing. Their decision wins outright.
  const learned = Learned.empty().withSender(
    'deals@shop.example.com',
    Category.Transactional,
  );
  const f = facts({
    fromAddr: 'deals@shop.example.com',
    subject: '20% off everything this week only',
    listId: 'deals.shop.example.com',
  });

  const result = new Classifier(learned).classify(f);
  assert.equal(result.category, Category.Transactional);
  assert.equal(result.confidence, 1.0);
  assert.equal(result.reasons[0].rule, 'learned.sender');
});

test('a learned list is matched case-insensitively', () => {
  const learned = Learned.empty().withList('News.RustWeekly.Example', Category.Marketing);
  const f = facts({
    fromAddr: 'hello@news.rustweekly.example',
    listId: 'news.rustweekly.example',
  });
  assert.equal(new Classifier(learned).classify(f).category, Category.Marketing);
});

test('every verdict can explain itself', () => {
  // The explanation is the point of the rules layer; a verdict with no
  // reasons would be as opaque as the model it is meant to keep honest.
  const f = facts({
    fromAddr: 'rechnung@hosting.example.de',
    subject: 'Ihre Rechnung 4471182',
  });
  const result = Classifier.withoutHistory().classify(f);
  assert.ok(result.reasons.length > 0);
  assert.ok(result.reasons.every((r) => r.category === result.category));
});

test('ties are broken the same way on every run', () => {
  // Not in the Rust suite, but the Rust classifier sorts ties by category name
  // for exactly this reason, and a Map-ordering port is the obvious way to
  // lose that. The promotional blast above is a genuine 3.5/3.5 tie.
  const f = facts({
    fromAddr: 'deals@shop.example.com',
    subject: '20% off everything this week only',
    listId: 'deals.shop.example.com',
    listUnsubscribe: '<https://shop.example.com/u>',
  });
  const verdicts = new Set();
  for (let i = 0; i < 50; i += 1) {
    verdicts.add(Classifier.withoutHistory().classify(f).category);
  }
  assert.deepEqual([...verdicts], [Category.Marketing]);
});

/**
 * The header-to-facts mapping.
 *
 * This is the part with no Rust equivalent — `kuverta` got these fields from
 * `mail-parser`, and here they come from Thunderbird's `MessageHeader` and
 * `MessagePart.headers`. So it is the part most likely to be wrong, and the
 * cases below are the ones §3.6 of the brief says real mail actually presents.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { buildFacts, cleanListId, splitAuthor, snippet } from '../core/facts.js';

test('extracts the bracketed list id', () => {
  assert.equal(cleanListId('Rust Weekly <news.rustweekly.example>'), 'news.rustweekly.example');
});

test('a list id with no brackets is kept whole', () => {
  assert.equal(cleanListId('  news.rustweekly.example '), 'news.rustweekly.example');
});

test('splits a display name from an angle address', () => {
  assert.deepEqual(splitAuthor('Anna Weber <anna@example.de>'), {
    name: 'Anna Weber',
    addr: 'anna@example.de',
  });
});

test('unquotes a display name that needed quoting', () => {
  // Thunderbird hands back the raw RFC 5322 form, and a name containing a
  // comma arrives quoted. Left in, the quotes defeat the role-word check.
  assert.deepEqual(splitAuthor('"Weber, Anna" <anna@example.de>'), {
    name: 'Weber, Anna',
    addr: 'anna@example.de',
  });
});

test('a bare address has no display name', () => {
  assert.deepEqual(splitAuthor('cron@example.org'), { name: null, addr: 'cron@example.org' });
});

test('an absent author is not an error', () => {
  assert.deepEqual(splitAuthor(null), { name: null, addr: null });
  assert.deepEqual(splitAuthor('   '), { name: null, addr: null });
});

test('headers arrive as arrays and only the first value counts', () => {
  const facts = buildFacts({
    subject: 'Ihre Rechnung',
    author: 'Hosting AG <rechnung@hosting.example.de>',
    headers: {
      'list-id': ['Hosting News <news.hosting.example.de>'],
      precedence: ['bulk'],
      'auto-submitted': ['auto-generated'],
    },
  });
  assert.equal(facts.listId, 'news.hosting.example.de');
  assert.equal(facts.precedence, 'bulk');
  assert.equal(facts.autoSubmitted, 'auto-generated');
});

test('a header given as a plain string still reads', () => {
  // Defensive: the API documents arrays, and a single-valued header arriving
  // bare would otherwise silently disable a whole signal.
  const facts = buildFacts({ headers: { precedence: 'bulk' } });
  assert.equal(facts.precedence, 'bulk');
});

test('missing headers are null rather than undefined', () => {
  // The classifier tests `!= null`; `undefined` would pass that check by
  // accident and `''` would not, so both are normalised here.
  const facts = buildFacts({ headers: { 'list-id': [''] } });
  assert.equal(facts.listId, null);
  assert.equal(facts.listUnsubscribe, null);
  assert.equal(facts.precedence, null);
  assert.equal(facts.inReplyTo, null);
});

test('recipients are counted across to and cc', () => {
  const facts = buildFacts({
    recipients: ['a@example.com'],
    ccList: ['b@example.com', 'c@example.com'],
  });
  assert.equal(facts.recipientCount, 3);
});

test('a message with nothing in it produces usable facts', () => {
  // §3.6: a message with no Message-ID is legal and does happen, and the same
  // goes for almost every other header. Nothing here may throw.
  const facts = buildFacts();
  assert.equal(facts.fromAddr, null);
  assert.equal(facts.recipientCount, 0);
  assert.equal(facts.hasAttachments, false);
});

test('a snippet collapses whitespace and truncates on a character boundary', () => {
  assert.equal(snippet('  Dienstag\r\n  passt.  '), 'Dienstag passt.');

  // Multi-byte characters must not be cut in half: 300 umlauts truncate to
  // 220 umlauts plus an ellipsis, not to 220 bytes of broken UTF-8.
  const long = 'ä'.repeat(300);
  const cut = snippet(long);
  assert.equal([...cut].length, 221);
  assert.ok(cut.endsWith('…'));
});

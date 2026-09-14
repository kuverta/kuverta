/**
 * The triage surface, in a DOM.
 *
 * Everything else in this directory tests the model, the adapters or the
 * classifier; this is the only file that exercises the view — the part a person
 * actually touches. It had no tests at all, and produced three real bugs before
 * it had any: keys that went nowhere a person could see, a teardown that would
 * have made every keystroke act twice after switching accounts, and an
 * explanation panel that never opened on first launch although it was written
 * to.
 *
 * jsdom, not a browser, which means no layout and no CSS cascade. So these
 * tests can say "the element is hidden" and cannot say "the element is not on
 * screen" — the class of bug where `display: flex` beat the hidden attribute is
 * outside what they can see. That wants a real engine.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { JSDOM } from 'jsdom';

import { Triage } from '../core/triage.js';
import { mountTriage } from '../core/view/triage.js';
import { EMPTY_FACTS } from '../core/facts.js';
import { MemoryMailbox } from './support/mailbox.js';

// -- a DOM per test ------------------------------------------------------------

/**
 * A fresh document, installed as the globals the view reads.
 *
 * A URL because localStorage needs an origin, and the explanation panel keeps
 * "you have seen this" there.
 */
function dom() {
  const jsdom = new JSDOM('<!doctype html><div id="root"></div>', {
    url: 'http://localhost/',
    pretendToBeVisual: true,
  });
  globalThis.window = jsdom.window;
  globalThis.document = jsdom.window.document;
  globalThis.localStorage = jsdom.window.localStorage;
  return jsdom;
}

/** Lets the model's promises and the view's redraws run to completion. */
async function settle() {
  for (let i = 0; i < 10; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
}

async function mounted({ host = new MemoryMailbox(), seen = true, title = 'you@example.com' } = {}) {
  const jsdom = dom();
  if (seen) localStorage.setItem('fuckbird.explained', '1');

  const triage = new Triage(host);
  const root = document.getElementById('root');
  const unmount = mountTriage({ root, triage, title });
  await triage.start();
  await settle();

  const $ = (selector) => root.querySelector(selector);
  const press = async (key, target = document) => {
    target.dispatchEvent(new jsdom.window.KeyboardEvent('keydown', { key, bubbles: true }));
    await settle();
  };
  const selectedSubject = () => $('.fb-row.fb-selected .fb-subject')?.textContent ?? null;

  return { jsdom, triage, host, root, unmount, $, press, selectedSubject };
}

// -- saying what it is -----------------------------------------------------------

test('the surface says which mailbox it is, what it shows, and which keys do what', async () => {
  const { $ } = await mounted();

  assert.equal($('#fb-title').textContent, 'you@example.com');
  // Always, not only when filtered: an unfiltered list that says nothing
  // leaves you working out where you are from the rows.
  assert.equal($('#fb-scope-label').textContent, 'Everything · 10 messages');
  assert.equal($('#fb-scope-out').hidden, true, 'no way out of a list that is not filtered');

  const legend = $('#fb-legend').textContent;
  for (const word of ['move', 'read', 'archive', 'trash', 'file', 'undo', 'more']) {
    assert.ok(legend.includes(word), `legend is missing "${word}": ${legend}`);
  }
});

test('the explanation opens by itself the first time', async () => {
  // Written to, and never did: the line that opened it was dropped by a patch
  // that matched nothing. The person it was for never saw it.
  const { $ } = await mounted({ seen: false });
  assert.equal($('#fb-help').hidden, false, 'the panel should be open on first launch');
  assert.match($('#fb-help').textContent, /What this is for/);
});

test('once dismissed, the explanation stays out of the way', async () => {
  const first = await mounted({ seen: false });
  first.$('.fb-help-done').click();
  assert.equal(first.$('#fb-help').hidden, true);
  assert.equal(localStorage.getItem('fuckbird.explained'), '1');
  first.unmount();

  // Same origin, same storage: a second launch remembers.
  const storage = localStorage;
  const again = dom();
  globalThis.localStorage = storage;
  const triage = new Triage(new MemoryMailbox());
  const root = again.window.document.getElementById('root');
  mountTriage({ root, triage });
  assert.equal(root.querySelector('#fb-help').hidden, true);
});

// -- keys ------------------------------------------------------------------------

test('j and k move the cursor and the list shows where it is', async () => {
  const { press, selectedSubject } = await mounted();
  assert.match(selectedSubject(), /^Message 00/);

  await press('j');
  assert.match(selectedSubject(), /^Message 01/);

  await press('k');
  assert.match(selectedSubject(), /^Message 00/);
});

test('archiving keeps the cursor where it was, and the next message slides under it', async () => {
  const { press, selectedSubject, $, triage } = await mounted();
  await press('j');
  assert.match(selectedSubject(), /^Message 01/);

  await press('e');

  assert.equal(triage.total, 9);
  assert.match(selectedSubject(), /^Message 02/, 'the next message should be under the cursor');
  assert.equal($('#fb-scope-label').textContent, 'Everything · 9 messages');
  assert.match($('#fb-toast').textContent, /archived/);
});

test('typing into the search box never triggers a shortcut', async () => {
  // Without this, typing a word with an `e` in it archives something.
  const { press, $, triage } = await mounted();
  const search = $('#fb-search');

  for (const key of ['e', '#', 'j', 'z']) await press(key, search);

  assert.equal(triage.total, 10, 'nothing should have been archived or trashed');
});

test('digits file the message under the cursor by category', async () => {
  const { press, $, triage } = await mounted();
  await press('3');

  assert.equal(triage.selectedRow.category, 'marketing');
  assert.match($('#fb-toast').textContent, /filed as marketing/);
});

test('a row that refuses an action says so on screen', async () => {
  // Post allows filing and nothing else; the refusal has to be visible, not a
  // key that silently does nothing.
  const host = new MemoryMailbox();
  const page = host.page.bind(host);
  host.page = async (request) => {
    const result = await page(request);
    return { ...result, rows: result.rows.map((row) => ({ ...row, actions: ['setCategory'] })) };
  };
  const { press, $, triage } = await mounted({ host });

  await press('e');

  assert.equal(triage.total, 10);
  assert.match($('#fb-toast').textContent, /cannot be archived from here/);
  assert.ok($('#fb-toast').classList.contains('fb-error'));
});

test('unmounting lets go of the keyboard', async () => {
  // Switching accounts mounts the surface again. A mount that kept listening
  // would act on every key twice — once on a list nobody can see, which with
  // `e` bound is a message archived that nobody looked at.
  const { unmount, press, triage } = await mounted();
  unmount();

  await press('e');
  await press('#');

  assert.equal(triage.total, 10);
});

// -- filters -----------------------------------------------------------------------

test('an empty filtered list says why it is empty and offers the way out', async () => {
  const { triage, $ } = await mounted();
  await triage.setScope({ kind: 'search', value: 'nothing matches this', label: 'Search: nothing' });
  await settle();

  const empty = $('#fb-empty-list');
  assert.equal(empty.hidden, false);
  assert.match(empty.textContent, /Nothing in Search: nothing/);
  assert.equal($('#fb-scope-out').hidden, false);

  $('#fb-scope-out').click();
  await settle();
  assert.equal(triage.isFiltered, false);
  assert.equal($('#fb-empty-list').hidden, true);
});

test('Escape closes the explanation before it touches the filter', async () => {
  const { triage, $, press } = await mounted();
  await triage.setScope({ kind: 'category', value: 'personal', label: 'Personal' });
  await settle();

  await press('?');
  assert.equal($('#fb-help').hidden, false);

  await press('Escape');
  assert.equal($('#fb-help').hidden, true);
  assert.equal(triage.isFiltered, true, 'the first Escape belongs to the panel');
});

// -- reading -----------------------------------------------------------------------

test('what the sender wrote is shown as text and never parsed as markup', async () => {
  const host = new MemoryMailbox([]);
  host.add({
    subject: '<img src=x onerror="window.pwned = true">',
    body: '<b>not bold</b><script>window.pwned = true</script>',
    date: Date.UTC(2026, 8, 14),
  });
  const { press, $, root } = await mounted({ host });

  await press('Enter');

  assert.equal($('.fb-read-body').textContent, '<b>not bold</b><script>window.pwned = true</script>');
  assert.equal(root.querySelector('.fb-read-body b'), null);
  assert.equal(root.querySelector('img'), null, 'the subject must not become an element');
  assert.equal(globalThis.window.pwned, undefined);
});

/** A mailbox whose opened messages carry facts, as fuckmail's and Thunderbird's do. */
class ExplainedMailbox extends MemoryMailbox {
  async message(id) {
    const opened = await super.message(id);
    return {
      ...opened,
      facts: {
        ...EMPTY_FACTS,
        fromAddr: 'rechnung@hosting.example.de',
        fromName: 'Hosting AG',
        subject: opened.row.subject,
        recipientCount: 1,
      },
    };
  }
}

test('an opened message says why the rules would file it where they would', async () => {
  const { press, $ } = await mounted({ host: new ExplainedMailbox() });
  await press('Enter');

  const reasons = [...$('#fb-why').querySelectorAll('li')].map((item) => item.textContent);
  assert.ok(reasons.some((reason) => reason.includes('rechnung')), reasons.join(' | '));
});

test('a filing the rules disagree with is shown as the user’s, beside what the rules said', async () => {
  const { press, $ } = await mounted({ host: new ExplainedMailbox() });
  await press('1');
  await press('Enter');

  assert.match(
    $('#fb-why').textContent,
    /You filed this as Personal\. On the headers alone the rules would have said Transactional\./,
  );
});

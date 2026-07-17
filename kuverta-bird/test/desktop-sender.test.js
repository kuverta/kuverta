/**
 * The sender card, in a DOM.
 *
 * The card is the one place in the window where several small facts about a
 * person are put next to each other, and every one of them is a sentence that
 * could be drawn wrong: a count with no number, a direction arrow the wrong
 * way round, an action button in the hover copy that nothing can click. That
 * is what this checks.
 *
 * `sender.js` is a classic script: it defines functions for the window rather
 * than exporting them, and it reads globals the other files define. So it is
 * evaluated inside a function that hands those globals in and gives back what
 * the test needs — the same trick `desktop-i18n.test.js` uses — rather than
 * booting the whole window.
 *
 * jsdom, so there is no layout: where the hover card lands on screen is
 * outside what this can see.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { JSDOM } from 'jsdom';

const UI = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', 'apps', 'desktop', 'ui');

/** A card for somebody, with the fields a test does not care about filled in. */
function view(over = {}) {
  return {
    address: 'erika@example.de',
    name: 'Erika Mustermann',
    received: 12,
    sent: 3,
    unread: 2,
    with_attachments: 0,
    first_utc: 1_600_000_000,
    last_from_them_utc: 1_700_000_000,
    last_to_them_utc: 1_700_000_500,
    category: 'personal',
    category_count: 10,
    bulk: false,
    folders: [{ name: 'INBOX', messages: 9 }, { name: 'Archive', messages: 3 }],
    waiting: null,
    recent: [
      { id: 3, from_me: true, date_utc: 1_700_000_500, subject: 'Re: Rechnung', snippet: 'geht klar', unread: false },
      { id: 2, from_me: false, date_utc: 1_700_000_000, subject: 'Rechnung', snippet: 'anbei', unread: true },
      { id: 1, from_me: false, date_utc: 1_600_000_000, subject: 'Hallo', snippet: 'hi', unread: false },
    ],
    ...over,
  };
}

/** `sender.js`, evaluated with the globals the window would have given it. */
function load({ answer = view(), fail = null } = {}) {
  const jsdom = new JSDOM(
    `<!doctype html>
     <section id="list"><div id="viewport"></div></section>
     <aside id="aside" hidden>
       <section id="sender" hidden></section>
       <section id="assistant" hidden></section>
     </aside>
     <div id="sender-hover" hidden></div>`,
    { url: 'http://localhost/' },
  );
  const { window } = jsdom;
  const { document } = window;
  globalThis.window = window;
  globalThis.document = document;

  const el = (id) => document.getElementById(id);
  const state = { account: 1, view: 'mail', postbox: null, trash: 'Trash', rows: new Map() };
  const asked = [];
  const invoke = async (command, args) => {
    asked.push({ command, args });
    if (command !== 'sender' && command !== 'paper_sender') return null;
    if (fail) throw new Error(fail);
    return answer;
  };
  // Enough of `t` to keep the placeholders honest without loading the
  // catalogue, which `desktop-i18n.test.js` covers on its own.
  const t = (text, vars = null) => {
    let out = text.split('|')[0];
    for (const [name, value] of Object.entries(vars ?? {})) out = out.replaceAll(`{${name}}`, String(value));
    return out;
  };
  const stamp = (seconds) => (seconds ? new Date(seconds * 1000).toISOString().slice(0, 10) : '');

  // What the window's other files would have defined for it.
  const opened = [];
  const said = [];
  const reloads = [];
  const source = readFileSync(`${UI}/sender.js`, 'utf8');
  const make = new Function(
    'el', 'invoke', 'state', 't', 'initials', 'listDate', 'formatDate', 'LEVELS',
    'viewport', 'window', 'document', 'openWholeMessage', 'syncAside', 'select', 'say',
    'iconSvg', 'reload',
    `${source}
     return { senderCard, showSender, hideSender, forgetSenders, hideHover, hoverTarget, senderName };`,
  );
  const api = make(
    el,
    invoke,
    state,
    t,
    (name) => name.slice(0, 1).toUpperCase(),
    stamp,
    stamp,
    { 3: 'today', 2: 'this week', 1: 'can wait', 0: 'nothing to do' },
    el('viewport'),
    window,
    document,
    (id) => opened.push({ message: id }),
    () => {
      const open = !el('assistant').hidden || !el('sender').hidden;
      el('aside').hidden = !open;
    },
    (index) => opened.push({ row: index }),
    (text) => said.push(text),
    () => document.createElement('span'),
    async () => reloads.push(true),
  );
  return { ...api, el, state, asked, opened, said, reloads, document, window };
}

/** Lets the card's fetch settle. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

test('the card says how much mail there is each way, and never a count of none', () => {
  const { senderCard, document } = load();
  const host = document.createElement('div');
  host.append(senderCard(view()));

  assert.equal(host.querySelector('.sender-name').textContent, 'Erika Mustermann');
  assert.equal(host.querySelector('.sender-address').textContent, 'erika@example.de');
  const stats = [...host.querySelectorAll('.sender-stats span')].map((s) => s.textContent);
  assert.deepEqual(stats, ['12 from them', '3 from you', '2 unread']);
  assert.ok(
    !stats.some((s) => s.startsWith('0 ')),
    'a count of nothing is left out rather than shown as zero',
  );
});

test('somebody with no mail at all reads as that, not as an empty card', () => {
  const { senderCard, document } = load();
  const host = document.createElement('div');
  host.append(
    senderCard(
      view({
        received: 0,
        sent: 0,
        unread: 0,
        first_utc: null,
        last_from_them_utc: null,
        last_to_them_utc: null,
        category: null,
        folders: [],
        recent: [],
      }),
    ),
  );
  assert.equal(host.querySelector('.sender-stats').textContent, 'no mail with them yet');
  assert.equal(host.querySelector('.sender-recent'), null);
  // "You wrote — never" is the one line worth keeping: it is the answer to
  // "have I ever replied to these people".
  assert.match(host.textContent, /You wrote/);
  assert.match(host.textContent, /never/);
});

test('bulk mail is named as bulk, and a waiting verdict is carried over', () => {
  const { senderCard, document } = load();
  const host = document.createElement('div');
  host.append(
    senderCard(
      view({
        category: 'newsletter',
        bulk: true,
        waiting: { score: 3, reason: 'an invoice is due', action: 'pay', deadline: null, source: 'rules', model: null },
      }),
    ),
  );
  assert.match(host.textContent, /newsletter — bulk mail/);
  assert.equal(
    host.querySelector('.sender-waiting').textContent,
    'Waiting on you today: an invoice is due',
  );
});

test('the exchange reads newest first, each line saying which way it went', () => {
  const { senderCard, document } = load();
  const host = document.createElement('div');
  host.append(senderCard(view(), { interactive: true }));

  const rows = [...host.querySelectorAll('.sender-msg')];
  assert.deepEqual(
    rows.map((row) => [row.querySelector('.way').textContent, row.querySelector('.subject').textContent]),
    [['→', 'Re: Rechnung'], ['←', 'Rechnung'], ['←', 'Hallo']],
  );
  assert.ok(rows[1].classList.contains('unread'), 'unread mail of theirs is marked');
  assert.equal(rows[0].tagName, 'BUTTON', 'in the sidebar a message can be opened');
});

test('the hover copy leaves out what it could not be clicked to do', () => {
  const { senderCard, document } = load();
  const host = document.createElement('div');
  host.append(senderCard(view(), { recent: 3 }));

  assert.equal(host.querySelector('.sender-actions'), null);
  assert.equal(host.querySelector('button'), null, 'the floating card takes no pointer events');
  assert.equal(host.querySelectorAll('.sender-msg').length, 3);
});

test('the sidebar card is filled once and then asks the core no more', async () => {
  const { showSender, hideSender, forgetSenders, el, asked } = load();
  await showSender({ id: 7, asked: true });
  await settle();
  assert.equal(el('sender').hidden, false);
  assert.match(el('sender').textContent, /Erika Mustermann/);
  assert.deepEqual(asked[0], { command: 'sender', args: { account: 1, id: 7, address: null } });

  await showSender({ id: 7, asked: true });
  await settle();
  assert.equal(asked.length, 1, 'the same message is not asked about twice');

  // A reload moves the counts, so the cards are thrown away.
  forgetSenders();
  await showSender({ id: 7, asked: true });
  await settle();
  assert.equal(asked.length, 2);

  hideSender();
  assert.equal(el('sender').hidden, true);
  assert.equal(el('sender').textContent, '');
});

test('a card that cannot be built leaves the message it belongs to alone', async () => {
  const { showSender, el } = load({ fail: 'no such account' });
  await showSender({ id: 7, asked: true });
  await settle();
  assert.equal(el('sender').hidden, true, 'no error is put over the open message');
});

test('what a hovered row is about depends on which list it is in', () => {
  const { hoverTarget, state, document } = load();
  const row = document.createElement('div');
  row.className = 'row';
  row.dataset.index = '0';
  document.getElementById('viewport').append(row);
  state.rows.set(0, { id: 42, key: 'erika@example.de' });

  assert.deepEqual(hoverTarget(row), { node: row, id: 42 });

  // In People a row is a person, so the card is asked for by address.
  state.view = 'people';
  assert.deepEqual(hoverTarget(row), { node: row, address: 'erika@example.de' });

  // In a postbox it is a letter, and the card comes from Paperless.
  state.view = 'mail';
  state.postbox = { id: 7 };
  assert.deepEqual(hoverTarget(row), { node: row, id: 42, postbox: 7 });

  // A page still in flight has nothing to say yet.
  row.classList.add('pending');
  assert.equal(hoverTarget(row), null);
});

test('a scanned letter asks Paperless, and shows the address off the page', async () => {
  const letter = view({
    address: '',
    postal: 'Musterstraße 1\n80331 München',
    name: 'Stadtwerke Musterstadt GmbH',
    sent: 0,
    last_to_them_utc: null,
    with_attachments: 0,
    folders: [{ name: 'Rechnungen', messages: 4 }],
    recent: [{ id: 91, from_me: false, date_utc: 1_700_000_000, subject: 'Stromrechnung', snippet: '98,40 EUR', unread: true }],
  });
  const { showSender, el, asked, opened, said, state } = load({ answer: letter });
  state.postbox = { id: 7 };
  await showSender({ id: 91, postbox: 7, asked: true });
  await settle();

  assert.deepEqual(asked[0], { command: 'paper_sender', args: { id: 7, documentId: 91 } });
  assert.equal(el('sender').querySelector('.sender-postal').textContent, 'Musterstraße 1\n80331 München');
  assert.equal(el('sender').querySelector('.sender-address'), null, 'a letter carries no e-mail address');
  // Nothing was ever sent back to a scanner, so there is no conversation.
  assert.equal(el('sender').querySelector('.sender-actions'), null);
  assert.match(el('sender').textContent, /Filed in/);
  assert.match(el('sender').textContent, /Rechnungen/);

  // One of their other letters opens in the postbox, not as a message: the
  // list is already showing it, so it is the cursor that moves.
  state.rows.set(3, { id: 91 });
  el('sender').querySelector('.sender-msg').click();
  await settle();
  assert.deepEqual(opened, [{ row: 3 }]);

  // A letter further down than the list has fetched says so rather than
  // silently doing nothing.
  state.rows.clear();
  el('sender').querySelector('.sender-msg').click();
  await settle();
  assert.equal(opened.length, 1);
  assert.match(said[0], /further down the list/);
});

test('the card opens the right-hand column and gives it back', async () => {
  const { showSender, hideSender, el } = load();
  assert.equal(el('aside').hidden, true);
  await showSender({ id: 7, asked: true });
  await settle();
  assert.equal(el('aside').hidden, false, 'the column is there because the card is');
  hideSender();
  assert.equal(el('aside').hidden, true);

  // With the assistant open the column stays, card or no card.
  el('assistant').hidden = false;
  await showSender({ id: 7, asked: true });
  await settle();
  hideSender();
  assert.equal(el('aside').hidden, false);
});

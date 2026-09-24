/**
 * What the filed letters suggest, as the setup assistant shows it.
 *
 * The core decides which words are worth proposing and `core-paper`'s own
 * tests cover that. What is checked here is everything between that answer
 * and the shelf: that a folder with nothing to say is not given a row, that
 * "there is not enough filed yet" is told apart from "nothing stood out",
 * that clicking a word puts it in the folder's own words rather than
 * anywhere near Paperless, and that clicking the same word twice does not
 * write it twice.
 *
 * `setup.js` is a classic script with side effects at the top of it, so the
 * block that draws this is evaluated on its own with the globals the window
 * would have given it — the trick `desktop-sender.test.js` uses, with a
 * narrower slice.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { JSDOM } from 'jsdom';

const UI = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', 'apps', 'desktop', 'ui');

/** The block of `setup.js` that draws the suggestions. */
function source() {
  const whole = readFileSync(`${UI}/setup.js`, 'utf8');
  const from = whole.indexOf('async function showLearned()');
  const to = whole.indexOf('el("shelf-learn").onclick');
  assert.ok(from > 0 && to > from, 'the suggestion block moved; this test slices setup.js');
  return whole.slice(from, to);
}

/** It, evaluated over a DOM with the shelf page's ids in it. */
function load({ answer = [], fail = null, shelf = [] } = {}) {
  const { window } = new JSDOM(
    `<!doctype html>
     <div id="shelf"></div>
     <div id="household"></div>
     <button id="shelf-learn"></button>
     <div id="shelf-learned" hidden><div id="shelf-learned-list"></div></div>`,
    { url: 'http://localhost/' },
  );
  const { document } = window;
  const el = (id) => document.getElementById(id);
  const node = (tag, props = {}, ...children) => {
    const element = Object.assign(document.createElement(tag), props);
    element.append(...children.filter((child) => child !== null && child !== undefined));
    return element;
  };
  const asked = [];
  const invoke = async (command, args) => {
    asked.push({ command, args });
    if (fail) throw new Error(fail);
    return answer;
  };
  const draws = [];
  const make = new Function(
    'el', 'node', 't', 'invoke', 'shelfAddress', 'shelf', 'drawShelf',
    `${source()}
     return { showLearned, drawLearned, giveWord };`,
  );
  const api = make(
    el,
    node,
    (text) => text,
    invoke,
    async () => ({ id: 7 }),
    shelf,
    () => draws.push(true),
  );
  return { ...api, el, document, asked, shelf, draws };
}

const KASSENZEICHEN = { word: 'Kassenzeichen', letters: 4 };

test('a suggested word goes into the folder it was suggested for', async () => {
  const shelf = [
    { name: 'Taxes', words: 'Finanzamt', person: false },
    { name: 'House', words: '', person: false },
  ];
  const view = load({
    shelf,
    answer: [
      { folder: 'Taxes', from: 4, words: [KASSENZEICHEN] },
      { folder: 'House', from: 2, words: [{ word: 'Nebenkostenabrechnung', letters: 2 }] },
    ],
  });
  await view.showLearned();

  const rows = [...view.el('shelf-learned-list').querySelectorAll('.learned-row')];
  assert.equal(rows.length, 2);
  assert.equal(rows[0].querySelector('strong').textContent, 'Taxes');

  const chip = rows[0].querySelector('.learned-words button');
  assert.equal(chip.textContent, 'Kassenzeichen');
  chip.click();

  assert.equal(shelf[0].words, 'Finanzamt, Kassenzeichen', 'added to the words already there');
  assert.equal(shelf[1].words, '', 'and to nothing else');
  assert.ok(chip.disabled, 'the word says it has been taken');
  assert.ok(view.draws.length, 'the shelf was redrawn so the box shows it');
});

test('the first word of an empty folder does not come with a comma in front of it', async () => {
  const shelf = [{ name: 'House', words: '', person: false }];
  const view = load({ shelf, answer: [{ folder: 'House', from: 3, words: [KASSENZEICHEN] }] });
  await view.showLearned();
  view.el('shelf-learned-list').querySelector('button').click();
  assert.equal(shelf[0].words, 'Kassenzeichen');
});

test('a word the folder already has is not added again', async () => {
  const shelf = [{ name: 'Taxes', words: 'kassenzeichen, Finanzamt', person: false }];
  const view = load({ shelf, answer: [{ folder: 'Taxes', from: 4, words: [KASSENZEICHEN] }] });
  await view.showLearned();
  view.el('shelf-learned-list').querySelector('button').click();
  // Whatever its case: Paperless matches without regard to it, so a second
  // spelling would be a second copy of the same word.
  assert.equal(shelf[0].words, 'kassenzeichen, Finanzamt');
});

test('a word for a folder that is no longer on the shelf changes nothing', async () => {
  const shelf = [{ name: 'Taxes', words: '', person: false }];
  const view = load({ shelf, answer: [{ folder: 'Ferien', from: 3, words: [KASSENZEICHEN] }] });
  await view.showLearned();
  view.el('shelf-learned-list').querySelector('button').click();
  assert.equal(shelf[0].words, '');
});

test('a folder with nothing to suggest gets no row at all', async () => {
  const view = load({
    shelf: [{ name: 'Taxes', words: '', person: false }],
    answer: [
      { folder: 'Taxes', from: 4, words: [KASSENZEICHEN] },
      { folder: 'Car', from: 0, words: [] },
    ],
  });
  await view.showLearned();
  const rows = [...view.el('shelf-learned-list').querySelectorAll('.learned-row strong')];
  assert.deepEqual(rows.map((row) => row.textContent), ['Taxes']);
});

test('nothing filed yet and nothing stood out are two different sentences', async () => {
  // One folder with letters in it: the core cannot tell a keyword from a
  // greeting yet, and saying "nothing stood out" would be a lie.
  const early = load({ answer: [{ folder: 'Taxes', from: 6, words: [] }, { folder: 'Car', from: 0, words: [] }] });
  await early.showLearned();
  const first = early.el('shelf-learned-list').textContent;
  assert.match(first, /two different folders/);

  // Two folders with letters, and still nothing: that is a real answer.
  const later = load({ answer: [{ folder: 'Taxes', from: 6, words: [] }, { folder: 'Car', from: 4, words: [] }] });
  await later.showLearned();
  const second = later.el('shelf-learned-list').textContent;
  assert.match(second, /Nothing stood out/);
  assert.notEqual(first, second);
});

test('the panel asks the core once, for the address it is set up for, and asks for nothing else', async () => {
  const view = load({ answer: [{ folder: 'Taxes', from: 4, words: [KASSENZEICHEN] }] });
  await view.showLearned();
  assert.deepEqual(view.asked, [{ command: 'paper_suggested_words', args: { id: 7 } }]);
  view.el('shelf-learned-list').querySelector('button').click();
  assert.equal(view.asked.length, 1, 'taking a word writes nothing anywhere');
});

test('a core that will not answer says so and leaves the button usable', async () => {
  const view = load({ fail: 'paperless is not running' });
  await view.showLearned();
  assert.match(view.el('shelf-learned-list').textContent, /paperless is not running/);
  assert.equal(view.el('shelf-learn').disabled, false, 'it can be tried again');
});

/**
 * Who the message is with: the card, and the column it shares with the
 * assistant, in the desktop app's own window.
 *
 * A browser rather than jsdom because the whole question here is layout. The
 * card and the assistant are stacked in one flex column that only exists when
 * something is in it, and what has to hold is that neither ever squeezes the
 * other off the screen — which is a thing you can only ask an engine that
 * does layout. The card's contents are covered in `test/desktop-sender.test.js`.
 *
 * Kept out of `npm test` with the rest of `e2e/`: they need a browser.
 */

import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import http from 'node:http';
import { readFile } from 'node:fs/promises';
import { dirname, extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { chromium } from 'playwright';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const PAGE = '/apps/desktop/ui/index.html';
const FAKE = '/kuverta-bird/test/support/fake-invoke.js';

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
};

let browser;
let server;
let base;

function serve(root) {
  return new Promise((ready) => {
    const instance = http.createServer(async (request, response) => {
      const path = normalize(decodeURIComponent(new URL(request.url, 'http://x').pathname));
      const file = join(root, path);
      if (!file.startsWith(root)) {
        response.writeHead(403).end();
        return;
      }
      try {
        const body = await readFile(file);
        response.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream' });
        response.end(body);
      } catch {
        response.writeHead(404).end();
      }
    });
    instance.listen(0, '127.0.0.1', () => ready(instance));
  });
}

before(async () => {
  server = await serve(ROOT);
  base = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch();
});

after(async () => {
  await browser?.close();
  server?.close();
});

/** Three from one sender, three from three others. */
function runFromOneSender() {
  const base = Math.floor(Date.UTC(2026, 8, 11, 12, 0, 0) / 1000);
  const message = (i, from, subject) => ({
    subject,
    from,
    date_utc: base - i * 3600,
    unread: false,
    has_attachments: false,
    category: null,
    folder: 'INBOX',
    message_id: `msg-${i}@example.com`,
    snippet: `Body of message ${i}.`,
    body: `Body of message ${i}.`,
  });
  return [
    message(0, 'Shop <shop@example.de>', 'Ihre Bestellung 1041'),
    message(1, 'Shop <shop@example.de>', 'Ihre Bestellung 1052'),
    message(2, 'Shop <shop@example.de>', 'Ihre Bestellung 1063'),
    message(3, 'Erika Mustermann <erika@example.de>', 'Mittwoch?'),
    message(4, 'Finanzamt <post@example.de>', 'Bescheid'),
    message(5, 'Verein <verein@example.de>', 'Einladung'),
  ];
}

async function openWindow({ seed = null } = {}) {
  const context = await browser.newContext({ viewport: { width: 1200, height: 800 } });
  const page = await context.newPage();
  const problems = [];
  page.on('pageerror', (error) => problems.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') problems.push(message.text());
  });

  await page.addInitScript(
    (options) => {
      // Far enough away that only the sync at startup happens.
      window.__kuvertaSyncEveryMs = 600000;
      window.__fakeBridge = import(options.fake).then((module) =>
        module.fakeInvoke(options.seed ? { seed: options.seed } : {}),
      );
      window.__TAURI__ = {
        core: {
          invoke: async (command, args) => (await window.__fakeBridge)(command, args),
          Channel: class {
            onmessage = null;
          },
        },
      };
    },
    { fake: FAKE, seed },
  );

  await page.goto(`${base}${PAGE}`);
  await page.waitForFunction(() => document.getElementById('status')?.textContent === 'you@example.com');
  await page.waitForFunction(() => document.querySelectorAll('#content .row:not([hidden])').length > 0);
  return { page, context, problems };
}


const rows = (page) => page.locator('#content .row:not([hidden])');
const aside = (page) => page.locator('#aside');
const card = (page) => page.locator('#sender');
const grip = (page, which) => page.locator(`.col-grip[data-resizes="${which}"]`);

/// Opens a message and then asks who it is from, which is the only way the
/// card opens: reading a message must not take a quarter of the window.
async function openCard(page, row = 3) {
  await rows(page).nth(row).click();
  await page.waitForSelector('#reading:not([hidden])');
  await page.locator('#reading-meta .sender-link').click();
  await card(page).waitFor();
}

/** Where something is on screen, or null when it is not. */
const box = (locator) => locator.boundingBox();

test('reading a message does not open the card; asking who it is from does', async () => {
  const { page, context, problems } = await openWindow({ seed: runFromOneSender() });
  assert.equal(await aside(page).isVisible(), false, 'nothing is open yet');

  // The bug this is here for: the column opened itself on every message.
  await rows(page).nth(3).click();
  await page.waitForSelector('#reading:not([hidden])');
  assert.equal(await aside(page).isVisible(), false, 'reading a message opened it');

  await page.locator('#reading-meta .sender-link').click();
  await card(page).waitFor();
  assert.match(await card(page).innerText(), /Erika Mustermann/);
  assert.match(await card(page).innerText(), /erika@example.de/);

  // The column is to the right of the message being read, not over it.
  const reading = await box(page.locator('#reading'));
  const column = await box(aside(page));
  assert.ok(column.x >= reading.x + reading.width - 1, `${column.x} vs ${reading.x + reading.width}`);
  assert.ok(column.width >= 300, `the column is ${column.width}px wide`);
  assert.deepEqual(problems, []);
  await context.close();
});

test('the card and the assistant share the column, neither pushed off it', async () => {
  const { page, context, problems } = await openWindow({ seed: runFromOneSender() });
  await openCard(page);
  const alone = await box(card(page));

  await page.locator('#assistant-toggle').click();
  await page.waitForSelector('#assistant:not([hidden])');
  const together = await box(card(page));
  const assistant = await box(page.locator('#assistant'));
  const column = await box(aside(page));

  // One column, not two: same width, and the assistant starts where the card
  // stops.
  assert.equal(Math.round(together.width), Math.round(alone.width));
  assert.ok(assistant.y >= together.y + together.height - 1, 'the assistant is under the card');
  assert.ok(
    assistant.y + assistant.height <= column.y + column.height + 1,
    'the assistant still fits in the window',
  );
  // The card gives way rather than the typing surface: it is capped, and what
  // does not fit scrolls inside it.
  assert.ok(together.height <= column.height * 0.5, `card is ${together.height} of ${column.height}`);
  assert.ok(await page.locator('#chat-input').isVisible(), 'there is still somewhere to type');
  assert.deepEqual(problems, []);
  await context.close();
});

test('the column comes out of the reading pane, and goes back into it', async () => {
  const { page, context } = await openWindow({ seed: runFromOneSender() });
  await rows(page).nth(3).click();
  await page.waitForSelector('#reading:not([hidden])');
  const list = (await box(page.locator('#list'))).width;
  const wide = (await box(page.locator('#detail'))).width;

  await openCard(page);
  const narrow = (await box(page.locator('#detail'))).width;
  // The sidebar and the list are set by hand now, so the reading pane is the
  // one that gives: it is the column with no width of its own.
  assert.ok(narrow < wide, `the reading pane gave up width: ${narrow} from ${wide}`);
  assert.equal(Math.round((await box(page.locator('#list'))).width), Math.round(list));

  await page.locator('#sender .sender-close').click();
  await page.waitForFunction(() => document.getElementById('aside')?.hidden === true);
  assert.equal(Math.round((await box(page.locator('#detail'))).width), Math.round(wide));

  // And it stays shut: the next message read does not bring it back.
  await rows(page).nth(4).click();
  await page.waitForTimeout(120);
  assert.equal(await aside(page).isVisible(), false);
  await context.close();
});

test('the card follows the cursor once it has been asked for', async () => {
  const { page, context } = await openWindow({ seed: runFromOneSender() });
  await openCard(page, 3);
  assert.match(await card(page).innerText(), /Erika Mustermann/);

  // Having asked who one message is with, you are usually asking of the next
  // one too — so it keeps up rather than making you click again.
  await rows(page).nth(4).click();
  await page.waitForFunction(
    () => /Finanzamt/.test(document.getElementById('sender')?.innerText ?? ''),
    { timeout: 4000 },
  );
  await context.close();
});

test('a message can be thrown away from the card it is listed on', async () => {
  const { page, context, problems } = await openWindow({ seed: runFromOneSender() });
  await openCard(page, 0);
  const shop = page.locator('#sender .sender-msg-row');
  const before = await shop.count();
  assert.ok(before >= 3, `the shop's three orders are listed: ${before}`);
  const subject = await shop.first().locator('.subject').innerText();

  const listed = await rows(page).count();
  await shop.first().locator('.sender-bin').click();
  await page.waitForFunction(
    (gone) => !(document.getElementById('sender')?.innerText ?? '').includes(gone),
    subject,
    { timeout: 5000 },
  );

  // It has left the list as well as the card, and the card is still open and
  // still about the same person.
  assert.equal(await rows(page).count(), listed - 1);
  assert.match(await card(page).innerText(), /Shop/);
  assert.match(await page.locator('#toast').innerText(), /Trash/i);
  assert.deepEqual(problems, []);
  await context.close();
});

// -- the width of the columns -------------------------------------------------

test('the seams between the columns can be dragged, and remember where', async () => {
  const { page, context, problems } = await openWindow({ seed: runFromOneSender() });
  const widthOf = async (selector) => (await box(page.locator(selector))).width;

  for (const [which, pane, by] of [
    ['sidebar', '#sidebar', 70],
    ['list', '#list', -60],
  ]) {
    const before = await widthOf(pane);
    const seam = await box(grip(page, which));
    await page.mouse.move(seam.x + seam.width / 2, seam.y + seam.height / 2);
    await page.mouse.down();
    await page.mouse.move(seam.x + seam.width / 2 + by, seam.y + seam.height / 2, { steps: 8 });
    await page.mouse.up();
    const after = await widthOf(pane);
    assert.ok(
      Math.abs(after - (before + by)) < 6,
      `${which}: ${before} + ${by} should be about ${after}`,
    );
  }

  // A column dragged past what its pane can work at stops at the limit
  // rather than disappearing.
  const seam = await box(grip(page, 'sidebar'));
  await page.mouse.move(seam.x + seam.width / 2, seam.y + seam.height / 2);
  await page.mouse.down();
  await page.mouse.move(2, seam.y + seam.height / 2, { steps: 8 });
  await page.mouse.up();
  assert.ok(await widthOf('#sidebar') >= 170, 'the sidebar kept a usable width');

  // Double-click puts it back where it started.
  await grip(page, 'sidebar').dblclick();
  assert.equal(Math.round(await widthOf('#sidebar')), 236);

  // The widths outlive the window.
  const kept = await page.evaluate(() => localStorage.getItem('column.list'));
  assert.ok(Number(kept) > 0, `the list width was remembered: ${kept}`);
  assert.deepEqual(problems, []);
  await context.close();
});

test('resting on a row shows the card without opening anything', async () => {
  const { page, context, problems } = await openWindow({ seed: runFromOneSender() });
  const hover = page.locator('#sender-hover');
  assert.equal(await hover.isVisible(), false);

  await rows(page).nth(3).hover();
  await hover.waitFor({ timeout: 4000 });
  assert.match(await hover.innerText(), /Erika Mustermann/);
  // Nothing was opened and nothing was read: this is a question about a row.
  assert.equal(await page.locator('#reading').isHidden(), true);
  assert.equal(await aside(page).isVisible(), false);

  // It sits beside the list, clear of it, and inside the window.
  const list = await box(page.locator('#list'));
  const floating = await box(hover);
  assert.ok(floating.x >= list.x + list.width - 1, 'clear of the list it belongs to');
  assert.ok(floating.x + floating.width <= 1200, 'and inside the window');

  // Reaching for the keyboard puts it away.
  await page.keyboard.press('j');
  await page.waitForFunction(() => document.getElementById('sender-hover')?.hidden === true);
  assert.deepEqual(problems, []);
  await context.close();
});

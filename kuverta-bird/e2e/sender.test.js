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

/** Where something is on screen, or null when it is not. */
const box = (locator) => locator.boundingBox();

test('opening a message puts the card in a column of its own', async () => {
  const { page, context, problems } = await openWindow({ seed: runFromOneSender() });
  assert.equal(await aside(page).isVisible(), false, 'nothing is open yet');

  await rows(page).nth(3).click();
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
  await rows(page).nth(3).click();
  await card(page).waitFor();
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

test('closing the message gives the column back to the list', async () => {
  const { page, context } = await openWindow({ seed: runFromOneSender() });
  const wide = (await box(page.locator('#list'))).width;

  await rows(page).nth(3).click();
  await card(page).waitFor();
  const narrow = (await box(page.locator('#list'))).width;
  assert.ok(narrow < wide, `the list gave up width: ${narrow} from ${wide}`);

  // A reload — here, switching to another mailbox — closes what was open.
  await page.locator('#folders .nav-item').nth(1).click();
  await page.waitForFunction(() => document.getElementById('aside')?.hidden === true);
  assert.equal(Math.round((await box(page.locator('#list'))).width), Math.round(wide));
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

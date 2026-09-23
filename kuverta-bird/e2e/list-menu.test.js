/**
 * Picking several messages with the mouse, and the menu under the right
 * button, in the desktop app's own window.
 *
 * A browser rather than jsdom because both halves of this are things jsdom
 * cannot answer: whether shift-clicking a row also dragged a text selection
 * across it, and where a menu positioned against the window ends up.
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

/** Three from one sender, three from three others — a run inside a mailbox. */
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
const menu = (page) => page.locator('#context-menu');
const items = (page) => page.locator('#context-menu button');
const picked = (page) => page.evaluate(() => document.querySelectorAll('#content .row.picked').length);
const scope = (page) => page.locator('#scope').innerText();

// -- picking with the mouse --------------------------------------------------

test('shift-clicking picks a block of messages and drags no text along with it', async () => {
  const { page, context, problems } = await openWindow();
  await rows(page).nth(0).click();
  await rows(page).nth(2).click({ modifiers: ['Shift'] });

  assert.equal(await picked(page), 3);
  // The bug this is here for: a block picked with shift used to come with the
  // browser's own text selection smeared across the rows it passed.
  assert.equal(await page.evaluate(() => window.getSelection().toString()), '');
  assert.deepEqual(problems, []);
  await context.close();
});

test('text in the message being read can still be selected', async () => {
  const { page, context } = await openWindow();
  await rows(page).nth(0).click();
  await page.waitForFunction(() => !document.getElementById('reading')?.hidden);
  await page.locator('#reading-body').selectText();
  assert.match(await page.evaluate(() => window.getSelection().toString()), /Body of message/);
  await context.close();
});

// -- the menu ----------------------------------------------------------------

test('the right button over a picked block keeps the block', async () => {
  const { page, context } = await openWindow();
  await rows(page).nth(0).click();
  await rows(page).nth(2).click({ modifiers: ['Shift'] });
  await rows(page).nth(1).click({ button: 'right' });

  await menu(page).waitFor();
  assert.equal(await picked(page), 3);
  // Three messages, so there is nothing to open: the menu says what can be
  // done to all of them.
  assert.equal(await items(page).filter({ hasText: /^Open$/ }).count(), 0);
  await context.close();
});

test('the right button outside the block takes the row under the pointer instead', async () => {
  const { page, context } = await openWindow();
  await rows(page).nth(0).click();
  await rows(page).nth(1).click({ modifiers: ['Shift'] });
  await rows(page).nth(4).click({ button: 'right' });

  await menu(page).waitFor();
  assert.equal(await picked(page), 0);
  await assert.doesNotReject(items(page).filter({ hasText: /^Open$/ }).waitFor());
  // Right-clicking asks what can be done to a message, which is not the same
  // as asking to read it: the one already open stays open.
  assert.match(await page.locator('#reading-subject').innerText(), /Message 00/);
  await context.close();
});

test('a picked block is moved to a mailbox chosen from the menu', async () => {
  const { page, context, problems } = await openWindow();
  assert.match(await scope(page), /6 messages/);

  await rows(page).nth(0).click();
  await rows(page).nth(2).click({ modifiers: ['Shift'] });
  await rows(page).nth(1).click({ button: 'right' });
  await items(page).filter({ hasText: 'Move to mailbox…' }).click();
  await items(page).filter({ hasText: /^Archive$/ }).click();

  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('3 messages'));
  assert.match(await page.locator('#toast').innerText(), /moved to Archive 3 messages/);
  assert.deepEqual(problems, []);
  await context.close();
});

// -- the smart mailbox it suggests -------------------------------------------

test('several messages from one sender offer a smart mailbox already written', async () => {
  const { page, context, problems } = await openWindow({ seed: runFromOneSender() });
  await rows(page).nth(0).click();
  await rows(page).nth(2).click({ modifiers: ['Shift'] });
  await rows(page).nth(1).click({ button: 'right' });
  await items(page).filter({ hasText: 'New smart mailbox from these 3…' }).click();

  await page.locator('#smart-sheet').waitFor();
  assert.equal(await page.locator('#smart-form-title').innerText(), 'New smart mailbox');
  assert.equal(await page.locator('#smart-form [name=name]').inputValue(), 'Shop <shop@example.de>');
  const rule = page.locator('#smart-rules .rule').first();
  assert.equal(await rule.locator('[name=field]').inputValue(), 'from');
  assert.equal(await rule.locator('[name=op]').inputValue(), 'contains');
  assert.equal(await rule.locator('[name=value]').inputValue(), 'Shop <shop@example.de>');
  // And the rule it wrote gathers exactly the three that were picked, which
  // is the whole claim the offer makes.
  await page.waitForFunction(
    () => document.getElementById('smart-preview-count')?.textContent === '3 messages match',
  );
  assert.deepEqual(problems, []);
  await context.close();
});

test('messages with nothing in common are offered no rule', async () => {
  const { page, context } = await openWindow({ seed: runFromOneSender() });
  // Two different senders, two unrelated subjects: there is no rule that
  // catches these two and nothing else, and guessing one would be worse than
  // not offering.
  await rows(page).nth(3).click();
  await rows(page).nth(5).click({ modifiers: ['Shift'] });
  await rows(page).nth(4).click({ button: 'right' });

  await menu(page).waitFor();
  assert.equal(await items(page).filter({ hasText: 'smart mailbox' }).count(), 0);
  await context.close();
});

test('one message on its own is offered no rule either', async () => {
  const { page, context } = await openWindow({ seed: runFromOneSender() });
  await rows(page).nth(0).click({ button: 'right' });

  await menu(page).waitFor();
  assert.equal(await items(page).filter({ hasText: 'smart mailbox' }).count(), 0);
  await context.close();
});

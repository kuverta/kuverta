/**
 * The window in German, in a real browser.
 *
 * The language follows the system unless someone says otherwise, and what is
 * translated is translated everywhere: the static chrome index.html carries,
 * the sidebar the window builds as it goes, and the settings sheet. Changing
 * it does not reload the page — a reload would lose the open message and
 * anything half typed — so these check that the text changes where it stands.
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

async function openWindow({ locale = 'en-GB', choice = null } = {}) {
  const context = await browser.newContext({ viewport: { width: 1200, height: 800 }, locale });
  const page = await context.newPage();
  const problems = [];
  page.on('pageerror', (error) => problems.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') problems.push(message.text());
  });

  await page.addInitScript((options) => {
    // The window remembers the choice where it remembers the sort order.
    if (options.choice) localStorage.setItem('language', options.choice);
    window.__fakeBridge = import(options.fake).then((module) => module.fakeInvoke());
    window.__TAURI__ = {
      core: {
        invoke: async (command, args) => (await window.__fakeBridge)(command, args),
        // Progress arrives on a channel in the real bridge, and the window
        // makes one for every sync, so without this nothing syncs here.
        Channel: class {
          onmessage = null;
        },
      },
    };
  }, { fake: FAKE, choice });

  await page.goto(`${base}${PAGE}`);
  await page.waitForFunction(() => document.getElementById('status')?.textContent === 'you@example.com');
  return { page, context, problems };
}

const text = (page, selector) =>
  page.locator(selector).first().textContent().then((value) => value.trim().replace(/\s+/g, ' '));

test('a German system gets a German window', async () => {
  const { page, context, problems } = await openWindow({ locale: 'de-DE' });

  assert.equal(await page.evaluate(() => document.documentElement.lang), 'de');
  // Static text index.html carries…
  assert.equal(await text(page, '#empty'), 'Nachricht auswählen');
  assert.equal(await text(page, '#new-message'), 'Neue Nachricht');
  // …and text the window builds as it goes.
  assert.equal(await text(page, '#folders-heading span'), 'Ordner');
  assert.match(await text(page, '#folders'), /Alle E-Mails/);

  assert.deepEqual(problems, []);
  await context.close();
});

test('an English system gets an English window', async () => {
  const { page, context, problems } = await openWindow({ locale: 'en-GB' });

  assert.equal(await page.evaluate(() => document.documentElement.lang), 'en');
  assert.equal(await text(page, '#empty'), 'Select a message');
  assert.equal(await text(page, '#folders-heading span'), 'Mailboxes');

  assert.deepEqual(problems, []);
  await context.close();
});

test('the choice beats the system, and changing it needs no reload', async () => {
  // German system, English asked for: what was asked for wins.
  const { page, context, problems } = await openWindow({ locale: 'de-DE', choice: 'en' });
  assert.equal(await text(page, '#empty'), 'Select a message');

  await page.keyboard.press(',');
  await page.waitForSelector('#settings', { state: 'visible' });
  assert.equal(await page.locator('#pref-language').inputValue(), 'en');

  await page.locator('#settings-list').getByText('General', { exact: true }).click();
  await page.locator('#pref-language').selectOption('de');
  // Same page, no reload: the text changes where it stands.
  await page.waitForFunction(
    () => document.getElementById('empty')?.textContent?.trim() === 'Nachricht auswählen',
  );
  assert.equal(await page.evaluate(() => document.documentElement.lang), 'de');
  assert.equal(await page.evaluate(() => localStorage.getItem('language')), 'de');

  assert.deepEqual(problems, []);
  await context.close();
});

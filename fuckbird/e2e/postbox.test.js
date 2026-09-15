/**
 * Postboxes in the desktop app's main window, in a real browser.
 *
 * Each postal address is an inbox of its own: selecting it lists its post the
 * way mail is listed, opening a letter shows its text, and the things post does
 * not have — moving, deleting, replying — say so instead of reaching for the
 * mail account behind the window. Driven by the same fake bridge as the
 * settings tests, whose default postbox is "Home" with two letters.
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
const FAKE = '/fuckbird/test/support/fake-invoke.js';

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

async function openWindow() {
  const context = await browser.newContext({ viewport: { width: 1200, height: 800 } });
  const page = await context.newPage();
  const problems = [];
  page.on('pageerror', (error) => problems.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') problems.push(message.text());
  });

  await page.addInitScript((fake) => {
    let bridge;
    window.__TAURI__ = {
      core: {
        invoke: async (command, args) => {
          bridge ??= import(fake).then((module) => module.fakeInvoke());
          return (await bridge)(command, args);
        },
      },
    };
  }, FAKE);

  await page.goto(`${base}${PAGE}`);
  await page.waitForFunction(() => document.getElementById('status')?.textContent === 'you@example.com');
  return { page, context, problems };
}

const postbox = (page) => page.locator('#postboxes .nav-item', { hasText: 'Home' });

test('each postal address is an inbox in the sidebar, beside the account', async () => {
  const { page, context, problems } = await openWindow();

  assert.equal(await page.locator('#postboxes-heading').isVisible(), true);
  await postbox(page).waitFor();
  // The count arrives from Paperless in the background.
  await page.waitForFunction(() => document.querySelector('#postboxes .count')?.textContent === '2');
  // With post beside it, the one account is shown too: it is the way back.
  assert.equal(await page.locator('#accounts .nav-item', { hasText: 'you@example.com' }).isVisible(), true);

  assert.deepEqual(problems, []);
  await context.close();
});

test('selecting a postbox lists its post like mail, without folders or categories', async () => {
  const { page, context, problems } = await openWindow();

  await postbox(page).click();
  await page.waitForFunction(() => document.getElementById('status')?.textContent === 'Home');
  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('2 letters to Home'));

  const subjects = await page.locator('#content .row:not([hidden]) .subject').allInnerTexts();
  assert.deepEqual(subjects, ['Ihre Abschlagszahlung für April', 'Bescheid über Einkommensteuer'], 'newest first');
  assert.equal(await postbox(page).evaluate((item) => item.classList.contains('active')), true);
  assert.equal(await page.locator('#folders').isVisible(), false);
  assert.equal(await page.locator('#categories').isVisible(), false);
  assert.equal(await page.locator('#categories-heading').isVisible(), false);

  assert.deepEqual(problems, []);
  await context.close();
});

test('opening a letter shows its text, where it was sent and how many pages it has', async () => {
  const { page, context, problems } = await openWindow();
  await postbox(page).click();
  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('letters'));

  await page.locator('#content .row', { hasText: 'Bescheid über Einkommensteuer' }).click();
  await page.waitForSelector('#reading', { state: 'visible' });

  assert.equal(await page.locator('#reading-subject').innerText(), 'Bescheid über Einkommensteuer');
  const meta = await page.locator('#reading-meta').innerText();
  assert.match(meta, /Finanzamt/);
  assert.match(meta, /Home/);
  assert.match(meta, /4 pages/);
  assert.equal(await page.locator('#reading-body').innerText(), 'Ihr Steuerbescheid liegt bei.');

  assert.deepEqual(problems, []);
  await context.close();
});

test('a letter opens on its text or its scan, and the choice is kept for the next letter', async () => {
  const { page, context, problems } = await openWindow();
  await postbox(page).click();
  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('2 letters'));
  await page.locator('#content .row', { hasText: 'Ihre Abschlagszahlung für April' }).click();
  await page.waitForSelector('#reading', { state: 'visible' });

  const active = () => page.locator('#reading-tabs button.active').innerText();
  const frameSrc = () => page.locator('#reading-pdf').getAttribute('src');

  assert.equal(await page.locator('#reading-tabs').isVisible(), true);
  assert.equal(await active(), 'Text');
  assert.equal(await page.locator('#reading-body').isVisible(), true);
  assert.equal(await page.locator('#reading-file').isVisible(), false);

  await page.locator('#reading-tabs button', { hasText: 'PDF' }).click();
  await page.waitForFunction(() => document.getElementById('reading-pdf')?.getAttribute('src')?.startsWith('blob:'));
  assert.equal(await active(), 'PDF');
  assert.equal(await page.locator('#reading-body').isVisible(), false);
  assert.equal(await page.locator('#reading-pdf').isVisible(), true);
  const first = await frameSrc();

  // The next letter opens on the scan too, and it is that letter's scan.
  await page.keyboard.press('j');
  await page.waitForFunction(
    (was) => {
      const src = document.getElementById('reading-pdf')?.getAttribute('src');
      return src?.startsWith('blob:') && src !== was;
    },
    first,
  );
  assert.equal(await page.locator('#reading-subject').innerText(), 'Bescheid über Einkommensteuer');
  assert.equal(await active(), 'PDF');

  // v goes back to the text.
  await page.keyboard.press('v');
  assert.equal(await active(), 'Text');
  assert.equal(await page.locator('#reading-body').isVisible(), true);

  // Mail has no scan, and no tabs.
  await page.locator('#accounts .nav-item', { hasText: 'you@example.com' }).click();
  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('messages'));
  await page.locator('#content .row').first().click();
  await page.waitForSelector('#reading', { state: 'visible' });
  assert.equal(await page.locator('#reading-tabs').isVisible(), false);
  assert.equal(await page.locator('#reading-body').isVisible(), true);

  assert.deepEqual(problems, []);
  await context.close();
});

test('archiving post is refused in words, and the account leads back to mail', async () => {
  const { page, context, problems } = await openWindow();
  await postbox(page).click();
  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('2 letters'));

  await page.keyboard.press('e');
  await page.waitForSelector('#toast:not([hidden])');
  assert.match(await page.locator('#toast').innerText(), /Home is post/);
  assert.match(await page.locator('#scope').innerText(), /2 letters/, 'nothing was taken out of the list');

  await page.locator('#accounts .nav-item', { hasText: 'you@example.com' }).click();
  await page.waitForFunction(() => document.getElementById('status')?.textContent === 'you@example.com');
  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('messages'));
  assert.equal(await page.locator('#folders').isVisible(), true);
  assert.equal(await postbox(page).evaluate((item) => item.classList.contains('active')), false);

  assert.deepEqual(problems, []);
  await context.close();
});

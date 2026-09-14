/**
 * The desktop app's settings sheet, in a real browser.
 *
 * This is `fuckmail`'s own window rather than the shared surface, and it lives
 * here because the browser harness does. It exists for one class of bug in
 * particular: the sheet shows either the account form or the postal-address
 * form in one place, and both carry `display: flex`, which beats the hidden
 * attribute's own `display: none`. With the rule that fixes it removed, both
 * forms are on screen at once and nothing says which Save belongs to which.
 * jsdom cannot see that. A browser can.
 */

import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import http from 'node:http';
import { readFile } from 'node:fs/promises';
import { dirname, extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { chromium } from 'playwright';

// The repository root: the window's page is under apps/, the double under fuckbird/.
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

async function openSettings() {
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
          bridge ??= import(fake).then((module) =>
            module.fakeInvoke({ paper: { paperMailboxes: [], documents: [] } }),
          );
          return (await bridge)(command, args);
        },
      },
    };
  }, FAKE);

  await page.goto(`${base}${PAGE}`);
  await page.waitForFunction(() => document.getElementById('status')?.textContent === 'you@example.com');

  await page.keyboard.press(',');
  await page.waitForSelector('#settings', { state: 'visible' });
  return { page, context, problems };
}

test('the settings sheet opens on the account form alone', async () => {
  const { page, context } = await openSettings();
  assert.equal(await page.locator('#settings-form').isVisible(), true);
  assert.equal(await page.locator('#paper-form').isVisible(), false, 'the address form must be off screen');
  await context.close();
});

test('adding a postal address swaps the forms rather than stacking them', async () => {
  // The display: flex bug, in the only place it can be seen: with it, both
  // forms stay on screen and a Save could be for either.
  const { page, context } = await openSettings();

  await page.locator('#settings-add-paper').click();
  assert.equal(await page.locator('#paper-form').isVisible(), true);
  assert.equal(await page.locator('#settings-form').isVisible(), false, 'the account form must be off screen');
  assert.equal(await page.locator('#paper-save').isVisible(), true);
  assert.equal(await page.locator('#settings-save').isVisible(), false, 'only one Save may be on screen');

  await page.locator('#settings-add').click();
  assert.equal(await page.locator('#settings-form').isVisible(), true);
  assert.equal(await page.locator('#paper-form').isVisible(), false);
  await context.close();
});

test('"all of them" hides the name field, and naming a tag shows it', async () => {
  const { page, context } = await openSettings();
  await page.locator('#settings-add-paper').click();

  assert.equal(await page.locator('#paper-selector-value').isVisible(), false);

  await page.locator('#paper-form select[name="selector_kind"]').selectOption('tag');
  assert.equal(await page.locator('#paper-selector-value').isVisible(), true);

  await page.locator('#paper-form select[name="selector_kind"]').selectOption('everything');
  assert.equal(await page.locator('#paper-selector-value').isVisible(), false);
  await context.close();
});

test('the list says where addresses go, even before there are any', async () => {
  const { page, context, problems } = await openSettings();
  const list = await page.locator('#settings-list').innerText();
  assert.match(list, /you@example\.com/);
  assert.match(list, /Postal addresses/);
  assert.deepEqual(problems, []);
  await context.close();
});

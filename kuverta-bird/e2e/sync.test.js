/**
 * Fetching mail in the desktop app's main window, in a real browser.
 *
 * Mail only ever arrived when someone pressed r, so an account nobody was
 * looking at sat still. It is now fetched when the window opens, on a timer
 * after that, and for one account from its row in the sidebar — where the
 * button's tooltip is the one place the window says when mail last arrived.
 * The fake bridge counts syncs, which is what these watch.
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

async function openWindow({ every = null } = {}) {
  const context = await browser.newContext({ viewport: { width: 1200, height: 800 } });
  const page = await context.newPage();
  const problems = [];
  page.on('pageerror', (error) => problems.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') problems.push(message.text());
  });

  await page.addInitScript((options) => {
    // A test cannot wait five minutes for the next sync. Null leaves the
    // timer far enough away that only the sync at startup happens.
    if (options.every !== null) window.__kuvertaSyncEveryMs = options.every;
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
  }, { fake: FAKE, every });

  await page.goto(`${base}${PAGE}`);
  await page.waitForFunction(() => document.getElementById('status')?.textContent === 'you@example.com');
  return { page, context, problems };
}

const syncs = (page) => page.evaluate(async () => (await window.__fakeBridge)('__syncs'));

/**
 * Waits for the window to have asked for `atLeast` syncs.
 *
 * Polled here rather than with `waitForFunction`, which takes the promise an
 * async callback returns as the answer — and a promise is truthy, so such a
 * wait ends at once and proves nothing.
 */
async function untilSyncs(page, atLeast) {
  for (let left = 100; left > 0; left -= 1) {
    if ((await syncs(page)) >= atLeast) return;
    await page.waitForTimeout(100);
  }
  throw new Error(`waited for ${atLeast} syncs and saw ${await syncs(page)}`);
}

test('opening the window fetches mail without being asked', async () => {
  const { page, context, problems } = await openWindow();
  await untilSyncs(page, 1);
  assert.deepEqual(problems, []);
  await context.close();
});

test('mail keeps being fetched while the window is open', async () => {
  const { page, context, problems } = await openWindow({ every: 300 });
  // More than the one at startup: the timer is what this is about.
  await untilSyncs(page, 3);
  assert.deepEqual(problems, []);
  await context.close();
});

test('an account row syncs that account, and says when it last did', async () => {
  const { page, context, problems } = await openWindow();
  await untilSyncs(page, 1);
  const before = await syncs(page);

  const row = page.locator('#accounts .nav-row', { hasText: 'you@example.com' });
  const button = row.locator('.nav-action');
  // Hidden until the row is under the pointer, so it is not a column of
  // buttons; hovering is what a person does before clicking one.
  assert.equal(await button.isVisible(), true, 'the button is in the row');
  await row.hover();
  await button.click();

  await untilSyncs(page, before + 1);
  // The tooltip is the one place the window says when mail last arrived.
  await page.waitForFunction(
    () => document.querySelector('#accounts .nav-action')?.title?.startsWith('Last synced'),
  );
  assert.deepEqual(problems, []);
  await context.close();
});

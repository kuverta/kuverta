/**
 * The triage surface, in a real browser.
 *
 * `test/view.test.js` drives the same surface in jsdom, which has no layout and
 * no CSS cascade: it can say an element is marked hidden and cannot say it is
 * off screen. These tests load the page `fuckmail` actually mounts —
 * `hosts/fuckmail/ui/triage.html`, over HTTP, since ES modules will not load
 * from a file — into Chromium, with a stand-in for the Tauri bridge, and ask
 * what a person would see.
 *
 * Kept out of `npm test` on purpose: they need a browser download and take
 * seconds rather than milliseconds. `npm run e2e`.
 */

import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import http from 'node:http';
import { readFile } from 'node:fs/promises';
import { dirname, extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { chromium } from 'playwright';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const PAGE = '/hosts/fuckmail/ui/triage.html';

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json',
};

let browser;
let server;
let base;

/** A static server over this directory, refusing anything outside it. */
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

/**
 * The page, with a stand-in Tauri bridge.
 *
 * The bridge is the same double the adapter's unit tests use, loaded into the
 * page by URL, so the browser tests and the Node tests cannot drift apart on
 * what a command returns.
 */
async function open({ count = 0, paper = false, seen = true, width = 1200, height = 800 } = {}) {
  const context = await browser.newContext({ viewport: { width, height } });
  const page = await context.newPage();
  const problems = [];
  page.on('pageerror', (error) => problems.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') problems.push(message.text());
  });

  await page.addInitScript(
    ({ count, paper, seen }) => {
      if (seen) localStorage.setItem('fuckbird.explained', '1');
      let bridge;
      window.__TAURI__ = {
        core: {
          invoke: async (command, args) => {
            bridge ??= import('/test/support/fake-invoke.js').then((fake) =>
              fake.fakeInvoke({
                ...(count ? { seed: fake.manySeed(count) } : {}),
                ...(paper ? {} : { paper: { paperMailboxes: [], documents: [] } }),
              }),
            );
            return (await bridge)(command, args);
          },
        },
      };
    },
    { count, paper, seen },
  );

  await page.goto(`${base}${PAGE}`);
  await page.waitForFunction(() => document.querySelector('.fb-row .fb-subject')?.textContent.length > 1);
  return { page, context, problems };
}

const selectedSubject = (page) =>
  page.evaluate(() => document.querySelector('.fb-row.fb-selected .fb-subject')?.textContent ?? null);

async function waitForSelected(page, pattern) {
  await page.waitForFunction(
    (source) => new RegExp(source).test(document.querySelector('.fb-row.fb-selected .fb-subject')?.textContent ?? ''),
    pattern.source,
  );
}

// -- what is on screen -------------------------------------------------------

test('first launch puts the explanation on screen, and it goes away for good', async () => {
  const { page, context } = await open({ seen: false });

  assert.equal(await page.locator('#fb-help').isVisible(), true, 'the panel should be on screen');
  assert.match(await page.locator('#fb-help').innerText(), /What this is for/);

  await page.getByRole('button', { name: 'Got it' }).click();
  assert.equal(await page.locator('#fb-help').isVisible(), false);

  await page.reload();
  await page.waitForSelector('.fb-row');
  assert.equal(await page.locator('#fb-help').isVisible(), false, 'dismissed once, dismissed after a reload');
  await context.close();
});

test('the keys are on screen, inside the window rather than below it', async () => {
  const { page, context } = await open();
  const legend = page.locator('#fb-legend');

  assert.equal(await legend.isVisible(), true);
  const box = await legend.boundingBox();
  assert.ok(box.y + box.height <= 800, `legend ends at ${box.y + box.height}px in an 800px window`);
  await context.close();
});

test('hidden things are off screen, not merely marked hidden', async () => {
  // The bug jsdom cannot see: a `display` rule beating the hidden attribute,
  // which leaves an element marked hidden and still drawn.
  const { page, context } = await open();

  assert.equal(await page.locator('#fb-scope-out').isVisible(), false, 'no way out of an unfiltered list');
  assert.equal(await page.locator('#fb-empty-list').isVisible(), false, 'the list is not empty');
  assert.equal(await page.locator('#fb-reading').isVisible(), false, 'nothing is open');
  assert.equal(await page.locator('#fb-help').isVisible(), false, 'already seen');

  await page.locator('#fb-search').fill('nothing matches this');
  await page.locator('#fb-search').press('Enter');
  await page.waitForSelector('#fb-empty-list', { state: 'visible' });
  assert.equal(await page.locator('#fb-scope-out').isVisible(), true);
  await context.close();
});

// -- keys --------------------------------------------------------------------

test('the cursor stays on screen as it moves through a long list', async () => {
  // Needs real element heights, which jsdom reports as zero.
  const { page, context } = await open({ count: 120 });

  for (let i = 0; i < 40; i += 1) await page.keyboard.press('j');
  await waitForSelected(page, /^Message 040/);

  const viewport = await page.locator('#fb-viewport').boundingBox();
  const row = await page.locator('.fb-row.fb-selected').boundingBox();
  assert.ok(row, 'the selected row should be drawn');
  assert.ok(
    row.y >= viewport.y - 1 && row.y + row.height <= viewport.y + viewport.height + 1,
    `row at ${row.y}–${row.y + row.height}px, list shows ${viewport.y}–${viewport.y + viewport.height}px`,
  );
  await context.close();
});

test('archiving keeps the cursor where it was in a real browser too', async () => {
  const { page, context } = await open();
  await page.keyboard.press('j');
  await waitForSelected(page, /^Message 01/);

  await page.keyboard.press('e');
  await waitForSelected(page, /^Message 02/);
  assert.match(await page.locator('#fb-toast').innerText(), /archived/);
  await context.close();
});

test('post sits in the same list as mail', async () => {
  const { page, context } = await open({ paper: true });
  await page.waitForFunction(() =>
    [...document.querySelectorAll('.fb-row .fb-subject')].some((s) => s.textContent.includes('Abschlagszahlung')),
  );
  const subjects = await page.locator('.fb-row:visible .fb-subject').allInnerTexts();
  const post = subjects.findIndex((subject) => subject.includes('Abschlagszahlung'));
  assert.ok(post > 0, `post should be interleaved by date, not first: ${subjects.join(' | ')}`);
  await context.close();
});

test('a narrow window gives up the reading pane and keeps the list', async () => {
  const { page, context } = await open({ width: 700 });
  assert.equal(await page.locator('.fb-detail').isVisible(), false);
  assert.equal(await page.locator('.fb-row').first().isVisible(), true);
  assert.equal(await page.locator('#fb-legend').isVisible(), true);
  await context.close();
});

test('an ordinary session leaves nothing in the console', async () => {
  const { page, context, problems } = await open({ paper: true });
  await page.keyboard.press('j');
  await page.keyboard.press('Enter');
  await page.waitForSelector('#fb-reading', { state: 'visible' });
  await page.keyboard.press('Escape');
  await page.keyboard.press('?');
  await page.keyboard.press('Escape');

  assert.deepEqual(problems, []);
  await context.close();
});

/**
 * The scanner page's preview, in a browser.
 *
 * Preview mode is a page you stand in front of with a letter in one hand and
 * a keyboard under the other: the page large, where it is going beside it,
 * and every key that does anything along the bottom. All of that has to be on
 * the screen at once. It was not — the panes were sized in fractions of the
 * viewport that added up to more than one, so on a laptop the keys were below
 * the fold — and whether it is now is a question about layout, which only an
 * engine that does layout can answer.
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
const PAGE = '/apps/scannerd/src/ui.html';

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


/** A sheet of paper, in the proportions a photographed page comes out in. */
const A4 =
  'data:image/svg+xml;base64,' +
  Buffer.from(
    '<svg xmlns="http://www.w3.org/2000/svg" width="1240" height="1754"><rect width="100%" height="100%" fill="#fff"/></svg>',
  ).toString('base64');

/** The page in preview mode, at a given window size. */
async function preview(width, height) {
  const context = await browser.newContext({ viewport: { width, height } });
  const page = await context.newPage();
  const problems = [];
  page.on('pageerror', (error) => problems.push(error.message));
  page.on('console', (message) => {
    // There is no scanner behind this page here, so its polling 404s. The
    // layout is what is being asked about.
    if (message.type() === 'error' && !/404|Failed to fetch/.test(message.text())) {
      problems.push(message.text());
    }
  });
  await page.goto(`${base}${PAGE}`);
  // Without a scanner to talk to, the status never arrives; the layout does
  // not depend on it, so preview is entered directly — with a page in it, of
  // the shape a photographed sheet has.
  await page.evaluate((sheet) => {
    document.getElementById('preview-page').src = sheet;
    document.body.classList.add('preview');
  }, A4);
  await page.waitForTimeout(120);
  return { page, context, problems };
}

const sizes = [
  [1920, 1080],
  [1536, 864],
  [1366, 768],
  [1280, 720],
];

for (const [width, height] of sizes) {
  test(`preview fits a ${width}x${height} window with nothing below the fold`, async () => {
    const { page, context, problems } = await preview(width, height);
    const seen = await page.evaluate(() => ({
      scrollHeight: document.documentElement.scrollHeight,
      inner: window.innerHeight,
      keys: document.getElementById('preview-keys').getBoundingClientRect().bottom,
      picture: document.getElementById('preview-page').getBoundingClientRect().height,
      text: document.getElementById('preview-text').getBoundingClientRect().height,
    }));

    assert.ok(
      seen.scrollHeight <= seen.inner + 1,
      `the page scrolls: ${seen.scrollHeight} in ${seen.inner}`,
    );
    // The keys along the bottom are the point of preview mode, so they have
    // to be on the screen, and near the bottom of it rather than halfway up.
    assert.ok(seen.keys <= seen.inner, `the keys end at ${seen.keys} of ${seen.inner}`);
    assert.ok(seen.keys > seen.inner * 0.9, `the window is only ${seen.keys} full of ${seen.inner}`);
    // And the room left over goes to the page, not to empty space.
    assert.ok(seen.picture > seen.inner * 0.4, `the page picture is only ${seen.picture} tall`);
    assert.ok(seen.text > 40, `there is no room for what it says: ${seen.text}`);
    assert.deepEqual(problems, []);
    await context.close();
  });
}

test('a section added to the page cannot push the keys off the bottom', async () => {
  const { page, context } = await preview(1440, 900);
  const before = await page.evaluate(
    () => document.getElementById('preview-keys').getBoundingClientRect().bottom,
  );
  // The bug this is here for: sections that are not the preview's were left
  // in the grid and took its height, so preview mode stopped filling the
  // window as soon as one of them had something to say.
  await page.evaluate(() => {
    const section = document.createElement('section');
    section.style.height = '200px';
    section.textContent = 'something new';
    document.querySelector('main').append(section);
  });
  await page.waitForTimeout(60);
  const after = await page.evaluate(() => ({
    keys: document.getElementById('preview-keys').getBoundingClientRect().bottom,
    scrollHeight: document.documentElement.scrollHeight,
    inner: window.innerHeight,
  }));
  assert.equal(Math.round(after.keys), Math.round(before));
  assert.ok(after.scrollHeight <= after.inner + 1, 'the page still does not scroll');
  await context.close();
});

test('a short window goes back to scrolling rather than squeezing the page flat', async () => {
  const { page, context } = await preview(1200, 480);
  const seen = await page.evaluate(() => ({
    picture: document.getElementById('preview-page').getBoundingClientRect().height,
    scrollHeight: document.documentElement.scrollHeight,
    inner: window.innerHeight,
  }));
  assert.ok(seen.scrollHeight > seen.inner, 'it scrolls');
  assert.ok(seen.picture > 200, `the page is still worth looking at: ${seen.picture}`);
  await context.close();
});

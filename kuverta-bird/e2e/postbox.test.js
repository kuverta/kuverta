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

async function openWindow() {
  const context = await browser.newContext({ viewport: { width: 1200, height: 800 } });
  const page = await context.newPage();
  const problems = [];
  page.on('pageerror', (error) => problems.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') problems.push(message.text());
  });

  await page.addInitScript((fake) => {
    // Post is polled for; a test cannot wait twenty seconds each time.
    window.__kuvertaPostPollMs = 300;
    window.__fakeBridge = import(fake).then((module) => module.fakeInvoke());
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
  }, FAKE);

  await page.goto(`${base}${PAGE}`);
  await page.waitForFunction(() => document.getElementById('status')?.textContent === 'you@example.com');
  return { page, context, problems };
}

const postbox = (page) => page.locator('#postboxes .nav-item', { hasText: 'Home' });

/** Waits for the open letter to be shown as the fake vision model read it. */
async function readByVision(page, subject) {
  await page.waitForFunction(
    (want) =>
      document.getElementById('reading-subject')?.textContent === want &&
      document.getElementById('reading-note')?.textContent.includes('fake-vision'),
    subject,
    { timeout: 5000 },
  );
  assert.equal(await page.locator('#reading-transcribe').innerText(), 'Read again');
}

/** A letter that has just come through the scanner: unread, never read by a model. */
function scanned(fields) {
  return {
    mailbox: 1,
    date_utc: Math.floor(Date.UTC(2026, 8, 1, 9, 0, 0) / 1000),
    added_utc: Math.floor(Date.now() / 1000),
    unread: true,
    has_attachments: true,
    category: 'notification',
    snippet: 'Unleserlich',
    tags: [],
    page_count: 1,
    ...fields,
  };
}

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

test('selecting a postbox lists its post like mail, sorted into categories but without folders', async () => {
  const { page, context, problems } = await openWindow();

  await postbox(page).click();
  await page.waitForFunction(() => document.getElementById('status')?.textContent === 'Home');
  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('2 letters to Home'));

  const subjects = await page.locator('#content .row:not([hidden]) .subject').allInnerTexts();
  assert.deepEqual(subjects, ['Ihre Abschlagszahlung für April', 'Bescheid über Einkommensteuer'], 'newest first');
  assert.equal(await postbox(page).evaluate((item) => item.classList.contains('active')), true);
  assert.equal(await page.locator('#folders').isVisible(), false);
  // Post is sorted like mail: the categories are there, counted for this postbox.
  assert.equal(await page.locator('#categories-heading').isVisible(), true);
  await page.locator('#categories .nav-item', { hasText: 'transactional' }).waitFor();
  assert.equal(await page.locator('#categories .nav-item', { hasText: 'transactional' }).locator('.count').innerText(), '2');
  // Unread-only is a filter over mail in the store; post has none.
  assert.equal(await page.locator('#categories .nav-item', { hasText: 'Unread only' }).count(), 0);

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
  // Opening a letter nobody has had read has the vision model read it, and the
  // text says whose reading it is.
  await readByVision(page, 'Bescheid über Einkommensteuer');
  assert.equal(await page.locator('#reading-body').innerText(), 'Transkript: Bescheid über Einkommensteuer');

  assert.deepEqual(problems, []);
  await context.close();
});

test('a letter opens on its text or its scan, and the choice is kept for the next letter', async () => {
  const { page, context, problems } = await openWindow();
  await postbox(page).click();
  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('2 letters'));
  await page.locator('#content .row', { hasText: 'Ihre Abschlagszahlung für April' }).click();
  await page.waitForSelector('#reading', { state: 'visible' });
  await readByVision(page, 'Ihre Abschlagszahlung für April');

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

  // The note on whose reading the text is belongs to the text, not the scan.
  assert.equal(await page.locator('#reading-post').isVisible(), false);

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
  await page.waitForFunction(async () =>
    (await window.__fakeBridge)._documents.find((d) => d.id === 40)?.transcribed_by === 'fake-vision');

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

test('a letter scanned while the postbox is open appears on its own, and the letter being read stays open', async () => {
  // Found on the Pi: a page went to Paperless in eleven seconds and never
  // showed in the window, which only asked when the postbox was opened.
  const { page, context, problems } = await openWindow();
  await postbox(page).click();
  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('2 letters'));
  await page.locator('#content .row', { hasText: 'Bescheid über Einkommensteuer' }).click();
  await page.waitForSelector('#reading', { state: 'visible' });
  await readByVision(page, 'Bescheid über Einkommensteuer');

  await page.evaluate(async () => {
    const invoke = await window.__fakeBridge;
    invoke._documents.push({
      id: 42,
      mailbox: 1,
      date_utc: Math.floor(Date.UTC(2026, 8, 12, 9, 0, 0) / 1000),
      from: 'Hausverwaltung Nord',
      subject: 'Ankündigung Treppenhausreinigung',
      unread: false,
      has_attachments: true,
      category: 'notification',
      snippet: 'Am Montag wird das Treppenhaus gereinigt.',
      tags: ['home'],
      page_count: 1,
    });
  });

  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('3 letters'), null, { timeout: 5000 });
  const subjects = await page.locator('#content .row:not([hidden]) .subject').allInnerTexts();
  assert.equal(subjects[0], 'Ankündigung Treppenhausreinigung', 'new post on top');
  await page.waitForSelector('#toast:not([hidden])');
  assert.match(await page.locator('#toast').innerText(), /1 new letter to Home/);

  // The letter being read was not pulled out from under the reader.
  assert.equal(await page.locator('#reading').isVisible(), true);
  assert.equal(await page.locator('#reading-subject').innerText(), 'Bescheid über Einkommensteuer');
  assert.equal(
    await page.locator('#content .row.selected .subject').innerText(),
    'Bescheid über Einkommensteuer',
  );

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

test('a scanned letter is unread until it is opened, and u makes it unread again', async () => {
  const { page, context, problems } = await openWindow();
  await page.evaluate(async (letter) => {
    (await window.__fakeBridge)._documents.push(letter);
  }, scanned({ id: 43, from: 'Hausverwaltung Nord', subject: 'Wasserablesung' }));

  await postbox(page).click();
  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('3 letters'));
  const row = page.locator('#content .row', { hasText: 'Wasserablesung' });
  assert.equal(await row.locator('.dot').evaluate((dot) => dot.classList.contains('on')), true);
  // The postbox says how much post is unread, as a mail folder does.
  await page.waitForFunction(() => document.querySelector('#postboxes .count.unread')?.textContent === '1');

  // New post is read by the vision model as soon as it is seen, before anyone opens it.
  await page.waitForFunction(async () =>
    (await window.__fakeBridge)._documents.find((d) => d.id === 43)?.transcribed_by === 'fake-vision');

  await row.click();
  await page.waitForSelector('#reading', { state: 'visible' });
  await page.waitForFunction(() => !document.querySelector('#postboxes .count.unread'));
  assert.equal(await row.locator('.dot').evaluate((dot) => dot.classList.contains('on')), false);
  assert.equal(await page.locator('#reading-body').innerText(), 'Transkript: Wasserablesung');

  await page.keyboard.press('u');
  await page.waitForFunction(() => document.querySelector('#postboxes .count.unread')?.textContent === '1');
  assert.equal(await row.locator('.dot').evaluate((dot) => dot.classList.contains('on')), true);
  assert.equal(
    await page.evaluate(async () => (await window.__fakeBridge)._documents.find((d) => d.id === 43).unread),
    true,
    'kept, not just drawn',
  );

  assert.deepEqual(problems, []);
  await context.close();
});

test('post is listed by the date on the letter or by when it was scanned, and the choice is kept', async () => {
  const { page, context, problems } = await openWindow();
  await postbox(page).click();
  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('2 letters'));
  const subjects = () => page.locator('#content .row:not([hidden]) .subject').allInnerTexts();
  const order = () => page.locator('#scope .order button.active').innerText();

  assert.equal(await order(), 'letter date');
  assert.deepEqual(await subjects(), ['Ihre Abschlagszahlung für April', 'Bescheid über Einkommensteuer']);

  // The tax letter is older but came through the scanner last.
  await page.locator('#scope .order button', { hasText: 'scanned' }).click();
  await page.waitForFunction(() => document.querySelector('#scope .order .active')?.textContent === 'scanned');
  await page.waitForFunction(
    () => document.querySelector('#content .row:not([hidden]) .subject')?.textContent === 'Bescheid über Einkommensteuer',
  );
  assert.deepEqual(await subjects(), ['Bescheid über Einkommensteuer', 'Ihre Abschlagszahlung für April']);

  await page.reload();
  await page.waitForFunction(() => document.getElementById('status')?.textContent === 'you@example.com');
  await postbox(page).click();
  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('2 letters'));
  assert.equal(await order(), 'scanned');
  assert.deepEqual(await subjects(), ['Bescheid über Einkommensteuer', 'Ihre Abschlagszahlung für April']);

  assert.deepEqual(problems, []);
  await context.close();
});

test('a category narrows the postbox to the post sorted into it', async () => {
  const { page, context, problems } = await openWindow();
  await page.evaluate(async (letter) => {
    const invoke = await window.__fakeBridge;
    invoke._documents.push(letter);
    // Read already, so nothing is sent to the vision model under the test.
    for (const d of invoke._documents) {
      d.unread = false;
      d.transcribed_by = 'fake-vision';
    }
  }, scanned({ id: 44, from: 'Hausverwaltung Nord', subject: 'Treppenhausreinigung' }));

  await postbox(page).click();
  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('3 letters'));
  const notification = page.locator('#categories .nav-item', { hasText: 'notification' });
  await notification.waitFor();
  assert.equal(await notification.locator('.count').innerText(), '1');

  await notification.click();
  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('1 in notification'));
  assert.deepEqual(await page.locator('#content .row:not([hidden]) .subject').allInnerTexts(), ['Treppenhausreinigung']);

  await page.locator('#scope button', { hasText: 'clear' }).click();
  await page.waitForFunction(() => document.getElementById('scope')?.textContent.includes('3 letters'));

  assert.deepEqual(problems, []);
  await context.close();
});

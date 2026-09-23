/**
 * Attachments and the assistant's found-message card, in the desktop app's
 * own window, in a real browser.
 *
 * The question these answer is the one that started them: "where is the eSIM
 * they sent me?" — the assistant finds the mail and shows it as a card, and
 * the QR code in it opens on screen, where a phone can scan it. A browser is
 * needed for the part jsdom cannot do: decoding the picture.
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
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  const problems = [];
  page.on('pageerror', (error) => problems.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') problems.push(message.text());
  });

  await page.addInitScript((fake) => {
    try {
      localStorage.clear();
    } catch {
      // A fresh context has nothing to clear.
    }
    window.__fakeBridge = import(fake).then((module) =>
      module.fakeInvoke({
        seed: [...module.defaultSeed(), module.esimMessage()],
        paper: { paperMailboxes: [], documents: [] },
      }),
    );
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

const log = (page) => page.evaluate(async () => (await window.__fakeBridge)('attachment_log', {}));

test('an open message lists its attachments, and a picture shows in the viewer', async () => {
  const { page, context, problems } = await openWindow();

  await page.locator('#content .row', { hasText: 'Ihre neue e-SIM ist da' }).click();
  await page.waitForSelector('#reading-attachments:not([hidden])');
  const chips = page.locator('#reading-attachments .attachment-chip');
  // The logo in the text is not listed until asked for.
  assert.deepEqual(await chips.locator('.name').allInnerTexts(), ['eSIM QR.png', 'Vertrag.pdf', 'login.html']);
  await page.locator('#reading-attachments .attachment-more').click();
  assert.equal(await chips.count(), 4);

  await chips.filter({ hasText: 'eSIM QR.png' }).click();
  await page.waitForSelector('#attachment-sheet:not([hidden])');
  assert.equal(await page.locator('#attachment-title').innerText(), 'eSIM QR.png');
  // Decoded, not just pointed at: the picture has a size.
  await page.waitForFunction(() => {
    const image = document.getElementById('attachment-image');
    return !image.hidden && image.complete && image.naturalWidth === 1;
  });
  assert.equal(await page.locator('#attachment-open').isDisabled(), false);

  await page.locator('#attachment-save').click();
  await page.waitForSelector('#toast:not([hidden])');
  assert.match(await page.locator('#toast').innerText(), /saved to .*Downloads\/eSIM QR\.png/);
  await page.locator('#attachment-open').click();
  assert.deepEqual(await log(page), { saved: ['eSIM QR.png'], opened: ['eSIM QR.png'] });

  // Escape closes it and lets the picture go.
  await page.keyboard.press('Escape');
  await page.waitForSelector('#attachment-sheet', { state: 'hidden' });
  assert.equal(await page.locator('#attachment-image').getAttribute('src'), null);

  assert.deepEqual(problems, []);
  await context.close();
});

test('a file that could run something can be saved but not opened', async () => {
  const { page, context, problems } = await openWindow();

  await page.locator('#content .row', { hasText: 'Ihre neue e-SIM ist da' }).click();
  const risky = page.locator('#reading-attachments .attachment-chip', { hasText: 'login.html' });
  await risky.waitFor();
  assert.match(await risky.getAttribute('class'), /risky/);
  await risky.click();
  await page.waitForSelector('#attachment-sheet:not([hidden])');
  assert.equal(await page.locator('#attachment-open').isDisabled(), true);
  assert.match(await page.locator('#attachment-sub').innerText(), /could run something/);
  assert.match(await page.locator('#attachment-status').innerText(), /can't show this kind of file/);
  await page.locator('#attachment-close').click();
  await page.waitForSelector('#attachment-sheet', { state: 'hidden' });

  // Mail without attachments has no strip.
  await page.locator('#content .row', { hasText: 'Message 00' }).click();
  await page.waitForFunction(() => document.getElementById('reading-subject')?.textContent.includes('Message 00'));
  assert.equal(await page.locator('#reading-attachments').isHidden(), true);

  assert.deepEqual(problems, []);
  await context.close();
});

test('a message you pick is handed to the assistant, which drafts from it', async () => {
  const { page, context, problems } = await openWindow();

  // From the open message: the button hands it over and opens the chat.
  await page.locator('#content .row', { hasText: 'Ihre neue e-SIM ist da' }).click();
  await page.locator('#reading-actions [data-act=ask]').click();
  await page.waitForSelector('#assistant:not([hidden])');
  const about = page.locator('#chat-about .about-chip');
  assert.equal(await about.locator('.name').innerText(), 'Ihre neue e-SIM ist da');

  // The suggestions become things to do with it.
  assert.ok(
    (await page.locator('.chat-suggestions button').allInnerTexts()).includes('Reply and confirm.'),
  );

  await page.locator('#chat-input').fill('Reply and confirm the appointment.');
  await page.locator('#chat-send').click();

  // Asked with the message attached, and the bar empties once it is asked.
  const draft = page.locator('#chat-log .chat-card', { hasText: 'Reply to' });
  await draft.waitFor();
  assert.equal(await page.locator('#chat-about').isHidden(), true);
  assert.match(await page.locator('#chat-log .chat-activity').last().innerText(), /about “Ihre neue e-SIM ist da”/);
  assert.match(await draft.locator('textarea').inputValue(), /1 message\(s\) read/);

  // Picking a block with the keyboard hands all of it over — and clicking
  // back into the list takes the keys back from the chat input.
  await page.locator('#content .row', { hasText: 'Message 00' }).click();
  await page.keyboard.press('J');
  await page.keyboard.press('I');
  await page.waitForFunction(() => document.querySelectorAll('#chat-about .about-chip').length === 2);
  // One can be left out again.
  await page.locator('#chat-about .about-chip .remove').first().click();
  assert.equal(await about.count(), 1);

  assert.deepEqual(problems, []);
  await context.close();
});

test('the assistant can draft a new message to someone else about what it read', async () => {
  const { page, context, problems } = await openWindow();

  await page.locator('#content .row', { hasText: 'Ihre neue e-SIM ist da' }).click();
  await page.locator('#reading-actions [data-act=ask]').click();
  await page.waitForSelector('#chat-about:not([hidden])');
  await page.locator('#chat-input').fill('Write a summary for clara@example.com so I can send it.');
  await page.locator('#chat-send').click();

  const draft = page.locator('#chat-log .chat-card', { hasText: 'New message to clara@example.com' });
  await draft.waitFor();
  // Into compose, addressed and ready, rather than sent behind your back.
  await draft.locator('button', { hasText: 'Open in compose' }).click();
  await page.waitForFunction(() => !document.getElementById('compose').hidden);
  assert.equal(await page.locator('#compose-to').inputValue(), 'clara@example.com');
  assert.match(await page.locator('#compose-body').inputValue(), /e-SIM/);

  assert.deepEqual(problems, []);
  await context.close();
});

test('the assistant shows the mail it found as a card, attachments and all', async () => {
  const { page, context, problems } = await openWindow();

  await page.locator('#assistant-toggle').click();
  await page.waitForSelector('#assistant:not([hidden])');
  await page.locator('#chat-input').fill('I got an eSIM by mail a while ago — where is it?');
  await page.locator('#chat-send').click();

  const card = page.locator('#chat-log .chat-card.found');
  await card.waitFor();
  assert.equal(await card.locator('.card-title').innerText(), 'Ihre neue e-SIM ist da');
  assert.match(await card.locator('.card-note').innerText(), /eSIM QR\.png/);
  assert.deepEqual(await card.locator('.attachment-chip .name').allInnerTexts(), [
    'eSIM QR.png',
    'Vertrag.pdf',
    'login.html',
  ]);

  // The answer's Markdown is shown as such, and what it quotes stays text.
  const answer = page.locator('#chat-log .chat-msg.assistant').last();
  assert.equal(await answer.locator('strong').first().innerText(), 'Found:');
  assert.equal(await answer.locator('ol > li').count(), 2);
  assert.equal(await answer.locator('em').innerText(), 'attached');
  assert.equal(await answer.locator('img').count(), 0);
  assert.match(await answer.locator('ol > li').first().innerText(), /<img src=x onerror=/);
  assert.equal(await page.evaluate(() => window.pwned), undefined);

  // Straight to the QR code, from the chat.
  await card.locator('.attachment-chip', { hasText: 'eSIM QR.png' }).click();
  await page.waitForFunction(() => document.getElementById('attachment-image')?.naturalWidth === 1);
  await page.keyboard.press('Escape');
  await page.waitForSelector('#attachment-sheet', { state: 'hidden' });

  // And to the whole message.
  await card.locator('button', { hasText: 'Open message' }).click();
  await page.waitForFunction(
    () => document.getElementById('reading-subject')?.textContent === 'Ihre neue e-SIM ist da',
  );
  assert.equal(await page.locator('#reading-attachments .attachment-chip').count(), 3);

  assert.deepEqual(problems, []);
  await context.close();
});

/**
 * Files dropped on what is being written, and the assistant writing it, in
 * the desktop app's own window, in a real browser.
 *
 * A browser because jsdom has no drag and drop worth the name and no
 * FileReader that reads: the questions here are whether a file let go of over
 * compose ends up in the message that is sent, bytes and all, and whether one
 * click on Answer puts a reply in compose that ⌘Z can take back.
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

async function openWindow({ contacts = [] } = {}) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  const problems = [];
  page.on('pageerror', (error) => problems.push(error.message));
  // The app's webview answers a native confirm() "no" without showing it, so
  // one here is a question the person would never have been asked.
  page.on('dialog', async (dialog) => {
    problems.push(`a native ${dialog.type()} dialog: ${dialog.message()}`);
    await dialog.dismiss();
  });
  page.on('console', (message) => {
    if (message.type() === 'error') problems.push(message.text());
  });

  await page.addInitScript(({ fake, contacts }) => {
    try {
      localStorage.clear();
    } catch {
      // A fresh context has nothing to clear.
    }
    window.__fakeBridge = import(fake).then((module) =>
      module.fakeInvoke({ seed: module.defaultSeed(), paper: { paperMailboxes: [], documents: [] }, contacts }),
    );
    window.__TAURI__ = {
      core: {
        invoke: async (command, args) => (await window.__fakeBridge)(command, args),
        Channel: class {
          onmessage = null;
        },
      },
    };
  }, { fake: FAKE, contacts });

  await page.goto(`${base}${PAGE}`);
  await page.waitForFunction(() => document.getElementById('status')?.textContent === 'you@example.com');
  return { page, context, problems };
}

const outgoing = (page) => page.evaluate(async () => (await window.__fakeBridge)('outgoing_log', {}));

/** Drags `files` over `selector` and lets go, as Finder would. */
async function dropFiles(page, selector, files, { letGo = true } = {}) {
  await page.evaluate(
    ({ selector, files, letGo }) => {
      const transfer = new DataTransfer();
      for (const { name, type, text } of files) transfer.items.add(new File([text], name, { type }));
      const target = document.querySelector(selector);
      const fire = (type) =>
        target.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer: transfer }));
      fire('dragenter');
      fire('dragover');
      if (letGo) fire('drop');
    },
    { selector, files, letGo },
  );
}

test('files dropped on compose go out with the message, and only with the message', async () => {
  const { page, context, problems } = await openWindow();

  await page.locator('#new-message').click();
  await page.waitForSelector('#compose:not([hidden])');
  assert.equal(await page.locator('#compose-files').isHidden(), true);

  // Held over compose, it says where the file would go.
  await dropFiles(page, '#compose-body', [{ name: 'x.txt', type: 'text/plain', text: 'x' }], { letGo: false });
  assert.equal(await page.locator('#compose .drop-hint').isVisible(), true);
  assert.match(await page.locator('#compose').getAttribute('class'), /dropping/);

  await dropFiles(page, '#compose-body', [
    { name: 'Rechnung März.pdf', type: 'application/pdf', text: '%PDF-1.4 Rechnung' },
    { name: 'notiz.txt', type: 'text/plain', text: 'Grüße' },
  ]);
  const chips = page.locator('#compose-files .attachment-chip');
  await chips.first().waitFor();
  assert.deepEqual(await chips.locator('.name').allInnerTexts(), ['Rechnung März.pdf', 'notiz.txt']);
  assert.equal(await page.locator('#compose .drop-hint').isHidden(), true);

  // One taken back out again.
  await chips.filter({ hasText: 'notiz.txt' }).locator('.remove').click();
  assert.deepEqual(await chips.locator('.name').allInnerTexts(), ['Rechnung März.pdf']);

  // One more, picked rather than dropped.
  const chooser = page.waitForEvent('filechooser');
  await page.locator('#compose-attach').click();
  await (await chooser).setFiles({ name: 'plan.csv', mimeType: 'text/csv', buffer: Buffer.from('a;b\n1;2') });
  await page.waitForFunction(() => document.querySelectorAll('#compose-files .attachment-chip').length === 2);

  await page.locator('#compose-to').fill('jane@example.com');
  await page.locator('#compose-to').dispatchEvent('change');
  await page.locator('#compose-subject').fill('Die Rechnung');
  await page.locator('#compose-body').fill('anbei');
  await page.locator('#compose-send').click();
  await page.waitForSelector('#compose', { state: 'hidden' });

  const log = await outgoing(page);
  assert.equal(log.sent.length, 1);
  const files = log.sent[0].attachments;
  assert.deepEqual(
    files.map((f) => [f.name, f.content_type, Buffer.from(f.data, 'base64').toString('utf8')]),
    [
      ['Rechnung März.pdf', 'application/pdf', '%PDF-1.4 Rechnung'],
      ['plan.csv', 'text/csv', 'a;b\n1;2'],
    ],
  );
  // The envelope previews run on every change of recipient; they never carry
  // the files back and forth.
  assert.ok(log.previews.length > 0);
  assert.ok(log.previews.every((draft) => draft.attachments.length === 0));

  // Compose opened again starts with none.
  await page.locator('#new-message').click();
  assert.equal(await page.locator('#compose-files').isHidden(), true);

  assert.deepEqual(problems, []);
  await context.close();
});

test('one click on Answer writes the reply, from the notes and the files, and ⌘Z takes it back', async () => {
  const { page, context, problems } = await openWindow();

  // From the open message: the reply opens and is written, in one click.
  await page.locator('#content .row', { hasText: 'Message 00' }).click();
  await page.locator('#reading-actions [data-act=answer]').click();
  await page.waitForSelector('#compose:not([hidden])');
  await page.waitForFunction(() => document.getElementById('compose-body').value.includes('Viele Grüße'));
  const body = page.locator('#compose-body');
  assert.match(await body.inputValue(), /danke für „Message 00 about Rechnung 1000“/);
  assert.match(await page.locator('#compose-answer-status').innerText(), /Written by llama3\.2:3b/);
  assert.match(await page.locator('#compose-answer').innerText(), /Answer with assistant/);
  let log = await outgoing(page);
  assert.equal(log.answered.length, 1);
  assert.equal(typeof log.answered[0].reply_to, 'number');
  assert.equal(log.answered[0].body, '');

  // In compose, what is typed is the brief, and the files are read with it.
  await page.locator('#compose-cancel').click();
  await page.keyboard.press('a');
  await page.waitForFunction(() => document.getElementById('compose-body').value.includes('Viele Grüße'));
  await body.fill('zusagen, aber erst ab Mittwoch');
  await dropFiles(page, '#compose', [{ name: 'Formular.pdf', type: 'application/pdf', text: '%PDF' }]);
  await page.locator('#compose-files .attachment-chip').waitFor();
  await page.locator('#compose-answer').click();
  await page.waitForFunction(() => document.getElementById('compose-body').value.includes('Anbei: Formular.pdf'));
  assert.match(await body.inputValue(), /Wie gewünscht: zusagen, aber erst ab Mittwoch/);
  assert.match(await page.locator('#compose-answer-status').innerText(), /brings back your notes/);
  log = await outgoing(page);
  const asked = log.answered.at(-1);
  assert.equal(asked.body, 'zusagen, aber erst ab Mittwoch');
  assert.deepEqual(asked.attachments.map((f) => f.name), ['Formular.pdf']);

  // The notes come back with the field's own undo.
  await body.focus();
  await page.keyboard.press('ControlOrMeta+z');
  assert.equal(await body.inputValue(), 'zusagen, aber erst ab Mittwoch');

  assert.deepEqual(problems, []);
  await context.close();
});

test('a new message is written, not answered, and the button says so', async () => {
  const { page, context, problems } = await openWindow();

  await page.locator('#new-message').click();
  await page.waitForSelector('#compose:not([hidden])');
  assert.match(await page.locator('#compose-answer').innerText(), /Write with assistant/);
  await page.locator('#compose-to').fill('jane@example.com');
  await page.locator('#compose-body').fill('Termin nächste Woche vorschlagen');
  await page.locator('#compose-body').press('ControlOrMeta+j');
  await page.waitForFunction(() => document.getElementById('compose-body').value.startsWith('Hallo,'));
  const asked = (await outgoing(page)).answered[0];
  assert.equal(asked.reply_to, null);
  assert.deepEqual(asked.to, ['jane@example.com']);

  assert.deepEqual(problems, []);
  await context.close();
});

test('typing in To suggests who the mail is with, and takes one with Enter', async () => {
  const { page, context, problems } = await openWindow({
    contacts: [
      { address: 'erika@example.de', name: 'Mustermann, Erika', sent: 12, received: 3, last_utc: 1 },
      { address: 'eric@example.org', name: null, sent: 1, received: 0, last_utc: 1 },
    ],
  });

  await page.locator('#new-message').click();
  await page.waitForSelector('#compose:not([hidden])');
  const to = page.locator('#compose-to');
  const list = page.locator('#compose .address-suggestions').first();
  const names = () => list.locator('.address-suggestion .name').allInnerTexts();

  // A word of the name, as typed: most likely first.
  await to.pressSequentially('eri');
  await list.waitFor();
  assert.deepEqual(await names(), ['Mustermann, Erika', 'eric@example.org']);
  // The name found by its surname too, and by the domain.
  await to.fill('');
  await to.pressSequentially('muster');
  await page.waitForFunction(() => document.querySelectorAll('#compose .address-suggestion').length === 1);
  await to.fill('');
  await to.pressSequentially('example.org');
  await page.waitForFunction(
    () => document.querySelector('#compose .address-suggestion .name')?.textContent === 'eric@example.org',
  );

  // ↓ Enter takes the second; Escape closes the list and leaves the message.
  await to.fill('');
  await to.pressSequentially('eri');
  await list.waitFor();
  await to.press('ArrowDown');
  await to.press('Enter');
  assert.equal(await to.inputValue(), 'eric@example.org, ');
  assert.equal(await list.isHidden(), true);
  // The next one after a comma: the name with a comma in it is quoted, and
  // who is already there is not offered again.
  await to.pressSequentially('e');
  await list.waitFor();
  const offered = await names();
  assert.equal(offered[0], 'Mustermann, Erika');
  assert.ok(!offered.includes('eric@example.org'), `${offered}`);
  await to.press('Escape');
  assert.equal(await list.isHidden(), true);
  assert.equal(await page.locator('#compose').isVisible(), true, 'Escape closed the list, not compose');
  await to.pressSequentially('r');
  await list.waitFor();
  await to.press('Tab');
  assert.equal(await to.inputValue(), 'eric@example.org, "Mustermann, Erika" <erika@example.de>, ');

  // And both reach the core as two recipients, not three.
  await page.locator('#compose-subject').fill('Hallo');
  await page.locator('#compose-send').click();
  await page.waitForSelector('#compose', { state: 'hidden' });
  const sent = (await page.evaluate(async () => (await window.__fakeBridge)('outgoing_log', {}))).sent[0];
  assert.deepEqual(sent.to, ['eric@example.org', '"Mustermann, Erika" <erika@example.de>']);
  // The book was asked for once, when compose opened.
  assert.equal(await page.evaluate(async () => (await window.__fakeBridge)('address_book_asked', {})), 1);

  assert.deepEqual(problems, []);
  await context.close();
});

test('a sender from the seeded mail is suggested by the start of their address', async () => {
  const { page, context, problems } = await openWindow();
  await page.locator('#new-message').click();
  await page.waitForSelector('#compose:not([hidden])');
  await page.locator('#compose-cc').pressSequentially('sender3');
  const list = page.locator('#compose-cc + .address-suggestions');
  await list.waitFor();
  assert.equal(await list.locator('.address-suggestion .address').first().innerText(), 'sender3@example.com');
  await page.locator('#compose-cc').press('Enter');
  assert.equal(await page.locator('#compose-cc').inputValue(), 'Sender 3 <sender3@example.com>, ');
  assert.deepEqual(problems, []);
  await context.close();
});

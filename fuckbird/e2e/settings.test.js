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
  assert.equal(await page.locator('#ai-form').isVisible(), false, 'the models form must be off screen');
  assert.equal(await page.locator('#provider-form').isVisible(), false, 'the provider form must be off screen');
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

// -- models ------------------------------------------------------------------

const noteOf = (page, task) => page.locator(`[data-note="${task}"]`);

test('the models page says which model each job uses and what the provider has', async () => {
  const { page, context, problems } = await openSettings();
  await page.locator('#settings-models').click();
  await page.waitForSelector('#ai-form', { state: 'visible' });
  assert.equal(await page.locator('#settings-form').isVisible(), false, 'one form at a time');

  assert.equal(await page.locator('#ai-form select[name="vision_provider"] option:checked').innerText(), 'Ollama on this computer');
  assert.equal(await page.locator('#ai-form input[name="vision_model"]').inputValue(), 'qwen2.5vl:3b');
  assert.equal(await page.locator('#ai-form input[name="chat_model"]').inputValue(), 'llama3.2:3b');

  await page.waitForFunction(() => document.querySelectorAll('#vision-models option').length === 2);
  const offered = await page.locator('#vision-models option').evaluateAll((options) => options.map((o) => [o.value, o.label]));
  assert.deepEqual(
    offered,
    [
      ['llama3.2:3b', 'text only · 2.0 GB'],
      ['qwen2.5vl:3b', 'sees images · 3.2 GB'],
    ],
    'an embedding model is no answer to either job',
  );
  assert.equal(await noteOf(page, 'vision').textContent(), '', 'a local model that sees needs no note');

  // A model that cannot see is no use for scans, and the page says so.
  await page.locator('#ai-form input[name="vision_model"]').fill('llama3.2:3b');
  await page.waitForFunction(() => document.querySelector('[data-note="vision"]').textContent.includes('cannot see images'));

  // One that is not pulled says how to get it.
  await page.locator('#ai-form input[name="vision_model"]').fill('qwen2.5vl:7b');
  await page.waitForFunction(() => document.querySelector('[data-note="vision"]').textContent.includes('ollama pull qwen2.5vl:7b'));

  await page.locator('#ai-try').click();
  await page.waitForFunction(() => document.getElementById('settings-report')?.textContent.includes('sorting mail'));
  const report = await page.locator('#settings-report').innerText();
  assert.match(report, /reading scans — qwen2\.5vl:7b at Ollama on this computer/);
  assert.match(report, /ok: read the sample page/);

  assert.deepEqual(problems, []);
  await context.close();
});

test('a hosted service is added with its key, lists its models, and a job sent to it says what leaves the computer', async () => {
  const { page, context, problems } = await openSettings();

  await page.locator('#settings-add-provider').click();
  await page.waitForSelector('#provider-form', { state: 'visible' });
  assert.equal(await page.locator('#settings-form').isVisible(), false, 'one form at a time');
  assert.equal(await page.locator('#provider-form input[name="label"]').inputValue(), 'DeepSeek');
  assert.equal(await page.locator('#provider-form input[name="base_url"]').inputValue(), 'https://api.deepseek.com');
  assert.equal(await page.locator('#provider-key').isVisible(), true);
  assert.equal(await page.locator('#provider-delete').isVisible(), false, 'nothing to remove yet');

  // Choosing another service fills in its address.
  await page.locator('#provider-form select[name="preset"]').selectOption('openrouter');
  assert.equal(await page.locator('#provider-form input[name="base_url"]').inputValue(), 'https://openrouter.ai/api/v1');
  await page.locator('#provider-form select[name="preset"]').selectOption('deepseek');

  await page.locator('#provider-form input[name="key"]').fill('sk-test');
  await page.locator('#provider-save').click();
  await page.waitForFunction(() => document.getElementById('provider-models-status')?.textContent === '2 models');
  const rows = await page.locator('#provider-models tbody tr').evaluateAll((trs) => trs.map((tr) => [...tr.cells].map((td) => td.textContent)));
  assert.deepEqual(rows, [
    ['deepseek-chat', 'not said', ''],
    ['deepseek-reasoner', 'not said', ''],
  ]);
  assert.match(await page.locator('#settings-list').innerText(), /DeepSeek/);
  assert.match(await page.locator('#provider-key-state').innerText(), /A key is stored in the keychain/);
  assert.equal(await page.locator('#provider-form input[name="key"]').inputValue(), '', 'the key is never shown again');
  assert.equal(await page.locator('#provider-preset').isVisible(), false, 'a saved provider is edited by its address');

  // Sorting mail on DeepSeek: the page says what that sends.
  await page.locator('#settings-models').click();
  await page.waitForSelector('#ai-form', { state: 'visible' });
  await page.locator('#ai-form select[name="chat_provider"]').selectOption({ label: 'DeepSeek' });
  await page.locator('#ai-form input[name="chat_model"]').fill('deepseek-chat');
  await page.waitForFunction(() => document.querySelector('[data-note="chat"]').textContent.includes('will be sent to DeepSeek'));
  assert.equal(await noteOf(page, 'chat').evaluate((note) => note.classList.contains('warn')), true);

  // Reading scans there too: DeepSeek does not say whether its model can see.
  await page.locator('#ai-form select[name="vision_provider"]').selectOption({ label: 'DeepSeek' });
  await page.locator('#ai-form input[name="vision_model"]').fill('deepseek-chat');
  await page.waitForFunction(() => document.querySelector('[data-note="vision"]').textContent.includes('does not say whether deepseek-chat can see images'));
  assert.match(await noteOf(page, 'vision').textContent(), /Scans of your letters will be sent to DeepSeek/);

  await page.locator('#ai-save').click();
  await page.waitForSelector('#toast:not([hidden])');

  // Back from elsewhere, the page shows what was kept rather than what was typed.
  await page.locator('#settings-add').click();
  await page.locator('#settings-models').click();
  await page.waitForFunction(() => document.querySelector('#ai-form select[name="chat_provider"] option:checked')?.textContent === 'DeepSeek');
  assert.equal(await page.locator('#ai-form input[name="chat_model"]').inputValue(), 'deepseek-chat');

  assert.deepEqual(problems, []);
  await context.close();
});

test('the Ollama on this computer needs no key and cannot be removed', async () => {
  const { page, context, problems } = await openSettings();
  await page.locator('#settings-list .nav-item', { hasText: 'Ollama on this computer' }).click();
  await page.waitForSelector('#provider-form', { state: 'visible' });

  assert.equal(await page.locator('#provider-key').isVisible(), false);
  assert.equal(await page.locator('#provider-key-state').innerText(), 'Ollama needs no key.');
  assert.equal(await page.locator('#provider-delete').isVisible(), false);
  await page.waitForFunction(() => document.getElementById('provider-models-status')?.textContent === '3 models');
  const sees = await page.locator('#provider-models tbody tr').evaluateAll((trs) => trs.map((tr) => tr.cells[1].textContent));
  assert.deepEqual(sees, ['no', 'embeddings only', 'yes']);

  assert.deepEqual(problems, []);
  await context.close();
});

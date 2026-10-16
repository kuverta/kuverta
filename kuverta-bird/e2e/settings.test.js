/**
 * The desktop app's settings sheet, in a real browser.
 *
 * This is `kuverta`'s own window rather than the shared surface, and it lives
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

// The repository root: the window's page is under apps/, the double under kuverta-bird/.
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

async function openSettings({ hostedModels, holdModels = false, modelError } = {}) {
  // Assertions use English regardless of the Windows account's language.
  const context = await browser.newContext({ viewport: { width: 1200, height: 800 }, locale: 'en-US' });
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

  await page.addInitScript(({ fake, hostedModels, holdModels, modelError }) => {
    let bridge;
    window.modelRequests = [];
    window.__TAURI__ = {
      core: {
        invoke: async (command, args) => {
          bridge ??= import(fake).then((module) =>
            module.fakeInvoke({ paper: { paperMailboxes: [], documents: [] } }),
          );
          if (command === 'ai_models' && args.providerId !== 1) {
            if (holdModels) {
              return new Promise((resolve) => window.modelRequests.push(resolve));
            }
            if (modelError) throw new Error(modelError);
            if (hostedModels) return hostedModels;
          }
          return (await bridge)(command, args);
        },
        // Progress arrives on a channel in the real bridge, and the window
        // makes one for every sync, so without this nothing syncs here.
        Channel: class {
          onmessage = null;
        },
      },
    };
  }, { fake: FAKE, hostedModels, holdModels, modelError });

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

const CLAUDE_MODELS = [
  { name: 'claude-haiku-4-5', size_bytes: null, vision: true, embedding: false },
  { name: 'claude-sonnet-4-5', size_bytes: null, vision: true, embedding: false },
];

async function addClaude(page) {
  await page.evaluate(async () => {
    const { invoke } = window.__TAURI__.core;
    const id = await invoke('save_ai_provider', { input: {
      id: null, kind: 'anthropic', label: 'Claude', base_url: 'https://api.anthropic.com/v1',
    } });
    await invoke('set_ai_key', { id, key: 'test-key-never-sent' });
  });
  await page.locator('#settings-models').click();
  await page.waitForFunction(() => document.querySelector('select[name="chat_provider"]')?.options.length === 2);
}

test('changing provider replaces the old model for every job and keeps the choices after saving', async (t) => {
  const { page, context, problems } = await openSettings({ hostedModels: CLAUDE_MODELS });
  t.after(() => context.close());
  await addClaude(page);
  for (const task of ['vision', 'chat', 'assistant']) {
    await page.locator(`select[name="${task}_provider"]`).selectOption({ label: 'Claude' });
    await page.waitForFunction((task) => document.querySelector(`#${task}-models option`)?.value.startsWith('claude-'), task);
    assert.equal(await page.locator(`input[name="${task}_model"]`).inputValue(), 'claude-haiku-4-5');
    assert.doesNotMatch(await noteOf(page, task).textContent(), /does not list/);
  }
  // A deliberate choice wins over the initial suggestion, including on reopening.
  await page.locator('#assistant-models').selectOption('claude-sonnet-4-5');
  await page.locator('#ai-save').click();
  await page.waitForFunction(() => document.querySelector('#toast')?.textContent === 'saved');
  await page.locator('#settings-add').click();
  await page.locator('#settings-models').click();
  await page.waitForFunction(() => document.querySelector('select[name="assistant_provider"] option:checked')?.textContent === 'Claude');
  const saved = await page.evaluate(() => window.__TAURI__.core.invoke('ai_tasks'));
  assert.deepEqual(saved.map(({ task, provider_id, model }) => ({ task, provider_id, model })), [
    { task: 'vision', provider_id: 2, model: 'claude-haiku-4-5' },
    { task: 'chat', provider_id: 2, model: 'claude-haiku-4-5' },
    { task: 'assistant', provider_id: 2, model: 'claude-sonnet-4-5' },
  ]);
  assert.equal(await page.locator('input[name="assistant_model"]').inputValue(), 'claude-sonnet-4-5');
  // Returning to Ollama must select a model it actually has; scans need vision.
  await page.locator('select[name="vision_provider"]').selectOption('1');
  await page.waitForFunction(() => document.querySelector('input[name="vision_model"]').value === 'qwen2.5vl:3b');
  assert.deepEqual(problems, []);
});

test('a provider change never chooses an embedding or a known text-only model for scans', async (t) => {
  const { page, context, problems } = await openSettings({ hostedModels: [
    { name: 'embedding-only', embedding: true, vision: false },
    { name: 'text-only', embedding: false, vision: false },
    { name: 'unspecified', embedding: false, vision: null },
    { name: 'sees-images', embedding: false, vision: true },
  ] });
  t.after(() => context.close());
  await addClaude(page);
  await page.locator('select[name="vision_provider"]').selectOption('2');
  await page.waitForFunction(() => document.querySelectorAll('#vision-models option').length === 4);
  assert.equal(await page.locator('input[name="vision_model"]').inputValue(), 'sees-images');
  await page.locator('select[name="chat_provider"]').selectOption('2');
  await page.waitForFunction(() => document.querySelectorAll('#chat-models option').length === 4);
  assert.equal(await page.locator('input[name="chat_model"]').inputValue(), 'text-only');
  assert.deepEqual(problems, []);
});

for (const failure of ['empty', 'unavailable']) {
  test(`a provider change clears the old model when the new model list is ${failure}`, async (t) => {
    const { page, context, problems } = await openSettings({
      hostedModels: [], modelError: failure === 'unavailable' ? 'test model server unavailable' : undefined,
    });
    t.after(() => context.close());
    await addClaude(page);
    await page.locator('select[name="chat_provider"]').selectOption('2');
    await page.waitForFunction(() => document.querySelector('[data-note="chat"]').textContent.includes('Claude'));
    assert.equal(await page.locator('input[name="chat_model"]').inputValue(), '');
    assert.equal(await page.locator('#ai-form').evaluate((form) => form.checkValidity()), false);
    // Providers can omit a usable model from their list, so explicit names remain possible.
    await page.locator('input[name="chat_model"]').fill('my-custom-model');
    await page.locator('#ai-save').click();
    await page.waitForFunction(() => document.querySelector('#toast')?.textContent === 'saved');
    assert.equal(await page.locator('input[name="chat_model"]').inputValue(), 'my-custom-model');
    assert.deepEqual(problems, []);
  });
}

test('a delayed model list cannot overwrite typing or a newer provider choice', async (t) => {
  const { page, context, problems } = await openSettings({ holdModels: true });
  t.after(() => context.close());
  await addClaude(page);
  await page.locator('select[name="chat_provider"]').selectOption('2');
  await page.waitForFunction(() => window.modelRequests.length === 1);
  assert.equal(await page.locator('input[name="chat_model"]').inputValue(), '');
  await page.locator('input[name="chat_model"]').fill('my-explicit-model');
  await page.locator('select[name="vision_provider"]').selectOption('2');
  await page.locator('select[name="vision_provider"]').selectOption('1');
  await page.evaluate((offered) => window.modelRequests.shift()(offered), CLAUDE_MODELS);
  await page.waitForFunction(() => document.querySelector('#chat-models option')?.value.startsWith('claude-'));
  assert.equal(await page.locator('input[name="chat_model"]').inputValue(), 'my-explicit-model');
  assert.equal(await page.locator('input[name="vision_model"]').inputValue(), 'qwen2.5vl:3b');
  assert.equal(await page.locator('#vision-models option').first().getAttribute('value'), 'llama3.2:3b');
  assert.deepEqual(problems, []);
});

test('the model dropdown keeps every offered model available after a selection', async (t) => {
  const { page, context, problems } = await openSettings({ hostedModels: CLAUDE_MODELS });
  t.after(() => context.close());
  await addClaude(page);
  for (const task of ['vision', 'chat', 'assistant']) {
    await page.locator(`select[name="${task}_provider"]`).selectOption('2');
    const choice = page.locator(`#${task}-models`);
    await page.waitForFunction((task) => document.querySelector(`#${task}-models`)?.value === 'claude-haiku-4-5', task);
    assert.equal(await page.locator(`input[name="${task}_model"]`).isVisible(), false);
    // Choose another model directly, without erasing the current name first.
    await choice.selectOption('claude-sonnet-4-5');
    assert.deepEqual(await choice.locator('option').evaluateAll((options) => options.map((o) => o.value)),
      ['claude-haiku-4-5', 'claude-sonnet-4-5', '']);
    await choice.focus();
    await choice.press('Home');
    await page.waitForFunction((task) => document.querySelector(`input[name="${task}_model"]`).value === 'claude-haiku-4-5', task);
    await choice.press('ArrowDown');
    await page.waitForFunction((task) => document.querySelector(`input[name="${task}_model"]`).value === 'claude-sonnet-4-5', task);
  }
  assert.deepEqual(problems, []);
});

test('an explicit model name survives reopening and can be replaced from the full list', async (t) => {
  const { page, context, problems } = await openSettings({ hostedModels: CLAUDE_MODELS });
  t.after(() => context.close());
  await addClaude(page);
  await page.locator('select[name="chat_provider"]').selectOption('2');
  await page.waitForFunction(() => document.querySelector('#chat-models').value === 'claude-haiku-4-5');
  await page.locator('#chat-models').selectOption('');
  await page.locator('input[name="chat_model"]').fill('my-unlisted-model');
  await page.locator('#ai-save').click();
  await page.waitForFunction(() => document.querySelector('#toast')?.textContent === 'saved');
  await page.locator('#settings-add').click();
  await page.locator('#settings-models').click();
  await page.waitForFunction(() => document.querySelector('#chat-models option')?.value === 'claude-haiku-4-5');
  assert.equal(await page.locator('#chat-models').inputValue(), '');
  assert.equal(await page.locator('input[name="chat_model"]').isVisible(), true);
  assert.equal(await page.locator('input[name="chat_model"]').inputValue(), 'my-unlisted-model');
  await page.locator('#chat-models').selectOption('claude-sonnet-4-5');
  assert.equal(await page.locator('input[name="chat_model"]').isVisible(), false);
  assert.equal(await page.locator('input[name="chat_model"]').inputValue(), 'claude-sonnet-4-5');
  assert.deepEqual(problems, []);
});

test('the models page says which model each job uses and what the provider has', async () => {
  const { page, context, problems } = await openSettings();
  await page.locator('#settings-models').click();
  await page.waitForSelector('#ai-form', { state: 'visible' });
  assert.equal(await page.locator('#settings-form').isVisible(), false, 'one form at a time');

  assert.equal(await page.locator('#ai-form select[name="vision_provider"] option:checked').innerText(), 'Ollama on this computer');
  assert.equal(await page.locator('#ai-form input[name="vision_model"]').inputValue(), 'qwen2.5vl:3b');
  assert.equal(await page.locator('#ai-form input[name="chat_model"]').inputValue(), 'llama3.2:3b');

  await page.waitForFunction(() => document.querySelectorAll('#vision-models option').length === 3);
  const offered = await page.locator('#vision-models option').evaluateAll((options) => options.map((o) => [o.value, o.label]));
  assert.deepEqual(
    offered,
    [
      ['llama3.2:3b', 'llama3.2:3b · text only · 2.0 GB'],
      ['qwen2.5vl:3b', 'qwen2.5vl:3b · sees images · 3.2 GB'],
      ['', 'Enter model name…'],
    ],
    'an embedding model is no answer to either job',
  );
  assert.equal(await noteOf(page, 'vision').textContent(), '', 'a local model that sees needs no note');

  // A model that cannot see is no use for scans, and the page says so.
  await page.locator('#vision-models').selectOption('llama3.2:3b');
  await page.waitForFunction(() => document.querySelector('[data-note="vision"]').textContent.includes('cannot see images'));

  // One that is not pulled says how to get it.
  await page.locator('#vision-models').selectOption('');
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
  // Claude is asked over Anthropic's own API, and says where its key comes from.
  await page.locator('#provider-form select[name="preset"]').selectOption('claude');
  assert.equal(await page.locator('#provider-form input[name="base_url"]').inputValue(), 'https://api.anthropic.com/v1');
  assert.equal(await page.locator('#provider-key').isVisible(), true);
  assert.match(await page.locator('#provider-key-state').innerText(), /console\.anthropic\.com/);
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
  await page.locator('#chat-models').selectOption('deepseek-chat');
  await page.waitForFunction(() => document.querySelector('[data-note="chat"]').textContent.includes('will be sent to DeepSeek'));
  assert.equal(await noteOf(page, 'chat').evaluate((note) => note.classList.contains('warn')), true);

  // Reading scans there too: DeepSeek does not say whether its model can see.
  await page.locator('#ai-form select[name="vision_provider"]').selectOption({ label: 'DeepSeek' });
  await page.locator('#vision-models').selectOption('deepseek-chat');
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

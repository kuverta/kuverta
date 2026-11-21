/**
 * Records the two-minute demo: the desktop window in a real Chromium, on the
 * test double with a made-up mailbox. Setting up, deleting a newsletter and
 * unsubscribing, deleting one of a run and gathering the rest in a smart
 * mailbox, asking the assistant a question. No real mail, no network, no model.
 *
 *     node kuverta-bird/tools/demo-video.mjs            # writes website/docs/assets/video/kuverta-demo.mp4
 */
import http from 'node:http';
import { readFile, readdir, mkdir, rm } from 'node:fs/promises';
import { dirname, extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { chromium } from 'playwright';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const PAGE = '/apps/desktop/ui/index.html';
const FAKE = '/kuverta-bird/test/support/fake-invoke.js';
const OUT = join(ROOT, 'website', 'docs', 'assets', 'video', 'kuverta-demo.mp4');
const SIZE = { width: 1440, height: 900 };
const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.jpg': 'image/jpeg' };

function serve(root) {
  return new Promise((ready) => {
    const s = http.createServer(async (req, res) => {
      const path = normalize(decodeURIComponent(new URL(req.url, 'http://x').pathname));
      const file = join(root, path);
      if (!file.startsWith(root)) return res.writeHead(403).end();
      try { res.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream', 'access-control-allow-origin': '*' }); res.end(await readFile(file)); }
      catch { res.writeHead(404).end(); }
    });
    s.listen(0, '127.0.0.1', () => ready(s));
  });
}

// The mailbox. Newest first in the window.
const T0 = Math.floor(Date.UTC(2026, 10, 20, 9, 0, 0) / 1000);
const m = (hoursAgo, from, subject, body, extra = {}) => ({
  subject, from, date_utc: T0 - hoursAgo * 3600, unread: hoursAgo < 6, has_attachments: false,
  category: null, folder: 'INBOX', message_id: `${Math.random().toString(36).slice(2)}@example`, snippet: body.slice(0, 90), body, ...extra,
});
const GNOME = 'Gnome King Garden Gnomes <news@gnomeking.example>';
const KEVIN = 'Kevin (Accounting) <kevin@office.example>';
const SOCIETY = 'Society for Applied Cosiness <board@cosiness.example>';
const SEED = [
  m(1, GNOME, 'Today only: 40% off garden gnomes', 'Dear gnome friend, today only: 40% off every gnome with a lantern. Your lawn says thank you.', { category: 'newsletter', list_id: 'news.gnomeking.example' }),
  m(2, KEVIN, 'Who drank my oat milk? (Part 9)', 'Dear colleagues, the level dropped by another 400 ml. Between 09:10 and 09:25. I will just say: stand-up.', { category: 'personal' }),
  m(3, 'Grandma <grandma@example.org>', 'Your parcel of socks is on its way', 'My boy, the socks are on their way. The green ones are for Sunday. Do call.', { category: 'personal' }),
  m(5, SOCIETY, 'Invitation to the annual general meeting', 'Dear members, the annual general meeting is on Thursday 12 December at 19:00 in the clubhouse. Please bring cake. The only item on the agenda: does the sofa stay? Sign up by 5 December.', { category: 'personal' }),
  m(9, 'City Tax Office <post@taxoffice.example>', 'Dog tax assessment 2026', 'Dear Sir or Madam, please find attached your dog tax assessment. If you do not own a dog, kindly let us know in writing.', { category: 'transactional' }),
  m(14, KEVIN, 'Who drank my oat milk? (Part 8)', 'I have now put a tally sheet on the fridge.', { category: 'personal' }),
  m(20, GNOME, 'Gnome of the week: Hubert', 'Hubert holds a wheelbarrow and has been in the range since 1987. Now with moss.', { category: 'newsletter', list_id: 'news.gnomeking.example' }),
  m(27, 'Dr. Schneider <practice@schneider.example>', 'Question about the quote', 'Hello, could you firm up the quote for the practice website by Friday? Best regards, Dr. Schneider', { category: 'personal' }),
  m(31, KEVIN, 'Oat milk update: the fridge is silent', 'No new findings. The tally sheet has been removed. By someone.', { category: 'personal' }),
  m(44, GNOME, 'Last chance: gnomes in packs of three', 'Three gnomes, one price. While stocks last (they will).', { category: 'newsletter', list_id: 'news.gnomeking.example' }),
  m(52, KEVIN, 'Who drank my oat milk? (Part 7)', 'Someone was at the fridge again. I have evidence (a glass).', { category: 'personal' }),
  m(70, 'City Cleaning <info@citycleaning.example>', 'Your bulky waste collection: 3 December', 'Please put the bulky waste out from 6 am. Sofas will be taken.', { category: 'notification' }),
];

const P0 = Math.floor(Date.UTC(2026, 10, 19, 11, 0, 0) / 1000);
const PAPER = {
  paperMailboxes: [{ id: 1, label: 'Home', base_url: 'http://paperless.local:8000', selector_kind: 'everything', selector_value: null, has_token: true }],
  documents: [
    { id: 41, mailbox: 1, date_utc: P0, added_utc: P0 + 600, from: 'City Tax Office', subject: 'Dog tax assessment 2026', unread: true, has_attachments: true, category: 'transactional',
      snippet: 'You have not registered a dog. Please let us know by 15 December whether that is still the case.', tags: ['Tax'], page_count: 2,
      transcript: 'City Tax Office\nTax number 41/123/45678\n\nDog tax assessment 2026\n\nDear Ms Muster,\n\nAccording to our records you have not registered a dog. Please let us know in writing by 15 December 2026 whether that is still the case. Otherwise we will assess dog tax for a medium-sized dog (96.00 EUR per year).\n\nYours faithfully\npp.', transcribed_by: 'qwen2.5vl:3b' },
    { id: 40, mailbox: 1, date_utc: P0 - 86400 * 3, added_utc: P0 - 86400 * 2, from: 'Sunshine Allotment Society', subject: 'Warning: garden gnome over the plot boundary', unread: false, has_attachments: true, category: 'personal',
      snippet: 'Your gnome Hubert has been standing 11 cm into plot 14 since 3 October. Please move him back.', tags: ['Society'], page_count: 1,
      transcript: 'Sunshine Allotment Society\n\nWarning\n\nYour garden gnome (Hubert, with wheelbarrow) has been standing 11 cm into plot 14 since 3 October. We ask you to move him back by the weekend. The committee.', transcribed_by: 'qwen2.5vl:3b' },
  ],
};

const browser = await chromium.launch();
const server = await serve(ROOT);
const base = `http://127.0.0.1:${server.address().port}`;
const videoDir = join(ROOT, 'target', 'demo-video');
await rm(videoDir, { recursive: true, force: true });
await mkdir(videoDir, { recursive: true });

const context = await browser.newContext({ viewport: SIZE, locale: 'en-US', recordVideo: { dir: videoDir, size: SIZE }, colorScheme: 'light' });
const page = await context.newPage();
page.on('pageerror', (e) => console.error('page error:', (e.stack ?? e.message).split('\n').slice(0, 3).join(' | ')));
page.on('dialog', (d) => d.accept());

await page.addInitScript(({ fake, seed, paper }) => {
  window.__kuvertaSyncEveryMs = 600000;
  let lastTrash = null;
  const ids = {};
  window.__fakeBridge = import(fake).then((mod) => {
    const real = mod.fakeInvoke({ seed, paper });
    // Ids follow seed order, starting at 1.
    seed.forEach((msg, i) => { ids[msg.subject] = i + 1; });
    const gnomeIds = seed.map((s, i) => (s.from.startsWith('Gnome King') ? i + 1 : null)).filter(Boolean);
    const kevinIds = seed.map((s, i) => (s.from.startsWith('Kevin') ? i + 1 : null)).filter(Boolean);
    const over = {
      // -- setup assistant --
      ollama_status: async () => ({ local: true, installed: true, running: true, version: '0.40.16', base_url: 'http://127.0.0.1:11434',
        needed: [{ task: 'chat', model: 'qwen3:4b-instruct', present: true }, { task: 'vision', model: 'qwen2.5vl:3b', present: true }] }),
      find_paperless: async () => [{ base_url: 'http://paperless.local:8000', answering: true, configured: true, installed_by_kuverta: false }],
      paper_folders: async () => [],
      set_paper_folders: async () => null,
      paper_suggested_words: async () => [],
      docker_status: async () => ({ installed: false, running: false }),
      import_accounts: async () => ({ passwords_locked: false,
        sources: [{ name: 'Thunderbird', state: 'found', detail: '1 accounts' }, { name: 'Apple Mail', state: 'none', detail: 'nothing to import' }],
        accounts: [{ complete: true, already_added: false, source: 'Thunderbird', full_name: 'Mona Muster', password_known: true, notes: [], input: { email: 'mona@example.org', label: 'Mona', username: 'mona@example.org', imap_host: 'imap.example.org', imap_port: 993, imap_security: 'tls', smtp_host: 'smtp.example.org', smtp_port: 465, smtp_security: 'tls', auth_method: 'app_password' } }] }),
      save_account_with_found_password: async () => 7,
      save_account: async () => 7,
      set_password: async () => null,
      verify_account: async () => ({ imap_ok: true, total_messages: 1284, smtp_target: 'smtp.example.org', smtp_ok: true }),
      finish_setup: async () => null,
      start_ollama: async () => null,
      // -- the newsletter --
      unsubscribe_for_message: async ({ id }) => gnomeIds.includes(Number(id))
        ? { name: 'Gnome King Garden Gnomes', key: 'gnomeking', method: { kind: 'one_click' }, messages: gnomeIds.length }
        : null,
      unsubscribe: async () => [{ state: 'done' }],
      trash_from_sender: async ({ account }) => {
        let n = 0;
        for (const id of gnomeIds) { try { await real('move_to', { account, id, target: lastTrash ?? 'Trash' }); n += 1; } catch {} }
        return n;
      },
      // -- the run from Kevin --
      similar: async ({ id }) => {
        if (!kevinIds.includes(Number(id))) return { matches: [], suggestion: { name: '', query: { match_all: true, rules: [] } } };
        const rest = kevinIds.filter((k) => k !== Number(id));
        const matches = [];
        for (const k of rest) { const msg = await real('message', { id: k }).catch(() => null); if (msg) matches.push({ id: k, from: msg.from ?? 'Kevin (Accounting)', subject: msg.subject, date_utc: msg.date_utc, reason: /Part/.test(msg.subject) ? 'same sender, numbered subject' : 'same sender' }); }
        return { matches, suggestion: { name: "Kevin's oat-milk saga", query: { match_all: true, rules: [{ field: 'from', op: 'contains', value: 'kevin@office.example' }] } } };
      },
      // -- the assistant --
      assistant_ask: async ({ turns, message, onEvent }) => {
        const id = ids['Invitation to the annual general meeting'];
        const msg = await real('message', { id });
        const events = [
          { kind: 'looked', what: 'searched the mailbox for “cosiness”, “meeting”, “bring” — 1 found' },
          { kind: 'message', id, subject: msg.subject, from: msg.from, date_utc: msg.date_utc, attachments: [], note: null },
        ];
        await new Promise((r) => setTimeout(r, 1200));
        for (const e of events) onEvent?.onmessage?.(e);
        await new Promise((r) => setTimeout(r, 900));
        const reply = '**Thursday 12 December, 19:00**, in the clubhouse — from “Invitation to the annual general meeting”.\n\nBring a cake. The only item on the agenda is whether the sofa stays, and sign-up closes on **5 December**. Shall I draft the sign-up reply?';
        return { turns: [...turns, { role: 'user', content: message }, { role: 'assistant', content: reply, calls: [] }], reply, events, model: 'qwen3:4b-instruct', local: true };
      },
    };
    return async (command, args) => {
      if (command === 'move_to' && args?.target) lastTrash = args.target;
      if (command in over) return over[command](args ?? {}, real);
      return real(command, args);
    };
  });
  window.__TAURI__ = { core: { invoke: async (command, args) => (await window.__fakeBridge)(command, args), Channel: class { onmessage = null; } } };
}, { fake: FAKE, seed: SEED, paper: PAPER });

const wait = (ms) => page.waitForTimeout(ms);
const caption = (text) => page.evaluate((text) => {
  let bar = document.getElementById('__cap');
  if (!bar) {
    bar = document.createElement('div'); bar.id = '__cap';
    bar.style.cssText = 'position:fixed;left:0;right:0;bottom:0;z-index:2147483647;padding:16px 36px;font:500 21px/1.35 -apple-system,Inter,Segoe UI,sans-serif;color:#fff;background:rgba(12,14,18,.86);letter-spacing:-.01em;pointer-events:none;transition:opacity .25s';
    document.body.appendChild(bar);
  }
  bar.textContent = text; bar.style.opacity = text ? '1' : '0';
}, text);
const card = (title, sub) => page.evaluate(([title, sub]) => {
  let c = document.getElementById('__card');
  if (!title) { c?.remove(); return; }
  c ??= document.createElement('div'); c.id = '__card';
  c.style.cssText = 'position:fixed;inset:0;z-index:2147483646;background:#0b0d11;color:#fff;display:flex;align-items:center;justify-content:center;font-family:-apple-system,Inter,Segoe UI,sans-serif';
  c.innerHTML = `<div style="max-width:960px;padding:48px"><div style="font-size:64px;font-weight:650;letter-spacing:-.03em">${title}</div><div style="font-size:27px;color:#9aa3b2;margin-top:20px;line-height:1.4">${sub}</div></div>`;
  document.body.appendChild(c);
}, [title, sub]);
const rowWith = (text) => page.locator('#content .row:not([hidden])', { hasText: text }).first();

// 1. Title over the loading app.
await page.goto(`${base}${PAGE}`);
await page.waitForFunction(() => document.getElementById('status')?.textContent === 'you@example.com');
await page.waitForFunction(() => document.querySelectorAll('#content .row:not([hidden])').length > 0);
await card('kuverta', 'A mail client that sorts, reads and answers with small models on your own laptop. Nothing leaves it.');
await wait(4500);
await card(null);

// 2. Setup assistant.
await caption('First run: the setup assistant. Profiles keep private and work mail apart.');
await page.evaluate(() => openSetup());
await page.waitForSelector('#setup', { state: 'visible' });
await wait(1500);
for (const chip of await page.locator('#setup [data-suggest]').all()) { await chip.click(); await wait(600); if ((await page.locator('#setup-profiles input').count()) >= 2) break; }
await wait(2000);
await page.click('#setup-next');
await caption('It checks Ollama and the two small models: a 4B model for sorting and the assistant, a 3B one that reads scanned letters.');
await page.waitForFunction(() => /ready/.test(document.getElementById('ollama-card')?.textContent ?? ''));
await wait(5000);
await page.click('#setup-next');
await caption('Paper post: a Paperless scanner at home is found and connected. Letters will arrive next to email.');
await page.waitForFunction(() => /connected/.test(document.getElementById('paper-found')?.textContent ?? ''));
await wait(4000);
await page.click('#setup-next');
await caption('Folders for the paper, suggested and editable: where a letter goes once it is read.');
await page.waitForFunction(() => (document.querySelectorAll('#shelf input').length) > 0);
await wait(4000);
await page.click('#setup-next');
await caption('Mail accounts: it finds what Thunderbird already knows, saved passwords included.');
await page.waitForFunction(() => /mona@example.org/.test(document.getElementById('setup')?.textContent ?? ''));
await wait(4000);
await page.click('#setup-next');
await page.waitForFunction(() => /Signed in/.test(document.getElementById('setup')?.textContent ?? ''));
await caption('Signed in, 1,284 messages, nothing typed. The summary, then straight into the inbox.');
await wait(4500);
await page.click('#setup-next');
await page.waitForSelector('#setup', { state: 'hidden' });

// 3. Inbox, newsletter, unsubscribe.
await caption('The inbox, sorted on arrival: newsletters, personal mail, invoices, notifications.');
await wait(4500);
await rowWith('40% off garden gnomes').click();
await caption('A garden-gnome newsletter. Delete it with #…');
await wait(3000);
await page.keyboard.press('#');
await page.waitForSelector('#unsub-sheet', { state: 'visible' });
await caption('…and kuverta offers to take you off the list right there. One click, nothing else sent.');
await wait(3500);
await page.check('#unsub-also');
await wait(1500);
await page.click('#unsub-go');
await caption('Unsubscribed, and the other two gnome mails are gone too.');
await wait(3500);

// 4. Kevin, look-alikes, smart mailbox.
await rowWith('oat milk? (Part 9)').click();
await caption('Kevin from accounting, episode 9. Delete it…');
await wait(3000);
await page.keyboard.press('#');
await page.waitForSelector('#similar-sheet', { state: 'visible' });
await caption('…and kuverta finds the rest of the saga. Trash them all, or give Kevin a mailbox of his own.');
await wait(4500);
await page.click('#similar-smart');
await page.waitForSelector('#smart-sheet', { state: 'visible' });
await caption('A smart mailbox with the rule already written. Future episodes land there, not in the inbox.');
await page.waitForFunction(() => /match/.test(document.getElementById('smart-preview-count')?.textContent ?? ''));
await wait(4500);
await page.click('#smart-save');
await page.waitForSelector('#smart-sheet', { state: 'hidden' });
await wait(3000);

// 4b. Paper post.
await page.locator('#postboxes .nav-item', { hasText: 'Home' }).first().click();
await caption('Post: letters from the Paperless scanner, or photographed with the laptop camera, in the same window as email.');
await page.waitForFunction(() => document.querySelectorAll('#content .row:not([hidden])').length > 0);
await wait(3500);
await rowWith('Dog tax').click();
await caption('Read by OCR and the small vision model, sorted like email, the sender read off the page. A reply goes back as a printed letter.');
await wait(7000);
await page.locator('#accounts .nav-item').first().click();
await page.waitForFunction(() => document.querySelectorAll('#content .row:not([hidden])').length > 0);
await wait(1000);

// 5. The assistant.
await caption('Ask the assistant. It searches your mail with the local model; the question never leaves the laptop.');
await page.click('#assistant-toggle');
await page.waitForSelector('#chat-input', { state: 'visible' });
await wait(1500);
await page.type('#chat-input', 'When is the Society for Applied Cosiness meeting, and what do I bring?', { delay: 45 });
await wait(800);
console.error('chat input disabled?', await page.evaluate(() => document.getElementById('chat-input')?.disabled), 'value:', await page.inputValue('#chat-input'));
await page.keyboard.press('Enter');
try {
  await page.waitForFunction(() => /sofa/.test(document.getElementById('chat-log')?.textContent ?? ''), null, { timeout: 20000 });
} catch (err) {
  console.error('chat log at timeout:', await page.evaluate(() => document.getElementById('chat-log')?.innerText));
  console.error('toast:', await page.evaluate(() => document.getElementById('toast')?.innerText));
  throw err;
}
await caption('It says where it looked and shows the message it answered from. Every claim can be checked.');
await wait(7000);

// 6. The scanner rig, then the end card.
await caption('');
await page.evaluate((src) => {
  const c = document.createElement('div'); c.id = '__photo';
  c.style.cssText = 'position:fixed;inset:0;z-index:2147483646;background:#0b0d11;display:flex;align-items:center;justify-content:center;gap:48px;padding:40px;font-family:-apple-system,Inter,Segoe UI,sans-serif;color:#fff';
  c.innerHTML = `<img src="${src}" style="height:100%;max-height:820px;border-radius:10px;box-shadow:0 20px 60px rgba(0,0,0,.6)"><div style="max-width:440px"><div style="font-size:40px;font-weight:650;letter-spacing:-.02em;line-height:1.15">The other end: the letter scanner</div><div style="font-size:22px;color:#9aa3b2;margin-top:18px;line-height:1.45">A Raspberry Pi with a camera and a touchscreen over a fixed bed. Put a letter down, it is photographed, handed to Paperless and lands in kuverta. Open hardware, in the repository.</div></div>`;
  document.body.appendChild(c);
}, `${base}/website/docs/assets/scanner-rig.jpg`);
await page.waitForFunction(() => { const i = document.querySelector('#__photo img'); return i && i.complete && i.naturalWidth > 0; });
await wait(7000);
await page.evaluate(() => document.getElementById('__photo')?.remove());

// 7. End card.
await caption('');
await card('kuverta', 'Open source, AGPL-3.0 · kuverta.github.io/kuverta · desktop app and Thunderbird extension');
await wait(5000);

const video = page.video();
await context.close();
const webm = await video.path();
await browser.close();
server.close();

const ff = spawnSync('ffmpeg', ['-y', '-hide_banner', '-loglevel', 'error', '-i', webm, '-vf', `scale=${SIZE.width}:${SIZE.height}:flags=lanczos,format=yuv420p`, '-c:v', 'libx264', '-preset', 'slow', '-crf', '22', '-r', '30', '-movflags', '+faststart', OUT], { stdio: 'inherit' });
if (ff.status !== 0) throw new Error('ffmpeg failed');
const probe = spawnSync('ffprobe', ['-v', 'error', '-show_entries', 'format=duration,size', '-of', 'default=nw=1', OUT], { encoding: 'utf8' });
console.log(`wrote ${OUT}\n${probe.stdout.trim()}`);

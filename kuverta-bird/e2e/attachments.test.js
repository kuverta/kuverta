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

async function openWindow({ threadSeed = false } = {}) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, locale: 'en-US' });
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

  await page.addInitScript(({fake, threadSeed}) => {
    // Only the app window needs a bridge, not its sandboxed attachment frames.
    if (window !== window.top || location.protocol !== 'http:') return;
    try {
      localStorage.clear();
    } catch {
      // A fresh context has nothing to clear.
    }
    window.__fakeBridge = import(fake).then((module) => {
      const seed = [...module.defaultSeed(), module.esimMessage()];
      if (threadSeed) seed[1].subject = `Re: ${seed[0].subject}`;
      return module.fakeInvoke({
        seed,
        paper: { paperMailboxes: [], documents: [] },
      });
    });
    window.__TAURI__ = {
      core: {
        invoke: async (command, args) => {
          if (window.threadHtml && command === 'message') return {...await (await window.__fakeBridge)(command,args),has_html:true};
          if (window.threadHtml && command === 'mail_page_url') {
            (window.htmlRequests??=[]).push(args);
            return 'data:text/html,<p>Safe thread preview</p>';
          }
          if (window.modelStopped && command === 'find_urgent') {
            (window.urgencyRequests??=[]).push(args.useModel);
            return {by_rules:0,by_model:0,model:'local',stopped:args.useModel?'Ollama unavailable':null};
          }
          if (window.filingTest && command === 'filing_tree') return {root:'C:/Test filing',folders:['','Contracts'],skipped:0,truncated:false};
          if (window.filingTest && command === 'propose_attachment_filing') return {...args,root:'C:/Test filing',name:'eSIM QR.png',size:12,reason:''};
          if (window.filingTest && command === 'confirm_attachment_filing') {
            (window.filingSaves ??= []).push(args.proposal);
            return 'C:/Test filing/Contracts/eSIM QR.png';
          }
          if (command === 'check_mail_domain') {
            (window.domainRequests ??= []).push(args);
            if (window.holdDomainCheck) return new Promise(resolve => { window.releaseDomainCheck = resolve; });
            if (window.failDomainCheck) throw new Error('DNS temporarily unavailable');
            return {domain:args.domain, checks:[
              {protocol:'SPF',name:args.domain,status:'present',notes:['SPF is published. Sending IPs, include/redirect chains and their lookup limits still need a real sending check.'],records:['v=spf1 -all']},
              {protocol:'DMARC',name:`_dmarc.${args.domain}`,status:'warning',notes:['DMARC p=none monitors mail but does not request quarantine or rejection.'],records:['v=DMARC1; p=none']},
              {protocol:'DKIM',name:'',status:'not_checked',notes:['Enter the DKIM selector supplied by your mail provider. Without it, DKIM cannot be checked.'],records:[]},
            ]};
          }
          if (command === 'set_read') {
            if (window.failRead) throw new Error('Test read update failed');
            (window.readChanges ??= []).push(args);
          }
          if (command === 'urgent_messages' && window.holdUrgency) {
            return new Promise(resolve => { window.releaseUrgency = resolve; });
          }
          if (command === 'urgent_messages' && window.testUrgency) return window.testUrgency;
          if (command === 'assistant_ask') {
            (window.assistantRequests ??= []).push(args.message);
            if (window.failNextAssistant) {
              window.failNextAssistant = false;
              throw new Error('Test provider unavailable');
            }
          }
          return (await window.__fakeBridge)(command, args);
        },
        // Progress arrives on a channel in the real bridge, and the window
        // makes one for every sync, so without this nothing syncs here.
        Channel: class {
          onmessage = null;
        },
      },
    };
  }, {fake:FAKE, threadSeed});

  await page.goto(`${base}${PAGE}`);
  await page.waitForFunction(() => document.getElementById('status')?.textContent === 'you@example.com');
  return { page, context, problems };
}

const log = (page) => page.evaluate(async () => (await window.__fakeBridge)('attachment_log', {}));

test('optional navigation restores messages and folders, branches forward history and never undoes mail',async()=>{
  const {page,context,problems}=await openWindow();
  assert.equal(await page.locator('#mail-navigation').isVisible(),false);
  await page.locator('#open-settings').click();
  await page.locator('#settings-nav .nav-item',{hasText:/^General$/}).click();
  await page.locator('#pref-mail-navigation').check();await page.locator('#settings-close').click();
  await page.evaluate(()=>setLanguageChoice('de'));
  assert.equal(await page.locator('#mail-back').getAttribute('aria-label'),'Zurück');
  assert.equal(await page.locator('#mail-forward').getAttribute('aria-label'),'Vor');
  await page.evaluate(()=>setLanguageChoice('en'));
  await page.locator('#content .row').nth(0).click();
  const first=await page.locator('#reading-subject').innerText();
  await page.locator('#content .row').nth(1).click();
  const second=await page.locator('#reading-subject').innerText();assert.notEqual(first,second);
  await page.locator('#mail-back').click();
  await page.waitForFunction(s=>document.getElementById('reading-subject').textContent===s,first);
  await page.locator('#mail-forward').click();
  await page.waitForFunction(s=>document.getElementById('reading-subject').textContent===s,second);
  await page.locator('#folders .nav-item',{hasText:/^Inbox/}).click();
  await page.waitForFunction(()=>state.filter.folder!==null);
  await page.locator('#mail-back').click();
  await page.waitForFunction(s=>!document.getElementById('reading').hidden&&document.getElementById('reading-subject').textContent===s,second);
  assert.equal(await page.evaluate(()=>state.filter.folder),null);
  await page.locator('#content .row').nth(2).click();
  assert.equal(await page.locator('#mail-forward').isDisabled(),true);
  assert.deepEqual(await page.evaluate(()=>window.readChanges??[]),[]);
  assert.deepEqual(problems,[]);await context.close();
});

test('thread HTML uses an empty sandbox, blocks remote images by default and switches back to text',async()=>{
  const {page,context,problems}=await openWindow({threadSeed:true});
  await page.evaluate(()=>{window.threadHtml=true;});
  await page.locator('#list-threads').click();
  await page.locator('#content .row',{hasText:'2 messages'}).click();
  await page.locator('.mail-thread-entry summary').first().click();
  const entry=page.locator('.mail-thread-entry[open]');
  await entry.getByRole('tab',{name:'HTML',exact:true}).click();
  await page.waitForFunction(()=>window.htmlRequests?.length===1);
  assert.equal(await entry.locator('iframe').getAttribute('sandbox'),'');
  assert.equal((await page.evaluate(()=>window.htmlRequests))[0].remote,false);
  await page.evaluate(()=>pageMade({id:window.htmlRequests[0].id,remote:false,remote_blocked:2}));
  await entry.getByRole('button',{name:'Show pictures',exact:true}).click();
  await page.waitForFunction(()=>window.htmlRequests?.length===2);
  assert.equal((await page.evaluate(()=>window.htmlRequests))[1].remote,true);
  await entry.getByRole('tab',{name:'Text',exact:true}).click();
  assert.equal(await entry.locator('iframe').isVisible(),false);
  assert.equal(await entry.locator('iframe').getAttribute('src'),'about:blank');
  assert.deepEqual(problems,[]);await context.close();
});

test('stopped automatic urgency analysis backs off but a manual retry runs immediately',async()=>{
  const {page,context,problems}=await openWindow();
  await page.evaluate(async()=>{window.modelStopped=true;await judgeUrgency({useModel:true});await judgeUrgency({useModel:true});await judgeUrgency({useModel:true,loud:true});});
  assert.deepEqual(await page.evaluate(()=>window.urgencyRequests),[true,false,true]);
  assert.deepEqual(problems,[]);await context.close();
});

test('categories can be hidden independently of unread filtering and clear an active category', async () => {
  const {page,context,problems}=await openWindow();
  await page.locator('#categories .nav-item').nth(1).click();
  await page.locator('#open-settings').click();
  await page.locator('#settings-nav .nav-item',{hasText:/^General$/}).click();
  await page.locator('#pref-show-categories').uncheck();
  await page.locator('#settings-close').click();
  assert.equal(await page.locator('#categories').isVisible(),false);
  assert.equal(await page.evaluate(()=>state.filter.category),null);
  await page.locator('#list-unread-filter').click();
  await page.waitForFunction(()=>state.filter.unreadOnly);
  assert.equal(await page.evaluate(()=>localStorage.getItem('showCategories')),'no');
  assert.deepEqual(problems,[]);
  await context.close();
});

test('attachment filing requires preview and confirmation and cancellation saves nothing', async () => {
  const {page,context,problems}=await openWindow();
  await page.evaluate(()=>{window.filingTest=true;});
  await page.locator('#content .row',{hasText:'Ihre neue e-SIM ist da'}).click();
  await page.locator('#reading-attachments .attachment-chip',{hasText:'eSIM QR.png'}).click();
  // Translated buttons must fit without hidden horizontal overflow. Playwright
  // would otherwise scroll a clipped button into view, unlike a normal click.
  await page.evaluate(()=>setLanguageChoice('de'));
  for (const width of [1200, 600]) {
    await page.setViewportSize({width,height:850});
    assert.equal(await page.locator('#attachment-sheet .dialog-foot').evaluate(foot => {
      const bounds=foot.getBoundingClientRect();
      return [...foot.querySelectorAll('button')].filter(b=>!b.hidden).every(button=>{
        const r=button.getBoundingClientRect();
        return r.left>=bounds.left && r.right<=bounds.right && r.bottom<=innerHeight;
      });
    }),true,`German attachment actions fit at ${width}px`);
  }
  await page.setViewportSize({width:1200,height:850});
  await page.evaluate(()=>setLanguageChoice('en'));
  await page.locator('#attachment-file').click();
  await page.locator('#attachment-filing-folder').selectOption('Contracts');
  await page.getByRole('button',{name:'Preview destination',exact:true}).click();
  assert.deepEqual(await page.evaluate(()=>window.filingSaves ?? []),[]);
  await page.getByRole('button',{name:'Confirm and save',exact:true}).click();
  await page.locator('#ask-no').click();
  assert.deepEqual(await page.evaluate(()=>window.filingSaves ?? []),[]);
  await page.getByRole('button',{name:'Confirm and save',exact:true}).click();
  await page.locator('#ask-yes').click();
  await page.waitForFunction(()=>window.filingSaves?.length===1);
  assert.equal(await page.evaluate(()=>window.filingSaves[0].folder),'Contracts');
  assert.deepEqual(problems,[]);
  await context.close();
});

test('optional thread opening reads only the newest message and never cascades on reload', async () => {
  const {page,context,problems}=await openWindow({threadSeed:true});
  await readOnOpen(page,true);
  await page.locator('#open-settings').click();
  await page.locator('#settings-nav .nav-item',{hasText:/^General$/}).click();
  await page.locator('#pref-thread-open-latest').check();
  await page.locator('#settings-close').click();
  await page.locator('#list-threads').click();
  await page.locator('#content .row',{hasText:'2 messages'}).click();
  await page.locator('.mail-thread-entry[open]').waitFor();
  assert.equal(await page.locator('.mail-thread-entry[open]').count(),1);
  await page.waitForFunction(()=>window.readChanges?.length===1);
  await page.evaluate(()=>reload({keepPosition:true}));
  assert.equal(await page.evaluate(()=>window.readChanges.length),1);
  assert.deepEqual(problems,[]);
  await context.close();
});

async function readOnOpen(page, enabled) {
  await page.locator('#open-settings').click();
  await page.locator('#settings-nav .nav-item', {hasText:/^General$/}).click();
  await page.locator('#pref-read-on-open').setChecked(enabled);
  await page.keyboard.press('Escape');
}

test('attention keywords persist, defaults need saving, and assistant proposals require a click', async () => {
  const {page,context,problems} = await openWindow();
  await page.locator('#open-settings').click();
  await page.locator('#settings-nav .nav-item', {hasText:/^General$/}).click();
  await page.locator('#attention-fields:not([hidden])').waitFor();
  const account = await page.evaluate(()=>state.account);
  await page.locator('#attention-priority').fill('shop outage\ncustomer complaint');
  await page.locator('#attention-save').click();
  await page.waitForFunction(()=>document.querySelector('#attention-status').textContent.includes('saved'));
  let settings = await page.evaluate(async account=>(await invoke('attention_settings',{account})).settings, account);
  assert.deepEqual(settings.priority_words,['shop outage','customer complaint']);
  await page.locator('#attention-defaults').click();
  assert.equal(await page.locator('#attention-priority').inputValue(),'');
  settings = await page.evaluate(async account=>(await invoke('attention_settings',{account})).settings, account);
  assert.equal(settings.priority_words.length,2,'loading defaults does not save');
  await page.keyboard.press('Escape');
  await page.locator('#assistant-toggle').click();
  await page.evaluate(account=>showEvent({kind:'attention_suggested',account,group:'priority_words',words:['failed payment'],reason:'Review failed payments.'}),account);
  settings = await page.evaluate(async account=>(await invoke('attention_settings',{account})).settings, account);
  assert.equal(settings.priority_words.length,2,'a model proposal changes nothing');
  await page.getByRole('button',{name:'Apply suggestion',exact:true}).click();
  await page.getByRole('button',{name:'Applied',exact:true}).waitFor();
  settings = await page.evaluate(async account=>(await invoke('attention_settings',{account})).settings, account);
  assert.deepEqual(settings.priority_words,['shop outage','customer complaint','failed payment']);
  assert.deepEqual(problems,[]);
  await context.close();
});

test('read-on-open is opt-in and never cascades through unread mail on refresh', async () => {
  const {page,context,problems} = await openWindow();
  const unread = page.locator('#content .row:has(.dot.on)').first();
  await unread.click();
  await page.locator('#reading:not([hidden])').waitFor();
  assert.deepEqual(await page.evaluate(()=>window.readChanges ?? []),[]);
  await readOnOpen(page,true);
  await page.evaluate(()=>openSelected());
  assert.deepEqual(await page.evaluate(()=>window.readChanges ?? []),[],'background redraw must not mark read');
  await page.locator('#list-unread-filter').click();
  await page.locator('#content .row:has(.dot.on)').first().click();
  await page.waitForFunction(()=>window.readChanges?.length===1);
  const id = await page.evaluate(()=>state.rows.get(state.selected).id);
  assert.equal(await page.evaluate(()=>state.rows.get(state.selected).unread),false);
  await page.evaluate(()=>reload({keepPosition:true}));
  assert.equal(await page.evaluate(()=>window.readChanges.length),1,'refresh must not open the next unread message');
  assert.equal(await page.evaluate(id=>[...state.rows.values()].some(r=>r.id===id),id),false);
  await readOnOpen(page,false);
  await page.locator('#content .row:has(.dot.on)').first().click();
  assert.equal(await page.evaluate(()=>window.readChanges.length),1);
  assert.deepEqual(problems,[]);
  await context.close();
});

test('failed read-on-open retains unread state and can be retried', async () => {
  const {page,context,problems} = await openWindow();
  await readOnOpen(page,true);
  await page.evaluate(()=>{window.failRead=true;});
  await page.locator('#content .row:has(.dot.on)').first().click();
  await page.waitForFunction(()=>document.querySelector('#toast')?.textContent.includes('Could not mark'));
  assert.equal(await page.evaluate(()=>state.rows.get(state.selected).unread),true);
  await page.evaluate(()=>{window.failRead=false;});
  await page.keyboard.press('Enter');
  await page.waitForFunction(()=>window.readChanges?.length===1);
  assert.equal(await page.evaluate(()=>state.rows.get(state.selected).unread),false);
  assert.deepEqual(problems,[]);
  await context.close();
});

test('clicking successive unread messages clears their dots and grey backgrounds', async () => {
  const {page,context,problems} = await openWindow();
  await readOnOpen(page,true);
  for (let n=1;n<=2;n++) {
    await page.locator('#content .row:has(.dot.on)').first().click();
    await page.waitForFunction(n=>window.readChanges?.length===n,n);
    assert.equal(await page.evaluate(()=>state.rows.get(state.selected).unread),false);
    assert.equal(await page.locator('#content .row.selected .dot.on').count(),0);
  }
  assert.equal(new Set((await page.evaluate(()=>window.readChanges)).map(r=>r.id)).size,2);
  assert.equal(await page.evaluate(()=>localStorage.getItem('readOnOpen')),'yes');
  assert.deepEqual(problems,[]);
  await context.close();
});

test('domain diagnostics send only a domain/selector, show limits, and recover from lookup failure', async () => {
  const {page,context,problems} = await openWindow();
  await page.locator('#open-settings').click();
  const box = page.locator('#account-domain-check');
  assert.deepEqual(await page.evaluate(()=>window.domainRequests ?? []),[],'existing account is checked on demand');
  await box.getByRole('button',{name:'Check domain',exact:true}).click();
  await box.getByText('SPF · Record found',{exact:true}).waitFor();
  assert.deepEqual(await page.evaluate(()=>window.domainRequests),[{domain:'example.com',selector:null}]);
  assert.equal(await box.getByText('DKIM · Not checked',{exact:true}).count(),1);
  assert.match(await box.textContent(),/does not test delivery/);
  await box.getByLabel('DKIM selector').fill('mail2026');
  await page.evaluate(()=>{window.failDomainCheck=true;});
  await box.getByRole('button',{name:'Check domain',exact:true}).click();
  await box.getByText('Lookup unavailable: Error: DNS temporarily unavailable',{exact:true}).waitFor();
  assert.equal(await page.locator('#settings-save').isEnabled(),true);
  await page.evaluate(()=>{window.failDomainCheck=false;});
  await box.getByRole('button',{name:'Check domain',exact:true}).click();
  await box.getByText('SPF · Record found',{exact:true}).waitFor();
  assert.deepEqual(await page.evaluate(()=>window.domainRequests.at(-1)),{domain:'example.com',selector:'mail2026'});
  assert.deepEqual(problems,[]);
  await context.close();
});

test('new-account domain check runs automatically and never mixes late results between accounts', async () => {
  const {page,context,problems} = await openWindow();
  await page.locator('#open-settings').click();
  await page.locator('#settings-add').click();
  await page.evaluate(()=>{window.holdDomainCheck=true;});
  await page.locator('#settings-form input[name=email]').fill('new@first.example');
  await page.locator('#settings-form input[name=email]').press('Tab');
  await page.waitForFunction(()=>window.domainRequests?.length===1);
  await page.evaluate(()=>{window.holdDomainCheck=false;});
  await page.locator('#settings-form input[name=email]').fill('new@second.example');
  await page.locator('#settings-form input[name=email]').press('Tab');
  await page.locator('#account-domain-check .domain-checked-name').getByText('second.example',{exact:true}).waitFor();
  await page.evaluate(()=>window.releaseDomainCheck({domain:'first.example',checks:[]}));
  assert.equal(await page.locator('#account-domain-check .domain-checked-name').textContent(),'second.example');
  assert.deepEqual(problems,[]);
  await context.close();
});

test('optional threads keep one Inbox row and show read replies in an expandable conversation', async () => {
  const {page,context,problems} = await openWindow({threadSeed:true});
  await readOnOpen(page,true);
  const total = await page.evaluate(()=>state.total);
  await page.locator('#list-threads').click();
  await page.waitForFunction(()=>[...state.rows.values()].some(r=>r.thread_count===2));
  assert.equal(await page.evaluate(()=>state.total),total-1);
  assert.equal(await page.evaluate(()=>localStorage.getItem('threadMessages')),'yes');
  const group = page.locator('#content .row', {hasText:'2 messages'});
  await group.click();
  await page.locator('#mail-thread:not([hidden])').waitFor();
  assert.equal(await page.locator('.mail-thread-entry').count(),2);
  assert.equal(await page.evaluate(()=>state.total),total-1,'the list remains grouped');
  assert.deepEqual(await page.evaluate(()=>window.readChanges ?? []),[],'opening a group must not mark its members');
  await page.locator('#list-unread-filter').click();
  await page.waitForFunction(()=>state.filter.unreadOnly);
  await page.locator('#content .row', {hasText:'Message 03 about Rechnung'}).click();
  await page.locator('#mail-thread:not([hidden])').waitFor();
  assert.equal(await page.locator('.mail-thread-entry').count(),2,'read replies stay visible even from an unread-only group');
  const unread = page.locator('.mail-thread-entry').filter({has:page.locator('summary strong',{hasText:'●'})});
  await unread.locator('summary').click();
  await page.waitForFunction(()=>window.readChanges?.length===1);
  assert.ok((await page.locator('.mail-thread-entry[open] .mail-thread-body').innerText()).length > 0);
  await page.waitForFunction(()=>!state.rows.get(state.selected).unread);
  assert.equal(await page.locator('#content .row.selected .dot.on').count(),0,'the group dot clears after its final unread message');
  await page.locator('.mail-thread-entry[open] button').click();
  await page.locator('#reading:not([hidden])').waitFor();
  assert.equal(await page.locator('#list-threads').innerText(),'Back to threads');
  await page.locator('#list-threads').click();
  await page.waitForFunction(()=>state.filter.thread===null && state.filter.unreadOnly);
  await page.locator('#list-unread-filter').click();
  await page.locator('#list-threads').click();
  await page.waitForFunction(()=>!document.querySelector('#pref-thread-messages').checked && state.total===7);
  assert.deepEqual(problems,[]);
  await context.close();
});

test('unread filtering stays in mail, search, people and attention views', async () => {
  const { page, context, problems } = await openWindow();
  await page.locator('#list-unread-filter').click();
  await page.waitForFunction(() => state.filter.unreadOnly && [...state.rows.values()].every(row => row.unread));
  await page.locator('#folders .nav-item', {hasText:'Inbox'}).click();
  assert.equal(await page.locator('#list-unread-filter').getAttribute('aria-pressed'), 'true');
  await page.locator('#search').fill('Message');
  await page.locator('#search').press('Enter');
  await page.waitForFunction(() => state.searching && state.total > 0);
  assert.equal(await page.evaluate(() => [...state.rows.values()].every(row => row.unread)), true);
  await page.locator('#list-unread-filter').click();
  await page.waitForFunction(() => state.searching && !state.filter.unreadOnly && [...state.rows.values()].some(row => !row.unread));
  assert.equal(await page.locator('#search').inputValue(), 'Message');

  await page.locator('#search').fill('');
  await page.locator('#scope .view-toggle button', {hasText:'People'}).click();
  await page.locator('#list-unread-filter').click();
  await page.waitForFunction(() => state.view === 'people' && state.filter.unreadOnly && [...state.rows.values()].every(row => row.unread));
  await page.evaluate(() => {
    const row = [...state.rows.values()][0];
    window.testUrgency = [true, false].map((unread,i) => ({row:{...row,id:100+i,unread},urgency:{score:3,reason:'test',action:'read'}}));
    return showView('urgent');
  });
  assert.equal(await page.evaluate(() => state.total), 1);
  await page.locator('#list-unread-filter').click();
  await page.waitForFunction(() => state.view === 'urgent' && state.total === 2);
  assert.deepEqual(problems, []);
  await context.close();
});

test('attention navigation remains visible while its next count is loading', async () => {
  const { page, context, problems } = await openWindow();
  await page.locator('#attention-nav button').waitFor();
  await page.evaluate(() => { window.holdUrgency = true; window.attentionRefresh = renderAttentionNav(); });
  assert.equal(await page.locator('#attention-nav button').isVisible(), true);
  await page.evaluate(async () => { window.holdUrgency = false; window.releaseUrgency([{},{}]); await window.attentionRefresh; });
  assert.match(await page.locator('#attention-nav button').innerText(), /2/);
  assert.deepEqual(problems, []);
  await context.close();
});

test('assistant starter, Send, Enter and Retry submit the intended text', async () => {
  const { page, context, problems } = await openWindow();
  await page.locator('#assistant-toggle').click();
  await page.getByRole('button', { name: 'Find what matters', exact: true }).click();
  const prompt = await page.locator('#chat-input').inputValue();
  assert.ok(prompt.length > 0);
  assert.deepEqual(await page.evaluate(() => window.assistantRequests ?? []), []);
  await page.locator('#chat-send').click();
  await page.locator('.chat-msg.assistant').waitFor();
  assert.equal(await page.locator('#chat-input').inputValue(), '');
  assert.deepEqual(await page.evaluate(() => window.assistantRequests), [prompt]);

  await page.locator('#chat-input').fill('Follow up');
  await page.locator('#chat-input').press('Shift+Enter');
  assert.equal(await page.locator('.chat-msg.user').count(), 1);
  await page.evaluate(() => { window.failNextAssistant = true; });
  await page.locator('#chat-input').press('Enter');
  await page.locator('.chat-msg.error .retry').waitFor();
  await page.locator('.chat-msg.error .retry').click();
  await page.waitForFunction(() => document.querySelectorAll('.chat-msg.assistant').length === 2);
  assert.deepEqual(await page.evaluate(() => window.assistantRequests), [prompt, 'Follow up', 'Follow up']);
  assert.equal(await page.locator('.chat-msg.user').count(), 2);
  assert.equal(await page.locator('#chat-send').isEnabled(), true);
  assert.deepEqual(problems, []);
  await context.close();
});

test('chat model picker saves only the assistant choice and marks hosted models red', async () => {
  const { page, context, problems } = await openWindow();
  await page.evaluate(async () => {
    const invoke = await window.__fakeBridge;
    window.tasksBefore = await invoke('ai_tasks');
    window.hostedId = await invoke('save_ai_provider', { input: {id:null,kind:'anthropic',label:'Claude',base_url:'https://api.anthropic.com/v1'} });
    await invoke('set_ai_key', {id:window.hostedId,key:'test-only'});
  });
  await page.locator('#assistant-toggle').click();
  await page.locator('#chat-model summary').click();
  await page.locator('#chat-model-choice:not([disabled])').waitFor();
  const hosted = await page.evaluate(() => String(window.hostedId));
  await page.locator('#chat-provider').selectOption(hosted);
  await page.waitForFunction(() => document.querySelector('#chat-model-choice')?.options.length > 1 && !document.querySelector('#chat-model-choice').disabled);
  assert.equal(await page.locator('#chat-model-save').isDisabled(), true);
  const offered = await page.locator('#chat-model-choice option').evaluateAll(options => options.map(o => o.value).filter(Boolean));
  assert.ok(offered.length > 1);
  await page.locator('#chat-model-choice').selectOption(offered[1]);
  await page.locator('#chat-model-save').click();
  await page.locator('#chat-model.warn summary').waitFor();
  assert.match(await page.locator('#chat-model summary').innerText(), /Online/);
  const result = await page.evaluate(async () => ({before:window.tasksBefore,after:await (await window.__fakeBridge)('ai_tasks')}));
  assert.deepEqual(result.after.filter(t => t.task !== 'assistant'), result.before.filter(t => t.task !== 'assistant'));
  assert.equal(result.after.find(t => t.task === 'assistant').model, offered[1]);
  assert.equal(await page.locator('#chat-model').evaluate(node => getComputedStyle(node).color), 'rgb(194, 54, 44)');

  await page.locator('#chat-model summary').click();
  await page.locator('#chat-model-choice:not([disabled])').waitFor();
  await page.locator('#chat-provider').selectOption('1');
  await page.waitForFunction(() => document.querySelector('#chat-model-choice')?.options.length > 1 && !document.querySelector('#chat-model-choice').disabled);
  const local = await page.locator('#chat-model-choice option').first().getAttribute('value');
  await page.locator('#chat-model-choice').selectOption(local);
  await page.locator('#chat-model-save').click();
  await page.waitForFunction(() => document.querySelector('#chat-model summary')?.textContent.includes('Local'));
  assert.equal(await page.locator('#chat-model.warn').count(), 0);
  assert.deepEqual(problems, []);
  await context.close();
});

test('an open message lists its attachments, and a picture shows in safe preview', async () => {
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
  // Drawn, and decoded: the page is a PNG kuverta wrote, with a size. The
  // file itself never reaches the window.
  await page.waitForFunction(() => {
    const image = document.querySelector('#attachment-pages img');
    return image?.complete && image.naturalWidth === 1;
  });
  assert.match(await page.locator('#attachment-pages img').getAttribute('src'), /^data:image\/png;base64,/);
  assert.equal(await page.locator('#attachment-image').isHidden(), true);
  assert.equal(await page.locator('#attachment-safe').isVisible(), true);

  await page.locator('#attachment-save').click();
  await page.waitForSelector('#toast:not([hidden])');
  assert.match(await page.locator('#toast').innerText(), /saved to .*Downloads\/eSIM QR\.png/);

  // Another app is outside safe preview: the first click only says so.
  await page.locator('#attachment-open').click();
  assert.equal(await page.locator('#attachment-open').innerText(), 'Open anyway?');
  assert.match(await page.locator('#attachment-status').innerText(), /outside safe preview/);
  assert.deepEqual((await log(page)).opened, []);
  await page.locator('#attachment-open').click();
  const after = await log(page);
  assert.deepEqual(after.saved, ['eSIM QR.png']);
  assert.deepEqual(after.opened, ['eSIM QR.png']);
  assert.deepEqual(after.previewed.map((p) => p.name), ['eSIM QR.png']);

  // Escape closes it and lets the pages go; opened again, it asks again.
  await page.keyboard.press('Escape');
  await page.waitForSelector('#attachment-sheet', { state: 'hidden' });
  assert.equal(await page.locator('#attachment-pages img').count(), 0);
  await chips.filter({ hasText: 'eSIM QR.png' }).click();
  await page.waitForSelector('#attachment-pages img');
  assert.equal(await page.locator('#attachment-open').innerText(), 'Open in another app');

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
  // Safe preview reads the page's words out of it; the page itself is never
  // shown, and its scripts never run.
  await page.waitForSelector('#attachment-text:not([hidden])');
  assert.match(await page.locator('#attachment-text').innerText(), /Bitte melden Sie sich an/);
  assert.equal(await page.locator('#attachment-safe').isVisible(), true);
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
  await page.waitForFunction(() => document.querySelector('#attachment-pages img')?.naturalWidth === 1);
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

test('a PDF in safe preview comes a few pages at a time', async () => {
  const { page, context, problems } = await openWindow();

  await page.locator('#content .row', { hasText: 'Ihre neue e-SIM ist da' }).click();
  await page.locator('#reading-attachments .attachment-chip', { hasText: 'Vertrag.pdf' }).click();
  await page.waitForFunction(() => document.querySelectorAll('#attachment-pages img').length === 2);
  const more = page.locator('#attachment-more');
  assert.equal(await more.innerText(), 'More pages (2 of 3)');
  await more.click();
  await page.waitForFunction(() => document.querySelectorAll('#attachment-pages img').length === 3);
  assert.equal(await more.isHidden(), true);
  // The button stays last, under the pages.
  assert.equal(await page.evaluate(() => document.querySelector('#attachment-pages').lastElementChild.id), 'attachment-more');
  assert.deepEqual(
    (await log(page)).previewed.map(({ first, count }) => [first, count]),
    [
      [0, 2],
      [2, 4],
    ],
  );
  // No frame was ever given the file.
  assert.equal(await page.locator('#attachment-pdf').getAttribute('src'), null);

  assert.deepEqual(problems, []);
  await context.close();
});

test('with safe preview off in Settings, the system viewer shows it and opens it at once', async () => {
  const { page, context, problems } = await openWindow();

  await page.locator('#open-settings').click();
  await page.locator('#settings-nav .nav-item', { hasText: /^General$/ }).click();
  const pref = page.locator('#pref-safe-preview');
  assert.equal(await pref.isChecked(), true, 'on unless turned off');
  await pref.uncheck();
  await page.keyboard.press('Escape');

  await page.locator('#content .row', { hasText: 'Ihre neue e-SIM ist da' }).click();
  await page.locator('#reading-attachments .attachment-chip', { hasText: 'eSIM QR.png' }).click();
  await page.waitForFunction(() => {
    const image = document.getElementById('attachment-image');
    return !image.hidden && image.complete && image.naturalWidth === 1;
  });
  assert.equal(await page.locator('#attachment-safe').isHidden(), true);
  await page.locator('#attachment-open').click();
  assert.deepEqual((await log(page)).opened, ['eSIM QR.png']);
  assert.deepEqual((await log(page)).previewed, []);

  assert.deepEqual(problems, []);
  await context.close();
});

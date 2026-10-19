import assert from 'node:assert/strict';
import {before,after,test} from 'node:test';
import http from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve,extname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {chromium} from 'playwright';
const ROOT=fileURLToPath(new URL('../../',import.meta.url));
let server,browser,base;
before(async()=>{
  server=http.createServer(async(req,res)=>{
    const path=resolve(ROOT,'.'+new URL(req.url,'http://x').pathname);
    if(!path.startsWith(ROOT)){res.writeHead(403).end();return;}
    try {res.setHeader('Content-Type',({'.html':'text/html','.js':'text/javascript','.css':'text/css'})[extname(path)]??'application/octet-stream');res.end(await readFile(path));}
    catch{res.writeHead(404).end();}
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));base=`http://127.0.0.1:${server.address().port}`;
  browser=await chromium.launch();
});
after(async()=>{await browser?.close();server?.close();});
async function open(locale='en-US',chat=false,unconfigured=false) {
  const context=await browser.newContext({viewport:{width:1100,height:800},locale});
  const page=await context.newPage(),errors=[];page.on('pageerror',e=>errors.push(e.message));
  page.setDefaultTimeout(8000);
  await page.addInitScript(({unconfigured})=>{
    if(window!==window.top)return;
    Object.defineProperty(navigator,'platform',{value:'Win32'});
    window.invoiceCalls=[];
    const setup={company:'Test Company GmbH',aliases:[],address:'',people:[],incoming:true,outgoing:false,credits:true,review_terms:['test invoice'],accounts:[1,2],root:'C:/Test invoices',period:'quarter',custom_from:'',custom_to:''};
    window.invoiceSetup=JSON.parse(localStorage.getItem('fixture.invoice.setup')??'null')??{revision:unconfigured?0:1,confirmed:unconfigured?null:setup,draft:null,needs_confirmation:unconfigured};
    window.invoiceCorrections=JSON.parse(localStorage.getItem('fixture.invoice.corrections')??'{}');
    window.invoicePreview=JSON.parse(localStorage.getItem('fixture.invoice.preview')??'null');
    const remember=()=>{localStorage.setItem('fixture.invoice.preview',JSON.stringify(window.invoicePreview));};
    const enrich=()=>{window.invoicePreview.rows=window.invoicePreview.rows.map(r=>({...r,review:window.invoiceCorrections[r.digest]??null}));remember();return window.invoicePreview;};
    let bridge;
    const attachment={index:0,name:'invoice.pdf',content_type:'application/pdf',size:100,inline:false,risky:false,preview:'pdf'};
    const row={account:1,message:1,subject:'September invoice',sender:'vendor@example.test',mail_date:1790899200,attachment,digest:'a',issue:null,previous_path:null,duplicate_of:null,same_name:false,eligibility:{status:'suggested',direction:'incoming',reason:'invoice_profile_matches'}};
    window.__TAURI__={core:{Channel:class{},invoke:async(command,args)=>{
      bridge??=import('/kuverta-bird/test/support/fake-invoke.js').then(m=>m.fakeInvoke({paper:{paperMailboxes:[],documents:[]}}));
      if(command==='profiles')return [{id:10,name:'Example Company',accounts:[1,2],postboxes:[]},{id:20,name:'Private',accounts:[],postboxes:[]}];
      if(command==='pick_invoice_folder'){window.invoiceCalls.push({command,args});if(window.failPicker)throw Error('Folder chooser failed');return window.pickedFolder??null;}
      if(command==='invoice_settings'){window.invoiceCalls.push({command,args});return {root:window.noInvoiceDestination?null:'C:/Test invoices',exports:[],setup:window.invoiceSetup,resumable:window.invoicePreview?.complete?window.invoicePreview:null};}
      if(command==='save_invoice_setup'){
        window.invoiceCalls.push({command,args});if(window.failSetup)throw Error('Cannot save setup');
        window.invoiceSetup=args.confirm?{revision:window.invoiceSetup.revision+1,confirmed:args.setup,draft:null,needs_confirmation:false}:{...window.invoiceSetup,draft:args.setup};
        localStorage.setItem('fixture.invoice.setup',JSON.stringify(window.invoiceSetup));return window.invoiceSetup;
      }
      if(command==='save_invoice_review'){
        window.invoiceCalls.push({command,args});if(window.delayReview)await new Promise(r=>setTimeout(r,150));if(window.failReview)throw Error('Cannot save correction');
        const digest=window.invoicePreview.rows[args.row].digest;
        if(args.review)window.invoiceCorrections[digest]=args.review;else delete window.invoiceCorrections[digest];
        localStorage.setItem('fixture.invoice.corrections',JSON.stringify(window.invoiceCorrections));return enrich();
      }
      if(command==='assistant_ask') {
        window.invoiceCalls.push({command,args:{message:args.message,account:args.account}});
        const event={kind:'invoice_collection_suggested',proposal:{profile:10,profile_name:'Example Company',from:'2026-07-01',to:'2026-09-30'}};
        args.onEvent?.onmessage?.(event);
        return {turns:[],reply:'Die gemeinsame Vorschau ist angefordert. Es wurde noch nichts abgelegt.',events:[event],local:true,model:'test'};
      }
      if(command==='save_chat'){window.savedInvoiceChat={id:77,account_id:args.account,turns:args.turns,log:args.log};return {id:77};}
      if(command==='chat')return window.savedInvoiceChat;
      if(command==='start_invoice_collection'){
        window.invoiceCalls.push({command,args});window.invoicePreview={token:'draft',layout:window.invoiceSetup.confirmed.layout??'flat',setup_revision:window.invoiceSetup.revision,request:args.request,profile_name:'Example Company',accounts:[{id:1,email:'one@example.test'},{id:2,email:'two@example.test'}],scanned:0,total:100,complete:false,limited:false,rows:[]};return window.invoicePreview;
      }
      if(command==='scan_invoice_collection'){
        window.invoiceCalls.push({command,args});window.invoicePreview={...window.invoicePreview,scanned:100,complete:true,rows:[row,{...row,account:2,message:2,duplicate_of:0},{...row,message:3,attachment:null,digest:null,issue:'no_attachment'},{...row,message:4,digest:'b',previous_path:'C:/Test invoices/old.pdf'}]};
        if(window.mixedInvoiceKinds) {
          for(const [n,name] of ['Resolution.pdf','Price list.pdf','Project outline.pdf','Meeting notes.pdf'].entries())window.invoicePreview.rows.push({...row,message:5+n,digest:`other${n}`,attachment:{...attachment,name},analysis:{kind:'other',supported:false,reason:'Not an issued invoice'}});
          window.invoicePreview.rows.push({...row,message:9,digest:'outside',analysis:{kind:'invoice',invoice_date:'2026-10-07',supported:true}}, {...row,message:10,digest:'unknown',analysis:{kind:'invoice',invoice_date:'',supported:false}});
        }
        if(window.legacyInvoiceAnalysis)window.invoicePreview.rows[0]={...row,analysis:{kind:'invoice',invoice_date:'',supported:false},analysis_needs_refresh:true};
        return enrich();
      }
      if(command==='analyze_invoice_document'){
        window.invoiceCalls.push({command,args});
        if(window.skipScannedDocument && args.row===0) {
          window.invoicePreview.rows[0]={...window.invoicePreview.rows[0],analysis_error:'invoice_text_unavailable'};
          return window.invoicePreview;
        }
        const analysis={kind:'invoice',invoice_date:'2026-09-15',issuer:'Test GmbH',number:'R-42',total:'119,00',currency:'EUR',model:'fixture',supported:true,date_quote:'Rechnungsdatum: 15.09.2026',reason:'Printed invoice date'};
        const preview=window.invoicePreview;
        preview.rows=preview.rows.map(r=>r.digest===preview.rows[args.row].digest?{...r,analysis,analysis_needs_refresh:false,versions:window.hasVersions?[3]:[],previous_versions:[]}:r);
        return enrich();
      }
      if(command==='confirm_invoice_collection'){
        window.invoiceCalls.push({command,args});if(window.failExport)throw Error('Profile accounts changed. Start a new preview.');
        return {token:'run',profile_name:'Example Company',created_at:1792250000,manifest:'C:/Test invoices/log.json',outcomes:[{row:0,name:'invoice.pdf',invoice_date:args.selection[0].invoice_date,kind:args.selection[0].kind,status:'saved',path:'C:/Test invoices/invoice.pdf'}]};
      }
      if(command==='message')return {body_text:'Invoice is in the portal.'};
      return (await bridge)(command,args);
    }}};
  },{unconfigured});
  await page.goto(`${base}/apps/desktop/ui/index.html`);
  await page.waitForFunction(()=>document.getElementById('status')?.textContent==='you@example.com');
  if(chat) {
    await page.locator('#assistant-toggle').click();await page.locator('#chat-input').waitFor();
    return {page,context,errors};
  }
  await page.keyboard.press(',');
  await page.locator('#settings-nav').getByText(locale==='de-DE'?'Profile':'Profiles',{exact:true}).click();
  await page.locator('[data-invoice-profile="10"]').click();
  if(unconfigured){await page.locator('#invoice-setup-form').waitFor();return {page,context,errors};}
  await page.locator('#invoice-root').waitFor();
  await page.locator('#invoice-from').fill('2026-07-01');await page.locator('#invoice-to').fill('2026-09-30');
  return {page,context,errors};
}
async function build(page){await page.locator('#invoice-analyze').uncheck();await page.locator('#invoice-form button[type=submit]').click();await page.locator('#invoice-rows .invoice-row').first().waitFor();}
test('joint preview is profile-bound, opt-in and dates are checked; cancel writes nothing',async()=>{
  const {page,context,errors}=await open();await build(page);
  assert.match(await page.locator('#invoice-accounts').innerText(),/one@example.test.*two@example.test/);
  assert.equal(await page.locator('#invoice-rows input[type=checkbox]').count(),1,'only the new unique attachment is selectable');
  assert.equal(await page.locator('#invoice-export').isDisabled(),true);
  const row=page.locator('.invoice-row[data-row="0"]');await row.locator('[type=checkbox]').check();
  await row.locator('[type=date]').fill('2026-10-01');assert.equal(await page.locator('#invoice-export').isDisabled(),true);
  await row.locator('[type=date]').fill('2026-09-30');await row.locator('select').selectOption('credit');
  await page.locator('#invoice-export').click();await page.locator('#ask-no').click();
  assert.equal((await page.evaluate(()=>window.invoiceCalls.filter(c=>c.command==='confirm_invoice_collection'))).length,0);
  await page.locator('#invoice-export').click();await page.locator('#ask-yes').click();
  await page.locator('.invoice-receipt').waitFor();
  const calls=await page.evaluate(()=>window.invoiceCalls);
  assert.equal(calls.find(c=>c.command==='start_invoice_collection').args.request.profile,10);
  assert.deepEqual(calls.find(c=>c.command==='confirm_invoice_collection').args,{token:'draft',selection:[{row:0,invoice_date:'2026-09-30',kind:'credit'}]});
  assert.deepEqual(errors,[]);await context.close();
});
test('German preview fits a smaller window and keeps missing/duplicate notices visible',async()=>{
  const {page,context,errors}=await open('de-DE');await page.setViewportSize({width:850,height:650});await build(page);
  assert.match(await page.locator('#invoice-rows').innerText(),/Portal-Link/);
  await page.locator('#invoice-filter').selectOption('duplicates');assert.equal(await page.locator('.invoice-row').count(),2);
  await page.locator('#invoice-filter').selectOption('issues');
  await page.locator('.invoice-row summary').click();await page.getByText('Invoice is in the portal.',{exact:true}).waitFor();
  const geometry=await page.locator('#invoice-close').boundingBox();assert.ok(geometry.x>=0&&geometry.y+geometry.height<=650);
  assert.deepEqual(errors,[]);await context.close();
});
test('failed confirmation stays reviewable and can be retried',async()=>{
  const {page,context,errors}=await open();await build(page);
  const row=page.locator('.invoice-row[data-row="0"]');await row.locator('[type=checkbox]').check();await row.locator('[type=date]').fill('2026-09-03');
  await page.evaluate(()=>window.failExport=true);await page.locator('#invoice-export').click();await page.locator('#ask-yes').click();
  await page.locator('#toast').getByText(/Profile accounts changed/).waitFor();
  assert.equal(await row.locator('[type=checkbox]').isChecked(),true);assert.equal(await page.locator('#invoice-close').isEnabled(),true);
  assert.deepEqual(errors,[]);await context.close();
});

test('AI pre-fills a shared review, uses one call per digest, and keeps possible versions unselected',async()=>{
  const {page,context,errors}=await open();
  await page.locator('#invoice-form button[type=submit]').click();await page.locator('#invoice-rows .invoice-row').first().waitFor();
  const row=page.locator('.invoice-row[data-row="0"]');
  assert.equal(await row.locator('[type=checkbox]').isChecked(),true);
  assert.equal(await row.locator('[type=date]').inputValue(),'2026-09-15');
  assert.equal(await page.locator('#invoice-export').isEnabled(),true);
  const requests=await page.evaluate(()=>window.invoiceCalls.filter(c=>c.command==='analyze_invoice_document'));
  assert.equal(requests.length,2,'two distinct digests; cross-account copy is not read twice');
  assert.equal((await page.evaluate(()=>window.invoiceCalls.filter(c=>c.command==='confirm_invoice_collection'))).length,0);
  await page.getByRole('button',{name:'Start again',exact:true}).click();
  await page.locator('#invoice-from').fill('2026-07-01');await page.locator('#invoice-to').fill('2026-09-30');
  await page.evaluate(()=>{window.hasVersions=true;});
  await page.locator('#invoice-form button[type=submit]').click();await page.locator('#invoice-rows .invoice-row').first().waitFor();
  assert.equal(await page.locator('.invoice-row[data-row="0"] [type=checkbox]').isChecked(),false);
  assert.match(await page.locator('#invoice-rows').innerText(),/Possible versions/);
  assert.deepEqual(errors,[]);await context.close();
});

test('unreadable scans stay manual and do not prevent subsequent text documents being analyzed',async()=>{
  const {page,context,errors}=await open();
  await page.evaluate(()=>{window.skipScannedDocument=true;});
  await page.locator('#invoice-form button[type=submit]').click();await page.locator('#invoice-filter').waitFor();await page.locator('#invoice-filter').selectOption('review');
  assert.match(await page.locator('.invoice-row[data-row="0"]').innerText(),/Scans and images are not analyzed/);
  assert.equal(await page.locator('.invoice-row[data-row="0"] [type=checkbox]').isChecked(),false);
  assert.equal((await page.evaluate(()=>window.invoiceCalls.filter(c=>c.command==='analyze_invoice_document'))).length,2);
  assert.deepEqual(errors,[]);await context.close();
});

test('invoice view separates other documents, other periods and unresolved dates without losing evidence',async()=>{
  const {page,context,errors}=await open('de-DE');await page.evaluate(()=>{window.mixedInvoiceKinds=true;});
  await page.locator('#invoice-form button[type=submit]').click();await page.locator('#invoice-filter').waitFor();
  assert.equal(await page.locator('#invoice-filter').inputValue(),'invoices');
  assert.equal(await page.locator('.invoice-row').count(),1);
  await page.locator('#invoice-filter').selectOption('other');assert.equal(await page.locator('.invoice-row').count(),4);
  assert.equal(await page.locator('.invoice-row input[type=checkbox]:checked').count(),0);
  await page.locator('#invoice-filter').selectOption('outside');assert.equal(await page.locator('.invoice-row').count(),1);
  await page.locator('#invoice-filter').selectOption('review');assert.ok(await page.locator('.invoice-row[data-row="9"]').count());
  assert.equal(await page.getByText('Price list.pdf',{exact:true}).count(),0);
  await page.locator('#invoice-filter').selectOption('all');assert.equal(await page.locator('.invoice-row').count(),10);
  await page.locator('#invoice-filter').selectOption('selected');assert.equal(await page.locator('.invoice-row').count(),1);
  assert.equal((await page.evaluate(()=>window.invoiceCalls.filter(c=>c.command==='confirm_invoice_collection'))).length,0);
  assert.deepEqual(errors,[]);await context.close();
});

test('cached dates needing local validation are refreshed before suggesting a selection',async()=>{
  const {page,context,errors}=await open();await page.evaluate(()=>{window.legacyInvoiceAnalysis=true;});
  await page.locator('#invoice-form button[type=submit]').click();await page.locator('#invoice-filter').waitFor();
  assert.ok((await page.evaluate(()=>window.invoiceCalls)).some(c=>c.command==='analyze_invoice_document'&&c.args.row===0));
  assert.equal(await page.locator('.invoice-row[data-row="0"] [type=checkbox]').isChecked(),true);
  assert.equal(await page.locator('.invoice-row[data-row="0"] [type=date]').inputValue(),'2026-09-15');
  assert.deepEqual(errors,[]);await context.close();
});

async function requestFromChat(page) {
  await page.locator('#chat-input').fill('Sammle alle Rechnungen für Example Company aus Q3 2026');
  await page.locator('#chat-send').click();
}

test('chat request opens the joint profile preview with the requested dates; only explicit approval saves',async()=>{
  const {page,context,errors}=await open('de-DE',true);
  await requestFromChat(page);
  await page.locator('#invoice-rows .invoice-row').first().waitFor();
  assert.match(await page.locator('#invoice-title').innerText(),/Example Company/);
  const requests=await page.evaluate(()=>window.invoiceCalls);
  const start=requests.find(c=>c.command==='start_invoice_collection').args.request;
  assert.deepEqual({profile:start.profile,from:start.from,to:start.to},{profile:10,from:'2026-07-01',to:'2026-09-30'});
  assert.equal(start.root,'C:/Test invoices');
  assert.ok(start.mail_from<start.from && start.mail_to>start.to,'receipt-date range includes later delivery');
  assert.equal(requests.filter(c=>c.command==='confirm_invoice_collection').length,0);
  assert.equal(await page.locator('.invoice-collection-card').count(),1);
  await page.locator('#invoice-export').click();await page.locator('#ask-no').click();
  assert.equal((await page.evaluate(()=>window.invoiceCalls.filter(c=>c.command==='confirm_invoice_collection'))).length,0);
  await page.locator('#invoice-export').click();await page.locator('#ask-yes').click();
  await page.locator('.invoice-receipt').waitFor();
  assert.equal((await page.evaluate(()=>window.invoiceCalls.filter(c=>c.command==='confirm_invoice_collection'))).length,1);
  assert.deepEqual(errors,[]);await context.close();
});

test('chat collection without a saved destination waits for the person and retains the requested quarter',async()=>{
  const {page,context,errors}=await open('de-DE',true);
  await page.evaluate(()=>{window.noInvoiceDestination=true;});
  await requestFromChat(page);await page.locator('#invoice-root').waitFor();
  await page.getByText(/Wähle den Ablageordner dieses Profils/).waitFor();
  assert.equal(await page.locator('#invoice-from').inputValue(),'2026-07-01');
  assert.equal(await page.locator('#invoice-to').inputValue(),'2026-09-30');
  assert.equal((await page.evaluate(()=>window.invoiceCalls.filter(c=>c.command==='start_invoice_collection'||c.command==='analyze_invoice_document'))).length,0);
  await page.locator('#invoice-root').fill('C:/Chosen Example Company invoices');
  await page.locator('#invoice-form button[type=submit]').click();
  await page.locator('#invoice-rows .invoice-row').first().waitFor();
  assert.equal((await page.evaluate(()=>window.invoiceCalls.find(c=>c.command==='start_invoice_collection').args.request.root)),'C:/Chosen Example Company invoices');
  assert.deepEqual(errors,[]);await context.close();
});

test('reopening a saved chat only shows its invoice card; reopening its preview is a deliberate action',async()=>{
  const {page,context,errors}=await open('en-US',true);
  await requestFromChat(page);await page.locator('#invoice-rows .invoice-row').first().waitFor();
  await page.locator('#invoice-close').click();
  await page.evaluate(()=>openChat(77));
  assert.equal(await page.locator('#invoice-sheet').isHidden(),true);
  assert.equal((await page.evaluate(()=>window.invoiceCalls.filter(c=>c.command==='start_invoice_collection'))).length,1);
  await page.locator('.invoice-collection-card').getByRole('button',{name:'Open joint preview',exact:true}).click();
  await page.locator('#invoice-rows .invoice-row').first().waitFor();
  assert.equal((await page.evaluate(()=>window.invoiceCalls.filter(c=>c.command==='start_invoice_collection'))).length,2);
  assert.equal((await page.evaluate(()=>window.invoiceCalls.filter(c=>c.command==='confirm_invoice_collection'))).length,0);
  assert.deepEqual(errors,[]);await context.close();
});

async function wizardNext(page){await page.locator('#invoice-setup-form button[type=submit]').click();}
async function reopenProfiles(page){
  await page.keyboard.press(',');await page.locator('#settings-nav').getByText('Profiles',{exact:true}).click();
  await page.locator('[data-invoice-profile="10"]').click();
}
test('wizard preserves drafts, confirms each profile and supports a custom period across years without analyzing mail',async()=>{
  const {page,context,errors}=await open('en-US',false,true);
  await page.locator('#invoice-setup-company').fill('Example Company GmbH');
  await page.locator('#invoice-setup-aliases').fill('Example Co GmbH');
  await page.locator('#invoice-close').click();await page.locator('#invoice-sheet').waitFor({state:'hidden'});
  await page.reload();await page.waitForFunction(()=>document.getElementById('status')?.textContent==='you@example.com');await reopenProfiles(page);
  assert.equal(await page.locator('#invoice-setup-company').inputValue(),'Example Company GmbH');
  await wizardNext(page);await page.locator('#invoice-setup-review_terms').waitFor();
  await wizardNext(page);await page.locator('#invoice-setup-accounts').check();await wizardNext(page);
  await page.locator('#invoice-setup-root').fill('C:/Test invoices');
  assert.equal(await page.locator('#invoice-setup-layout').inputValue(),'year_quarter_month');
  await page.evaluate(()=>window.pickedFolder='C:/Chosen invoices');
  await page.locator('.invoice-folder-picker').click();
  assert.equal(await page.locator('#invoice-setup-root').inputValue(),'C:/Chosen invoices');
  assert.equal(await page.locator('#invoice-setup-custom_from').isVisible(),false);
  await page.locator('#invoice-setup-period').selectOption('custom');
  assert.equal(await page.locator('#invoice-setup-custom_from').isVisible(),true);
  await page.locator('#invoice-setup-custom_from').fill('2025-11-15');await page.locator('#invoice-setup-custom_to').fill('2026-02-09');await wizardNext(page);
  await page.getByText('Review setup',{exact:true}).waitFor();assert.match(await page.locator('.invoice-setup-summary').innerText(),/2025-11-15.*2026-02-09/);
  await page.getByRole('button',{name:'Confirm setup',exact:true}).click();await page.locator('#invoice-form').waitFor();
  assert.equal(await page.locator('#invoice-from').inputValue(),'2025-11-15');assert.equal(await page.locator('#invoice-to').inputValue(),'2026-02-09');
  const calls=await page.evaluate(()=>window.invoiceCalls);
  assert.equal(calls.filter(c=>['start_invoice_collection','analyze_invoice_document','confirm_invoice_collection'].includes(c.command)).length,0);
  assert.deepEqual(calls.find(c=>c.command==='save_invoice_setup'&&c.args.confirm).args.setup.accounts,[1,2]);
  assert.equal(calls.find(c=>c.command==='save_invoice_setup'&&c.args.confirm).args.setup.root,'C:/Chosen invoices');
  assert.equal(calls.find(c=>c.command==='save_invoice_setup'&&c.args.confirm).args.setup.layout,'year_quarter_month');
  assert.deepEqual(errors,[]);await context.close();
});

test('month year and arbitrary collection dates keep invoice and mail periods explicit',async()=>{
  const {page,context,errors}=await open();
  await page.locator('#invoice-period').selectOption('month');
  const dates=await page.evaluate(()=>{const now=new Date(),end=new Date(now.getFullYear(),now.getMonth(),0);return {month:invoiceDate(new Date(now.getFullYear(),now.getMonth()-1,1)),end:invoiceDate(end),year:`${now.getFullYear()-1}-01-01`};});
  assert.equal(await page.locator('#invoice-from').inputValue(),dates.month);assert.equal(await page.locator('#invoice-to').inputValue(),dates.end);
  await page.locator('#invoice-period').selectOption('year');assert.equal(await page.locator('#invoice-from').inputValue(),dates.year);
  await page.locator('#invoice-from').fill('2025-12-12');await page.locator('#invoice-to').fill('2026-02-03');
  await page.getByRole('button',{name:/Adjust mail search/}).click();assert.equal(await page.locator('#invoice-period').inputValue(),'custom');
  await build(page);const request=await page.evaluate(()=>window.invoiceCalls.find(c=>c.command==='start_invoice_collection').args.request);
  assert.equal(request.from,'2025-12-12');assert.equal(request.to,'2026-02-03');assert.equal(request.mail_from,'2025-11-11');assert.equal(request.mail_to,'2026-03-06');
  assert.deepEqual(errors,[]);await context.close();
});

test('folder picker cancel and failure retain the path and never start a collection',async()=>{
  const {page,context,errors}=await open();
  await page.locator('.invoice-folder-picker').click();
  assert.equal(await page.locator('#invoice-root').inputValue(),'C:/Test invoices');
  await page.evaluate(()=>window.failPicker=true);await page.locator('.invoice-folder-picker').click();
  assert.equal(await page.locator('#invoice-root').inputValue(),'C:/Test invoices');
  await page.evaluate(()=>{window.failPicker=false;window.pickedFolder='C:/Selected invoices';});
  await page.locator('.invoice-folder-picker').click();
  assert.equal(await page.locator('#invoice-root').inputValue(),'C:/Selected invoices');
  assert.equal(await page.evaluate(()=>window.invoiceCalls.filter(c=>c.command==='start_invoice_collection').length),0);
  assert.deepEqual(errors,[]);await context.close();
});

test('preview and final approval show date folders and follow corrected invoice dates',async()=>{
  const {page,context,errors}=await open();
  await page.evaluate(()=>window.invoiceSetup.confirmed.layout='year_quarter_month');
  await page.locator('#invoice-form button[type=submit]').click();await page.locator('#invoice-filter').waitFor();
  const row=page.locator('.invoice-row[data-row="0"]');
  assert.match(await row.locator('.invoice-row-destination').innerText(),/2026\/Q3\/09/);
  await row.locator('[type=date]').fill('2026-07-03');
  await page.waitForFunction(()=>window.invoiceCorrections.a?.invoice_date==='2026-07-03');
  assert.match(await row.locator('.invoice-row-destination').innerText(),/2026\/Q3\/07/);
  await page.locator('#invoice-export').click();
  assert.match(await page.locator('#ask-sheet').innerText(),/2026\/Q3\/07/);
  assert.equal(await page.evaluate(()=>window.invoiceCalls.filter(c=>c.command==='confirm_invoice_collection').length),0);
  assert.deepEqual(errors,[]);await context.close();
});

test('explicit corrections survive reload, resume and new previews while reset restores only the suggestion',async()=>{
  const {page,context,errors}=await open();await page.locator('#invoice-form button[type=submit]').click();await page.locator('#invoice-filter').waitFor();
  let row=page.locator('.invoice-row[data-row="0"]');await row.locator('[type=checkbox]').uncheck();await row.getByRole('textbox',{name:'Correction reason',exact:true}).fill('Private transaction');await row.getByRole('button',{name:'Save correction',exact:true}).click();
  await page.waitForFunction(()=>window.invoiceCorrections.a?.reason==='Private transaction');
  await page.reload();await page.waitForFunction(()=>document.getElementById('status')?.textContent==='you@example.com');await reopenProfiles(page);await page.locator('#invoice-resume').click();
  row=page.locator('.invoice-row[data-row="0"]');assert.equal(await row.locator('[type=checkbox]').isChecked(),false);assert.equal(await row.getByRole('textbox',{name:'Correction reason',exact:true}).inputValue(),'Private transaction');
  await page.getByRole('button',{name:'Start again',exact:true}).click();await page.locator('#invoice-form button[type=submit]').click();await page.locator('#invoice-filter').waitFor();
  assert.equal(await page.locator('.invoice-row[data-row="0"] [type=checkbox]').isChecked(),false);
  await page.locator('.invoice-row[data-row="0"]').getByRole('button',{name:'Use suggestion again',exact:true}).click();
  await page.waitForFunction(()=>!window.invoiceCorrections.a);assert.equal(await page.locator('.invoice-row[data-row="0"] [type=checkbox]').isChecked(),true);
  assert.equal((await page.evaluate(()=>window.invoiceCalls.filter(c=>c.command==='confirm_invoice_collection'))).length,0);
  assert.deepEqual(errors,[]);await context.close();
});

test('failed correction saves block export and closing until retried or explicitly reset',async()=>{
  const {page,context,errors}=await open();await page.locator('#invoice-form button[type=submit]').click();await page.locator('#invoice-filter').waitFor();
  const row=page.locator('.invoice-row[data-row="0"]');await page.evaluate(()=>window.failReview=true);
  await row.getByRole('textbox',{name:'Correction reason',exact:true}).fill('Confirmed business purpose');await row.getByRole('button',{name:'Save correction',exact:true}).click();
  await row.locator('.invoice-save-status').getByText(/Could not save/).waitFor();assert.equal(await page.locator('#invoice-export').isDisabled(),true);
  await page.locator('#invoice-close').click();assert.equal(await page.locator('#invoice-sheet').isVisible(),true);
  await page.evaluate(()=>window.failReview=false);await row.getByRole('button',{name:'Save correction',exact:true}).click();
  await page.waitForFunction(()=>window.invoiceCorrections.a?.reason==='Confirmed business purpose');assert.equal(await page.locator('#invoice-export').isEnabled(),true);
  assert.deepEqual(errors,[]);await context.close();
});

test('an older successful save cannot clear a newer invalid date edit',async()=>{
  const {page,context,errors}=await open();await page.locator('#invoice-form button[type=submit]').click();await page.locator('#invoice-filter').waitFor();
  const row=page.locator('.invoice-row[data-row="0"]');await page.evaluate(()=>window.delayReview=true);
  await row.locator('[type=date]').fill('2026-09-16');await row.locator('[type=date]').fill('2026-10-01');
  await page.waitForFunction(()=>window.invoiceCorrections.a?.invoice_date==='2026-09-16');assert.equal(await page.locator('#invoice-export').isDisabled(),true);
  assert.match(await row.locator('.invoice-save-status').innerText(),/Enter a document date/);
  await row.getByRole('button',{name:'Use suggestion again',exact:true}).click();await page.waitForFunction(()=>!window.invoiceCorrections.a);
  assert.equal(await page.locator('#invoice-export').isEnabled(),true);assert.deepEqual(errors,[]);await context.close();
});

test('filters honor saved date and type corrections and retain explicit review requests',async()=>{
  const {page,context,errors}=await open();
  const result=await page.evaluate(()=>{
    const preview={setup_revision:2,request:{from:'2026-07-01',to:'2026-09-30'}};
    const row={analysis:{kind:'other',invoice_date:'2026-10-01'},review:{setup_revision:2,decision:'include',kind:'invoice',invoice_date:'2026-09-30'}};
    const inPeriod=invoiceFilterMatches(row,0,'invoices',preview);
    const outside=invoiceFilterMatches(row,0,'outside',preview);
    row.review.decision='review';row.review.invoice_date='2026-10-01';
    const needsReview=invoiceFilterMatches(row,0,'review',preview);
    preview.setup_revision=3;
    return {inPeriod,outside,needsReview,stale:invoiceFilterMatches(row,0,'review',preview),staleInPeriod:invoiceFilterMatches(row,0,'invoices',preview)};
  });
  assert.deepEqual(result,{inPeriod:true,outside:false,needsReview:true,stale:true,staleInPeriod:false});
  assert.deepEqual(errors,[]);await context.close();
});

// Inspect the isolated invoice chat fixture before or after explicit approval.
// node tools/check-invoice-chat-fixture.mjs FIXTURE_DIRECTORY [--saved]
import assert from 'node:assert/strict';
import {DatabaseSync} from 'node:sqlite';
import {readFile,readdir} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import path from 'node:path';
const root=path.resolve(process.argv[2]);
const saved=process.argv.includes('--saved');
const db=new DatabaseSync(path.join(root,'kuverta.db'),{readOnly:true});
try {
  assert.equal(db.prepare('PRAGMA integrity_check').get().integrity_check,'ok');
  assert.equal(db.prepare('SELECT COUNT(*) n FROM message').get().n,4);
  assert.equal(db.prepare("SELECT COUNT(*) n FROM message_location WHERE flags<>''").get().n,0);
  assert.equal(db.prepare('SELECT COUNT(*) n FROM operation').get().n,0);
  const chat=db.prepare('SELECT turns,log FROM assistant_chat ORDER BY id DESC LIMIT 1').get();
  assert.ok(chat);
  const calls=JSON.parse(chat.turns).flatMap(t=>t.calls??[]);
  assert.ok(calls.some(c=>c.name==='invoice_profiles'));
  assert.ok(calls.some(c=>c.name==='prepare_invoice_collection'));
  assert.ok(calls.every(c=>['invoice_profiles','prepare_invoice_collection'].includes(c.name)));
  const event=JSON.parse(chat.log).flatMap(x=>x.events).find(e=>e.kind==='invoice_collection_suggested');
  assert.equal(event.proposal.profile_name,'Example Company');
  assert.equal(event.proposal.from,'2026-07-01');
  assert.equal(event.proposal.to,'2026-09-30');
  const draft=JSON.parse(db.prepare('SELECT json FROM invoice_draft WHERE profile_id=?').get(event.proposal.profile).json);
  assert.equal(draft.complete,true);
  assert.equal(draft.accounts.length,2);
  assert.equal(draft.rows.length,3);
  assert.equal(draft.rows.filter(r=>r.duplicate_of!=null).length,1);
  const copies=db.prepare('SELECT * FROM invoice_copy').all();
  const files=await readdir(path.join(root,'output'));
  assert.equal(copies.length,saved?2:0);
  assert.equal(files.length,saved?3:0);
  for(const receipt of copies) {
    assert.equal(receipt.profile_id,event.proposal.profile);
    assert.equal(receipt.saved,1);
    const file=path.resolve(receipt.path.replace(/^\\\\\?\\/,''));
    assert.equal(path.dirname(file),path.join(root,'output'));
    assert.equal(createHash('sha256').update(await readFile(file)).digest('hex'),receipt.digest);
  }
  console.log(JSON.stringify({integrity:'ok',tools:calls.map(c=>c.name),profile:'Example Company',quarter:'Q3 2026',profileAccounts:2,previewRows:3,exactDuplicates:1,sourceMailUnchanged:true,invoiceCopies:copies.length,outputFiles:files.length,explicitApprovalTest:saved}));
} finally {db.close();}

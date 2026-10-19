// Read-only audit after configuring Firma A, saving one manual review reason,
// closing and reopening its preview in the native offline invoice_fixture.
// node tools/check-invoice-setup-fixture.mjs FIXTURE_DIRECTORY
import assert from 'node:assert/strict';
import {DatabaseSync} from 'node:sqlite';
import {readdir} from 'node:fs/promises';
import path from 'node:path';
const root=path.resolve(process.argv[2]);
const db=new DatabaseSync(path.join(root,'kuverta.db'),{readOnly:true});
try {
  assert.equal(db.prepare('PRAGMA integrity_check').get().integrity_check,'ok');
  assert.equal(db.prepare('PRAGMA user_version').get().user_version,28);
  assert.deepEqual(db.prepare('SELECT email FROM account ORDER BY id').all().map(a=>a.email),['test1@example.invalid','test2@example.invalid','test3@example.invalid']);
  assert.equal(db.prepare('SELECT COUNT(*) n FROM message').get().n,4);
  assert.equal(db.prepare("SELECT COUNT(*) n FROM message_location WHERE flags<>''").get().n,0);
  for(const table of ['operation','invoice_copy','invoice_export','invoice_analysis'])assert.equal(db.prepare(`SELECT COUNT(*) n FROM ${table}`).get().n,0,table);
  const setups=db.prepare('SELECT * FROM invoice_setup WHERE json IS NOT NULL').all();
  assert.equal(setups.length,1);
  const setup=JSON.parse(setups[0].json);
  assert.equal(setup.company,'Testfirma A GmbH');assert.deepEqual(setup.accounts,[1,2]);
  assert.equal(setup.period,'month');assert.equal(setups[0].draft_json,null);
  const draft=JSON.parse(db.prepare('SELECT json FROM invoice_draft').get().json);
  assert.equal(draft.complete,true);assert.equal(draft.rows.length,3);
  assert.equal(draft.setup_revision,setups[0].revision);
  const reviews=db.prepare('SELECT * FROM invoice_review').all();assert.equal(reviews.length,1);
  assert.equal(reviews[0].profile_id,setups[0].profile_id);
  const review=JSON.parse(reviews[0].json);
  assert.equal(review.decision,'review');assert.equal(review.invoice_date,'');
  assert.equal(review.reason,'Nur synthetischer Test');
  assert.ok(db.prepare('SELECT COUNT(*) n FROM invoice_review_event').get().n>0);
  assert.deepEqual(await readdir(path.join(root,'output')),[]);
  console.log(JSON.stringify({integrity:'ok',schema:28,syntheticMessages:4,configuredProfiles:1,persistedCorrections:1,completePreviewRows:3,mailChanges:0,modelAnalyses:0,exports:0,outputFiles:0}));
} finally {db.close();}

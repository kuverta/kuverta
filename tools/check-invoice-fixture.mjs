// Verify the offline native fixture AFTER confirming its invoice and credit.
// node tools/check-invoice-fixture.mjs FIXTURE_DIRECTORY
import assert from 'node:assert/strict';
import {DatabaseSync} from 'node:sqlite';
import {readFile,readdir} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import path from 'node:path';
const root=path.resolve(process.argv[2]);
const db=new DatabaseSync(path.join(root,'kuverta.db'),{readOnly:true});
const integrity=db.prepare('PRAGMA integrity_check').get().integrity_check;
assert.equal(integrity,'ok');
assert.equal(db.prepare('SELECT COUNT(*) n FROM message').get().n,4);
assert.equal(db.prepare("SELECT COUNT(*) n FROM message_location WHERE flags<>''").get().n,0);
assert.equal(db.prepare('SELECT COUNT(*) n FROM operation').get().n,0);
const receipts=db.prepare('SELECT * FROM invoice_copy ORDER BY profile_id,digest').all();
assert.equal(receipts.length,2);
assert.equal(new Set(receipts.map(r=>r.profile_id)).size,1);
for(const receipt of receipts){
  assert.equal(receipt.saved,1);
  const file=path.resolve(receipt.path.replace(/^\\\\\?\\/,''));
  assert.equal(path.dirname(file),path.join(root,'output'));
  const bytes=await readFile(file);
  assert.equal(createHash('sha256').update(bytes).digest('hex'),receipt.digest);
  if(process.platform==='win32')assert.match(await readFile(`${file}:Zone.Identifier`,'utf8'),/ZoneId=3/);
}
const draft=JSON.parse(db.prepare('SELECT json FROM invoice_draft').get().json);
assert.equal(draft.complete,true);assert.equal(draft.rows.length,3);
assert.equal(draft.rows.filter(r=>r.previous_path).length,3,'new preview recognizes all copies');
const files=await readdir(path.join(root,'output'));
assert.equal(files.filter(f=>f.endsWith('.txt')).length,2);
assert.equal(files.filter(f=>f.endsWith('.json')).length,1);
console.log(JSON.stringify({integrity,sourceMessages:4,mailFlagsUnchanged:true,mailOperations:0,copies:2,matchingHashes:true,originMarked:process.platform==='win32',duplicatePreviewRows:3,manifests:1}));
db.close();

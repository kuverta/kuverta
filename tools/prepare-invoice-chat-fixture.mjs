// Opt-in native chat smoke test. First create a NEW directory with invoice_fixture.
// node tools/prepare-invoice-chat-fixture.mjs FIXTURE_DIRECTORY SOURCE_DB
// Copies only the assistant's provider/model configuration and keychain reference;
// no secret, real mail or account credential is copied. Run the test app with
// KUVERTA_DATA_DIR set to this fixture and KUVERTA_INSTANCE unset so its configured
// provider can use the existing keychain entry. AI calls can incur costs.
import assert from 'node:assert/strict';
import {DatabaseSync} from 'node:sqlite';
import {realpath} from 'node:fs/promises';
import path from 'node:path';
const fixture=await realpath(process.argv[2]);
const targetFile=await realpath(path.join(fixture,'kuverta.db'));
const sourceFile=await realpath(process.argv[3]);
assert.notEqual(targetFile.toLowerCase(),sourceFile.toLowerCase());
const db=new DatabaseSync(targetFile);
const source=new DatabaseSync(sourceFile,{readOnly:true});
try {
  const accounts=db.prepare('SELECT email,imap_host FROM account ORDER BY id').all();
  assert.deepEqual(accounts.map(a=>a.email),['test1@example.invalid','test2@example.invalid','test3@example.invalid']);
  assert.ok(accounts.every(a=>a.imap_host==='127.0.0.1'));
  assert.equal(db.prepare('SELECT COUNT(*) n FROM message').get().n,4);
  const profile=db.prepare('SELECT id FROM profile WHERE name=?').get('Rechnungstest Firma A');
  assert.ok(profile,'Only an unmodified invoice_fixture is accepted');
  assert.equal(db.prepare('SELECT COUNT(*) n FROM invoice_copy').get().n,0);
  const settings=source.prepare("SELECT p.*,t.model FROM ai_provider p JOIN ai_task t ON t.provider_id=p.id WHERE t.task='assistant'").get();
  assert.ok(settings,'An explicit assistant model is required');
  db.exec('BEGIN');
  db.prepare('UPDATE profile SET name=? WHERE id=?').run('Example Company',profile.id);
  db.prepare('INSERT INTO ai_provider(id,kind,label,base_url,key_name,created_at) VALUES(?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET kind=excluded.kind,label=excluded.label,base_url=excluded.base_url,key_name=excluded.key_name')
    .run(settings.id,settings.kind,settings.label,settings.base_url,settings.key_name,settings.created_at);
  db.prepare("INSERT INTO ai_task(task,provider_id,model) VALUES('assistant',?,?) ON CONFLICT(task) DO UPDATE SET provider_id=excluded.provider_id,model=excluded.model").run(settings.id,settings.model);
  db.exec('COMMIT');
  console.log(JSON.stringify({fixture,profile:'Example Company',syntheticMessages:4,assistant:settings.model,realMailCopied:false,secretsCopied:false}));
} finally {db.close();source.close();}

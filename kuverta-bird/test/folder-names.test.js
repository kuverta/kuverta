import assert from 'node:assert/strict';
import {test} from 'node:test';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
const source = readFileSync(new URL('../../apps/desktop/ui/folder-names.js', import.meta.url), 'utf8');
const context = {t:s => ({Inbox:'Posteingang',Drafts:'Entwürfe',Sent:'Gesendet',Trash:'Papierkorb',Archives:'Archive',"Archive|a folder's name":'Archiv'}[s] ?? s)};
runInNewContext(source, context);
test('folder presentation preserves custom names and distinguishes paths', () => {
  const display = name => context.folderPresentation({name}).label;
  assert.equal(display('INBOX'), 'Posteingang');
  assert.equal(display('INBOX.Archive'), 'Archiv');
  assert.equal(display('INBOX.Archives.2020'), 'Archive › 2020');
  assert.equal(display('[Gmail]/Sent'), 'Gesendet');
  assert.equal(display('Customers/Sent'), 'Customers › Sent');
  assert.equal(display('Suppliers/Sent'), 'Suppliers › Sent');
  assert.equal(display('Invoices 2026.pdf'), 'Invoices 2026.pdf');
});
test('special use names and original IMAP names remain separate', () => {
  const folder = {name:'INBOX.Custom bin',special_use:'\\Trash'};
  assert.equal(context.folderPresentation(folder).label, 'Papierkorb');
  assert.equal(context.folderPresentation(folder).icon, 'trash');
  assert.equal(folder.name, 'INBOX.Custom bin');
});

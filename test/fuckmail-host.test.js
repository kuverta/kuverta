/**
 * The `fuckmail` adapter against the host contract.
 *
 * Same suite as the Thunderbird adapter and the in-memory reference. Where the
 * two real hosts differ — queued undo against compensating undo, and one of
 * them unable to file by category at all — the suite tests what each declares
 * rather than assuming they are the same client.
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { runConformance } from './support/conformance.js';
import { fakeInvoke } from './support/fake-invoke.js';
import { FuckmailHost } from '../hosts/fuckmail/host.js';

const account = { id: 1, email: 'you@example.com' };

// `open()` has to have finished before `capabilities` means anything — it is
// what discovers whether this account has Archive and Trash. The suite awaits
// whatever the factory returns, so handing it the promise is enough.
runConformance('fuckmail', () => new FuckmailHost(fakeInvoke(), account).open());

// -- things specific to this host ---------------------------------------

test('fuckmail: undo is queued, which is a different promise from compensating', async () => {
  const host = await new FuckmailHost(fakeInvoke(), account).open();
  assert.equal(host.capabilities.undo, 'queued');
});

test('fuckmail: filing a message is not queued, so undo does not reverse it', async () => {
  // A category lives in the store and never reaches a server, so there is no
  // round trip for the undo window to protect. Changing your mind is another
  // correction — which is what the log wants anyway, an event and not an edit.
  const host = await new FuckmailHost(fakeInvoke(), account).open();
  const { rows } = await host.page({ scope: { kind: 'all' }, offset: 0, limit: 1 });

  await host.setCategory(rows[0].id, 'marketing');
  assert.equal(await host.undo(), null, 'filing should not have gone on the queue');

  const after = await host.page({ scope: { kind: 'all' }, offset: 0, limit: 1 });
  assert.equal(after.rows[0].category, 'marketing');
});

test('fuckmail: a category that is not one is refused by the store, not the adapter', async () => {
  // `core-rpc` validates against `core-rules`. The adapter deliberately does
  // not second-guess it: two places deciding what a category is would be two
  // places to disagree.
  const host = await new FuckmailHost(fakeInvoke(), account).open();
  const { rows } = await host.page({ scope: { kind: 'all' }, offset: 0, limit: 1 });

  await assert.rejects(() => host.setCategory(rows[0].id, 'invoices'), /not a category/);
});

test('fuckmail: an account with no Archive folder cannot archive', async () => {
  const invoke = fakeInvoke();
  const bare = async (command, args) =>
    command === 'special_folders' ? { archive: null, trash: 'Trash' } : invoke(command, args);

  const host = await new FuckmailHost(bare, account).open();
  assert.equal(host.capabilities.archive, false);
  assert.equal(host.capabilities.trash, true);
});

test('fuckmail: dates cross the bridge in seconds and arrive in milliseconds', async () => {
  // The store keeps seconds. Treating them as milliseconds dates every message
  // to 1970, which sorts the whole list wrongly and looks like a sync bug.
  const host = await new FuckmailHost(fakeInvoke(), account).open();
  const { rows } = await host.page({ scope: { kind: 'all' }, offset: 0, limit: 1 });

  assert.ok(rows[0].date > 1e12, 'a 2026 date in milliseconds is above 1e12');
  assert.equal(new Date(rows[0].date).getUTCFullYear(), 2026);
});

test('fuckmail: a refused change is called out in the sync line', async () => {
  // Refused means the server no longer matched what the change was recorded
  // against — the §3.5 conflict check doing its job, and the one number in a
  // sync summary that means something needs looking at.
  const invoke = fakeInvoke();
  const withRefusal = async (command, args) =>
    command === 'sync'
      ? { ...(await invoke('sync', args)), changes_refused: 2, inserted: 5 }
      : invoke(command, args);

  const host = await new FuckmailHost(withRefusal, account).open();
  const said = await host.sync();

  assert.match(said, /5 new/);
  assert.match(said, /2 refused/);
});

test('fuckmail: an unknown command is an error, not a silent null', async () => {
  // Guards the adapter against drifting from the commands core-rpc registers.
  // Tauri throws for a command that was never registered, so the fake must
  // too — otherwise an adapter calling a command that does not exist would
  // look like a command that returned nothing.
  const invoke = fakeInvoke();
  await assert.rejects(() => invoke('set_flag', {}), /unknown command/);
});

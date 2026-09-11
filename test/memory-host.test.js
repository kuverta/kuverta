/**
 * The reference host against the contract.
 *
 * If this fails, the contract is wrong rather than the host — nothing else can
 * tell the difference between "no adapter satisfies this" and "this adapter is
 * broken".
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { runConformance } from './support/conformance.js';
import { MemoryMailbox, ReadOnlyMailbox } from './support/mailbox.js';
import { Triage } from '../core/triage.js';
import { ACTIONS } from '../core/host.js';

runConformance('memory', () => new MemoryMailbox());

// A host that declares almost nothing, so the contract's capability gates and
// its refusal rule keep being exercised now that every real adapter can do
// everything. Most of this run is skips, and that is the point.
runConformance('read-only', () => new ReadOnlyMailbox());

test('the surface over a read-only host offers nothing it cannot do', async () => {
  // The other half of the same idea: declaring a capability false has to reach
  // the user as "this cannot be done", not as a key that appears to work.
  const triage = new Triage(new ReadOnlyMailbox());
  await triage.start();

  const before = triage.total;
  await triage.act(ACTIONS.Archive);

  assert.equal(triage.total, before, 'nothing should have happened');
  assert.match(triage.notice.text, /cannot archive/);
  assert.equal(triage.notice.tone, 'error');
});

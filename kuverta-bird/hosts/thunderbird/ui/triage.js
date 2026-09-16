/**
 * The triage tab, in Thunderbird.
 *
 * All this file does is put the shared surface on top of the Thunderbird
 * adapter. If it grows past that, something that belongs in `core/` has ended
 * up here.
 */

import { Triage } from '../../../core/triage.js';
import { mountTriage } from '../../../core/view/triage.js';
import { ThunderbirdHost } from '../host.js';

const host = new ThunderbirdHost(messenger);
const triage = new Triage(host);

// The triage list spans every account's triage folders, so it says so unless
// there is only one and the name would be more use than the fact.
const accounts = await messenger.accounts.list();
const title = accounts.length === 1 ? accounts[0].name : `${accounts.length} accounts`;

mountTriage({
  root: document.getElementById('root'),
  triage,
  title,
  // Compose is Thunderbird's own window, not something to rebuild.
  onCompose: () => messenger.compose.beginNew(),
});

await triage.start();

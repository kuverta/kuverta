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

mountTriage({
  root: document.getElementById('root'),
  triage,
  // Compose is Thunderbird's own window, not something to rebuild.
  onCompose: () => messenger.compose.beginNew(),
});

await triage.start();

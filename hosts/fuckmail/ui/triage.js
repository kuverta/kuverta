/**
 * The triage surface, in the standalone client.
 *
 * The mirror image of `hosts/thunderbird/ui/triage.js`: same surface, same
 * model, a different adapter underneath. That the two mount files are this
 * short is the point of the whole arrangement.
 */

import { Triage } from '../../../core/triage.js';
import { mountTriage } from '../../../core/view/triage.js';
import { FuckmailHost } from '../host.js';

// Tauri v2 puts `invoke` here; v1 put it on `window.__TAURI__.invoke`.
const invoke = window.__TAURI__?.core?.invoke ?? window.__TAURI__?.invoke;
if (!invoke) throw new Error('no Tauri bridge — this page has to run inside the app');

const [account] = await invoke('accounts');
if (!account) throw new Error('no accounts configured');

const host = await new FuckmailHost(invoke, account).open();
const triage = new Triage(host);

mountTriage({
  root: document.getElementById('root'),
  triage,
  // The standalone client has its own compose pane; this surface does not
  // rebuild it, so the key is left to the app that mounts this.
  onCompose: null,
});

await triage.start();

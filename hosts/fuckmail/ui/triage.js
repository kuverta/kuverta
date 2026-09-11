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

const root = document.getElementById('root');

try {
  // Tauri v2 puts `invoke` here; v1 put it on `window.__TAURI__.invoke`.
  const invoke = window.__TAURI__?.core?.invoke ?? window.__TAURI__?.invoke;
  if (!invoke) throw new Error('no Tauri bridge — this page has to run inside the app');

  const accounts = await invoke('accounts');
  if (!accounts?.length) throw new Error('no accounts are configured');

  // The first account, for now. Which account to show is a question this
  // surface does not answer yet — the sidebar shows one account's folders and
  // categories, and adding a switcher is worth doing once it is being used.
  const host = await new FuckmailHost(invoke, accounts[0]).open();
  const triage = new Triage(host);

  mountTriage({
    root,
    triage,
    // The standalone client has its own compose pane; this surface does not
    // rebuild it, so the key is left to the app that mounts this.
    onCompose: null,
  });

  await triage.start();
} catch (error) {
  // A module that throws leaves a blank window and no clue what happened,
  // which during a first run is the least useful failure available.
  root.replaceChildren();
  const said = document.createElement('pre');
  said.style.cssText = 'padding:24px;white-space:pre-wrap;color:#b3261e';
  said.textContent = `fuckbird could not start:\n\n${error.stack ?? error.message}`;
  root.append(said);
  console.error('fuckbird: failed to start', error);
}

/**
 * The triage surface, in the standalone client.
 *
 * The mirror image of `hosts/thunderbird/ui/triage.js`: same surface, same
 * model, a different adapter underneath. What lives here and not in `core/` is
 * everything that is a `kuverta` idea rather than a mail idea — the Tauri
 * bridge, and which account is being looked at.
 */

import { Triage } from '../../../core/triage.js';
import { mountTriage } from '../../../core/view/triage.js';
import { KuvertaHost } from '../host.js';

const root = document.getElementById('root');
const picker = document.getElementById('account');

/** Undoes the last mount, so switching accounts does not stack key handlers. */
let unmount = null;

try {
  // Tauri v2 puts `invoke` here; v1 put it on `window.__TAURI__.invoke`.
  const invoke = window.__TAURI__?.core?.invoke ?? window.__TAURI__?.invoke;
  if (!invoke) throw new Error('no Tauri bridge — this page has to run inside the app');

  const accounts = await invoke('accounts');
  if (!accounts?.length) throw new Error('no accounts are configured');

  // Which account is a question the core cannot answer: it has no idea there
  // is more than one mailbox in the world, and should not learn.
  picker.replaceChildren(
    ...accounts.map((account) => {
      const option = document.createElement('option');
      option.value = String(account.id);
      option.textContent = account.email;
      return option;
    }),
  );
  picker.hidden = accounts.length < 2;
  picker.addEventListener('change', () => {
    const chosen = accounts.find((a) => String(a.id) === picker.value);
    if (chosen) open(invoke, chosen);
  });

  await open(invoke, accounts[0]);
} catch (error) {
  fail(error);
}

async function open(invoke, account) {
  try {
    // The previous surface has to let go of the document first — two mounts
    // would both be listening for keys, and every keystroke would act twice.
    unmount?.();

    const host = await new KuvertaHost(invoke, account).open();
    const triage = new Triage(host);

    unmount = mountTriage({
      root,
      triage,
      // Which mailbox this is. The core cannot know, and the person reading it
      // needs to — especially here, where the store may hold several.
      title: account.email,
      // The standalone client has its own compose pane; this surface does not
      // rebuild it, so the key is left to the app that mounts this.
      onCompose: null,
    });

    await triage.start();
  } catch (error) {
    fail(error);
  }
}

/**
 * Says what went wrong, on the page.
 *
 * A module that throws leaves a blank window and no clue, which during a first
 * run is the least useful failure available.
 */
function fail(error) {
  unmount?.();
  unmount = null;
  root.replaceChildren();
  const said = document.createElement('pre');
  said.style.cssText = 'padding:24px;white-space:pre-wrap;color:#b3261e';
  said.textContent = `kuverta-bird could not start:\n\n${error.stack ?? error.message}`;
  root.append(said);
  console.error('kuverta-bird: failed to start', error);
}

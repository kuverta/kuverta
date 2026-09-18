/**
 * Unsubscribe: every sender whose mail says how to stop it, and stopping it.
 *
 * The client does the unsubscribing — `unsubscribe` in core-rpc — the way
 * each sender allows: RFC 8058's one-click POST, or the mail its header asks
 * for. A sender that offers only a web page gets that page opened in the
 * browser for the person to finish, because what such a page asks for cannot
 * be seen from here, and following those links unasked is how a mail client
 * tells a spammer that an address is read.
 *
 * The panel lies over the list; while it is open, the list's keys are held
 * back so an `e` meant for nothing here does not archive a message nobody
 * can see.
 */

const METHOD_NAMES = {
  one_click: ['one click', 'Unsubscribes with one request to the sender, as RFC 8058 describes'],
  mailto: ['by mail', 'Sends the unsubscribe mail the sender asks for, from this account'],
  browser: ['opens a page', 'The sender only offers a web page; it opens in your browser to finish'],
};

const dateFormat = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short', year: 'numeric' });

export class UnsubscribePanel {
  #invoke;
  #account = null;
  #senders = [];
  #chosen = new Set();
  #busy = false;
  #onChange;
  #el = {};

  /**
   * @param {Function} invoke  The Tauri bridge.
   * @param {() => void} onChange  Called when the count of senders left may have changed.
   */
  constructor(invoke, onChange) {
    this.#invoke = invoke;
    this.#onChange = onChange;
    this.node = this.#build();
    this.count = 0;
    window.addEventListener('keydown', (event) => this.#holdKeys(event), true);
  }

  get open() {
    return !this.node.hidden;
  }

  /** Senders left to unsubscribe from, for the sidebar entry. */
  async refreshCount(account) {
    this.#account = account;
    try {
      const counts = await this.#invoke('cleanup_counts', { account: account.id });
      this.count = counts.unsubscribable;
    } catch {
      this.count = 0;
    }
    return this.count;
  }

  async show(account) {
    this.#account = account;
    this.node.hidden = false;
    this.#el.search.value = '';
    this.#chosen.clear();
    await this.#backfill();
    await this.#load();
  }

  hide() {
    this.node.hidden = true;
  }

  // -- building -------------------------------------------------------------

  #build() {
    const panel = document.createElement('section');
    panel.className = 'ku-panel';
    panel.hidden = true;
    panel.innerHTML = `
      <div class="ku-head">
        <h2>Unsubscribe</h2>
        <p>Senders whose mail says how to unsubscribe. Those marked <b>one click</b> or <b>by mail</b>
           are unsubscribed from here, all at once if you like; the rest open a page in your browser to finish.</p>
        <p class="ku-warn">Only unsubscribe from senders you recognise. For real spam, unsubscribing tells the
           sender your address is read — delete it instead.</p>
      </div>
      <div class="ku-tools">
        <input type="search" data-el="search" placeholder="Find a sender" spellcheck="false">
        <button type="button" data-el="all">Select all</button>
        <button type="button" data-el="none">Select none</button>
        <span class="ku-grow"></span>
        <label title="Each move is queued like any other, and can be undone from the main window with z">
          <input type="checkbox" data-el="trash"> Also move their mail to Trash
        </label>
        <button type="button" class="ku-primary" data-el="go" disabled>Unsubscribe</button>
      </div>
      <div class="ku-status" data-el="status"></div>
      <div class="ku-open" data-el="open" hidden>
        <span data-el="open-text"></span>
        <button type="button" data-el="open-go">Open them</button>
      </div>
      <div class="ku-list" data-el="list"></div>
    `;
    for (const node of panel.querySelectorAll('[data-el]')) this.#el[node.dataset.el] = node;

    this.#el.search.addEventListener('input', () => this.#draw());
    this.#el.all.addEventListener('click', () => {
      for (const sender of this.#visible()) if (!this.#finished(sender)) this.#chosen.add(sender.key);
      this.#draw();
    });
    this.#el.none.addEventListener('click', () => {
      this.#chosen.clear();
      this.#draw();
    });
    this.#el.go.addEventListener('click', () => this.#run());
    return panel;
  }

  #holdKeys(event) {
    if (!this.open) return;
    if (event.key === 'Escape') {
      event.stopPropagation();
      this.hide();
      this.#onChange();
      return;
    }
    // Typing in the search box is the one keyboard use here; everything else
    // would reach the list underneath.
    if (event.target === this.#el.search || event.key === 'Tab' || event.key === ' ') return;
    event.stopPropagation();
  }

  // -- data -------------------------------------------------------------------

  /** Reads the headers of mail synced before they were kept, once. */
  async #backfill() {
    let left = 1;
    let rounds = 0;
    while (left > 0 && rounds < 200) {
      rounds += 1;
      try {
        left = await this.#invoke('backfill_headers', { email: this.#account.email });
      } catch (error) {
        this.#say(`could not read older mail: ${error}`, true);
        return;
      }
      if (left > 0) this.#say(`Reading older mail for unsubscribe links… ${left}+ messages to go`);
    }
    this.#say('');
  }

  async #load() {
    try {
      this.#senders = await this.#invoke('unsubscribe_senders', { account: this.#account.id });
    } catch (error) {
      this.#senders = [];
      this.#say(String(error), true);
    }
    this.#draw();
  }

  #finished(sender) {
    return sender.last_attempt?.state === 'done';
  }

  #visible() {
    const needle = this.#el.search.value.trim().toLowerCase();
    if (!needle) return this.#senders;
    return this.#senders.filter((sender) =>
      [sender.name, sender.address, sender.list_id, sender.latest_subject]
        .filter(Boolean)
        .some((text) => text.toLowerCase().includes(needle)),
    );
  }

  // -- drawing ------------------------------------------------------------------

  #draw() {
    const list = this.#el.list;
    list.replaceChildren();
    const senders = this.#visible();
    if (!senders.length) {
      const empty = document.createElement('div');
      empty.className = 'ku-empty';
      empty.textContent = this.#senders.length
        ? 'No sender matches that.'
        : 'Nothing here offers a way to unsubscribe. Sync, and anything that does will show up.';
      list.append(empty);
    }
    for (const sender of senders) list.append(this.#row(sender));

    const chosen = this.#chosen.size;
    this.#el.go.disabled = chosen === 0 || this.#busy;
    this.#el.go.textContent = chosen ? `Unsubscribe from ${chosen}` : 'Unsubscribe';
  }

  #row(sender) {
    const row = document.createElement('label');
    row.className = 'ku-sender';

    const tick = document.createElement('input');
    tick.type = 'checkbox';
    tick.checked = this.#chosen.has(sender.key);
    tick.addEventListener('change', () => {
      if (tick.checked) this.#chosen.add(sender.key);
      else this.#chosen.delete(sender.key);
      this.#draw();
    });

    const who = document.createElement('div');
    who.className = 'ku-who';
    who.textContent = sender.name;
    const address = document.createElement('span');
    address.className = 'ku-addr';
    address.textContent = sender.list_id ?? sender.address ?? '';
    if (address.textContent && address.textContent !== sender.name) who.append(address);

    const what = document.createElement('div');
    what.className = 'ku-what';
    const counts = `${sender.messages} message${sender.messages === 1 ? '' : 's'}` +
      (sender.unread ? `, ${sender.unread} unread` : '');
    what.textContent = [counts, sender.latest_subject ? `latest: ${sender.latest_subject}` : '']
      .filter(Boolean)
      .join(' · ');

    const chips = document.createElement('div');
    chips.className = 'ku-chips';
    const [name, explain] = METHOD_NAMES[sender.method.kind] ?? [sender.method.kind, ''];
    const method = document.createElement('span');
    method.className = sender.method.kind === 'browser' ? 'ku-chip' : 'ku-chip ku-auto';
    method.textContent = name;
    // Where it goes, in full: nothing is sent anywhere the person cannot see.
    const target = sender.method.kind === 'mailto'
      ? `mails ${sender.method.address} (“${sender.method.subject}”)`
      : sender.method.url;
    method.title = `${explain}\n${target}`;
    chips.append(method);

    const attempt = sender.last_attempt;
    if (attempt) {
      const state = document.createElement('span');
      const when = dateFormat.format(new Date(attempt.at_utc * 1000));
      if (attempt.state === 'done') {
        state.className = 'ku-chip ku-done';
        state.textContent = `unsubscribed ${when}`;
      } else if (attempt.state === 'opened') {
        state.className = 'ku-chip';
        state.textContent = `page opened ${when}`;
      } else {
        state.className = 'ku-chip ku-failed';
        state.textContent = 'failed';
      }
      state.title = attempt.detail ?? '';
      chips.append(state);
    }

    row.append(tick, who, chips, what);
    return row;
  }

  #say(text, bad = false) {
    this.#el.status.textContent = text;
    this.#el.status.classList.toggle('ku-bad', bad);
  }

  // -- doing it -----------------------------------------------------------------

  async #run() {
    const keys = [...this.#chosen];
    if (!keys.length || this.#busy) return;
    // The account this run is for, whatever the picker says by the time the
    // senders have answered: keys from one account must never act on another.
    const account = this.#account;
    this.#busy = true;
    this.#draw();
    this.#say(`Unsubscribing from ${keys.length} sender${keys.length === 1 ? '' : 's'}…`);

    let results = [];
    try {
      results = await this.#invoke('unsubscribe', { email: account.email, keys });
    } catch (error) {
      this.#say(String(error), true);
      this.#busy = false;
      this.#draw();
      return;
    }

    const done = results.filter((r) => r.state === 'done');
    const failed = results.filter((r) => r.state === 'failed');
    const pages = results.filter((r) => r.state === 'open');

    let trashed = 0;
    if (this.#el.trash.checked) {
      // Only where unsubscribing worked: a sender still to be dealt with in
      // the browser keeps their mail until that is done.
      for (const result of done) {
        try {
          trashed += await this.#invoke('trash_from_sender', { account: account.id, key: result.key });
        } catch (error) {
          failed.push({ ...result, detail: String(error) });
        }
      }
    }

    const parts = [];
    if (done.length) parts.push(`unsubscribed from ${done.length}`);
    if (pages.length) parts.push(`${pages.length} need${pages.length === 1 ? 's' : ''} a page opened`);
    if (trashed) parts.push(`${trashed} message${trashed === 1 ? '' : 's'} on the way to Trash`);
    if (failed.length) {
      parts.push(`${failed.length} failed: ${failed.map((f) => `${f.name} (${f.detail})`).join('; ')}`);
    }
    this.#say(parts.join(' · '), failed.length > 0);
    this.#offerPages(pages, account);

    this.#chosen.clear();
    this.#busy = false;
    await this.#load();
    this.#onChange();
  }

  /** Senders who only offer a page: opened in the browser, in batches. */
  #offerPages(pages, account) {
    const bar = this.#el.open;
    if (!pages.length) {
      bar.hidden = true;
      return;
    }
    let left = [...pages];
    const draw = () => {
      if (!left.length) {
        bar.hidden = true;
        return;
      }
      bar.hidden = false;
      const batch = Math.min(left.length, 5);
      this.#el['open-text'].textContent =
        `${left.length} sender${left.length === 1 ? '' : 's'} can only be unsubscribed on their website.`;
      this.#el['open-go'].textContent = left.length > batch ? `Open the next ${batch}` : `Open ${batch === 1 ? 'it' : 'them'}`;
    };
    this.#el['open-go'].onclick = async () => {
      const batch = left.slice(0, 5);
      left = left.slice(5);
      for (const page of batch) {
        try {
          await this.#invoke('open_external', { url: page.url });
          await this.#invoke('unsubscribe_opened', {
            account: account.id,
            key: page.key,
            url: page.url,
          });
        } catch (error) {
          this.#say(`could not open ${page.url}: ${error}`, true);
        }
      }
      draw();
      await this.#load();
    };
    draw();
  }
}

/**
 * `fuckmail` as a mail host.
 *
 * Implements `core/host.js` over the standalone client's Tauri commands — the
 * same `invoke(command, args)` its desktop UI already uses. So the triage
 * surface in this repository can run inside that app, and the two front ends
 * stop being two implementations of the same list.
 *
 * The differences from the Thunderbird adapter are worth naming, because they
 * are the reason the port has a `capabilities` object rather than an
 * assumption that every host can do everything:
 *
 * - **Undo is `queued`, not `compensating`.** A change waits in the undo
 *   window before it is sent, so undoing cancels something that never
 *   happened. That is a stronger promise than Thunderbird can make, and the
 *   surface says so in different words.
 * - **The store pages and sorts**, so nothing here has to materialise a scope
 *   to answer an offset — where the Thunderbird adapter does.
 * - **Filing a message is not queued.** A category lives in the store and is
 *   never sent to a server, so there is no round trip to hold back and nothing
 *   for the undo window to protect. Changing your mind is another correction,
 *   which is what the log wants anyway — an event, not an edit.
 */

import { MailHost } from '../../core/host.js';
import { ALL_CATEGORIES, CATEGORY_LABELS } from '../../core/category.js';

export class FuckmailHost extends MailHost {
  #invoke;
  #account;
  #email;
  #special = { archive: null, trash: null };

  /**
   * @param {Function} invoke  `(command, args) => Promise<any>`, Tauri's.
   * @param {{id: number, email: string}} account
   */
  constructor(invoke, account) {
    super();
    this.#invoke = invoke;
    this.#account = account.id;
    this.#email = account.email;
  }

  /**
   * Must be awaited before use.
   *
   * Archive and Trash are looked up once, because whether this account has
   * them is part of what the host can do, and `capabilities` has to be able to
   * answer without going asynchronous.
   */
  async open() {
    this.#special = (await this.#invoke('special_folders', { account: this.#account })) ?? {};
    return this;
  }

  get capabilities() {
    return {
      archive: Boolean(this.#special.archive),
      trash: Boolean(this.#special.trash),
      setRead: true,
      setCategory: true,
      search: true,
      sync: true,
      undo: 'queued',
      compose: true,
    };
  }

  async scopes() {
    const [folders, counts] = await Promise.all([
      this.#invoke('folders', { account: this.#account }),
      this.#invoke('category_counts', { account: this.#account }),
    ]);

    // `category_counts` answers with pairs, not an object.
    const byCategory = new Map(counts ?? []);

    return [
      { kind: 'all', label: 'Everything' },
      ...(folders ?? []).map((folder) => ({
        kind: 'folder',
        value: folder.id,
        // `label` is the last path segment — what a sidebar shows — where
        // `name` is the full path IMAP commands take.
        label: folder.label || folder.name,
        count: folder.unread || null,
      })),
      ...ALL_CATEGORIES.map((category) => ({
        kind: 'category',
        value: category,
        label: CATEGORY_LABELS[category],
        count: byCategory.get(category) ?? null,
      })),
    ];
  }

  async page({ scope, offset, limit }) {
    // Search is a different command here and returns everything at once
    // rather than paging, so it is fetched whole and sliced.
    if (scope?.kind === 'search') {
      const rows = (await this.#invoke('search', {
        account: this.#account,
        query: String(scope.value),
        limit: 200,
      })) ?? [];
      return { total: rows.length, rows: rows.slice(offset, offset + limit).map(toRow) };
    }

    const page = await this.#invoke('messages', {
      account: this.#account,
      offset,
      limit,
      category: scope?.kind === 'category' ? scope.value : null,
      folder: scope?.kind === 'folder' ? scope.value : null,
      unreadOnly: false,
    });

    return { total: page.total, rows: page.rows.map(toRow) };
  }

  async message(id) {
    const detail = await this.#invoke('message', {
      account: this.#account,
      id: Number(id),
    });
    if (!detail) throw new Error(`no such message: ${id}`);

    return {
      row: {
        id: String(detail.id),
        subject: detail.subject ?? null,
        from: detail.from ?? null,
        fromAddr: addressOf(detail.from),
        date: secondsToMillis(detail.date_utc),
        // The detail view carries no flags; the list is where read state is
        // shown, and asking for a whole page to fill in one boolean would be a
        // round trip for nothing.
        unread: false,
        hasAttachments: false,
        category: null,
      },
      // An HTML-only message yields no text here — the store says so rather
      // than inventing a rendering.
      body: detail.body_text ?? null,
      facts: null,
    };
  }

  async archive(id) {
    if (!this.#special.archive) throw new Error('this account has no Archive folder');
    await this.#move(id, this.#special.archive);
  }

  async trash(id) {
    if (!this.#special.trash) throw new Error('this account has no Trash folder');
    // `move_to`, never a delete. Nothing destroys mail.
    await this.#move(id, this.#special.trash);
  }

  async setRead(id, read) {
    await this.#invoke('set_read', { account: this.#account, id: Number(id), read });
  }

  async setCategory(id, category) {
    await this.#invoke('set_category', {
      account: this.#account,
      id: Number(id),
      category,
    });
  }

  async undo() {
    // Null for an empty queue is a normal answer, not an error.
    const change = await this.#invoke('undo', { account: this.#account });
    return change ? { what: change.what } : null;
  }

  async sync() {
    const summary = await this.#invoke('sync', { email: this.#email });
    if (!summary) return null;
    return describeSync(summary);
  }

  async #move(id, target) {
    await this.#invoke('move_to', { account: this.#account, id: Number(id), target });
  }
}

/** A store row in the port's shape. */
function toRow(row) {
  return {
    id: String(row.id),
    subject: row.subject ?? null,
    from: row.from ?? null,
    fromAddr: addressOf(row.from),
    date: secondsToMillis(row.date_utc),
    unread: Boolean(row.unread),
    hasAttachments: Boolean(row.has_attachments),
    category: row.category ?? null,
  };
}

/**
 * The store keeps seconds since the epoch; the port uses milliseconds.
 *
 * Every field is normalised to null rather than left undefined, because the
 * contract distinguishes "this message has no date" from "this row has not
 * loaded" and `undefined` would blur the two.
 */
function secondsToMillis(seconds) {
  if (seconds == null) return null;
  const number = Number(seconds);
  return Number.isFinite(number) ? number * 1000 : null;
}

function addressOf(from) {
  if (!from) return null;
  const open = from.lastIndexOf('<');
  const close = from.lastIndexOf('>');
  if (open !== -1 && close > open) return from.slice(open + 1, close).trim() || null;
  return from.trim() || null;
}

/**
 * A sync summary in one line.
 *
 * The refused count is called out on its own because it is the one number that
 * means something needs looking at: the server no longer matched what the
 * change was recorded against, which is the conflict check of §3.5 doing its
 * job rather than a transient failure.
 */
function describeSync(summary) {
  const parts = [];
  if (summary.inserted) parts.push(`${summary.inserted} new`);
  if (summary.changes_sent) parts.push(`${summary.changes_sent} change(s) sent`);
  if (summary.changes_refused) parts.push(`${summary.changes_refused} refused — check them`);
  if (summary.non_atomic_moves) parts.push(`${summary.non_atomic_moves} move(s) left a copy behind`);
  return parts.length ? parts.join(', ') : 'up to date';
}

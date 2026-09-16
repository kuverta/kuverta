/**
 * `kuverta` as a mail host.
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

import { ACTIONS, MailHost, Unsupported } from '../../core/host.js';
import { ALL_CATEGORIES, CATEGORY_LABELS } from '../../core/category.js';

export class KuvertaHost extends MailHost {
  #invoke;
  #account;
  #email;
  #special = { archive: null, trash: null };
  /** Configured physical addresses that have a token, so they can be read. */
  #paper = [];

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

    // Postal addresses are read alongside mail. One without a token cannot be
    // read at all, and offering it would be a scope that can only ever fail.
    try {
      const boxes = (await this.#invoke('paper_mailboxes')) ?? [];
      this.#paper = boxes.filter((box) => box.has_token);
    } catch (error) {
      // Post is an addition, not a prerequisite: a Paperless that is down must
      // not stop the mail from being readable.
      console.warn('kuverta-bird: could not load postal addresses', error);
      this.#paper = [];
    }
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
      // One scope per physical address, so post can be read on its own as
      // well as in the same list as mail.
      ...this.#paper.map((box) => ({
        kind: 'paper',
        value: box.id,
        label: `${box.label} (post)`,
      })),
    ];
  }

  async page({ scope, offset, limit }) {
    // One address on its own.
    if (scope?.kind === 'paper') {
      return this.#paperPage(scope.value, offset, limit, null);
    }

    // A folder is an IMAP idea; post has none, so those lists are mail only.
    if (scope?.kind === 'folder' || this.#paper.length === 0) {
      return this.#mailPage(scope, offset, limit);
    }

    // Everything else — all mail, a category, a search — takes in post too.
    // That is the point of filing both by the same rules: "everything you have
    // been sent that is transactional" should not care which of them carried
    // it.
    const query = scope?.kind === 'search' ? String(scope.value) : null;
    const category = scope?.kind === 'category' ? scope.value : null;

    return merge(offset, limit, [
      (off, lim) => this.#mailPage(scope, off, lim),
      ...this.#paper.map(
        (box) => (off, lim) => this.#paperPage(box.id, off, lim, query, category),
      ),
    ]);
  }

  async #mailPage(scope, offset, limit) {
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

  /**
   * One window of one address's post.
   *
   * `category` filters here rather than in Paperless, which knows nothing
   * about the classifier's six. That means a category scope over post is
   * filtered after the fact and its total is the unfiltered one — honest, and
   * the alternative is teaching Paperless a taxonomy that is not its.
   */
  async #paperPage(mailboxId, offset, limit, query, category = null) {
    const page = await this.#invoke('paper_documents', {
      id: mailboxId,
      offset,
      limit,
      query,
    });

    let rows = page.rows.map((row) => paperRow(row, mailboxId));
    if (category) rows = rows.filter((row) => row.category === category);

    return { total: category ? rows.length : page.total, rows };
  }

  async message(id) {
    const paper = parsePaperId(id);
    if (paper) {
      const detail = await this.#invoke('paper_document', {
        id: paper.mailboxId,
        documentId: paper.documentId,
      });
      return {
        row: paperRow(detail.row, paper.mailboxId),
        body: detail.body_text ?? null,
        // What the rules read, so the reading pane can say why a letter was
        // filed where it was — the same explanation mail gets.
        facts: detail.facts ?? null,
      };
    }

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
      // The facts the rules read, re-parsed from the stored message. Without
      // them the reading pane can name a category but not say why, which is
      // the half of the rules layer that makes a wrong answer arguable.
      facts: detail.facts ?? null,
    };
  }

  async archive(id) {
    // Paperless owns its documents; this reads them. Refusing is the honest
    // answer, and a great deal better than appearing to file post somewhere it
    // has not gone.
    if (parsePaperId(id)) throw new Unsupported('archive post');
    if (!this.#special.archive) throw new Error('this account has no Archive folder');
    await this.#move(id, this.#special.archive);
  }

  async trash(id) {
    // Paperless owns its documents; this reads them. Refusing is the honest
    // answer, and a great deal better than appearing to file post somewhere it
    // has not gone.
    if (parsePaperId(id)) throw new Unsupported('move post to Trash');
    if (!this.#special.trash) throw new Error('this account has no Trash folder');
    // `move_to`, never a delete. Nothing destroys mail.
    await this.#move(id, this.#special.trash);
  }

  async setRead(id, read) {
    // Paperless tracks no read state, and inventing one here would be a
    // promise this cannot keep across a restart.
    if (parsePaperId(id)) throw new Unsupported('mark post read');
    await this.#invoke('set_read', { account: this.#account, id: Number(id), read });
  }

  async setCategory(id, category) {
    const paper = parsePaperId(id);
    if (paper) {
      // Post has its own corrections, pinned to the letter and learned from
      // its sender — the correspondent, or the letterhead when Paperless has
      // none. The command asks Paperless rather than trusting the row this
      // list happens to hold.
      await this.#invoke('file_post', {
        id: paper.mailboxId,
        documentId: paper.documentId,
        category,
      });
      return;
    }
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

/**
 * Merges several sorted sources into one window.
 *
 * Each source is asked for `offset + limit` rows, which is exactly what a merge
 * of newest-first lists needs: an item at index `offset` of the result cannot
 * be further than `offset` into any one source. Deep scrolling therefore costs
 * more than shallow scrolling, which is worth knowing and has not mattered
 * yet — the list pages a hundred at a time.
 */
async function merge(offset, limit, sources) {
  const need = offset + limit;
  const pages = await Promise.all(
    sources.map((source) =>
      source(0, need).catch((error) => {
        // One source being unreachable must not empty the whole list. It comes
        // back short, and the rest of the mail is still readable.
        console.warn('kuverta-bird: a source failed while merging', error);
        return { total: 0, rows: [] };
      }),
    ),
  );

  const rows = pages
    .flatMap((page) => page.rows)
    // Newest first, ties broken by id so two runs never disagree.
    .sort((a, b) => (b.date ?? 0) - (a.date ?? 0) || a.id.localeCompare(b.id));

  return {
    total: pages.reduce((sum, page) => sum + page.total, 0),
    rows: rows.slice(offset, offset + limit),
  };
}

/**
 * A document id, namespaced by the address it came from.
 *
 * The port says ids are opaque, which is what makes this possible: mail keeps
 * its store id and post gets a prefix, and the one list can carry both without
 * either side knowing about the other.
 */
const paperId = (mailboxId, documentId) => `paper:${mailboxId}:${documentId}`;

function parsePaperId(id) {
    const parts = String(id).split(':');
    if (parts.length !== 3 || parts[0] !== 'paper') return null;
    const mailboxId = Number(parts[1]);
    const documentId = Number(parts[2]);
    if (!Number.isFinite(mailboxId) || !Number.isFinite(documentId)) return null;
    return { mailboxId, documentId };
}

/** A document in the port's shape, indistinguishable from a message row. */
function paperRow(row, mailboxId) {
  return {
    id: paperId(mailboxId, row.id),
    subject: row.subject ?? null,
    from: row.from ?? null,
    fromAddr: row.from ?? null,
    date: secondsToMillis(row.date_utc),
    unread: Boolean(row.unread),
    hasAttachments: Boolean(row.has_attachments),
    category: row.category ?? null,
    // Filing is the one thing that can be done to post. Archive, trash and read
    // state are Paperless's documents' business, and this only reads them; a
    // category is this client's own record. Saying so on the row means the
    // surface refuses the rest immediately and specifically.
    actions: [ACTIONS.SetCategory],
  };
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

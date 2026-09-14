/**
 * The triage model.
 *
 * One keyboard-first list you act from, with the classifier's categories as
 * the primary axis rather than folders — brief §3.1. This file is that list's
 * behaviour with no DOM in it, because the behaviour is the part that is easy
 * to get wrong and the part worth testing.
 *
 * Two rules earn most of their keep here:
 *
 * **Acting keeps the cursor where it was.** Archive the message under the
 * cursor and the next one slides into its place, rather than the list jumping
 * back to the top and making you find your way again. This is most of what
 * makes triage bearable.
 *
 * **A filtered list says what it is showing, and offers a way out.** A list
 * that quietly narrows looks exactly like mail going missing, and mail going
 * missing is the thing a mail client may never look like it is doing.
 */

import { ACTIONS, Unsupported, describeScope } from './host.js';

/** How many rows to ask a host for at once. */
const PAGE_SIZE = 100;

/** How each action reads when a row refuses it. */
const VERBS = Object.freeze({
  [ACTIONS.Archive]: 'archived',
  [ACTIONS.Trash]: 'moved to Trash',
  [ACTIONS.ToggleRead]: 'marked read',
  [ACTIONS.SetCategory]: 'filed by category',
});

/**
 * Whether a row allows an action.
 *
 * A row that says nothing allows whatever its host can do, which keeps every
 * existing adapter correct without changing a line of it.
 */
function allows(row, action) {
  return !Array.isArray(row.actions) || row.actions.includes(action);
}

/** The scope shown before anything is chosen. */
const EVERYTHING = Object.freeze({ kind: 'all', label: 'Everything' });

export class Triage {
  #host;
  #pageSize;
  #listeners = new Set();

  /** Sparse: index -> Row. A mailbox does not fit in memory. */
  #rows = new Map();
  /** Offsets already asked for, so a scroll does not ask twice. */
  #requested = new Set();

  #scope = EVERYTHING;
  #total = 0;
  #selected = -1;
  #busy = false;
  #notice = null;
  #open = null;

  constructor(host, { pageSize = PAGE_SIZE } = {}) {
    this.#host = host;
    this.#pageSize = pageSize;
  }

  // -- reading ------------------------------------------------------------

  get host() {
    return this.#host;
  }

  get capabilities() {
    return this.#host.capabilities;
  }

  get scope() {
    return this.#scope;
  }

  get scopeLabel() {
    return describeScope(this.#scope);
  }

  /**
   * Whether the list is showing less than everything.
   *
   * The view uses this to decide whether it owes the user a way out. It is
   * deliberately true for a search of one word as much as for a category: the
   * question is not how narrow the filter is, it is whether one is on.
   */
  get isFiltered() {
    return this.#scope.kind !== 'all';
  }

  get total() {
    return this.#total;
  }

  get selected() {
    return this.#selected;
  }

  get busy() {
    return this.#busy;
  }

  get notice() {
    return this.#notice;
  }

  /** The opened message, if one is open. */
  get open() {
    return this.#open;
  }

  /** The row at an index, or null if it has not been fetched yet. */
  rowAt(index) {
    return this.#rows.get(index) ?? null;
  }

  get selectedRow() {
    return this.rowAt(this.#selected);
  }

  // -- change notification ------------------------------------------------

  /** @param {Function} listener @returns {Function} unsubscribe */
  subscribe(listener) {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  #changed() {
    for (const listener of this.#listeners) listener(this);
  }

  // -- loading ------------------------------------------------------------

  /**
   * Points the list at a scope and starts again.
   *
   * The cursor resets here, and only here: a filter change makes the old
   * position meaningless, where an action does not.
   */
  async setScope(scope) {
    this.#scope = scope ?? EVERYTHING;
    this.#selected = -1;
    this.#open = null;
    await this.#reload({ keepCursor: false });
  }

  /** Back to everything. The "way out" the filtered list owes the user. */
  async clearScope() {
    await this.setScope(EVERYTHING);
  }

  async start() {
    await this.#reload({ keepCursor: false });
  }

  /**
   * Throws away what was fetched and asks again.
   *
   * @param {{keepCursor: boolean}} options
   */
  async #reload({ keepCursor }) {
    const wasAt = this.#selected;

    this.#rows.clear();
    this.#requested.clear();
    this.#open = null;

    await this.#fetch(0);

    if (this.#total === 0) {
      this.#selected = -1;
    } else if (keepCursor) {
      // The row that was under the cursor has gone, so this same index is now
      // the one that followed it. Clamped, because acting on the last row has
      // nowhere below to go.
      this.#selected = Math.min(Math.max(wasAt, 0), this.#total - 1);
    } else {
      this.#selected = 0;
    }

    this.#changed();
  }

  /**
   * Makes sure the rows around an index are loaded.
   *
   * Called by the view as it scrolls. Safe to call for an index that is
   * already loaded, and safe to call twice for one that is in flight.
   */
  async ensureLoaded(index) {
    const offset = Math.floor(index / this.#pageSize) * this.#pageSize;
    if (this.#requested.has(offset)) return;
    await this.#fetch(offset);
    this.#changed();
  }

  async #fetch(offset) {
    this.#requested.add(offset);
    try {
      const page = await this.#host.page({
        scope: this.#scope,
        offset,
        limit: this.#pageSize,
      });
      page.rows.forEach((row, i) => this.#rows.set(offset + i, row));
      // A host that cannot count cheaply says -1; fall back to what arrived,
      // which at least lets the list draw and scroll.
      this.#total = page.total >= 0 ? page.total : offset + page.rows.length;
    } catch (error) {
      // A page that will not load is worth saying out loud — silently showing
      // an empty list is the "mail going missing" failure again.
      this.#requested.delete(offset);
      this.#say(`could not load messages: ${error.message}`, 'error');
    }
  }

  // -- moving -------------------------------------------------------------

  /**
   * Moves the cursor. Clamps rather than wrapping: wrapping from the last
   * message to the first is never what was meant by pressing `j` once more.
   */
  move(delta) {
    if (this.#total === 0) return;
    const next = Math.min(Math.max(this.#selected + delta, 0), this.#total - 1);
    if (next === this.#selected) return;
    this.#selected = next;
    // Opening follows the cursor only when something is already open, so `j`
    // through a list does not start fetching bodies nobody asked for.
    if (this.#open !== null) this.#open = null;
    this.#changed();
  }

  select(index) {
    if (index < 0 || index >= this.#total || index === this.#selected) return;
    this.#selected = index;
    this.#open = null;
    this.#changed();
  }

  // -- acting -------------------------------------------------------------

  /**
   * Acts on the message under the cursor.
   *
   * @param {string} action One of `ACTIONS`.
   */
  async act(action) {
    const row = this.selectedRow;
    if (!row) return;
    if (this.#busy) return;

    const can = this.#host.capabilities;
    if (action === ACTIONS.Archive && !can.archive) return this.#cannot('archive');
    if (action === ACTIONS.Trash && !can.trash) return this.#cannot('move to Trash');
    if (action === ACTIONS.ToggleRead && !can.setRead) return this.#cannot('change read state');

    // The host may be able to do it and this particular row still not allow
    // it — post in a list of mail. Checked here so the answer is immediate and
    // specific rather than a round trip that comes back refused.
    // Reading is not a change a row can refuse. Every row in a list can be
    // opened; only what it allows *done* to it is per-row. Checking Open here
    // made every letter unreadable from the keyboard — a list entry allowing
    // filing but not reading — and only a real browser noticed, because a
    // click opens a row without going through `act`.
    if (action !== ACTIONS.Open && !allows(row, action)) return this.#refuses(row, action);

    this.#busy = true;
    this.#changed();
    try {
      switch (action) {
        case ACTIONS.Archive:
          await this.#host.archive(row.id);
          this.#say(`archived${this.#undoHint()}`);
          break;
        case ACTIONS.Trash:
          await this.#host.trash(row.id);
          this.#say(`moved to Trash${this.#undoHint()}`);
          break;
        case ACTIONS.ToggleRead:
          await this.#host.setRead(row.id, row.unread);
          this.#say(row.unread ? 'marked read' : 'marked unread');
          break;
        case ACTIONS.Open:
          await this.openSelected();
          return;
        default:
          return;
      }
      await this.#reload({ keepCursor: true });
    } catch (error) {
      this.#say(this.#explain(error), 'error');
    } finally {
      this.#busy = false;
      this.#changed();
    }
  }

  /** Files the message under the cursor, which is also a correction. */
  async setCategory(category) {
    const row = this.selectedRow;
    if (!row) return;
    if (!this.#host.capabilities.setCategory) return this.#cannot('file by category');
    if (!allows(row, ACTIONS.SetCategory)) return this.#refuses(row, ACTIONS.SetCategory);

    try {
      await this.#host.setCategory(row.id, category);
      this.#say(`filed as ${category}`);
      // A category scope is a filter the message may just have left, so the
      // list has to agree with what was asked for.
      await this.#reload({ keepCursor: true });
    } catch (error) {
      this.#say(this.#explain(error), 'error');
    }
  }

  /**
   * Reverses the last change, in whichever of the two senses this host means.
   *
   * The wording is not cosmetic. A queued change that is cancelled never
   * happened; a compensating change is a second change that puts things back,
   * and anyone watching the mailbox saw both.
   */
  async undo() {
    const kind = this.#host.capabilities.undo;
    if (!kind) return this.#cannot('undo');

    try {
      const change = await this.#host.undo();
      if (!change) {
        this.#say('nothing to undo');
        return;
      }
      this.#say(kind === 'queued' ? `cancelled: ${change.what}` : `put back: ${change.what}`);
      await this.#reload({ keepCursor: true });
    } catch (error) {
      this.#say(this.#explain(error), 'error');
    }
  }

  async sync() {
    if (!this.#host.capabilities.sync) return this.#cannot('sync');
    this.#busy = true;
    this.#changed();
    try {
      const said = await this.#host.sync();
      if (said) this.#say(said);
      await this.#reload({ keepCursor: true });
    } catch (error) {
      this.#say(this.#explain(error), 'error');
    } finally {
      this.#busy = false;
      this.#changed();
    }
  }

  async openSelected() {
    const row = this.selectedRow;
    if (!row) return;
    try {
      this.#open = await this.#host.message(row.id);
      // Reading a message marks it read, which is a change to the row.
      if (row.unread && this.#host.capabilities.setRead) {
        await this.#host.setRead(row.id, true);
        this.#rows.set(this.#selected, { ...row, unread: false });
      }
      this.#changed();
    } catch (error) {
      this.#say(this.#explain(error), 'error');
    }
  }

  closeOpen() {
    if (this.#open === null) return;
    this.#open = null;
    this.#changed();
  }

  // -- notices ------------------------------------------------------------

  #undoHint() {
    const kind = this.#host.capabilities.undo;
    if (kind === 'queued') return ' — z to cancel';
    if (kind === 'compensating') return ' — z to put it back';
    return '';
  }

  #cannot(what) {
    this.#say(`this host cannot ${what}`, 'error');
  }

  /** This row, specifically, does not allow that. */
  #refuses(row, action) {
    const what = VERBS[action] ?? action;
    this.#say(`${row.subject ? 'this' : 'that'} cannot be ${what} from here`, 'error');
  }

  #explain(error) {
    return error instanceof Unsupported ? error.message : String(error.message ?? error);
  }

  #say(text, tone = 'info') {
    this.#notice = { text, tone, at: Date.now() };
    this.#changed();
  }
}

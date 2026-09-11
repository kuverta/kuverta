/**
 * The triage surface.
 *
 * Mounted by both hosts against a `Triage` over their own adapter, so the list
 * looks and behaves the same in the Thunderbird tab and in the standalone
 * client's window. It knows about the DOM and about `core/triage.js`, and
 * about neither Thunderbird nor `fuckmail`.
 *
 * The list is windowed: a screenful of rows exists in the DOM and a spacer
 * gives the scroller the height of the whole mailbox. A mailbox has more
 * messages than a browser will happily lay out, and finding that out at
 * 40,000 messages is worse than building it this way now.
 */

import { CATEGORY_LABELS, ALL_CATEGORIES } from '../category.js';
import { CHEAT_SHEET, INTENT, INTENT_ACTIONS, PRIMARY_KEYS, intentFor, isTyping } from '../keys.js';

const ROW_HEIGHT = 56;
/** Rows drawn above and below the visible window, so scrolling is not blank. */
const OVERSCAN = 6;

/**
 * @param {object} options
 * @param {HTMLElement} options.root   Where to build the surface.
 * @param {import('../triage.js').Triage} options.triage
 * @param {Function} [options.onCompose]  Host-specific; hidden when absent.
 * @param {string} [options.title]  What this mailbox is — an account address,
 *   usually. The core has no idea which account it is looking at, and the
 *   person reading it very much needs to.
 */
export function mountTriage({ root, triage, onCompose = null, title = '' }) {
  root.classList.add('fb-triage');
  root.innerHTML = LAYOUT;

  const el = (id) => root.querySelector(`#fb-${id}`);
  const scopeLabel = el('scope-label');
  const scopeOut = el('scope-out');
  const emptyList = el('empty-list');
  const sidebar = el('sidebar');
  const viewport = el('viewport');
  const spacer = el('spacer');
  const content = el('content');
  const reading = el('reading');
  const empty = el('empty');
  const toast = el('toast');
  const help = el('help');
  const search = el('search');
  const legend = el('legend');

  /** Reused row elements. Rebuilt only when the window is resized. */
  let pool = [];
  let firstRendered = -1;
  let lastNotice = 0;

  el('title').textContent = title;
  buildHelp();
  buildLegend();
  buildPool();
  drawSidebar();

  const unsubscribe = triage.subscribe(() => {
    draw(true);
    drawNotice();
    drawReading();
  });

  viewport.addEventListener('scroll', () => draw(false), { passive: true });

  // Everything else is attached to elements inside `root`, which the next
  // mount replaces wholesale. These two outlive it, so they are the two that
  // have to be given back.
  const onResize = () => {
    buildPool();
    draw(true);
  };
  window.addEventListener('resize', onResize);

  content.addEventListener('click', (event) => {
    const row = event.target.closest('.fb-row');
    if (!row) return;
    triage.select(Number(row.dataset.index));
    triage.openSelected();
  });

  scopeOut.addEventListener('click', () => triage.clearScope());

  el('helpbtn').addEventListener('click', () => {
    help.hidden = !help.hidden;
  });
  help.addEventListener('click', () => {
    help.hidden = true;
  });

  search.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      const query = search.value.trim();
      if (query) {
        triage.setScope({ kind: 'search', value: query, label: `Search: ${query}` });
      } else {
        triage.clearScope();
      }
      search.blur();
    }
    if (event.key === 'Escape') {
      search.value = '';
      search.blur();
      triage.clearScope();
    }
  });

  document.addEventListener('keydown', onKey);

  draw(true);

  /**
   * Lets go of the document.
   *
   * A host that can switch mailboxes mounts this more than once. Without a
   * teardown the old surface keeps listening, and every keystroke acts twice —
   * once on a list nobody can see, which with `e` and `#` bound means archiving
   * a message the user never looked at.
   */
  return () => {
    document.removeEventListener('keydown', onKey);
    window.removeEventListener('resize', onResize);
    unsubscribe();
  };

  // -- keys ---------------------------------------------------------------

  async function onKey(event) {
    // While the user is typing, every key belongs to the field. Without this,
    // typing `e` into the search box archives something.
    if (isTyping(event.target)) return;

    if (!help.hidden && event.key === 'Escape') {
      help.hidden = true;
      return;
    }

    // Digits file by category: 1 is the first category, and so on. Only
    // offered when the host can actually do it.
    if (/^[1-9]$/.test(event.key) && triage.capabilities.setCategory) {
      const category = ALL_CATEGORIES[Number(event.key) - 1];
      if (category) {
        event.preventDefault();
        await triage.setCategory(category);
      }
      return;
    }

    const intent = intentFor(event);
    if (intent === null) return;

    const action = INTENT_ACTIONS[intent];
    if (action) {
      event.preventDefault();
      await triage.act(action);
      return;
    }

    switch (intent) {
      case INTENT.Down:
        event.preventDefault();
        triage.move(1);
        break;
      case INTENT.Up:
        event.preventDefault();
        triage.move(-1);
        break;
      case INTENT.Undo:
        event.preventDefault();
        await triage.undo();
        break;
      case INTENT.Sync:
        event.preventDefault();
        await triage.sync();
        break;
      case INTENT.Search:
        event.preventDefault();
        search.focus();
        search.select();
        break;
      case INTENT.Close:
        if (triage.open) triage.closeOpen();
        else if (triage.isFiltered) triage.clearScope();
        break;
      case INTENT.Explain:
        event.preventDefault();
        help.hidden = !help.hidden;
        break;
      case INTENT.Compose:
        if (onCompose) {
          event.preventDefault();
          onCompose();
        }
        break;
      default:
        break;
    }
  }

  // -- drawing ------------------------------------------------------------

  function buildPool() {
    const visible = Math.ceil(viewport.clientHeight / ROW_HEIGHT) || 10;
    const size = visible + OVERSCAN * 2;
    content.replaceChildren();
    pool = Array.from({ length: size }, () => {
      const node = document.createElement('div');
      node.className = 'fb-row';
      node.innerHTML = ROW;
      content.append(node);
      return node;
    });
    firstRendered = -1;
  }

  function draw(force) {
    spacer.style.height = `${Math.max(triage.total, 0) * ROW_HEIGHT}px`;

    const first = Math.max(0, Math.floor(viewport.scrollTop / ROW_HEIGHT) - OVERSCAN);
    if (!force && first === firstRendered) return;
    firstRendered = first;
    content.style.transform = `translateY(${first * ROW_HEIGHT}px)`;

    pool.forEach((node, i) => {
      const index = first + i;
      const row = triage.rowAt(index);

      if (index >= triage.total) {
        node.hidden = true;
        return;
      }
      node.hidden = false;
      node.dataset.index = String(index);
      node.classList.toggle('fb-selected', index === triage.selected);

      if (!row) {
        // Not fetched yet. Asking here is what makes the list load as it
        // scrolls rather than all at once.
        node.classList.add('fb-pending');
        node.querySelector('.fb-subject').textContent = '…';
        node.querySelector('.fb-from').textContent = '';
        node.querySelector('.fb-date').textContent = '';
        node.querySelector('.fb-cat').textContent = '';
        triage.ensureLoaded(index);
        return;
      }

      node.classList.remove('fb-pending');
      node.classList.toggle('fb-unread', row.unread);
      node.querySelector('.fb-subject').textContent = row.subject || '(no subject)';
      node.querySelector('.fb-from').textContent = row.from || row.fromAddr || '';
      node.querySelector('.fb-date').textContent = when(row.date);

      const category = node.querySelector('.fb-cat');
      category.textContent = row.category ? CATEGORY_LABELS[row.category] : '';
      category.dataset.category = row.category ?? '';

      node.querySelector('.fb-clip').hidden = !row.hasAttachments;
    });

    keepCursorVisible();
    drawScope();
  }

  function keepCursorVisible() {
    if (triage.selected < 0) return;
    const top = triage.selected * ROW_HEIGHT;
    const bottom = top + ROW_HEIGHT;
    if (top < viewport.scrollTop) viewport.scrollTop = top;
    else if (bottom > viewport.scrollTop + viewport.clientHeight) {
      viewport.scrollTop = bottom - viewport.clientHeight;
    }
  }

  function drawScope() {
    // Always shown, never hidden. §3.1 asks a filtered list to say what it is
    // showing; a list that says nothing when unfiltered leaves you working out
    // where you are from the rows, which is the same problem in a milder form.
    const count = triage.total === 1 ? '1 message' : `${triage.total} messages`;
    scopeLabel.textContent = `${triage.scopeLabel} · ${count}`;

    // The way out appears only when there is something to get out of, so an
    // unfiltered list does not carry a button that does nothing.
    scopeOut.hidden = !triage.isFiltered;

    // An empty list has to say why it is empty and what to do about it.
    // Otherwise it is indistinguishable from mail having gone missing.
    emptyList.hidden = triage.total !== 0;
    if (triage.total === 0) {
      emptyList.textContent = triage.isFiltered
        ? `Nothing in ${triage.scopeLabel}. Press Escape, or use “Show everything”.`
        : 'No messages here yet. Sync with r, or pick a folder on the left.';
    }
  }

  function drawSidebar() {
    triage.host.scopes().then((scopes) => {
      sidebar.replaceChildren(
        ...scopes.map((scope) => {
          const button = document.createElement('button');
          button.className = 'fb-scope';
          button.textContent = scope.label;
          if (scope.count) {
            const count = document.createElement('span');
            count.className = 'fb-count';
            count.textContent = String(scope.count);
            button.append(count);
          }
          button.addEventListener('click', () => triage.setScope(scope));
          return button;
        }),
      );
    });
  }

  function drawReading() {
    const open = triage.open;
    reading.hidden = !open;
    empty.hidden = Boolean(open);
    if (!open) {
      // A highlighted row beside a pane saying "select a message" reads as
      // though the selection did not take. Say what to press instead.
      empty.textContent = triage.selectedRow
        ? 'Press ↵ to read this message'
        : 'Nothing selected';
      return;
    }

    reading.querySelector('.fb-read-subject').textContent = open.row.subject || '(no subject)';
    reading.querySelector('.fb-read-meta').textContent =
      `${open.row.from ?? ''} · ${when(open.row.date)}`;
    // textContent, never innerHTML: this is a mail body, which is to say it is
    // written by whoever sent it.
    reading.querySelector('.fb-read-body').textContent =
      open.body ?? '(no plain-text body)';
  }

  function drawNotice() {
    const notice = triage.notice;
    if (!notice || notice.at === lastNotice) return;
    lastNotice = notice.at;
    toast.textContent = notice.text;
    toast.className = `fb-toast${notice.tone === 'error' ? ' fb-error' : ''}`;
    toast.hidden = false;
    clearTimeout(toast._timer);
    toast._timer = setTimeout(() => {
      toast.hidden = true;
    }, 4000);
  }

  function buildLegend() {
    legend.replaceChildren(
      ...PRIMARY_KEYS.map(([keys, what]) => {
        const item = document.createElement('span');
        const kbd = document.createElement('kbd');
        kbd.textContent = keys;
        const text = document.createElement('span');
        text.textContent = what;
        item.append(kbd, text);
        return item;
      }),
    );
  }

  function buildHelp() {
    help.replaceChildren(
      ...CHEAT_SHEET.map(([keys, what]) => {
        const line = document.createElement('div');
        const kbd = document.createElement('kbd');
        kbd.textContent = keys;
        const text = document.createElement('span');
        text.textContent = what;
        line.append(kbd, text);
        return line;
      }),
    );
  }
}

/** A date a reader can scan: a time today, a weekday this week, else a date. */
function when(millis) {
  if (!millis) return '';
  const date = new Date(millis);
  const now = new Date();
  const sameDay = date.toDateString() === now.toDateString();
  if (sameDay) return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const days = (now - date) / 86_400_000;
  if (days < 7) return date.toLocaleDateString([], { weekday: 'short' });
  return date.toLocaleDateString([], { day: '2-digit', month: 'short' });
}

const ROW = `
  <span class="fb-cat"></span>
  <span class="fb-from"></span>
  <span class="fb-subject"></span>
  <span class="fb-clip" hidden>📎</span>
  <span class="fb-date"></span>
`;

const LAYOUT = `
  <aside id="fb-sidebar" class="fb-sidebar"></aside>
  <section class="fb-main">
    <div class="fb-head">
      <span id="fb-title" class="fb-title"></span>
      <input id="fb-search" class="fb-search" type="search" placeholder="/ to search" spellcheck="false">
      <button id="fb-helpbtn" class="fb-ghost" title="Shortcuts (?)">?</button>
    </div>
    <div class="fb-scopebar">
      <span id="fb-scope-label"></span>
      <button id="fb-scope-out" class="fb-ghost" hidden>Show everything</button>
    </div>
    <div id="fb-viewport" class="fb-viewport">
      <div id="fb-spacer"></div>
      <div id="fb-content"></div>
      <p id="fb-empty-list" class="fb-empty-list fb-muted" hidden></p>
    </div>
  </section>
  <section class="fb-detail">
    <div id="fb-empty" class="fb-muted">Nothing selected</div>
    <article id="fb-reading" hidden>
      <h2 class="fb-read-subject"></h2>
      <p class="fb-read-meta fb-muted"></p>
      <pre class="fb-read-body"></pre>
    </article>
  </section>
  <footer id="fb-legend" class="fb-legend"></footer>
  <div id="fb-toast" class="fb-toast" hidden></div>
  <div id="fb-help" class="fb-help" hidden></div>
`;

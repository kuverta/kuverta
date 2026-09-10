// Virtualized list + a self-measuring scroll benchmark.
//
// The list recycles a fixed pool of DOM nodes rather than rebuilding markup on
// every frame. That is what any real virtualizer does, so it is the fair test
// of the achievable ceiling — measuring a naive innerHTML rewrite would tell us
// about our own laziness rather than about Tauri.

const { invoke } = window.__TAURI__.core;

const ROW_HEIGHT = 28;
const OVERSCAN = 6; // rows rendered above and below the viewport

const viewport = document.getElementById("viewport");
const spacer = document.getElementById("spacer");
const content = document.getElementById("content");
const status = document.getElementById("status");
const results = document.getElementById("results");

let rows = [];
let pool = [];
let firstRendered = -1;

/** Builds the recycled node pool, sized to the viewport. */
function buildPool() {
  const visible = Math.ceil(viewport.clientHeight / ROW_HEIGHT);
  const size = visible + OVERSCAN * 2;

  content.textContent = "";
  pool = [];

  for (let i = 0; i < size; i++) {
    const row = document.createElement("div");
    row.className = "row";

    const date = document.createElement("span");
    date.className = "date";
    const sender = document.createElement("span");
    sender.className = "sender";
    const subject = document.createElement("span");
    subject.className = "subject";
    const meta = document.createElement("span");
    meta.className = "meta";

    row.append(date, sender, subject, meta);
    content.append(row);
    pool.push({ row, date, sender, subject, meta });
  }
}

/** Renders the slice of rows visible at the current scroll offset. */
function render() {
  const first = Math.max(
    0,
    Math.min(
      Math.floor(viewport.scrollTop / ROW_HEIGHT) - OVERSCAN,
      Math.max(0, rows.length - pool.length),
    ),
  );

  if (first === firstRendered) return;
  firstRendered = first;

  // One transform for the whole window instead of positioning each row.
  content.style.transform = `translateY(${first * ROW_HEIGHT}px)`;

  for (let i = 0; i < pool.length; i++) {
    const slot = pool[i];
    const item = rows[first + i];

    if (!item) {
      slot.row.hidden = true;
      continue;
    }
    slot.row.hidden = false;

    // textContent, never innerHTML: subjects are attacker-controlled strings.
    slot.date.textContent = item.date;
    slot.sender.textContent = item.sender;
    slot.subject.textContent = item.subject;

    const marks = [];
    if (item.has_attachments) marks.push("attachment");
    if (item.list_id) marks.push(item.list_id);
    slot.meta.textContent = marks.join(" · ");

    slot.row.classList.toggle("unread", item.unread);
  }
}

viewport.addEventListener("scroll", render, { passive: true });

/**
 * Scrolls the whole list under requestAnimationFrame, recording how long each
 * frame took. Frame deltas are what a user perceives as smooth or janky.
 */
function measureScroll() {
  return new Promise((resolve, reject) => {
    // requestAnimationFrame stops firing when the window is occluded or the
    // display sleeps, so without this the run hangs indefinitely rather than
    // reporting anything. The spike needs a visible, frontmost window.
    let lastProgress = performance.now();
    const watchdog = setInterval(() => {
      if (performance.now() - lastProgress > 5000) {
        clearInterval(watchdog);
        reject(new Error(
          "no frames for 5s — the window is probably occluded or the display " +
          "slept. requestAnimationFrame only runs while the window is visible.",
        ));
      }
    }, 1000);

    const maxScroll = viewport.scrollHeight - viewport.clientHeight;
    // ~700 frames over the full list: enough samples for a stable p95 without
    // making the run take longer than a few seconds.
    const step = Math.max(1, maxScroll / 700);

    const deltas = [];
    let last = performance.now();
    const started = last;
    viewport.scrollTop = 0;

    function frame(now) {
      lastProgress = performance.now();
      deltas.push(now - last);
      last = now;

      viewport.scrollTop += step;
      render();

      if (viewport.scrollTop < maxScroll - step) {
        requestAnimationFrame(frame);
      } else {
        clearInterval(watchdog);
        // Drop the first frame: it includes scheduling noise from the caller.
        resolve({ deltas: deltas.slice(1), elapsedMs: now - started });
      }
    }

    requestAnimationFrame(frame);
  });
}

function percentile(sorted, p) {
  if (sorted.length === 0) return 0;
  const index = Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length));
  return sorted[index];
}

async function run() {
  const config = await invoke("spike_config");

  status.textContent = `loading ${config.rows.toLocaleString()} rows…`;

  const ipcStart = performance.now();
  rows = await invoke("load_rows", { count: config.rows });
  const ipcMs = performance.now() - ipcStart;

  // Approximates what crossed the bridge. Measured after the fact so it does
  // not inflate ipcMs.
  const payloadBytes = new TextEncoder().encode(JSON.stringify(rows)).length;

  const paintStart = performance.now();
  spacer.style.height = `${rows.length * ROW_HEIGHT}px`;
  buildPool();
  render();
  const firstPaintMs = performance.now() - paintStart;

  status.textContent = "measuring scroll…";
  // Let the first paint settle so it is not attributed to the scroll.
  await new Promise((r) => setTimeout(r, 250));

  const { deltas, elapsedMs } = await measureScroll();
  const sorted = [...deltas].sort((a, b) => a - b);

  // WKWebView clamps performance.now() to 1ms as a Spectre mitigation, so a
  // perfect 60fps frame (16.67ms) is reported as 17ms. Comparing against a
  // hardcoded 16.7ms therefore flags every healthy frame as late. Instead take
  // the median as the display's frame budget — during a steady scroll most
  // frames hit vsync exactly — and count a frame as dropped only when it took
  // long enough to have missed a whole vsync interval.
  const budgetMs = percentile(sorted, 50);
  const droppedThreshold = budgetMs * 1.5;
  const dropped = deltas.filter((d) => d >= droppedThreshold).length;
  const achievedFps = deltas.length / (elapsedMs / 1000);

  const metrics = {
    rows: rows.length,
    ipc_ms: ipcMs,
    payload_bytes: payloadBytes,
    first_paint_ms: firstPaintMs,
    frames: deltas.length,
    p50_ms: percentile(sorted, 50),
    p95_ms: percentile(sorted, 95),
    worst_ms: sorted[sorted.length - 1] ?? 0,
    budget_ms: budgetMs,
    dropped,
    achieved_fps: achievedFps,
    elapsed_ms: elapsedMs,
    dom_nodes: content.querySelectorAll("*").length,
  };

  status.textContent = "done";
  results.hidden = false;
  results.textContent =
    `rows ${metrics.rows}  |  IPC ${metrics.ipc_ms.toFixed(0)} ms ` +
    `(${(metrics.payload_bytes / 1048576).toFixed(1)} MiB)  |  ` +
    `first paint ${metrics.first_paint_ms.toFixed(0)} ms\n` +
    `p50 ${metrics.p50_ms.toFixed(1)} ms  p95 ${metrics.p95_ms.toFixed(1)} ms  ` +
    `worst ${metrics.worst_ms.toFixed(1)} ms  ` +
    `dropped ${metrics.dropped}/${metrics.frames}  ` +
    `${metrics.achieved_fps.toFixed(1)} fps\n` +
    `DOM nodes in list: ${metrics.dom_nodes}`;

  await invoke("report", { metrics });
}

run().catch(async (err) => {
  status.textContent = `failed: ${err}`;
  console.error(err);
  // Report to the terminal as well; the window may not be where anyone looks.
  try {
    await invoke("report_failure", { message: String(err) });
  } catch (_) {
    // Nothing more we can do from here.
  }
});

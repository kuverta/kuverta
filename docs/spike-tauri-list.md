# Spike: can a Tauri v2 webview render a mail-sized list?

Run 2026-09-10 on macOS (Apple Silicon, 59Hz display), release build.
Code: [`apps/desktop`](../apps/desktop).

The harness has since been replaced by the real window, so there is no
`make spike` any more — the benchmark code is in the history, at the commit
that removed it. The finding stands, and the list is still built on it.

## The question

The plan bets on a Rust core behind a web UI. That bet fails if a webview cannot
scroll a message list smoothly, and it is far cheaper to find that out in week
one than in month four. Concretely: **can it hold the display's refresh rate
while scrolling 50k+ rows, and what does moving that data across the Rust/JS
bridge cost?**

## Results

| rows | IPC (Rust→JS) | payload | first paint | achieved | dropped frames | worst frame |
|---:|---:|---:|---:|---:|---:|---:|
| 50,000 | 111 ms | 8.1 MiB | 4 ms | 59.8 fps | 0.1 % | 53 ms |
| 50,000 | 113 ms | 8.1 MiB | 4 ms | 59.8 fps | 0.3 % | 48 ms |
| 200,000 | 418 ms | 32.8 MiB | 8 ms | 59.8 fps | 0.7 % | 46 ms |

**Verdict: PASS.** The webview holds 59.8 fps of a 59 Hz display while scrolling
200k rows, with under 1 % of frames missing a vsync. The UI stack is not the
risk. Tauri v2 + a recycled-node virtualizer is fine for this workload.

## What the numbers actually say

**Scroll cost is independent of row count.** 50k and 200k both scroll at 59.8
fps because only ~195 DOM nodes ever exist — a pool sized to the viewport, with
text updated in place and one `transform` per frame for the whole window. Row
count only changes a spacer's height.

**The bridge is the real constraint, and it scales linearly.** 8.1 MiB in 111 ms;
32.8 MiB in 418 ms — about 78 MiB/s, spent on JSON serialisation in Tauri's IPC.
At 50k messages a 111 ms load is unnoticeable. At 200k, 418 ms is a visible stall
on open.

**Design consequence:** never ship the whole mailbox across the bridge. The store
already answers windowed queries (`Store::recent(account, limit)`), so the list
should request the slice it is about to display plus a margin, and a total count
for the scrollbar. That keeps per-interaction IPC in single-digit kilobytes and
removes the only number here that grows badly.

## Two measurement traps worth remembering

**WKWebView clamps `performance.now()` to 1 ms** as a Spectre mitigation. A
perfect 60 fps frame is 16.67 ms and reports as 17 ms, so the obvious threshold
— "count frames over 16.7 ms" — flags *every healthy frame* as late. The first
run of this spike reported "60.8 % dropped frames" on what was in fact a locked
60 fps. The benchmark now takes the median frame time as the display's budget
and counts a frame as dropped only at ≥ 1.5× that, which is quantisation-proof
and adapts to 120 Hz ProMotion displays for free.

**`requestAnimationFrame` stops when the window is occluded or the display
sleeps.** An early run hung indefinitely rather than failing. There is now a
5-second watchdog that aborts with an explanation; the spike needs a visible,
frontmost window.

The first run of any session also shows a one-off ~600 ms frame from window and
compositor warm-up. Subsequent runs peak around 50 ms.

## What this does not prove

Only that scrolling and rendering are cheap. Still unmeasured:

- **HTML message rendering** — the sandboxed body view is the hard, security
  sensitive part, and nothing here touches it.
- **Sync running concurrently with the UI**, competing for the same machine.
- **Windows and Linux**, which use WebView2 and WebKitGTK, not WKWebView.
- **Mobile**, which is not a target until paper capture needs the camera.

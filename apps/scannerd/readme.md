# scannerd

Watches a surface through a camera. When a page is put down and stops moving,
photographs it and hands it to Paperless-ngx.

That is all it does, and the restraint is the design. Paperless already does
OCR, deskew, rotation, tagging, archiving and search; this does the one thing it
cannot, which is noticing that a letter has arrived. The brief budgets ~600
lines for the capture daemon and that is about what it costs.

## What it does not do

**No deskew.** A photograph of a letter on a desk is never square, and
straightening it is `ocrmypdf`'s job — Paperless does it on every document, and
the dev stack sets `PAPERLESS_OCR_DESKEW` explicitly so the dependency is
visible rather than inherited.

**No page-finding.** The camera is bolted above a fixed spot, so the page is
always in the same part of the frame. `--roi x,y,w,h` crops on the sensor —
for the frames it watches as well as the photograph, so "half the frame changed"
means half the page area and not half the desk — which makes cropping a line of
configuration instead of a page of geometry —
and geometry that cannot be tested against real photographs is geometry that is
guessed at.

**No image library at all.** `rpicam-still --encoding yuv420` hands back raw
YUV, and the first plane *is* the greyscale image. Detection is arithmetic over
a byte slice, which is why it can be tested on a laptop with no camera.

## How it decides

Two questions, both answered by comparing frames against a baseline of the
empty surface:

- **Is something there?** A sheet of paper is large and pale, so it changes a
  great many pixels. A shadow changes them slightly.
- **Has it stopped?** A hand is still in shot for a moment after the page is,
  and capturing then gets a photograph of a thumb.

Then a third rule, which is the one that stops a letter becoming forty: after a
capture it will not arm again until the surface has been seen empty. That also
re-learns the baseline, so daylight moving across a desk over an afternoon does
not slowly read as a page.

## Nothing is lost

A capture is written to the spool *before* any upload is attempted and removed
only once Paperless has confirmed it. A crash, a flat battery or a Wi-Fi drop in
between means a retry, not a letter nobody has. The spool is a directory of
files rather than a database, so a Pi that was unplugged mid-write is
recoverable with `ls`; a half-written photograph keeps a `.partial` extension
and is never uploaded.

Waiting uploads are retried on a timer (`--drain-every-secs`, 30 by default) as
well as straight after each capture, and whether or not the camera is working —
so a letter photographed while the network was down is sent once it is back, not
when the next letter arrives. Each capture backs off from its last failed attempt
to a five-minute ceiling rather than growing without bound: a Pi that has been
offline overnight tries again within minutes once the network returns. An upload
that has not finished in two minutes fails and is kept, rather than holding up
the loop that watches for the next page.

## Running it

```sh
# against the dev stack, with no camera: drain whatever is spooled and exit
PAPERLESS_TOKEN=... scannerd --drain-only --url http://localhost:8000

# on the Pi
PAPERLESS_TOKEN=... scannerd --tag post --tag home --roi 0.15,0.10,0.70,0.80
```

`--drain-only` is for a cron job, and for checking the upload half works before
there is a camera to test the other half with.

Tags are given by name and must exist in Paperless first. Its upload endpoint
only takes tag ids, so `scannerd` looks each name up once and sends the id; a
name Paperless does not know fails the upload by name and keeps the capture.

## On the Pi

Cross-compile from the workspace root:

```sh
rustup target add aarch64-unknown-linux-gnu
cargo build --release -p scannerd --target aarch64-unknown-linux-gnu
```

Then `scannerd.service` and `scannerd.env.example` in this directory. It runs as
its own user with the `video` group and nothing else: a daemon that can reach
the camera and holds a Paperless token should not also be able to read the `pi`
user's home directory.

## Tests

```sh
cargo test -p scannerd
```

Thirty-eight of them, none needing a camera: the detector against synthetic
frames, the spool against a real directory, the uploader against a server on
loopback, and the loop itself (`tests/run.rs`) turned by hand with a scripted
camera and a Paperless that fails when told to. What they cannot check is whether `rpicam-still` behaves as
documented — that is what `--drain-only` and a first run on the device are for.

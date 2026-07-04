---
description: scannerd — a Raspberry Pi with a camera that notices a letter being put down, photographs it, and hands it to Paperless-ngx.
---

# The scanner: scannerd

`apps/scannerd` watches a surface through a camera. When a page is put down and
stops moving, it photographs it and hands it to Paperless-ngx.

That is all it does, and the restraint is the design. Paperless already does
OCR, deskewing, rotation, tagging, archiving and search; scannerd does the one
thing Paperless cannot, which is noticing that a letter has arrived.

- **No deskew** — that is ocrmypdf's job inside Paperless.
- **No page-finding** — the camera is fixed above one spot, so cropping is a
  line of configuration (`--roi x,y,w,h`), not geometry.
- **No image library** — `rpicam-still --encoding yuv420` returns raw YUV, whose
  first plane already is the greyscale image. Detection is arithmetic over bytes,
  and can be tested on a laptop with no camera.

## How it decides

It compares frames with a baseline of the empty surface and asks two questions:
*is something there?* (a sheet of paper is large and pale; a shadow changes
pixels only slightly) and *has it stopped moving?* (a hand is in shot for a
moment after the page is). After a capture it will not arm again until it has
seen the surface empty — which also relearns the baseline, so daylight moving
across a desk does not slowly read as a page.

**Nothing is lost.** A capture is written to the spool directory before any
upload is tried, and removed only once Paperless has confirmed it. Waiting
uploads are retried every 30 seconds (`--drain-every-secs`), backing off to at
most five minutes; an upload that has not finished in two minutes is kept for
later rather than holding up the camera.

## Letters of several pages

With `--button`, pages collect until the button is pressed, and the letter goes
to Paperless as one PDF with the photographs embedded as they are. A letter
nobody closes is sent `--letter-idle-secs` (five minutes) after its last page.
Without `--button`, every page is its own document.

The button is a push button between GPIO 17 and ground (pins 11 and 9), made a
key by the kernel with one line in `/boot/firmware/config.txt`:

```text
dtoverlay=gpio-key,gpio=17,active_low=1,gpio_pull=up,keycode=28,label=scannerd
```

After a reboot it shows up under `/dev/input/by-path/`; point `SCANNERD_BUTTON`
at it. A USB keypad works too (`--button-key` for a key other than Enter), and
on a laptop `--button stdin` makes Enter the button.

## The setup page

`--ui 0.0.0.0:8080` (or `SCANNERD_UI`) serves a page from scannerd itself, so a
headless Pi needs no screen: open `http://<pi>.local:8080` on a phone. It shows
what the camera sees and what scannerd makes of it (*Ready — put a page down*,
*Hold still…*, *Photographed — take the page away*), the pages of the current
letter with **Finish letter**, what was sent and what is waiting, and setup:

- **Take a picture** finds the page and proposes the crop; adjust it and **Save
  this area**.
- **The camera looks at the table at an angle**: drag four corners onto a page,
  and every photograph is straightened (about five seconds a page on a Pi Zero W).
- **Turn left** / **Turn right**, for a camera mounted so letters are not upright.
- The Paperless address, token and tags, with **Check connection**, and **Learn
  empty table**.

Settings saved there go to `settings.toml` in the spool (owner-only, it holds the
token) and win over the env file. Anything but a loopback address needs
`SCANNERD_UI_PASSWORD`, because the page shows photographs of your post.

## Hardware

Tested on a **Raspberry Pi Zero W** (Rev 1.1) with the OV5647 camera, running
32-bit Raspberry Pi OS bookworm. A preview frame takes 1.1–1.7 seconds there,
because `rpicam-still` starts the camera each time, and a full capture about 3.7
— so a page is photographed eight or nine seconds after it is put down. The
`3dprints/stand/` folder of the repository has OpenSCAD sources and STL files
for a stand built from 2020 aluminium profile, with a camera mount.

## Building for the Pi

Cross-compile from the workspace root with
[cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) and zig:

```sh
rustup target add arm-unknown-linux-gnueabihf     # Pi Zero W: ARMv6, 32-bit
cargo install cargo-zigbuild                      # and zig from ziglang.org on PATH
make scannerd-pi                                  # target/arm-unknown-linux-gnueabihf/release/scannerd
make scannerd-to-pi PI_HOST=pi@raspberrypi.local  # copies it to ~/scannerd, installs nothing
```

A Pi Zero W runs neither 64-bit Raspberry Pi OS nor an aarch64 binary. A Zero 2
W, 3, 4 or 5 on the 64-bit OS wants `PI_TARGET=aarch64-unknown-linux-gnu.2.36`
(after `rustup target add aarch64-unknown-linux-gnu`); the `.2.36` pins glibc to
bookworm's.

## Installing on the Pi

It runs as its own user with access to the camera and input devices and nothing
else — a daemon that holds a Paperless token should not be able to read the
`pi` user's home:

```sh
sudo useradd --system --no-create-home --shell /usr/sbin/nologin --groups video,input scannerd
sudo install -m 0755 scannerd /usr/local/bin/
sudo install -m 0644 scannerd.service /etc/systemd/system/
sudo install -m 0600 scannerd.env.example /etc/scannerd.env   # then fill it in
sudo systemctl enable --now scannerd
journalctl -u scannerd -f
```

Tags are given by name and must exist in Paperless first; a name Paperless does
not know fails the upload by name and keeps the capture.

## Without a Pi

`apps/scannerd/tools/rpicam-still-ffmpeg` stands in for `rpicam-still` using
ffmpeg, so the whole daemon runs on a laptop against the dev stack:

```sh
# a still image as the camera; point the symlink elsewhere to "put a page down"
ln -sf desk.jpg /tmp/camera.jpg
SCANNERD_FAKE_INPUT=/tmp/camera.jpg PAPERLESS_TOKEN=... \
  scannerd --camera apps/scannerd/tools/rpicam-still-ffmpeg --spool /tmp/spool --tag "Musterstraße 1"

# or the laptop's own camera
SCANNERD_FAKE_INPUT=0 PAPERLESS_TOKEN=... \
  scannerd --camera apps/scannerd/tools/rpicam-still-ffmpeg --spool /tmp/spool
```

`scannerd --drain-only` uploads whatever is spooled and exits — for a cron job,
and for checking the upload half before there is a camera.

```sh
cargo test -p scannerd    # detector, spool, uploader, letters, button, PDF — no camera needed
```

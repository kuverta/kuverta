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
- **Does it look like paper?** How much changed is not enough on its own: an
  evening moves the light over the whole table, and the rig photographed a few
  sunsets as letters. So the frame is also read in 16×16 blocks against the
  table's own level: paper is **paler than the table it lies on** — thirty-five
  counts above it, at least — and it is **one lump** rather than a wash, so the
  blocks that look like paper must fill nearly half the box they span. A shadow
  is neither: it darkens, and it is spread over everything. Both numbers are on
  the setup page beside the change, so a rig that will not see a page shows why.
- **Has it stopped?** A hand is still in shot for a moment after the page is,
  and capturing then gets a photograph of a thumb.

Then a third rule, which is the one that stops a letter becoming forty: after a
capture it will not arm again until the surface has been seen empty — or until
a hand has been over the page and what lies there, once still, is another page.
A letter's pages are often laid one on top of the other, or turned over where
they lie, and the table is never clear between them. The page is compared with
the one photographed in 8×8 blocks, allowing a shift of up to eight pixels, so
a page nudged or turned a few degrees is the same page: on the rig a nudge
changed at most 17% of the blocks, the blank back of a page 31%, another letter
34%, and 22% is the line. That also
re-learns the baseline, so daylight moving across a desk over an afternoon does
not slowly read as a page — and when the light changes while nothing is lying
there at all, the table is learnt again after a few seconds of it, so the rig
settles into the new light instead of watching an empty table it no longer
recognises.

## Letters of more than one page

With `--button`, pages are not sent as they are photographed. They collect in
the spool's `open/` directory until the button is pressed, and the letter goes
to Paperless as one PDF — the photographs embedded as they are, a page each, so
nothing is re-encoded before the OCR. A press while the last page is still
settling waits for that page. A letter nobody closes is closed
`--letter-idle-secs` (five minutes) after its last page, so a forgotten press
delays post but never keeps it. Without `--button`, every page is its own
document.

The button is a push button between GPIO 27 and ground (pins 13 and 14), made
into a key by the kernel with one line in `/boot/firmware/config.txt`:

    dtoverlay=gpio-key,gpio=27,active_low=1,gpio_pull=up,keycode=28,label=scannerd

It used to be GPIO 17, which the e-paper display below needs for its reset
line. With the display's board on the header, the button's wires are soldered
to the underside of the Pi.

After a reboot it is a keyboard with one key: `ls /dev/input/by-path/` shows
the device, and `SCANNERD_BUTTON` points at it. The kernel debounces it and
`scannerd` reads key events from the device, so there is no GPIO library, and a
USB keypad works as well (`--button-key` for a key other than Enter). On a
laptop, `--button stdin` makes Enter in the terminal the button.

## The display

`--display epaper` (or `SCANNERD_DISPLAY=epaper`) drives a Waveshare 2.13″ e-paper HAT
(V3 or V4, 250×122, an SSD1680 controller) on the Pi's header, so the rig says
what it is doing without a phone:

- **Ready** — put a page down; **Hold still** while it settles; **Scanned** —
  take the page away; **No camera**; **Sending letter** after the button.
- Under that, whether the photograph can be read: a tick and *Page 2: good to
  read*, or white on black *Page 2: too bright*. It stays up for ten minutes, so
  it is still there when the next page goes down.
- Along the bottom, the pages in the open letter and what is waiting to send.

The check runs on every photograph, display or not, and a page that fails it
is also in the setup page's activity list. It looks at the middle of the
photograph, where the page lies: *no page / too dark* (the middle is not bright
enough to be paper), *too bright* (the paper clipped to white and the text
pale — every page of the first letters from the rig looked like this),
*blurry*, and *no text found* (a blank page, or a pale ceiling). A page that
fails is kept and sent all the same; delete it on the setup page and put it
down again if it matters. On a Pi Zero W the check costs about half a second a
page.

Between letters, once Paperless has read the last one, it says which folder
on the shelf that letter goes in — see below.

The panel needs SPI, which is off by default, and the `spi` and `gpio` groups,
which the unit file asks for:

    dtparam=spi=on          # in /boot/firmware/config.txt, then reboot
    sudo usermod -aG spi,gpio scannerd

It is refreshed only when what it says changes, mostly with a partial refresh
(about a third of a second, no flashing) and every twentieth time with a full
one, which flashes for two seconds and clears the ghosts partial refreshes
leave. `SCANNERD_DISPLAY_FLIP=true` turns the picture round for a HAT mounted
upside down. A display that does not answer is logged and done without.

### The 3.5″ LCD

`SCANNERD_DISPLAY=/dev/fb0` draws the same words in colour on a framebuffer
instead: on the rig, a 3.5″ SPI panel (ILI9486, 480×320, with an XPT2046
resistive touch controller — the Waveshare 3.5″ (A) layout), which the kernel
drives with one line in `/boot/firmware/config.txt`:

    dtoverlay=piscreen,speed=24000000,rotate=90

The screen is painted rather than drawn in blocks ([`paint`](src/paint.rs):
`tiny-skia` for shapes, Inter — SIL Open Font License, in `assets/fonts` —
rasterised by `fontdue`), because a panel of hard edges and a bitmap font looks
like a cash register. A card holds the headline with an icon in the colour of
the news: green ready, amber working, teal done, red look, blue a folder, grey
the bin. The buttons are pills, the one thing to do in colour. The picture is
dithered on its way into the panel's 16-bit colour, so gradients do not band. Add
`fbcon=map:9` to `/boot/firmware/cmdline.txt` so the login console does not
draw over it.

With `SCANNERD_TOUCH=/dev/spidev0.1` the screen is also how scanning starts
and stops. Between letters it shows the last letter's folder over two buttons,
**Start scanning** and **Learn empty table**; the camera is not watched at all
until Start, so nothing is photographed by accident. Scanning, it shows what
the camera sees beside the page it photographed last, with that page's verdict,
over **Finish letter** and **Stop scanning**. A finished letter ends the scan.
The setup page has the same Start and Stop.

The rig's touch film reports only how far down a finger is — its other axis
reads the same everywhere — and its pen interrupt is not wired, so the kernel's
driver never sees a touch. scannerd asks the controller itself thirty times a
second, and every button is a band the width of the screen. The kernel's driver
has to be kept off the controller:

    # /etc/modprobe.d/scannerd-touch.conf
    blacklist ads7846
    # /etc/udev/rules.d/90-scannerd-touch.rules
    ACTION=="add", SUBSYSTEM=="spi", KERNEL=="spi0.1", ATTR{driver_override}="spidev", RUN+="/sbin/modprobe spidev", RUN+="/bin/sh -c 'echo spi0.1 > /sys/bus/spi/drivers/spidev/bind || true'"

`SCANNERD_TOUCH_ROWS` (`x:204:4000`, channel, reading at the top, reading at
the bottom) is for a panel that differs — or one mounted the other way up: with
`rotate=270` in the overlay line instead of `rotate=90`, the rig's is
`x:4000:204`.

### Envelopes

An envelope put down is photographed like a page, and told from one by its
size: paper covering less than about seven tenths of what a page covers (DL is
about 0.4 of an A4 page, C5 about 0.6) is an envelope. The letter being
collected is finished and sent at once, and the envelope is the first page of
the next — which is not finished for the table being empty while it is opened.
The folders themselves are **Paperless's**, read from it every few minutes.
They are set up in the desktop app's assistant, which writes each one as a
Paperless tag with its words; the scanner reads them back and shows them. It
keeps no list of its own, because a second list could only ever disagree with
the one doing the filing — and did, until the two `Lawstuff` lists had three
words in common. The one thing the scanner still decides is which folder is
the bin, since a Paperless tag cannot say that: `--bin-folder`, the `-Name` in
the env file's list, or the one ticked on the setup page.

A letter goes in **one** folder, because it is a piece of paper. Where its
text fits more than one, the folder with the most of its words on the page
wins — a letter carrying five of Taxes' words and one of Rechnungen's is a tax
letter — and the others are said under it rather than beside it.

That decision is made **once, on the rig, while the paper is in your hand**,
and does not change afterwards. It is written beside the letter in the spool
as it is queued, so it survives a restart, and put on the document as it is
uploaded. Paperless does not match folder tags itself — they are saved with
its matching switched off — because a second opinion arriving a minute later,
from better OCR but after the paper is already in a drawer, could only make
the record and the drawer disagree. The words stay on the tags: they are what
the rig reads a page against, and where they are edited.

That is what makes kuverta useful for finding the paper again. An open letter
says which folder it is in, and that folder is the drawer you actually put it
in.

The words are matched against what tesseract made of a photograph, which is
bad text: a letter from the Wasserschutzpolizei came out with
"Strafprozessordnung" spelled three different wrong ways, and was filed by
"Staatsanwaltschaft" surviving on its second page. So the word lists the
setup assistant offers are longer than they look as if they need to be —
every extra spelling of the same idea is another chance that one of them
lands. Each list has to fit the 256 characters Paperless keeps of a tag's
match, which `kuverta-bird/test/desktop-shelf.test.js` checks, and
`cargo run --example match_check` runs a list against a real page's text.

What a page covers is learnt from the pages, so corners set generously round
where letters land still tell the two apart.

### Taking pages and letters back

While a letter is being scanned the display offers **Undo last page** — a
hand, the table, a page twice — and **Cancel letter**, which asks for a second
tap. For ten minutes after a letter is finished, **Undo last letter** takes it
back: out of the queue if it has not gone, or deleted from Paperless (into its
trash) if it has — as soon as Paperless has read it, if it has not yet. The
setup page has the same three. Nothing taken back is deleted on the Pi: it goes
to the spool's `discarded/`, and is deleted from there after a week.

### What is waiting to be sent

When letters are waiting — Paperless off, the network down — the display
offers **n waiting to send** between letters. That opens a list, a letter a row
with when it was photographed and how often sending has been tried; tapping one
asks for a second tap and then throws it away, into `discarded/`. **Back**
closes the list. The setup page lists the same with a **Remove** on each.

### Scanning from the start

The camera is watched from the moment scannerd starts: a page put down after a
boot is photographed without a button. **Stop scanning** on the display or the
page stops it; finishing a letter does not. While scanning, the display keeps
the last letter's folder in a strip under the top line.

The frames it watches come from a video stream (`rpicam-vid`, five a second,
`SCANNERD_PREVIEW_FPS`) rather than a still each: on the Pi 4 a still took half
a second to start the camera for every frame, and a page lay still for three or
four seconds before it was photographed; with the stream, one. The stream stops
for the photograph and starts again after it.

### Finishing a letter by clearing the table

A letter is also finished when its last page has been taken away and the table
stays empty for eight seconds (`SCANNERD_FINISH_WHEN_CLEAR_SECS`, 0 turns it
off) — the next page is laid down sooner than that, or on top of the last one,
or the last one turned over. The display counts the seconds down.

## Exposure

White paper fools a camera: it exposes for a grey world, and the rig's OV5647
made the paper pure white and the text pale grey — ink at 197 of 255 — until it
was told two stops less, when the paper came out at 210 and the ink at 84. So
the check above does not only report. A photograph that is too bright is taken
again a stop darker while the page is still lying there, up to twice, and the
best of them is kept; one too dark but with text in it, a stop brighter. The
exposure that worked is where the next page starts, nudged half a stop when the
paper comes out near clipping or dim, and is kept in `settings.toml` across
restarts. `SCANNERD_EV` gives the first guess (the rig's is `-2`); the setup
page shows the exposure in force and starts it again from the camera's own.

## Which folder

The paper still has to go somewhere. With folders set up — `SCANNERD_FOLDERS=Car,House,Work,Taxes,-Throw away`
in the env file, or **Folders on the shelf** on the setup page, which wins —
the display says, for each letter, which one it goes in, or **Throw away** for
the folder marked as the bin (a `-` in the env file).

### Filing it yourself

Paperless reads the words; it cannot know that a letter is worth keeping but
belongs on no shelf, or that the paper can go in the bin. While the display
shows where the last letter goes — and on the setup page — two buttons say so
instead:

- **No folder** takes the folders' tags off it. The letter stays in Paperless;
  the display says *No folder*.
- **Throw away** puts the bin's tag on it instead. The letter stays in
  Paperless too; only the paper goes.

Either works the moment the letter is sent, before Paperless has read it: the
answer is remembered and applied as soon as it has. Tags that are not folders —
the address, who the post is for — are left alone, and "Undo last letter" still
deletes the whole thing for ten minutes.

## Who the letter is for

More than one person in a household gets post — a partner, a child, someone
whose papers are being kept. `SCANNERD_PEOPLE=Erika Mustermann,Max Mustermann`
(or **Person** on a row of the setup page) makes each of them a Paperless tag
of their own, `Person: Erika Mustermann`, matched on their name — which is what
stands on the letter. The display then says both: **Taxes**, and under it *For
Erika Mustermann*, or *For Erika Mustermann and Max Mustermann* when a letter
carries both names. Somebody written to in more than one way can be given the
other spellings as words, like a folder.

The mark on the tag is what keeps the two apart, so a folder and a person of
the same name are two tags and neither is the other, and a shelf read back out
of Paperless still knows which is which. Nobody can be the bin.

Told neither way, scannerd takes the shelf from Paperless: every tag that
matches letters by itself is a folder, with the words it matches on. That is
what the kuverta app's setup assistant writes, so a shelf named there needs no
second naming here; it is asked for again every five minutes until Paperless
answers. Which of them is the bin is the one thing a tag cannot say —
`SCANNERD_BIN_FOLDER=Werbung` names it.

The Pi cannot read a letter; OCR on a Zero W takes minutes a page. Paperless
reads every one, and deciding what a document is is what its tags already do.
So each folder is a Paperless tag of the same name, which scannerd creates, or
updates, before the next letter is sent:

- A folder with **words** — a number plate, an insurer, "Finanzamt" — matches
  any letter with one of them in it (Paperless's "any word"). That works from
  the first letter.
- A folder without words uses Paperless's **auto** matching, which learns from
  the documents tagged with it. Until some are, it matches nothing, and the
  display asks *Which folder?* — tag the letter in Paperless, and the next one
  like it is known.

scannerd owns these tags' matching: the words set here replace whatever the
tag had. After an upload it asks Paperless every five seconds whether the
letter is done, then reads its tags: **Car** — *Put the letter in this folder*;
*Car / House* when it fits more than one; *Already filed* for a letter
Paperless had before. Paperless 2 and 3 are both understood; 3 reports its tasks differently. That stays up until the next letter is started, when it
moves to the bottom line; after fifteen minutes without an answer it is given
up on. The setup page's activity list has the same, a line a letter.

## Preview mode

The setup page has a second layout, **Preview** (the button at the top right,
or <kbd>p</kbd>), for working through a pile of post with a keyboard instead of
a phone:

- the page just photographed, large, with a tab for each page of the letter;
- **what it says** — scannerd reads every page itself with `tesseract`, badly
  and in a few seconds, so there is something to look at long before Paperless
  has the letter. What Paperless makes of it is still what the letter is filed
  under;
- **where it goes** — every folder on the shelf as a chip, the ones the text
  looks like marked, and who the letter is for. Press <kbd>1</kbd>…<kbd>9</kbd>
  or tap a chip to file the letter there; <kbd>0</kbd> is no folder and
  <kbd>b</kbd> the bin;
- **a new folder**, named from here: it is added to the shelf with the longest
  words of this letter as its words — the telling ones, in German post — and
  the letter goes in it. The words can be edited under Setup afterwards;
- every key that does anything, written along the bottom: finish, undo the
  page, undo the letter, cancel, start and stop, learn the table.

Reading is `SCANNERD_READER=tesseract` (`apt install tesseract-ocr
tesseract-ocr-deu`) and `SCANNERD_READER_LANGUAGES=deu+eng`. Without it
everything works as before, with no text in the preview and the folder known
once Paperless has read the letter.

## The setup page

`--ui 0.0.0.0:8080` (or `SCANNERD_UI`) serves a page from scannerd itself, so a
headless Pi needs no screen, no desktop and no second program: open
`http://<pi>.local:8080` on a phone. It shows:

- what the camera sees, refreshed every second or two, and what scannerd makes
  of it — *Ready — put a page down*, *Hold still…*, *Photographed — take the
  page away*;
- the pages of the letter so far, with **Finish letter** — the same as the
  button, so a hardware button is optional. Tap a page to see it large and
  delete it if it was photographed by mistake (a hand, the empty table, a page
  taken twice); a letter already sent is Paperless's to change;
- what was sent and what is waiting, with **Retry uploads now**;
- **Where it is going, from the first page.** The Pi reads each page itself
  and matches the shelf's words, so the panel shows the folder as an outline
  pill in the corner while you are still holding the letter — minutes before
  Paperless has it. An outline because it is a guess; what Paperless decides
  replaces it. The pill gives the corner back to the last letter's folder
  once this one is sent.
- **Learn empty table**, and setup. For the crop: put a page where letters will
  lie and press **Take a picture** — scannerd photographs the whole view in
  colour, finds the page (the largest bright area) and puts a box around it;
  move the box, drag its corners, or draw a new one, then **Save this area**.
  The crop may be any shape: the photograph is taken at the crop's own share
  of the sensor (`--sensor-width`/`--sensor-height`, a Pi camera v1 by
  default), so a page-shaped crop is not stretched to 4:3. Also the Paperless
  address, token and tags, and **Check connection**.
- **The camera looks at the table at an angle**, for a camera that cannot be
  mounted straight above the table, where a page shows as a trapezium. Instead
  of a box there are four corners to drag onto the corners of a page lying
  where letters go — **Take a picture** finds them, a hair outside the page's
  edges — and dashed lines show where the page's edges and middle will fall.
  A letter never lies exactly there, so after straightening each photograph
  is straightened once more onto the page it actually shows, cutting away the
  strips and wedges of table beside it; when no clear page covering at least
  half the picture is found, the photograph is kept as it is, and a side with
  paper still beyond it is never cut — a sheet's far edge falls off under a
  camera on a stalk and is found short, which used to take the right-hand
  column off letters. Every photograph is then warped so those corners become
  a rectangle's, before it joins the letter. The crop is the area the corners
  span **with 8% of it left round them**, so a letter put down further along,
  or a sheet bigger than the one set up with, is still photographed: what the
  sensor leaves out is gone for good, and the part of a page that overhangs
  is usually the letterhead, which is where the sender is read from. The room
  costs no sharpness — the photograph is taken at the crop's own share of the
  sensor, so a wider crop is a bigger picture rather than a coarser one. On a Pi Zero W this
  adds about five seconds a page (a 5-megapixel photograph: decoding, warping
  and encoding take a third each), and a page that cannot be straightened is
  kept as it was taken. `SCANNERD_CORNERS` sets the same from the env file.
- **Which way up**: **Turn left** and **Turn right** turn every photograph a
  quarter turn, for a camera mounted so that letters do not read upright. The
  live view turns with it; the picture for setting the area stays as the
  camera sees it. Turning goes through the same warp as straightening, so it
  costs the same five seconds a page on a Pi Zero W, with or without corners.
  `SCANNERD_ROTATE` (0, 90, 180 or 270, clockwise) sets it from the env file.

Settings saved on the page go to `settings.toml` in the spool directory
(owner-only, it holds the token) and win over the env file. The learnt empty
table is kept there too (`empty-table.gray`), so a restart with a letter lying
on the table sees the letter instead of taking it for the table.

The page shows photographs of post, so anything but a loopback address needs
`SCANNERD_UI_PASSWORD` (any user name). Its buttons also require a header that
another site open in the same browser cannot send. The page never touches the
camera or the queue itself: it reads the status the capture loop publishes and
leaves commands for it, so a button pressed mid-photograph waits a moment
instead of starting a second `rpicam-still`.

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

Cross-compile from the workspace root, with
[cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) and zig as the C
compiler and linker:

```sh
rustup target add arm-unknown-linux-gnueabihf     # Pi Zero W: ARMv6, 32-bit
cargo install cargo-zigbuild                      # and zig from ziglang.org on PATH
make scannerd-pi                                  # target/arm-unknown-linux-gnueabihf/release/scannerd
make scannerd-to-pi PI_HOST=pi@raspberrypi.local  # copies it to ~/scannerd, installs nothing
```

A Pi Zero W is ARMv6 and runs neither 64-bit Raspberry Pi OS nor an aarch64
binary; a Zero 2 W, 3, 4 or 5 on the 64-bit OS wants
`PI_TARGET=aarch64-unknown-linux-gnu.2.36` (after `rustup target add
aarch64-unknown-linux-gnu`). The `.2.36` pins glibc to bookworm's. On a Zero W
(checked on a Rev 1.1 with the OV5647 camera), a 320×240 preview frame takes 1.1–1.7
seconds because `rpicam-still` starts the camera every time, and a full capture
about 3.7 — so a page is photographed eight or nine seconds after it is put
down.

Then `scannerd.service` and `scannerd.env.example` in this directory. It runs as
its own user with the `video` group and nothing else: a daemon that can reach
the camera and holds a Paperless token should not also be able to read the `pi`
user's home directory. That user has to exist first:

```sh
sudo useradd --system --no-create-home --shell /usr/sbin/nologin --groups video,input scannerd
sudo install -m 0755 scannerd /usr/local/bin/
sudo install -m 0644 scannerd.service /etc/systemd/system/
sudo install -m 0600 scannerd.env.example /etc/scannerd.env   # then fill it in
sudo systemctl enable --now scannerd
journalctl -u scannerd -f
```

## Without a Pi

`tools/rpicam-still-ffmpeg` answers the two `rpicam-still` calls `scannerd`
makes using ffmpeg, so the whole daemon — detection, capture, spool, upload —
runs on a laptop against the dev stack:

```sh
# a still image as the camera; swap the symlink's target to put a page down
ln -sf desk.jpg /tmp/camera.jpg
SCANNERD_FAKE_INPUT=/tmp/camera.jpg PAPERLESS_TOKEN=... \
  scannerd --camera apps/scannerd/tools/rpicam-still-ffmpeg --spool /tmp/spool --tag "Hauptstraße 12"
ln -sf letter.jpg /tmp/camera.jpg

# or the Mac's own camera (the terminal will ask for access)
SCANNERD_FAKE_INPUT=0 PAPERLESS_TOKEN=... \
  scannerd --camera apps/scannerd/tools/rpicam-still-ffmpeg --spool /tmp/spool
```

Add `--button stdin` to collect pages into letters, and press Enter to finish
one.

It stands in for the program, not the sensor: how long `rpicam-still` takes to
start on a Pi, and how its exposure copes with a desk lamp, are still for the
device to answer.

## Tests

```sh
cargo test -p scannerd
```

Over a hundred of them, none needing a camera or a display: the detector against synthetic
frames, the spool against a real directory, the uploader against a server on
loopback, the loop itself (`tests/run.rs`) turned by hand with a scripted
camera and a Paperless that fails when told to, letters and the button
(`tests/letters.rs`, `tests/button.rs`) the same way, and the PDF writer
(`tests/pdf.rs`) read back through its own cross-reference table, the
photograph check (`tests/quality.rs`) against synthetic pages and blurred copies
of them, and the display (`tests/display.rs`) by what it draws and how often it
touches the panel. What they cannot check is whether `rpicam-still` behaves as
documented — that is what `--drain-only` and a first run on the device are for.

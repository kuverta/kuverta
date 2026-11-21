# scannerd

Watches a surface through a camera. When a page is put down and stops moving,
photographs it and hands it to Paperless-ngx.

Paperless already does OCR, deskew, rotation, tagging, archiving and search.
scannerd only notices that a letter has arrived. The brief budgets ~600 lines
for the capture daemon and that is about what it costs.

## What it does not do

No deskew. That is `ocrmypdf`'s job. Paperless does it on every document, and
the dev stack sets `PAPERLESS_OCR_DESKEW` explicitly so the dependency is
visible.

No page-finding. The camera is fixed above one spot, so the page is always in
the same part of the frame. `--roi x,y,w,h` crops on the sensor, for the
watched frames as well as the photograph, so "half the frame changed" means
half the page area. Cropping is one line of configuration and needs no geometry
code.

No image library. `rpicam-still --encoding yuv420` returns raw YUV, and the
first plane is the greyscale image. Detection is arithmetic over a byte slice,
so it is testable on a laptop with no camera.

## How it decides

Frames are compared against a baseline of the empty surface:

- Is something there? Paper is large and pale and changes many pixels. A
  shadow changes them slightly.
- Does it look like paper? Evening light moves over the whole table, and the
  rig photographed a few sunsets as letters. So the frame is also read in
  16×16 blocks against the table's own level. Paper is at least thirty-five
  counts paler than the table, and it is one lump: the paper-like blocks must
  fill nearly half the box they span. A shadow darkens and is spread over
  everything. Both numbers are on the setup page beside the change.
- Has it stopped? A hand stays in shot briefly after the page lands.

The paper test wobbles from frame to frame, so the last eight frames count. It
takes no offset out of the block comparison, because an offset let a shadow
over half the table make the other half look like paper. A camera still
choosing its exposure therefore moves every block at once. On the rig a page
lying still read a share of between 0.0 and 0.21 of the frame from one frame to
the next, and with a per-frame test the wait for it to settle was abandoned
four times in four seconds, so it was never photographed. Now one frame that
sees a page starts the wait, and giving up takes a second and a half with no
paper in any frame.

The same window stops the table being learnt with a page on it. A page inside
the baseline can never be seen again. The guard used to test only the current
frame, and on the rig the table was learnt again on a frame that read a share
of 0.013 while a page lay on it. Now the table is learnt only if no frame in
the window saw paper. If it happens anyway, take the page off: the empty table
is learnt again as soon as it is seen.

A rig that sees something and never photographs it logs that every ten seconds,
with the numbers: *something that could be paper is lying there, but
never enough of it to be a page*.

After a capture it will not arm again until the surface has been seen empty,
or until a hand has been over the page and what lies there, once still, is
another page. Pages are often stacked or turned over in place, so the table is
never clear between them. The page is compared with the one photographed in
8×8 blocks, allowing a shift of up to eight pixels, so a page nudged or turned
a few degrees is the same page. On the rig a nudge changed at most 17% of the
blocks, the blank back of a page 31%, another letter 34%, and 22% is the line.
This also re-learns the baseline, so daylight moving across the desk does not
read as a page. When the light changes on an empty table, the table is learnt
again after a few seconds.

An envelope is told from a page by being smaller (DL about 0.4 of an A4 page,
C5 about 0.6). It means a new letter, so the one being collected is finished
and sent. The decision is made twice with different bars, because the errors
cost differently. An envelope taken for a page just becomes page one of its
letter. A page taken for an envelope splits a letter: on the rig a six-page
SCHUFA letter was filed as two three-page letters. The measurement is where the
straightener found the paper's edge, which moves with the exposure (that
photograph had just been retaken darker). So only paper that is unmistakably
smaller ends a letter. A DL envelope still finishes the letter before it. A C5
is still called an envelope, but becomes page one of the letter it came with
and does not end the one before. Both numbers are logged for every photograph.

## Letters of more than one page

With `--button`, pages collect in the spool's `open/` directory until the
button is pressed, and the letter goes to Paperless as one PDF. The photographs
are embedded unchanged, a page each, so nothing is re-encoded before the OCR. A
press while the last page is still settling waits for that page. An unclosed
letter is closed `--letter-idle-secs` (five minutes) after its last page.
Without `--button`, every page is its own document.

The button is a push button between GPIO 27 and ground (pins 13 and 14), made
into a key by the kernel with one line in `/boot/firmware/config.txt`:

    dtoverlay=gpio-key,gpio=27,active_low=1,gpio_pull=up,keycode=28,label=scannerd

It used to be GPIO 17, which the e-paper display below needs for its reset
line. With the display's board on the header, the button's wires are soldered
to the underside of the Pi.

After a reboot it is a keyboard with one key: `ls /dev/input/by-path/` shows
the device, and `SCANNERD_BUTTON` points at it. The kernel debounces it and
`scannerd` reads key events, so there is no GPIO library, and a USB keypad
works too (`--button-key` for a key other than Enter). On a laptop,
`--button stdin` makes Enter in the terminal the button.

## The display

`--display epaper` (or `SCANNERD_DISPLAY=epaper`) drives a Waveshare 2.13″ e-paper HAT
(V3 or V4, 250×122, an SSD1680 controller) on the Pi's header. It shows:

- The state: **Ready** (put a page down), **Hold still** while it settles,
  **Scanned** (take the page away), **No camera**, and **Sending letter** after
  the button.
- Whether the photograph can be read: a tick and *Page 2: good to
  read*, or white on black *Page 2: too bright*. It stays up for ten minutes.
- Along the bottom, the pages in the open letter and what is waiting to send.

The check runs on every photograph, display or not, and a failed page is also
in the setup page's activity list. It looks at the middle of the photograph:
*no page / too dark* (not bright enough to be paper), *too bright* (paper
clipped to white and the text pale, as on every page of the rig's first
letters), *blurry*, and *no text found* (a blank page, or a pale ceiling). A
failed page is kept and sent anyway. Delete it on the setup page and put it
down again if it matters. On a Pi Zero W the check costs about half a second a
page.

Between letters, once Paperless has read the last one, it says which folder
on the shelf that letter goes in (see below).

The panel needs SPI, which is off by default, and the `spi` and `gpio` groups,
which the unit file asks for:

    dtparam=spi=on          # in /boot/firmware/config.txt, then reboot
    sudo usermod -aG spi,gpio scannerd

It is refreshed only when its content changes, mostly with a partial refresh
(about a third of a second, no flashing). Every twentieth refresh is a full
one, which flashes for two seconds and clears ghosting.
`SCANNERD_DISPLAY_FLIP=true` turns the picture for a HAT mounted upside down. A
display that does not answer is logged and skipped.

### The 3.5″ LCD

`SCANNERD_DISPLAY=/dev/fb0` draws the same words in colour on a framebuffer.
On the rig that is a 3.5″ SPI panel (ILI9486, 480×320, with an XPT2046
resistive touch controller, the Waveshare 3.5″ (A) layout), which the kernel
drives with one line in `/boot/firmware/config.txt`:

    dtoverlay=piscreen,speed=24000000,rotate=90

The screen is painted by [`paint`](src/paint.rs): `tiny-skia` for shapes, and
Inter (SIL Open Font License, in `assets/fonts`) rasterised by `fontdue`. A
card holds the headline with an icon coloured by state: green ready, amber
working, teal done, red look, blue a folder, grey the bin. Buttons are pills,
with the main action in colour. The picture is dithered into the panel's 16-bit
colour, so gradients do not band. Add `fbcon=map:9` to
`/boot/firmware/cmdline.txt` so the login console does not draw over it.

With `SCANNERD_TOUCH=/dev/spidev0.1` the screen also starts and stops scanning.
Between letters it shows the last letter's folder over **Start scanning** and
**Learn empty table**. The camera is not watched until Start. While scanning,
it shows the camera view beside the last page photographed, with that page's
verdict, over **Finish letter** and **Stop scanning**. A finished letter ends
the scan. The setup page has the same Start and Stop.

The rig's touch film reports only how far down a finger is (its other axis
reads the same everywhere), and its pen interrupt is not wired, so the kernel's
driver never sees a touch. scannerd polls the controller thirty times a second,
and every button is a band the width of the screen. The kernel's driver has to
be kept off the controller:

    # /etc/modprobe.d/scannerd-touch.conf
    blacklist ads7846
    # /etc/udev/rules.d/90-scannerd-touch.rules
    ACTION=="add", SUBSYSTEM=="spi", KERNEL=="spi0.1", ATTR{driver_override}="spidev", RUN+="/sbin/modprobe spidev", RUN+="/bin/sh -c 'echo spi0.1 > /sys/bus/spi/drivers/spidev/bind || true'"

`SCANNERD_TOUCH_ROWS` (`x:204:4000`, channel, reading at the top, reading at
the bottom) is for a different panel, or one mounted the other way up. With
`rotate=270` in the overlay line instead of `rotate=90`, the rig's is
`x:4000:204`.

### Envelopes

An envelope is photographed like a page and told from one by size: paper
covering less than about seven tenths of what a page covers (DL is about 0.4 of
an A4 page, C5 about 0.6). The letter being collected is finished and sent at
once, and the envelope is the first page of the next. That letter is not
finished by the empty table while the envelope is opened.

The folders are Paperless's, read from it every few minutes. They are set up in
the desktop app's assistant, which writes each as a Paperless tag with its
words. The scanner keeps no list of its own, because a second list drifts: the
two `Lawstuff` lists ended up with three words in common. The scanner only
decides which folder is the bin, since a tag cannot say that: `--bin-folder`,
the `-Name` in the env file's list, or the one ticked on the setup page.

A letter goes in one folder. Where its text fits more than one, the folder with
the most of its words on the page wins (five of Taxes' words and one of
Rechnungen's is a tax letter), and the others are shown under it.

That decision is made once, on the rig, while the paper is in your hand. It is
written beside the letter in the spool as it is queued, so it survives a
restart, and put on the document at upload. Folder tags are saved with
Paperless's matching switched off, because a later match from better OCR, after
the paper is in a drawer, would make the record and the drawer disagree. The
words stay on the tags: the rig reads pages against them, and they are edited
there.

So the folder kuverta shows for a letter is the drawer the paper is in.

The words are matched against tesseract's reading of a photograph, which is
poor: a letter from the Wasserschutzpolizei came out with "Strafprozessordnung"
spelled three different wrong ways, and was filed by "Staatsanwaltschaft"
surviving on its second page. So the setup assistant's word lists are long, to
give more spellings a chance. Each list has to fit the 256 characters Paperless
keeps of a tag's match, which `kuverta-bird/test/desktop-shelf.test.js` checks,
and `cargo run --example match_check` runs a list against a real page's text.

What a page covers is learnt from the pages, so generously set corners still
tell the two apart.

### Taking pages and letters back

While a letter is being scanned the display offers **Undo last page** (a hand,
the table, a page twice) and **Cancel letter**, which asks for a second tap.
For ten minutes after a letter is finished, **Undo last letter** takes it back:
out of the queue if it has not gone, or deleted from Paperless (into its trash)
if it has, as soon as Paperless has read it. The setup page has the same three.
Nothing taken back is deleted on the Pi: it goes to the spool's `discarded/`,
and is deleted from there after a week.

### What is waiting to be sent

When letters are waiting (Paperless off, the network down), the display offers
**n waiting to send** between letters. That opens a list, a letter a row, with
when it was photographed and how often sending was tried. Tapping one asks for
a second tap and then moves it to `discarded/`. **Back** closes the list. The
setup page lists the same with a **Remove** on each.

### Scanning from the start

The camera is watched from the moment scannerd starts, so a page put down after
a boot is photographed without a button. **Stop scanning** on the display or
the page stops it. Finishing a letter does not. While scanning, the display
keeps the last letter's folder in a strip under the top line.

The watched frames come from a video stream (`rpicam-vid`, five a second,
`SCANNERD_PREVIEW_FPS`). With a still per frame, the Pi 4 took half a second to
start the camera each time, and a page lay still for three or four seconds
before it was photographed. With the stream it is one. The stream stops for the
photograph and starts again after it.

### Finishing a letter by clearing the table

A letter is also finished when its last page has been taken away and the table
stays empty for eight seconds (`SCANNERD_FINISH_WHEN_CLEAR_SECS`, 0 turns it
off). The next page normally arrives sooner, or goes on top of the last one, or
the last one is turned over. The display counts the seconds down.

## Exposure

A camera exposes white paper as grey. The rig's OV5647 made the paper pure
white and the text pale grey (ink at 197 of 255). Told two stops less, the
paper came out at 210 and the ink at 84. So a photograph that is too bright is
retaken a stop darker while the page is still there, up to twice, and the best
is kept. One too dark but with text in it is retaken a stop brighter. The
exposure that worked is where the next page starts, nudged half a stop when the
paper comes out near clipping or dim, and is kept in `settings.toml` across
restarts. `SCANNERD_EV` gives the first guess (the rig's is `-2`). The setup
page shows the exposure in force and can restart it from the camera's own.

## Which folder

With folders set up, the display says for each letter which one it goes in, or
**Throw away** for the folder marked as the bin (a `-` in the env file). Set
them with `SCANNERD_FOLDERS=Car,House,Work,Taxes,-Throw away` in the env file,
or with **Folders on the shelf** on the setup page, which wins.

### Filing it yourself

Paperless cannot know that a letter is worth keeping but belongs on no shelf,
or that the paper can go in the bin. While the display shows where the last
letter goes, and on the setup page, there are two buttons for that:

- **No folder** takes the folders' tags off it. The letter stays in Paperless;
  the display says *No folder*.
- **Throw away** puts the bin's tag on it instead. The letter stays in
  Paperless too; only the paper goes.

Either works as soon as the letter is sent: the answer is remembered and
applied once Paperless has read it. Tags that are not folders (the address, who
the post is for) are left alone, and "Undo last letter" still deletes the whole
thing for ten minutes.

## Who the letter is for

`SCANNERD_PEOPLE=Erika Mustermann,Max Mustermann` (or **Person** on a row of
the setup page) gives each person in the household a Paperless tag,
`Person: Erika Mustermann`, matched on their name as written on the letter.
The display then says both: **Taxes**, and under it *For
Erika Mustermann*, or *For Erika Mustermann and Max Mustermann* when a letter
carries both names. Other spellings of a name can be added as words, like a
folder.

The mark on the tag keeps a folder and a person of the same name as two tags,
and a shelf read back from Paperless still knows which is which. Nobody can be
the bin.

With neither set, scannerd takes the shelf from Paperless: every tag that
matches letters by itself is a folder, with the words it matches on. The
kuverta app's setup assistant writes those, so a shelf named there needs no
naming here. It is asked for again every five minutes until Paperless answers.
A tag cannot say which folder is the bin, so `SCANNERD_BIN_FOLDER=Werbung`
names it.

OCR on a Zero W takes minutes a page, so Paperless reads every letter and its
tags classify it. Each folder is a Paperless tag of the same name, which
scannerd creates or updates before the next letter is sent:

- A folder with words (a number plate, an insurer, "Finanzamt") matches any
  letter containing one of them (Paperless's "any word"). That works from the
  first letter.
- A folder without words uses Paperless's auto matching, which learns from the
  documents tagged with it. Until some are, it matches nothing, and the display
  asks *Which folder?*. Tag the letter in Paperless, and the next one like it
  is known.

scannerd owns these tags' matching: the words set here replace whatever the
tag had. After an upload it asks Paperless every five seconds whether the
letter is done, then reads its tags: **Car** with *Put the letter in this folder*,
*Car / House* when it fits more than one, *Already filed* for a letter
Paperless had before. Paperless 2 and 3 are both understood; 3 reports its
tasks differently. That stays up until the next letter is started, then moves
to the bottom line. After fifteen minutes without an answer it gives up. The
setup page's activity list has the same, a line a letter.

## Preview mode

The setup page has a second layout, **Preview** (the button at the top right,
or <kbd>p</kbd>), for working through a pile of post with a keyboard:

- The page just photographed, large, with a tab for each page of the letter.
- What it says. scannerd reads every page with `tesseract`, badly and in a few
  seconds, so there is text long before Paperless has the letter. Paperless's
  reading still decides the filing.
- Where it goes. Every folder on the shelf is a chip, with the ones the text
  looks like marked, and who the letter is for. Press
  <kbd>1</kbd>…<kbd>9</kbd> or tap a chip to file the letter there;
  <kbd>0</kbd> is no folder and <kbd>b</kbd> the bin.
- A new folder, named from here. It is added to the shelf with the longest
  words of this letter as its words (the telling ones, in German post), and the
  letter goes in it. The words can be edited under Setup afterwards.
- Every key that does anything, along the bottom: finish, undo the page, undo
  the letter, cancel, start and stop, learn the table.

Reading is `SCANNERD_READER=tesseract` (`apt install tesseract-ocr
tesseract-ocr-deu`) and `SCANNERD_READER_LANGUAGES=deu+eng`. Without it there
is no text in the preview and the folder is known once Paperless has read the
letter.

The reading also decides whether a photograph was a page at all. A page in
which tesseract finds no word (three letters or digits in a row) is the table.
It is moved out of the letter into `discarded/` and the table is learnt again,
as the quality check does with a photograph that has no ink in it. The check
alone was not enough: the rig's table is creased paper with the corners drawn
on in marker, which has as much contrast as ink, and when the light changed the
bare table went to Paperless as a blank letter. A blank back of a sheet is
discarded the same way. A read that failed keeps the page, and so does a letter
of single pages, which is sent before it is read.

## The setup page

`--ui 0.0.0.0:8080` (or `SCANNERD_UI`) serves a page from scannerd itself, so a
headless Pi needs no screen, desktop or second program: open
`http://<pi>.local:8080` on a phone. It shows:

- What the camera sees, refreshed every second or two, and the state:
  `Ready — put a page down`, *Hold still…*,
  `Photographed — take the page away`.
- The pages of the letter so far, with **Finish letter**, which does the same
  as the button, so a hardware button is optional. Tap a page to see it large
  and delete it if it was a mistake (a hand, the empty table, a page taken
  twice). A letter already sent can only be changed in Paperless.
- What was sent and what is waiting, with **Retry uploads now**.
- Where the letter is going, from the first page. The Pi reads each page and
  matches the shelf's words, so the panel names the folder minutes before
  Paperless has the letter. The headline says *reading page 1...* while it
  reads, about ten seconds a page, then leads with the folder:
  *Taxes - next page?*, or *No folder yet* when no folder's words are on it.
  The letter is closed with that folder unless a later page says otherwise.
  While a further page is being read, the earlier pages' result is an outline
  pill in the corner, outlined because it is a guess. The corner returns to the
  last letter's folder once this one is sent.
- **Learn empty table**, and setup. For the crop, put a page where letters will
  lie and press **Take a picture**. scannerd photographs the whole view in
  colour, finds the page (the largest bright area) and puts a box around it.
  Move the box, drag its corners, or draw a new one, then **Save this area**.
  The crop may be any shape: the photograph is taken at the crop's own share
  of the sensor, so a page-shaped crop is not stretched to 4:3. The sensor's
  pixel count is asked of the camera at start
  (`rpicam-still --list-cameras`, and the log says which it found), so another
  camera is photographed at its own size. `--sensor-width` and
  `--sensor-height` override it, and a camera that cannot be asked is taken
  for a Pi camera v1. A camera that answered is also told which mode to read
  in (`--mode`): half the sensor each way for the live view and the picture of
  the whole view, the full one for photographs. Left to choose for a small
  picture, `rpicam-still` takes the sensor's smallest mode, and a v2's 640×480
  is only the middle two fifths of what it sees, so the live view showed the
  middle of a letter, two and a half times too big. Also the Paperless
  address, token and tags, and **Check
  connection**.
- **The camera looks at the table at an angle**, for a camera that cannot be
  mounted straight above the table, where a page shows as a trapezium. Instead
  of a box there are four corners to drag onto the corners of a page lying
  where letters go. **Take a picture** finds them, a hair outside the page's
  edges, and dashed lines show where the page's edges and middle will fall.
  Every photograph is warped so those corners become a rectangle's before it
  joins the letter. A letter never lies exactly there, so each photograph is
  then straightened again onto the page it shows, cutting away the strips and
  wedges of table beside it. If no clear page covering at least half the
  picture is found, the photograph is kept as it is. A side with paper still
  beyond it is never cut: a sheet's far edge falls off under a camera on a
  stalk and is found short, which used to cut the right-hand column off
  letters. The crop is the area the corners span with 8% of it left round
  them, so a letter put down further along, or a bigger sheet, is still
  photographed. What the sensor leaves out is lost, and the overhanging part
  is usually the letterhead, where the sender is read from. The margin costs
  no sharpness, because the photograph is taken at the crop's own share of the
  sensor. On a Pi Zero W this adds about five seconds a page (a 5-megapixel
  photograph: decoding, warping and encoding take a third each), and a page
  that cannot be straightened is kept as taken. `SCANNERD_CORNERS` sets the
  same from the env file.
- **Which way up**: **Turn left** and **Turn right** turn every photograph a
  quarter turn, for a camera mounted so that letters do not read upright. The
  live view turns with it; the picture for setting the area stays as the
  camera sees it. Turning uses the same warp as straightening, so it costs the
  same five seconds a page on a Pi Zero W, with or without corners.
  `SCANNERD_ROTATE` (0, 90, 180 or 270, clockwise) sets it from the env file.

Settings saved on the page go to `settings.toml` in the spool directory
(owner-only, it holds the token) and win over the env file. The learnt empty
table is kept there too (`empty-table.gray`), so a restart with a letter on the
table sees the letter and does not take it for the table.

The page shows photographs of post, so anything but a loopback address needs
`SCANNERD_UI_PASSWORD` (any user name). Its buttons also require a header that
another site open in the same browser cannot send. The page never touches the
camera or the queue: it reads the status the capture loop publishes and leaves
commands for it, so a button pressed mid-photograph waits a moment and does not
start a second `rpicam-still`.

## Nothing is lost

A capture is written to the spool before any upload is attempted and removed
only once Paperless has confirmed it. After a crash, a flat battery or a Wi-Fi
drop, the upload is retried. The spool is a directory of files, so a Pi
unplugged mid-write is recoverable with `ls`. A half-written photograph keeps a
`.partial` extension and is never uploaded.

Waiting uploads are retried on a timer (`--drain-every-secs`, 30 by default)
and straight after each capture, whether or not the camera is working. A letter
photographed while the network was down is sent once it is back, without
waiting for the next letter. Each capture backs off from its last failed
attempt to a five-minute ceiling, so a Pi that was offline overnight retries
within minutes of the network returning. An upload that has not finished in two
minutes fails and is kept, so it does not block the loop that watches for the
next page.

## Running it

```sh
# against the dev stack, with no camera: drain whatever is spooled and exit
PAPERLESS_TOKEN=... scannerd --drain-only --url http://localhost:8000

# on the Pi
PAPERLESS_TOKEN=... scannerd --tag post --tag home --roi 0.15,0.10,0.70,0.80
```

`--drain-only` is for a cron job, and for checking uploads before there is a
camera.

Tags are given by name and must exist in Paperless first. Its upload endpoint
only takes tag ids, so `scannerd` looks each name up once and sends the id. An
unknown name fails the upload by name and keeps the capture.

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
binary. A Zero 2 W, 3, 4 or 5 on the 64-bit OS wants
`PI_TARGET=aarch64-unknown-linux-gnu.2.36` (after `rustup target add
aarch64-unknown-linux-gnu`). The `.2.36` pins glibc to bookworm's. On a Zero W
(checked on a Rev 1.1 with the OV5647 camera), a 320×240 preview frame takes
1.1 to 1.7 seconds because `rpicam-still` starts the camera every time, and a
full capture about 3.7, so a page is photographed eight or nine seconds after
it is put down.

Then install `scannerd.service` and `scannerd.env.example` from this directory.
It runs as its own user with the `video` group and nothing else, so a daemon
that reaches the camera and holds a Paperless token cannot read the `pi` user's
home directory. That user has to exist first:

```sh
sudo useradd --system --no-create-home --shell /usr/sbin/nologin --groups video,input scannerd
sudo install -m 0755 scannerd /usr/local/bin/
sudo install -m 0644 scannerd.service /etc/systemd/system/
sudo install -m 0600 scannerd.env.example /etc/scannerd.env   # then fill it in
sudo systemctl enable --now scannerd
journalctl -u scannerd -f
```

The rig itself (a Pi 4 with the LCD, and Paperless on the same Pi) is set up
from [`rig/`](rig/readme.md) with `make rig-setup` instead. Only its data and
secrets are kept outside the repository.

## Without a Pi

`tools/rpicam-still-ffmpeg` answers the two `rpicam-still` calls `scannerd`
makes using ffmpeg, so the whole daemon (detection, capture, spool, upload)
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

It replaces the program only. How long `rpicam-still` takes to start on a Pi,
and how its exposure copes with a desk lamp, can only be checked on the device.

## Tests

```sh
cargo test -p scannerd
```

Over a hundred tests, none needing a camera or a display: the detector against
synthetic frames, the spool against a real directory, the uploader against a
server on loopback, the loop (`tests/run.rs`) turned by hand with a scripted
camera and a Paperless that fails when told to, letters and the button
(`tests/letters.rs`, `tests/button.rs`) the same way, the PDF writer
(`tests/pdf.rs`) read back through its own cross-reference table, the
photograph check (`tests/quality.rs`) against synthetic pages and blurred
copies of them, and the display (`tests/display.rs`) by what it draws and how
often it touches the panel. They cannot check whether `rpicam-still` behaves as
documented. Use `--drain-only` and a first run on the device for that.

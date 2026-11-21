# The rig

The scanner is a Raspberry Pi 4 on 64-bit Raspberry Pi OS Lite with a Pi camera
v2 (IMX219; an OV5647 before it) and a 3.5″ SPI LCD with touch. Paperless-ngx
runs beside scannerd, so the letters it photographs are read on the same Pi.

scannerd detects the sensor at start, so a different camera needs no setting
here. A lens with another angle of view moves the page in the frame: set the
crop or the corners again on the setup page and learn the empty table again.

Everything on the Pi comes from this directory except three things:

- **the secrets**: `/etc/scannerd.secrets` (Paperless token, setup page
  password) and `/opt/kuverta-paperless/paperless.secrets.env` (Paperless's
  secret key). `setup.sh` creates them and never overwrites them;
- **the data**: the spool in `/var/lib/scannerd` (letters not yet sent, and
  what the setup page saved: crop, corners, tags, the folders' words) and
  Paperless's documents;
- **the logs**.

`make rig-backup` copies the data and the logs to `~/Backups/kuverta-rig` on
the Mac. Nothing else on the Pi needs keeping. A new SD card and
`make rig-setup` make the same rig.

## A new card

1. Raspberry Pi Imager: Raspberry Pi OS Lite (64-bit). In its settings, set the
   hostname `scannerpi`, the user `pi`, an SSH key and the Wi-Fi.
2. From the workspace root:

   ```sh
   rustup target add aarch64-unknown-linux-gnu
   make rig-setup                 # RIG_HOST=pi@<address> if .local does not resolve
   ```

   That builds scannerd for aarch64, copies this directory, the binary and
   `../scannerd.service` to `~/kuverta-rig` on the Pi, and runs `setup.sh`:
   packages and Docker, the LCD and touch lines in the boot config, Paperless
   in `/opt/kuverta-paperless`, and scannerd as its own service. It says when
   a reboot is due.
3. Create a token for scannerd in Paperless (`http://scannerpi.local:8010`), or
   with `sudo docker compose --project-directory /opt/kuverta-paperless exec
   -u paperless webserver python3 manage.py drf_create_token <user>`. Put it in
   `/etc/scannerd.secrets`, then `sudo systemctl restart scannerd`.
4. Restore the data from the last `make rig-backup`: the spool into
   `/var/lib/scannerd` (owned by `scannerd`), and Paperless's export with
   `document_importer`. The export has to be from the same Paperless version
   as `paperless/compose.yml` pins.

After changing anything here, run `make rig-setup` again.

## What is here

| File | On the Pi |
| --- | --- |
| `scannerd.env` | `/etc/scannerd.env` |
| `config.txt` | a marked block at the end of `/boot/firmware/config.txt` |
| `etc/modprobe.d/scannerd-touch.conf`, `etc/udev/rules.d/90-scannerd-touch.rules` | the same paths: the kernel's touch driver kept off the controller scannerd polls |
| `paperless/` | `/opt/kuverta-paperless` |
| `setup.sh`, `backup.sh` | run by `make rig-setup` and `make rig-backup` |

`setup.sh` also adds `fbcon=map:9` to `cmdline.txt`, so the login console
does not draw over the LCD.

Run anything inside Paperless's container (`manage.py`, the exporter, the
importer) with `-u paperless`. `docker compose exec` defaults to root, and
the worker cannot open root-owned files in its data, such as the search index.
Every letter then fails after it has been read.

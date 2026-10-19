# The rig

The scanner on the shelf: a Raspberry Pi 4 on 64-bit Raspberry Pi OS Lite,
an OV5647 camera, a 3.5″ SPI LCD with touch, and Paperless-ngx running beside
scannerd so the letters it photographs are read on the same Pi.

Everything on it comes from this directory except three things:

- **the secrets**: `/etc/scannerd.secrets` (Paperless token, setup page
  password) and `/opt/kuverta-paperless/paperless.secrets.env` (Paperless's
  secret key). `setup.sh` creates them and never overwrites them;
- **the data**: the spool in `/var/lib/scannerd` (letters not yet sent, what
  the setup page saved — crop, corners, tags, the folders' words) and
  Paperless's documents;
- **the logs**.

`make rig-backup` copies the data and the logs to `~/Backups/kuverta-rig` on
the Mac. Nothing else on the Pi needs keeping: a new SD card and
`make rig-setup` make the same rig.

## A new card

1. Raspberry Pi Imager: Raspberry Pi OS Lite (64-bit). In its settings, the
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
3. A token for scannerd: in Paperless (`http://scannerpi.local:8010`), or
   `sudo docker compose --project-directory /opt/kuverta-paperless exec
   webserver python3 manage.py drf_create_token <user>`. Put it in
   `/etc/scannerd.secrets`, then `sudo systemctl restart scannerd`.
4. The data back, from the last `make rig-backup`: the spool into
   `/var/lib/scannerd` (owned by `scannerd`), and Paperless's export with
   `document_importer`. The export has to be from the same Paperless version
   as `paperless/compose.yml` pins.

`make rig-setup` again after changing anything here puts it on the Pi.

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

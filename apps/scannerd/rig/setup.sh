#!/bin/sh
#
# Makes a fresh 64-bit Raspberry Pi OS Lite into the rig: scannerd with its
# LCD, touch and camera, and Paperless-ngx beside it. Run from the Mac with
# `make rig-setup`, which copies this directory, the scannerd binary and its
# unit file to ~/kuverta-rig on the Pi and runs this there:
#
#   sudo sh ~/kuverta-rig/setup.sh
#
# Running it again puts back whatever the repo says and keeps what is not
# the repo's: the secrets, the spool, Paperless's documents.

set -eu

here=$(cd "$(dirname "$0")" && pwd)

if [ "$(id -u)" != 0 ]; then
    echo "run it as root: sudo sh $0" >&2
    exit 1
fi
# Paperless publishes images for amd64 and arm64 only.
if [ "$(dpkg --print-architecture)" != arm64 ]; then
    echo "needs 64-bit Raspberry Pi OS: this is $(dpkg --print-architecture)" >&2
    exit 1
fi

# --- Packages ---------------------------------------------------------------

apt-get update
# iw: raspi-config sets the Wi-Fi country with it. tesseract: the rig reads
# each page itself, and decides its folder from that while the paper is in
# your hand — without it the display waits for Paperless, which does not
# match folders at all.
apt-get install -y rpicam-apps-lite ca-certificates curl iw tesseract-ocr tesseract-ocr-deu

if ! command -v docker >/dev/null; then
    # Docker's own packages: Debian's lag behind, and older ones have no
    # `docker compose`.
    . /etc/os-release
    install -m 0755 -d /etc/apt/keyrings
    curl -fsSL https://download.docker.com/linux/debian/gpg -o /etc/apt/keyrings/docker.asc
    echo "deb [arch=arm64 signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/debian $VERSION_CODENAME stable" \
        >/etc/apt/sources.list.d/docker.list
    apt-get update
    apt-get install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin
fi

# --- Wi-Fi ------------------------------------------------------------------

# Raspberry Pi OS keeps the radio off until it knows the country. Which
# network, and its password, are not the repo's: Imager sets them, or
# `sudo nmcli dev wifi connect <name> password <password>`.
if [ "$(raspi-config nonint get_wifi_country 2>/dev/null)" != DE ]; then
    raspi-config nonint do_wifi_country DE
fi
nmcli radio wifi on

# --- Boot -------------------------------------------------------------------

reboot=no
config=/boot/firmware/config.txt
begin="# >>> kuverta rig, from apps/scannerd/rig/config.txt"
end="# <<< kuverta rig"
wanted=$(printf '%s\n%s\n%s\n' "$begin" "$(cat "$here/config.txt")" "$end")
current=$(sed -n "\|^$begin\$|,\|^$end\$|p" "$config")
if [ "$current" != "$wanted" ]; then
    sed -i "\|^$begin\$|,\|^$end\$|d" "$config"
    printf '\n%s\n' "$wanted" >>"$config"
    reboot=yes
fi

# The login console off the LCD.
cmdline=/boot/firmware/cmdline.txt
if ! grep -q 'fbcon=map:9' "$cmdline"; then
    sed -i '1 s/$/ fbcon=map:9/' "$cmdline"
    reboot=yes
fi

for file in etc/modprobe.d/scannerd-touch.conf etc/udev/rules.d/90-scannerd-touch.rules; do
    if ! cmp -s "$here/$file" "/$file"; then
        install -m 0644 "$here/$file" "/$file"
        reboot=yes
    fi
done

# --- Paperless --------------------------------------------------------------

paperless=/opt/kuverta-paperless
install -d -m 0755 "$paperless" "$paperless/export" "$paperless/consume"
install -m 0644 "$here/paperless/compose.yml" "$here/paperless/paperless.env" "$paperless/"
if [ ! -s "$paperless/paperless.secrets.env" ]; then
    umask 077
    echo "PAPERLESS_SECRET_KEY=$(head -c 48 /dev/urandom | base64 | tr -d '\n/+=')" \
        >"$paperless/paperless.secrets.env"
    umask 022
fi
docker compose --project-directory "$paperless" up -d

# --- scannerd ---------------------------------------------------------------

if ! id scannerd >/dev/null 2>&1; then
    useradd --system --no-create-home --shell /usr/sbin/nologin scannerd
fi
usermod -aG video,input,spi,gpio scannerd

if [ -f "$here/scannerd" ]; then
    # Installed under another name and renamed: a running executable cannot be
    # opened for writing, but a rename replaces it.
    install -m 0755 "$here/scannerd" /usr/local/bin/scannerd.new
    mv /usr/local/bin/scannerd.new /usr/local/bin/scannerd
fi
install -m 0644 "$here/scannerd.service" /etc/systemd/system/scannerd.service

# A rig set up by hand kept its token in scannerd.env, which is the repo's
# now: move the secrets out before it is replaced.
secrets=/etc/scannerd.secrets
if [ ! -f "$secrets" ]; then
    umask 077
    if [ -f /etc/scannerd.env ] && grep -qE '^(PAPERLESS_TOKEN|SCANNERD_UI_PASSWORD)=.' /etc/scannerd.env; then
        grep -E '^(PAPERLESS_TOKEN|SCANNERD_UI_PASSWORD)=' /etc/scannerd.env >"$secrets"
    else
        printf 'PAPERLESS_TOKEN=\nSCANNERD_UI_PASSWORD=\n' >"$secrets"
    fi
    umask 022
fi
install -m 0644 "$here/scannerd.env" /etc/scannerd.env

systemctl daemon-reload
systemctl enable scannerd
systemctl restart scannerd

# --- What is left -----------------------------------------------------------

if ! grep -qE '^PAPERLESS_TOKEN=.' "$secrets"; then
    echo
    echo "No Paperless token yet: put one in $secrets, then"
    echo "  sudo systemctl restart scannerd"
    echo "Until then letters wait in the spool."
fi
if [ "$reboot" = yes ]; then
    echo
    echo "The boot setup changed: sudo reboot"
fi

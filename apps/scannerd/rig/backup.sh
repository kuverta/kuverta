#!/bin/sh
#
# Writes the rig's data and logs to stdout as a .tar.gz: the spool (letters
# not yet sent, what the setup page saved, the empty table), an export of
# Paperless, and the last two weeks of both logs. Everything else on the Pi
# comes from the repo, through setup.sh. Run from the Mac with
# `make rig-backup`.
#
# The secrets are not in it: a new token is made in Paperless, and a new
# secret key only signs everybody out.

set -eu

paperless=/opt/kuverta-paperless
work=$(mktemp -d)
trap 'rm -rf "$work"; rm -rf "$paperless/export/backup"' EXIT

mkdir -p "$work/logs"
journalctl -u scannerd --since -14d -o short-iso --no-pager >"$work/logs/scannerd.log"
docker compose --project-directory "$paperless" logs --since 336h --no-color \
    >"$work/logs/paperless.log" 2>&1

# document_exporter writes inside the container, to the export directory
# mounted from $paperless/export.
mkdir -p "$paperless/export/backup"
docker compose --project-directory "$paperless" exec -T webserver \
    document_exporter ../export/backup --zip --zip-name paperless --no-progress-bar >&2

tar czf - \
    -C /var/lib scannerd \
    -C "$paperless/export/backup" paperless.zip \
    -C "$work" logs

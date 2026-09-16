#!/usr/bin/env python3
"""Seed the dev Dovecot with the .eml fixtures over IMAP APPEND.

Seeding over IMAP rather than by writing Maildir files directly keeps this
independent of the server's on-disk layout (Dovecot 2.4 defaults to an indexed
layout, not Maildir++), and doubles as a readiness check for the server.

Idempotent: a mailbox that already holds messages is left alone, so repeated
`docker compose up` runs do not duplicate mail. Remove the volume to reseed.
"""
import imaplib
import os
import pathlib
import sys
import time

HOST = os.environ.get("IMAP_HOST", "dovecot")
PORT = int(os.environ.get("IMAP_PORT", "31143"))
USER = os.environ.get("IMAP_USER", "dev@kuverta.test")
PASSWORD = os.environ.get("IMAP_PASS", "devpass")
SEED_ROOT = pathlib.Path(os.environ.get("SEED_ROOT", "/seed"))

# Dovecot needs a few seconds after the container starts; retry rather than
# relying on a healthcheck, since the distroless image has no shell tools.
def connect(timeout=120.0):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            client = imaplib.IMAP4(HOST, PORT)
            client.login(USER, PASSWORD)
            return client
        except Exception as exc:  # noqa: BLE001 - any failure means "not ready yet"
            last = exc
            time.sleep(2.0)
    sys.exit(f"mailseed: {HOST}:{PORT} never became ready: {last}")


def select_or_create(client, mailbox):
    typ, _ = client.select(mailbox)
    if typ != "OK":
        client.create(mailbox)
        typ, _ = client.select(mailbox)
        if typ != "OK":
            sys.exit(f"mailseed: cannot select or create {mailbox}")


def seed(client, mailbox, subdir):
    select_or_create(client, mailbox)

    typ, data = client.search(None, "ALL")
    if typ == "OK" and data and data[0].strip():
        count = len(data[0].split())
        print(f"mailseed: {mailbox} already holds {count} messages, skipping")
        return

    files = sorted((SEED_ROOT / subdir).glob("*.eml"))
    if not files:
        print(f"mailseed: no fixtures in {subdir}")
        return

    for path in files:
        # APPEND requires CRLF line endings; the fixtures are stored with LF.
        raw = path.read_bytes().replace(b"\r\n", b"\n").replace(b"\n", b"\r\n")
        typ, resp = client.append(mailbox, "", imaplib.Time2Internaldate(time.time()), raw)
        if typ != "OK":
            sys.exit(f"mailseed: APPEND of {path.name} failed: {resp}")
        print(f"mailseed: {mailbox} <- {path.name}")


def main():
    client = connect()
    try:
        seed(client, "INBOX", "inbox")
        seed(client, "Archive", "archive")
    finally:
        try:
            client.logout()
        except Exception:  # noqa: BLE001 - logout failures are not interesting here
            pass
    print("mailseed: done")


if __name__ == "__main__":
    main()

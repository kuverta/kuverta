"""Fill a test mailbox with enough varied mail to exercise the list.

APPEND rather than SMTP: it puts the same messages in the same INBOX, but lets
each one carry its own Date and INTERNALDATE, so the list has a realistic
spread to sort and format instead of a few hundred messages all timestamped
now. Nothing is sent: the messages are appended straight into the mailbox.

With `--dump`, the same messages are written to stdout as JSON instead of
being appended anywhere. That is what lets the classifier be measured on the
exact corpus the Rust one was measured on, without an IMAP server in the way:
the generator is seeded, so the sequence is the same either way.

Deliberately varied, because the point is to exercise things that fixtures
cannot: both languages the classifier knows, every category it has, HTML-only
bodies, attachments, long subjects, unicode, a threaded conversation, and
enough volume that the windowed list has to fetch a second page.
"""

import email
import email.header
import email.utils
import html as html_module
import imaplib
import re
import json
import random
import sys
import time
from datetime import datetime, timedelta, timezone

DUMP = "--dump" in sys.argv[1:2]

if DUMP:
    # No server, no credentials: just the messages, as JSON on stdout.
    HOST = PASSWORD = None
    USER = "you@example.com"
    COUNT = int(sys.argv[2]) if len(sys.argv) > 2 else 250
elif len(sys.argv) >= 4:
    HOST, USER, PASSWORD = sys.argv[1], sys.argv[2], sys.argv[3]
    COUNT = int(sys.argv[4]) if len(sys.argv) > 4 else 250
else:
    sys.exit("usage: fill-mailbox.py <imap-host> <user> <password> [count]\n"
             "       fill-mailbox.py --dump [count] > corpus.jsonl\n"
             "       the password is an argument so it never lands in a file")

random.seed(20260911)  # reproducible, so a re-run is comparable

GERMAN_TRANSACTIONAL = [
    ("Stadtwerke München", "abrechnung@swm.example.de",
     "Ihre Abschlagszahlung für {month} wurde geändert",
     "Sehr geehrter Kunde,\n\nIhr monatlicher Abschlag beträgt ab {month} nun {amount} EUR.\n\nMit freundlichen Grüßen\nIhre Stadtwerke"),
    ("Hosting AG", "rechnung@hosting.example.de",
     "Ihre Rechnung {num} für {month}",
     "Guten Tag,\n\nanbei Ihre Rechnung {num} über {amount} EUR, fällig am 15.\n\nVielen Dank."),
    ("Finanzamt München", "noreply@finanzamt.example.de",
     "Bescheid über Einkommensteuer {year}",
     "Ihr Steuerbescheid liegt zum Abruf bereit. Bitte prüfen Sie die Angaben."),
    ("DHL Paket", "noreply@dhl.example.de",
     "Ihre Sendung {num} wurde zugestellt",
     "Ihre Sendung wurde heute zugestellt. Empfänger: Nachbar.\n\nSendungsnummer: {num}"),
]

ENGLISH_TRANSACTIONAL = [
    ("Acme Billing", "billing@acme.example.com",
     "Invoice {num} is ready",
     "Your invoice {num} for {amount} EUR is available.\n\nDue in 14 days."),
    ("Sparkasse", "service@sparkasse.example.de",
     "Kontoauszug verfügbar",
     "Ihr neuer Kontoauszug steht im Postfach bereit."),
]

NEWSLETTERS = [
    ("Rust Weekly", "hello@news.rustweekly.example", "news.rustweekly.example",
     "Rust Weekly #{num} — {topic}",
     "This week in Rust: {topic}, plus a deep dive into lifetimes and three new crates worth a look."),
    ("Golem.de", "newsletter@golem.example.de", "newsletter.golem.example.de",
     "Golem.de Tagesrückblick: {topic}",
     "Die wichtigsten Meldungen des Tages: {topic}. Jetzt weiterlesen auf golem.de."),
    ("Heise Security", "sec@news.heise.example.de", "sec.heise.example.de",
     "Sicherheitsupdate: {topic}",
     "Ein kritisches Update steht bereit. Betroffen sind Versionen vor 4.2."),
]

MARKETING = [
    ("Shop Deals", "deals@shop.example.com", "deals.shop.example.com",
     "{pct}% off everything this week only",
     None),  # HTML-only, built below
    ("MediaMarkt", "angebote@mediamarkt.example.de", "angebote.mediamarkt.example.de",
     "Nur heute: {pct}% auf alles",
     None),
]

NOTIFICATIONS = [
    ("GitHub", "notifications@github.example.com",
     "[fuckmail] CI failed on main",
     "The build failed at step 'cargo clippy'.\n\n  error: unused variable `x`\n\nView the run online."),
    ("Legacy Cron", "root@server.example.de",
     "nightly backup completed",
     "Backup finished at 03:14 UTC. 0 errors, {num} warnings."),
    ("Uptime Robot", "alert@uptime.example.com",
     "Monitor is back UP: api.example.de",
     "api.example.de was down for 4 minutes and is now responding normally."),
]

PEOPLE = [
    ("Anna Weber", "anna.weber@example.de"),
    ("Tom Fisher", "tom.fisher@example.com"),
    ("Dr. Schneider", "schneider@kanzlei.example.de"),
    ("Müller, Jan", "j.mueller@partner.example.de"),
]

PERSONAL_SUBJECTS = [
    ("Termin nächste Woche", "Hallo,\n\npasst dir Dienstag um 14:00? Sonst ginge auch Donnerstag.\n\nViele Grüße"),
    ("Q3 roadmap", "Agreed on all three points. Let's revisit in October — I'll put something in the calendar."),
    ("Rückfrage zum Angebot", "Guten Tag,\n\neine kurze Rückfrage: ist die Wartung im Preis enthalten?\n\nBeste Grüße"),
    ("Fotos vom Wochenende", "Hier die Bilder — waren ein paar gute dabei! Sag Bescheid wenn du sie in groß willst."),
]

TOPICS = [
    "async traits everywhere", "const generics land", "the 2026 edition",
    "KI-Regulierung in der EU", "Neue Lücke in OpenSSL", "Rust im Kernel",
]


def rfc822(date, headers, body, html=None, attachment=False):
    """One message, built by hand so every header is deliberate."""
    lines = [f"{k}: {v}" for k, v in headers.items()]
    lines.append(f"Date: {email.utils.format_datetime(date)}")
    lines.append("MIME-Version: 1.0")

    if attachment:
        b = "b_att_%d" % random.randint(1000, 9999)
        lines.append(f'Content-Type: multipart/mixed; boundary="{b}"')
        lines.append("")
        lines.append(f"--{b}")
        lines.append('Content-Type: text/plain; charset=utf-8')
        lines.append("")
        lines.append(body)
        lines.append(f"--{b}")
        lines.append('Content-Type: application/pdf; name="rechnung.pdf"')
        lines.append("Content-Transfer-Encoding: base64")
        lines.append('Content-Disposition: attachment; filename="rechnung.pdf"')
        lines.append("")
        lines.append("JVBERi0xLjQKJeLjz9MKMSAwIG9iago8PC9UeXBlL0NhdGFsb2c+PgplbmRvYmoK")
        lines.append(f"--{b}--")
    elif html is not None:
        # HTML-only: no text/plain alternative at all, which is what a lot of
        # real marketing mail looks like and what the reading pane has to cope
        # with by rendering it down to text.
        lines.append('Content-Type: text/html; charset=utf-8')
        lines.append("")
        lines.append(html)
    else:
        lines.append('Content-Type: text/plain; charset=utf-8')
        lines.append("")
        lines.append(body)

    return "\r\n".join(lines).encode("utf-8")


def build(n, when):
    """Picks a shape for message `n`, weighted roughly like a real inbox."""
    mid = f"seed-{n}-{int(when.timestamp())}@example.com"
    roll = random.random()
    num = random.randint(100000, 999999)
    amount = f"{random.randint(9, 240)},{random.randint(0, 99):02d}"
    month = random.choice(["Januar", "Februar", "März", "April", "Mai", "Juni",
                           "Juli", "August", "September"])
    topic = random.choice(TOPICS)
    fmt = dict(num=num, amount=amount, month=month, topic=topic,
               year=2025, pct=random.choice([10, 20, 25, 30, 50]))

    if roll < 0.30:
        name, addr, subj, body = random.choice(GERMAN_TRANSACTIONAL + ENGLISH_TRANSACTIONAL)
        attach = random.random() < 0.25
        return rfc822(when, {
            "Message-ID": f"<{mid}>",
            "From": f'"{name}" <{addr}>',
            "To": f"<{USER}>",
            "Subject": subj.format(**fmt),
        }, body.format(**fmt), attachment=attach)

    if roll < 0.52:
        name, addr, list_id, subj, body = random.choice(NEWSLETTERS)
        return rfc822(when, {
            "Message-ID": f"<{mid}>",
            "From": f'"{name}" <{addr}>',
            "To": f"<{USER}>",
            "Subject": subj.format(**fmt),
            "List-Id": f"{name} <{list_id}>",
            "List-Unsubscribe": f"<https://{list_id}/unsubscribe>",
            "Precedence": "bulk",
        }, body.format(**fmt))

    if roll < 0.70:
        name, addr, list_id, subj, _ = random.choice(MARKETING)
        subject = subj.format(**fmt)
        html = (f"<html><body style='font-family:sans-serif'>"
                f"<h1>{subject}</h1>"
                f"<p>Nur für kurze Zeit: <b>{fmt['pct']}%</b> auf das gesamte Sortiment.</p>"
                f"<p><a href='https://{list_id}/sale'>Jetzt shoppen</a></p>"
                f"<img src='https://{list_id}/pixel.gif' width='1' height='1'>"
                f"</body></html>")
        return rfc822(when, {
            "Message-ID": f"<{mid}>",
            "From": f'"{name}" <{addr}>',
            "To": f"<{USER}>",
            "Subject": subject,
            "List-Id": f"{name} <{list_id}>",
            "Precedence": "bulk",
        }, None, html=html)

    if roll < 0.85:
        name, addr, subj, body = random.choice(NOTIFICATIONS)
        return rfc822(when, {
            "Message-ID": f"<{mid}>",
            "From": f'"{name}" <{addr}>',
            "To": f"<{USER}>",
            "Subject": subj.format(**fmt),
            "Auto-Submitted": "auto-generated",
        }, body.format(**fmt))

    # Personal, sometimes as a reply so threading has something to chew on.
    name, addr = random.choice(PEOPLE)
    subject, body = random.choice(PERSONAL_SUBJECTS)
    headers = {
        "Message-ID": f"<{mid}>",
        "From": f'"{name}" <{addr}>',
        "To": f"<{USER}>",
        "Subject": subject,
    }
    if random.random() < 0.5:
        parent = f"<seed-parent-{n}@example.com>"
        headers["Subject"] = f"Re: {subject}"
        headers["In-Reply-To"] = parent
        headers["References"] = parent
        body = f"{body}\n\n> Ursprüngliche Nachricht\n> Kurze Frage dazu."
    if random.random() < 0.3:
        headers["Cc"] = "team@example.de"
    return rfc822(when, headers, body)


def generate():
    """Yields (when, raw, seen) for each message, in the seeded order.

    Both modes consume this, so `--dump` sees exactly the mail `fill` appends.
    Every draw below is part of that sequence — moving one changes the whole
    corpus, which is the point of seeding it.
    """
    now = datetime.now(timezone.utc)
    for n in range(COUNT):
        # Spread over roughly seven months, densest recently — which is what a
        # real inbox looks like and what makes the relative dates worth having.
        days_ago = int(abs(random.gauss(0, 70))) % 210
        when = now - timedelta(days=days_ago,
                               hours=random.randint(0, 23),
                               minutes=random.randint(0, 59))
        raw = build(n, when)

        # Most older mail has been read; recent mail mostly has not.
        seen = random.random() < (0.15 if days_ago < 7 else 0.85)
        yield when, raw, seen


def body_text(message):
    """The text a classifier would read, HTML flattened if that is all there is.

    Stands in for `mail-parser`'s `body_text`, which is what produced the
    snippets the Rust classifier scored. Crude on purpose: it exists to make
    the two runs comparable, not to render mail.
    """
    plain, markup = None, None
    for part in message.walk():
        if part.get_content_maintype() == "multipart":
            continue
        if part.get_filename():
            continue
        try:
            text = part.get_payload(decode=True).decode(
                part.get_content_charset() or "utf-8", "replace")
        except (AttributeError, LookupError):
            continue
        if part.get_content_type() == "text/plain" and plain is None:
            plain = text
        elif part.get_content_type() == "text/html" and markup is None:
            markup = text
    if plain is not None:
        return plain
    if markup is None:
        return None
    stripped = re.sub(r"<[^>]+>", " ", markup)
    return html_module.unescape(stripped)


def decoded(value):
    """Header value as a plain string, RFC 2047 words folded out."""
    if value is None:
        return None
    return str(email.header.make_header(email.header.decode_header(str(value))))


def facts(raw):
    """The headers the classifier reads, shaped the way the extension sees them.

    Parsed twice on purpose. Headers come from the text form, because the
    generator writes 8-bit UTF-8 straight into them as plenty of real senders
    do, and the bytes parser replaces those characters before they can be
    recovered. Bodies come from the bytes form, because that is the one that
    decodes transfer encodings against the declared charset. Thunderbird gives
    an extension the result of both steps.
    """
    headers_from = email.message_from_string(raw.decode("utf-8", "replace"))
    body_from = email.message_from_bytes(raw)

    headers = {}
    for name, value in headers_from.items():
        headers.setdefault(name.lower(), []).append(decoded(value))

    return {
        "subject": decoded(headers_from["Subject"]),
        "author": decoded(headers_from["From"]),
        "recipients": [a for _, a in email.utils.getaddresses(headers_from.get_all("To", []))],
        "ccList": [a for _, a in email.utils.getaddresses(headers_from.get_all("Cc", []))],
        "headers": headers,
        "hasAttachments": any(part.get_filename() for part in body_from.walk()),
        "bodyText": body_text(body_from),
    }


def dump():
    for _, raw, _ in generate():
        print(json.dumps(facts(raw), ensure_ascii=False))


def fill():
    imap = imaplib.IMAP4_SSL(HOST, 993)
    imap.login(USER, PASSWORD)

    appended = 0
    for when, raw, seen in generate():
        flags = "(\\Seen)" if seen else None
        imap.append("INBOX", flags, imaplib.Time2Internaldate(when.timestamp()), raw)
        appended += 1
        if appended % 50 == 0:
            print(f"  {appended}/{COUNT}")
            time.sleep(0.4)  # be polite to a shared server

    print(f"appended {appended} messages")
    imap.select("INBOX", readonly=True)
    print("INBOX now holds", len(imap.search(None, "ALL")[1][0].split()), "messages")
    imap.logout()


dump() if DUMP else fill()

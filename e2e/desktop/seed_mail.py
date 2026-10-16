"""Delivers a few realistic messages to the test mailbox over SMTP, and puts
an old one, from before the app's sync window, straight into it over IMAP."""

import imaplib
import smtplib
import sys
from datetime import datetime, timedelta, timezone
from email.message import EmailMessage
from email.utils import format_datetime

# Older than the app's sync window (90 days): only a search of the whole
# mailbox on the server finds it.
OLD_MESSAGE_DAYS = 400


def main(host: str, port: int, imap_port: int) -> None:
    html = EmailMessage()
    html["From"] = "Bob Builder <bob@example.com>"
    html["To"] = "Alice Example <alice@example.com>"
    html["Subject"] = "Quarterly numbers"
    html.set_content("Hi Alice, the quarterly numbers are in. See https://example.com/report")
    html.add_alternative(
        "<p>Hi Alice,</p><p>The <b>quarterly numbers</b> are in. "
        '<a href="https://example.com/report">Open the report</a>.</p>'
        '<img src="https://tracker.example/pixel.gif" width="1" height="1">',
        subtype="html",
    )

    with_attachment = EmailMessage()
    with_attachment["From"] = "Bob Builder <bob@example.com>"
    with_attachment["To"] = "alice@example.com"
    with_attachment["Subject"] = "Signed contract"
    with_attachment.set_content("Attached is the signed contract.")
    with_attachment.add_attachment(
        b"%PDF-1.4 demo", maintype="application", subtype="pdf", filename="contract.pdf"
    )

    # Links to follow and point at in the reader, a link to further down, and
    # a script the sanitizer and the frame's policy must both keep from running.
    links = EmailMessage()
    links["From"] = "Mallory Example <mallory@example.com>"
    links["To"] = "Alice Example <alice@example.com>"
    links["Subject"] = "Links and a payload"
    links.set_content("Open the report: https://example.com/report")
    links.add_alternative(
        '<p id="para">Some text to click into before pressing a shortcut.</p>'
        '<p><a id="report" href="https://example.com/report">Open the report</a></p>'
        '<p><a id="spoof" href="https://evil.example/login">https://bank.example/login</a></p>'
        '<p><a id="carol" href="mailto:carol@example.com">Write to Carol</a></p>'
        '<p><a id="jump" href="#details">The details, further down</a></p>'
        "<img id=\"payload\" src=x onerror=\"parent.postMessage('payload ran', '*')\">"
        '<div style="height:1600px"></div>'
        '<p id="details">The details.</p>',
        subtype="html",
    )

    with smtplib.SMTP(host, port) as smtp:
        smtp.login("bob", "secret")
        for message in (html, with_attachment, links):
            smtp.send_message(message)

    received = datetime.now(timezone.utc) - timedelta(days=OLD_MESSAGE_DAYS)
    old = EmailMessage()
    old["From"] = "Bob Builder <bob@example.com>"
    old["To"] = "Alice Example <alice@example.com>"
    old["Subject"] = "Old zebracorn invoice"
    old["Date"] = format_datetime(received)
    old.set_content("The zebracorn invoice from last year, for your records.")
    with imaplib.IMAP4(host, imap_port) as imap:
        imap.login("alice", "secret")
        # The sync window goes by the server's arrival date (INTERNALDATE),
        # which APPEND sets.
        imap.append("INBOX", "", imaplib.Time2Internaldate(received), old.as_bytes())


if __name__ == "__main__":
    main(sys.argv[1], int(sys.argv[2]), int(sys.argv[3]) if len(sys.argv) > 3 else 3143)

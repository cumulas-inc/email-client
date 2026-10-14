"""Delivers a few realistic messages to the test mailbox over SMTP."""

import smtplib
import sys
from email.message import EmailMessage


def main(host: str, port: int) -> None:
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

    with smtplib.SMTP(host, port) as smtp:
        smtp.login("bob", "secret")
        for message in (html, with_attachment):
            smtp.send_message(message)


if __name__ == "__main__":
    main(sys.argv[1], int(sys.argv[2]))

"""End-to-end test of the real desktop app (Tauri + Rust core) through
tauri-driver: add an IMAP account, read mail (following, pointing at and
pressing shortcuts in a message, whose own script never runs; pointing at a
link again in a reading font chosen in Settings), send mail,
set up a signature and send it, edited; then, with the window
closed to the background, get notified of new mail (pushed over IMAP IDLE),
open a message from its notification, use its buttons, and see notifications
go once their mail is read elsewhere; get an event's reminder, snooze one and
dismiss another from their notifications; finally restart, and check that the
dismissed reminder doesn't go off again.

Run via ./run.sh, which provides the mail server, keyring and display.
"""

import email
import imaplib
import os
import smtplib
import subprocess
import time
import uuid
from datetime import datetime, timedelta, timezone
from email import policy
from email.message import EmailMessage
from html.parser import HTMLParser
from pathlib import Path

from fake_notifications import FakeNotificationServer
from Xlib import X, display, protocol

from selenium import webdriver
from selenium.common.exceptions import NoSuchElementException, StaleElementReferenceException
from selenium.webdriver.common.action_chains import ActionChains
from selenium.webdriver.common.by import By
from selenium.webdriver.common.keys import Keys
from selenium.webdriver.common.options import ArgOptions
from selenium.webdriver.remote.webelement import WebElement
from selenium.webdriver.support.select import Select

# The app, through app.sh, which keeps its output (tauri-driver drops it).
APP = str(Path(__file__).with_name("app.sh"))
OUT = Path(os.environ.get("E2E_OUTPUT", "e2e/.output"))
# What the app asked the desktop to open (bin/xdg-open records it).
OPENED = OUT / "opened.log"
IMAP_PORT = int(os.environ.get("IMAP_PORT", "3143"))
SMTP_PORT = int(os.environ.get("SMTP_PORT", "3025"))


def start() -> webdriver.Remote:
    options = ArgOptions()
    options.set_capability("browserName", "wry")
    options.set_capability("tauri:options", {"application": APP})
    driver = webdriver.Remote("http://127.0.0.1:4444", options=options)
    driver.set_window_size(1440, 900)
    return driver


def wait_for(fn, timeout=30.0, what="condition"):
    deadline = time.monotonic() + timeout
    last_error = None
    while time.monotonic() < deadline:
        try:
            result = fn()
            if result:
                return result
        except (NoSuchElementException, StaleElementReferenceException) as e:
            last_error = e
        time.sleep(0.2)
    raise AssertionError(f"timed out waiting for {what}: {last_error}")


def by_text(driver, text: str, tag: str = "*") -> WebElement:
    # The innermost element whose text is exactly `text` (not its ancestors).
    xpath = f"//{tag}[normalize-space(.)={quote(text)}][not(.//*[normalize-space(.)={quote(text)}])]"
    return wait_for(
        lambda: next((e for e in driver.find_elements(By.XPATH, xpath) if e.is_displayed()), None),
        what=f"{tag} with text {text!r}",
    )


def button(driver, name: str) -> WebElement:
    xpath = (
        f"//button[normalize-space(.)={quote(name)} or @aria-label={quote(name)}"
        f" or .//*[normalize-space(.)={quote(name)}]]"
    )
    return wait_for(
        lambda: next(
            (e for e in driver.find_elements(By.XPATH, xpath) if e.is_displayed() and e.is_enabled()),
            None,
        ),
        what=f"button {name!r}",
    )


def field(driver, label: str) -> WebElement:
    lbl = by_text(driver, label, "label")
    return driver.find_element(By.ID, lbl.get_attribute("for"))


def type_into(element: WebElement, text: str) -> None:
    element.click()
    element.send_keys(Keys.CONTROL, "a")
    element.send_keys(Keys.BACKSPACE)
    element.send_keys(text)


def ctrl(driver, key: str) -> None:
    """Presses Ctrl and `key` where the focus is."""
    ActionChains(driver).key_down(Keys.CONTROL).send_keys(key).key_up(Keys.CONTROL).perform()


def quote(s: str) -> str:
    return f'"{s}"' if '"' not in s else f"concat('{s}')"


def shot(driver, name: str) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    driver.save_screenshot(str(OUT / f"{name}.png"))


def add_imap_account(driver) -> None:
    button(driver, "Other email (IMAP)").click()
    type_into(field(driver, "Email address"), "alice@example.com")
    type_into(field(driver, "Your name"), "Alice Example")
    type_into(field(driver, "Password"), "secret")
    button(driver, "Advanced settings").click()
    type_into(field(driver, "IMAP server"), "127.0.0.1")
    ports = [e for e in driver.find_elements(By.XPATH, "//label[normalize-space(.)='Port']") if e.is_displayed()]
    type_into(driver.find_element(By.ID, ports[0].get_attribute("for")), str(IMAP_PORT))
    type_into(field(driver, "SMTP server"), "127.0.0.1")
    type_into(driver.find_element(By.ID, ports[1].get_attribute("for")), str(SMTP_PORT))
    securities = [
        e for e in driver.find_elements(By.XPATH, "//label[normalize-space(.)='Security']") if e.is_displayed()
    ]
    for label in securities:
        Select(driver.find_element(By.ID, label.get_attribute("for"))).select_by_visible_text("None")
    type_into(field(driver, "User name"), "alice")
    shot(driver, "01-imap-form")
    button(driver, "Add account").click()


def message_rows(driver) -> list[WebElement]:
    return [e for e in driver.find_elements(By.CSS_SELECTOR, "[role=option]") if e.is_displayed()]


def ensure_archive_folder() -> None:
    """GreenMail starts with an Inbox only; notifications offer Archive when
    the account has an Archive folder."""
    with imaplib.IMAP4("127.0.0.1", IMAP_PORT) as imap:
        imap.login("alice", "secret")
        imap.create("Archive")  # NO when it exists already, from an earlier run


def test() -> None:
    ensure_archive_folder()
    # The app finds the notification server when it starts.
    notifications = FakeNotificationServer()
    notifications.start()
    driver = start()
    try:
        first_run(driver)
        background_and_notifications(driver, notifications)
        dismissed = reminders(driver, notifications)
    except Exception:
        shot(driver, "failure")
        raise
    finally:
        driver.quit()

    # Restart: the account, cache and keyring password survive, and the
    # dismissed reminder stays dismissed.
    driver = start()
    try:
        wait_for(lambda: len(message_rows(driver)) >= 2, timeout=60, what="messages after restart")
        shot(driver, "06-after-restart")
        wait_for(lambda: invoke(driver, "list_active_reminders") is not None, what="reminders")
        active = invoke(driver, "list_active_reminders")
        assert all(r["title"] != dismissed for r in active), active
        time.sleep(3)
        shows = [n for n in notifications.notified if n["summary"] == dismissed]
        assert len(shows) == 1, f"the dismissed reminder went off again: {shows}"
    except Exception:
        shot(driver, "failure-restart")
        raise
    finally:
        driver.quit()

    keyring = subprocess.run(
        ["secret-tool", "search", "application", "co.cumulas.Mail"], capture_output=True, text=True
    )
    assert "secret" in keyring.stdout, "password not stored in the Secret Service"
    print("desktop end-to-end test passed")


def first_run(driver) -> None:
    if True:
        by_text(driver, "Welcome to Cumulas Mail")
        shot(driver, "00-welcome")
        add_imap_account(driver)

        # The account syncs in the background; both seeded messages appear.
        wait_for(lambda: len(message_rows(driver)) >= 2, timeout=60, what="synced messages")
        shot(driver, "02-inbox")
        # textContent: WebKitWebDriver's visible-text algorithm skips rows that
        # use content-visibility.
        subjects = " ".join(r.get_attribute("textContent") for r in message_rows(driver))
        assert "Quarterly numbers" in subjects and "Signed contract" in subjects, subjects

        # Reading a message renders the sanitized body in the sandboxed frame,
        # with the tracking pixel blocked.
        by_text(driver, "Quarterly numbers").click()
        frame = wait_for(lambda: driver.find_element(By.CSS_SELECTOR, "iframe"), what="message frame")
        # An opaque origin: only the app's bridge runs in it, and nothing in
        # it reaches the app.
        assert frame.get_attribute("sandbox") == "allow-scripts", frame.get_attribute("sandbox")
        button(driver, "Show blocked content")  # tracking pixel was blocked
        driver.switch_to.frame(frame)
        body = wait_for(lambda: driver.find_element(By.TAG_NAME, "body").text, what="message body")
        assert "quarterly numbers" in body.lower(), body
        assert not driver.find_elements(By.CSS_SELECTOR, "img[src^='https://tracker']")
        driver.switch_to.default_content()
        shot(driver, "03-reading")

        reader_links(driver)
        reading_font(driver)

        # Attachment metadata comes from the real MIME structure.
        by_text(driver, "Signed contract").click()
        by_text(driver, "contract.pdf")

        # Send a message through SMTP.
        button(driver, "New message").click()
        to = wait_for(lambda: driver.switch_to.active_element, what="focused To field")
        to.send_keys("bob@example.com", Keys.ENTER)
        subject = driver.find_element(By.CSS_SELECTOR, "input[aria-label='Subject']")
        subject.send_keys("Hello from the desktop test")
        editor = wait_for(
            lambda: driver.find_element(By.CSS_SELECTOR, "[contenteditable=true]"), what="editor"
        )
        editor.click()
        editor.send_keys("Sent by the end-to-end test.")
        shot(driver, "04-compose")
        button(driver, "Send").click()
        wait_for(lambda: bob_has("Hello from the desktop test"), timeout=30, what="delivery to bob")

        # The copy saved to Sent shows up once the post-send sync finishes.
        by_text(driver, "Sent").click()
        wait_for(
            lambda: any(
                "Hello from the desktop test" in (r.get_attribute("textContent") or "")
                for r in message_rows(driver)
            ),
            timeout=30,
            what="sent copy in the Sent folder",
        )
        shot(driver, "05-sent")

        signature(driver)


# --- links, the link's address and shortcuts in a message --------------------------


def opened() -> list[str]:
    return OPENED.read_text().splitlines() if OPENED.exists() else []


def reader_frame(driver) -> WebElement:
    """The open message's frame, once its document has the seeded links."""
    frame = wait_for(
        lambda: next(
            (
                e
                for e in driver.find_elements(By.CSS_SELECTOR, ".mail-reader iframe.mail-body-frame")
                if e.is_displayed()
            ),
            None,
        ),
        what="the message's frame",
    )
    in_frame(driver, frame, lambda: wait_for(lambda: driver.find_element(By.ID, "report"), what="its links"))
    return frame


def in_frame(driver, frame: WebElement, fn):
    driver.switch_to.frame(frame)
    try:
        return fn()
    finally:
        driver.switch_to.default_content()


def reader_status(driver) -> str | None:
    """The address in the reader's status line, if it shows one."""
    shown = [e for e in driver.find_elements(By.CSS_SELECTOR, ".mail-reader__status") if e.is_displayed()]
    return shown[0].get_attribute("textContent") if shown else None


def composer(driver) -> WebElement | None:
    return next((e for e in driver.find_elements(By.CSS_SELECTOR, ".mail-compose") if e.is_displayed()), None)


def discard_composer(driver) -> None:
    button(driver, "Discard").click()
    time.sleep(0.3)
    for dialog in driver.find_elements(By.CSS_SELECTOR, "dialog[open]"):
        for b in dialog.find_elements(By.XPATH, ".//button[normalize-space(.)='Discard']"):
            if b.is_displayed():
                b.click()
    wait_for(lambda: composer(driver) is None, timeout=10, what="the composer to close")


# Everything frames post to the app's window.
RECORD_MESSAGES = """
window.__e2eMessages = [];
window.addEventListener("message", (e) => {
  if (e.source !== window) window.__e2eMessages.push(e.data);
});
"""

# Script that got past the sanitizer, put first in the frame's document: the
# frame's policy runs the bridge and refuses all of it. Each would post a string;
# the bridge posts only objects.
INJECT_PAYLOAD = """
const [frame, done] = arguments;
const payload =
  `<p><a id="js" href="javascript:parent.postMessage('payload: javascript', '*')">js</a></p>` +
  `<img src="x" onerror="parent.postMessage('payload: onerror', '*')">` +
  `<script>parent.postMessage('payload: script', '*')</` + `script>` +
  `<svg onload="parent.postMessage('payload: svg', '*')"></svg>`;
frame.addEventListener("load", () => setTimeout(() => done(true), 500), { once: true });
frame.setAttribute("srcdoc", frame.getAttribute("srcdoc").replace(/<body[^>]*>/, (body) => body + payload));
"""


def reader_links(driver) -> None:
    """In a message's frame (an opaque origin, which only the app's bridge
    runs in): a web link opens in the browser, a mail link starts a message,
    pointing at a link shows its real address, a link to further down
    scrolls the reader, and Ctrl+R replies. The mail's own script never
    runs, past the sanitizer or not."""
    driver.execute_script(RECORD_MESSAGES)
    by_text(driver, "Links and a payload").click()
    frame = reader_frame(driver)
    assert frame.get_attribute("sandbox") == "allow-scripts", frame.get_attribute("sandbox")
    # The sanitizer took the handler off.
    payload = in_frame(
        driver, frame, lambda: driver.find_element(By.ID, "payload").get_dom_attribute("onerror")
    )
    assert payload is None, payload

    # Pointing at a link shows where it really goes, whatever its text says.
    in_frame(
        driver,
        frame,
        lambda: ActionChains(driver).move_to_element(driver.find_element(By.ID, "spoof")).perform(),
    )
    wait_for(
        lambda: reader_status(driver) == "https://evil.example/login",
        what="the link's address in the status line",
    )
    shot(driver, "03a-link-address")

    # A web link opens in the browser.
    in_frame(driver, frame, lambda: driver.find_element(By.ID, "report").click())
    wait_for(lambda: "https://example.com/report" in opened(), what="the link opened in the browser")

    # A link to further down scrolls the reader there.
    scroller = driver.find_element(By.CSS_SELECTOR, ".mail-reader__scroll")

    def scrolled() -> int:
        return driver.execute_script("return arguments[0].scrollTop", scroller)

    before = scrolled()
    in_frame(driver, frame, lambda: driver.find_element(By.ID, "jump").click())
    wait_for(lambda: scrolled() > before + 1000, what="the reader scrolled to the details")
    shot(driver, "03b-link-within")

    # Shortcuts work after clicking into the message: Ctrl+R replies.
    def reply():
        driver.find_element(By.ID, "para").click()
        ctrl(driver, "r")

    in_frame(driver, frame, reply)
    subject = wait_for(
        lambda: driver.find_element(By.CSS_SELECTOR, ".mail-compose input[aria-label='Subject']"),
        what="the reply",
    )
    assert subject.get_attribute("value") == "Re: Links and a payload", subject.get_attribute("value")
    shot(driver, "03c-reply-from-frame")
    discard_composer(driver)

    # A mail link starts a message to its address.
    frame = reader_frame(driver)
    in_frame(driver, frame, lambda: driver.find_element(By.ID, "carol").click())
    wait_for(
        lambda: "carol@example.com" in ((c := composer(driver)) and c.get_attribute("textContent") or ""),
        what="a message to Carol",
    )
    discard_composer(driver)

    frame = reader_frame(driver)
    before = driver.execute_script("return window.__e2eMessages.length")
    assert driver.execute_async_script(INJECT_PAYLOAD, frame)
    in_frame(driver, frame, lambda: driver.find_element(By.ID, "js").click())
    time.sleep(1)
    messages = driver.execute_script("return window.__e2eMessages")
    # Only the bridge's reports came from the frame, as delivered or with the
    # payload put in: none of the mail's script ran.
    assert all(isinstance(m, dict) and m.get("channel") for m in messages), messages
    # The bridge ran in that document too, and reported the javascript: link,
    # which the app left alone.
    assert any(
        m["type"] == "link" and m["href"].startswith("javascript:") for m in messages[before:]
    ), messages[before:]
    assert opened() == ["https://example.com/report"], opened()


def reading_font(driver) -> None:
    """Settings → General: messages read in the interface font (the bundled
    Inter, which the message's frame carries). The frame keeps its document
    while the reader changes around it, so pointing at a link shows its
    address, and keeps showing it."""
    ctrl(driver, ",")
    button(driver, "General").click()
    picker = wait_for(
        lambda: next(
            (
                e
                for e in driver.find_elements(By.CSS_SELECTOR, ".settings input[role='combobox']")
                if e.get_attribute("value") == "Charis SIL (default)"
            ),
            None,
        ),
        what="the reading font",
    )
    picker.click()
    by_text(driver, "Same as interface font").click()
    wait_for(
        lambda: invoke(driver, "get_settings")["readingFont"]["kind"] == "interface",
        what="the reading font saved",
    )
    button(driver, "Close").click()

    def in_interface_font() -> WebElement | None:
        frame = reader_frame(driver)
        return frame if "Inter Variable" in (frame.get_attribute("srcdoc") or "") else None

    frame = wait_for(in_interface_font, what="the message in the interface font")
    document = frame.get_attribute("srcdoc")
    in_frame(
        driver,
        frame,
        lambda: ActionChains(driver).move_to_element(driver.find_element(By.ID, "spoof")).perform(),
    )
    wait_for(
        lambda: reader_status(driver) == "https://evil.example/login",
        what="the link's address in the status line, in the interface font",
    )
    time.sleep(1)
    assert reader_status(driver) == "https://evil.example/login", reader_status(driver)
    assert frame.get_attribute("srcdoc") == document, "the message's frame was given a new document"
    shot(driver, "03d-link-address-reading-font")


# --- signatures ------------------------------------------------------------------


class SignatureText(HTMLParser):
    """The text inside <div class="cumulas-signature">."""

    def __init__(self) -> None:
        super().__init__()
        self.depth = 0
        self.text: list[str] = []
        self.found = False

    def handle_starttag(self, tag, attrs):
        if self.depth:
            if tag == "div":
                self.depth += 1
            if tag in ("div", "p", "br"):
                self.text.append("\n")
        elif tag == "div" and "cumulas-signature" in (dict(attrs).get("class") or "").split():
            self.depth = 1
            self.found = True

    def handle_endtag(self, tag):
        if self.depth and tag == "div":
            self.depth -= 1
            if self.depth:
                self.text.append("\n")

    def handle_data(self, data):
        if self.depth:
            self.text.append(data)


def bob_newest_html(subject: str) -> str:
    """The HTML part of the newest message in bob's Inbox with `subject`."""
    with imaplib.IMAP4("127.0.0.1", IMAP_PORT) as imap:
        imap.login("bob", "secret")
        imap.select("INBOX")
        _, data = imap.search(None, "SUBJECT", f'"{subject}"')
        _, parts = imap.fetch(data[0].split()[-1], "(RFC822)")
    message = email.message_from_bytes(parts[0][1], policy=policy.default)
    return message.get_body(preferencelist=("html",)).get_content()


def signature_frame(driver, scope: str) -> WebElement:
    return wait_for(
        lambda: next(
            (
                e
                for e in driver.find_elements(By.CSS_SELECTOR, f"{scope} iframe[title='Signature']")
                if e.is_displayed()
            ),
            None,
        ),
        what=f"the signature editor in {scope}",
    )


def signature(driver) -> None:
    """Settings → Signatures: an own signature typed into the signature
    editor (a sandboxed frame, edited in place) goes under the next message,
    with a word added to it in the composer."""
    ctrl(driver, ",")
    button(driver, "Signatures").click()
    frame = signature_frame(driver, ".settings")
    # Scripts "allowed" only so the app's listeners in it are called; its
    # policy allows none.
    assert frame.get_attribute("sandbox") == "allow-same-origin allow-scripts", frame.get_attribute(
        "sandbox"
    )
    driver.switch_to.frame(frame)
    body = wait_for(
        lambda: driver.find_element(By.CSS_SELECTOR, "body[contenteditable=true]"),
        what="the editable signature",
    )
    body.click()
    body.send_keys("Alice Example")
    body.send_keys(Keys.ENTER)
    body.send_keys("Example Corp")
    assert "Example Corp" in body.text, body.text
    driver.switch_to.default_content()
    # Saved after a pause in typing.
    accounts = invoke(driver, "list_accounts")
    account_id = accounts[0]["id"]
    wait_for(
        lambda: "Example Corp" in invoke(driver, "get_account_signature", {"accountId": account_id})["ownHtml"],
        timeout=10,
        what="the own signature saved",
    )
    shot(driver, "07-signature-settings")
    button(driver, "Close").click()

    button(driver, "New message").click()
    to = wait_for(lambda: driver.switch_to.active_element, what="focused To field")
    to.send_keys("bob@example.com", Keys.ENTER)
    subject = "Signed by the desktop test"
    driver.find_element(By.CSS_SELECTOR, "input[aria-label='Subject']").send_keys(subject)
    editor = driver.find_element(By.CSS_SELECTOR, ".mail-editor [contenteditable=true]")
    editor.click()
    editor.send_keys("With a signature below.")

    # The signature is under the body; a word goes on its last line.
    frame = signature_frame(driver, ".mail-compose")
    assert frame.location["y"] > editor.location["y"], (frame.location, editor.location)
    driver.switch_to.frame(frame)
    body = wait_for(
        lambda: next(
            (e for e in driver.find_elements(By.TAG_NAME, "body") if "Example Corp" in e.text), None
        ),
        what="the signature in the composer",
    )
    body.click()
    body.send_keys(Keys.CONTROL, Keys.END)
    body.send_keys(" Labs")
    assert "Example Corp Labs" in body.text, body.text
    driver.switch_to.default_content()
    shot(driver, "08-signature-compose")
    button(driver, "Send").click()
    wait_for(lambda: bob_has(subject), timeout=30, what="the signed message delivered to bob")

    html = bob_newest_html(subject)
    parser = SignatureText()
    parser.feed(html)
    assert parser.found, html
    lines = [line.strip() for line in "".join(parser.text).splitlines() if line.strip()]
    assert lines == ["Alice Example", "Example Corp Labs"], (lines, html)
    # Under the body, as Outlook puts it.
    assert html.index("With a signature below.") < html.index("cumulas-signature"), html


# --- new mail in the background ------------------------------------------------


def app_windows() -> list:
    """The app's top-level X windows: titled "Cumulas Mail", closable."""
    d = display.Display()
    protocols = d.intern_atom("WM_PROTOCOLS")
    delete = d.intern_atom("WM_DELETE_WINDOW")

    def walk(window):
        yield window
        for child in window.query_tree().children:
            yield from walk(child)

    found = []
    for window in walk(d.screen().root):
        try:
            if window.get_wm_name() != "Cumulas Mail":
                continue
            prop = window.get_full_property(protocols, X.AnyPropertyType)
            if prop and delete in prop.value:
                found.append((d, window, protocols, delete))
        except Exception:  # noqa: BLE001 - windows come and go
            continue
    return found


def window_shown() -> bool:
    return any(w.get_attributes().map_state == X.IsViewable for _, w, _, _ in app_windows())


def close_window() -> None:
    """Closes the window as the window manager's close button would."""
    for d, window, protocols, delete in app_windows():
        if window.get_attributes().map_state != X.IsViewable:
            continue
        event = protocol.event.ClientMessage(
            window=window, client_type=protocols, data=(32, [delete, X.CurrentTime, 0, 0, 0])
        )
        window.send_event(event, event_mask=X.NoEventMask)
        d.flush()
    wait_for(lambda: not window_shown(), timeout=10, what="the window to hide")


def deliver(subject: str, sender: str = "Pat Push <pat@example.com>") -> None:
    message = EmailMessage()
    message["From"] = sender
    message["To"] = "alice@example.com"
    message["Subject"] = subject
    message.set_content(f"{subject}: sent by the end-to-end test.")
    with smtplib.SMTP("127.0.0.1", SMTP_PORT) as smtp:
        smtp.login("bob", "secret")
        smtp.send_message(message)


def alice_flags(subject: str, folder: str = "INBOX") -> str | None:
    """The message's flags on the server, or None if it isn't in `folder`."""
    with imaplib.IMAP4("127.0.0.1", IMAP_PORT) as imap:
        imap.login("alice", "secret")
        imap.select(folder)
        _, data = imap.search(None, "SUBJECT", f'"{subject}"')
        uids = data[0].split()
        if not uids:
            return None
        _, flags = imap.fetch(uids[-1], "(FLAGS)")
        return flags[0].decode()


def mark_read_elsewhere(subject: str) -> None:
    with imaplib.IMAP4("127.0.0.1", IMAP_PORT) as imap:
        imap.login("alice", "secret")
        imap.select("INBOX")
        _, data = imap.search(None, "SUBJECT", f'"{subject}"')
        for uid in data[0].split():
            imap.store(uid, "+FLAGS", "\\Seen")


def action_key(notification: dict, label: str) -> str:
    """The action key of a notification's button."""
    actions = notification["actions"]
    return actions[actions.index(label) - 1]


def new_mail_notification(notifications: FakeNotificationServer, subject: str) -> dict:
    return notifications.wait_for(
        lambda n: n["body"] == subject and n["summary"] == "Pat Push",
        timeout=30,
        what=f"a notification for {subject!r}",
    )


def background_and_notifications(driver, notifications: FakeNotificationServer) -> None:
    run = uuid.uuid4().hex[:6]

    # Closing the window keeps the app running, and says so once. With no
    # tray here, quitting is in the main menu (Ctrl+Q is Mark as read).
    close_window()
    notice = notifications.wait_for(
        lambda n: n["summary"] == "Still running in the background", what="the background notice"
    )
    assert "Quit in the main menu" in notice["body"], notice["body"]
    assert "Ctrl+Q" not in notice["body"], notice["body"]
    assert driver.title is not None  # still running

    # New mail arrives over IMAP IDLE (the periodic sync is minutes away) and
    # raises a notification with the sender, subject and buttons.
    first = f"Pushed {run} one"
    deliver(first)
    shown = new_mail_notification(notifications, first)
    assert shown["actions"][:2] == ["default", "Open"], shown["actions"]
    assert "Mark as read" in shown["actions"] and "Archive" in shown["actions"], shown["actions"]
    hints = shown["hints"]
    assert hints.get("category") == "email.arrived", hints
    assert hints.get("urgency") == 1, hints
    assert hints.get("desktop-entry"), hints

    # Clicking it (the server then closes it) opens the window on the
    # message, which is marked read as it opens.
    notifications.click(shown, token="e2e-activation-token")
    wait_for(window_shown, timeout=15, what="the window to come back")
    by_text(driver, first, "h1")
    shot(driver, "07-opened-from-notification")
    wait_for(
        lambda: "\\Seen" in (alice_flags(first) or ""), timeout=20, what="read on the server"
    )

    # Ctrl+U marks it unread, and Ctrl+Q read again, as in Outlook, without
    # quitting.
    by_text(driver, first, "h1").click()
    ctrl(driver, "u")
    wait_for(
        lambda: "\\Seen" not in (alice_flags(first) or "\\Seen"),
        timeout=20,
        what="unread on the server",
    )
    ctrl(driver, "q")
    wait_for(
        lambda: "\\Seen" in (alice_flags(first) or ""), timeout=20, what="read again on the server"
    )
    time.sleep(1)
    assert window_shown(), "Ctrl+Q closed the app"

    # "Mark as read" works without the window.
    close_window()
    second = f"Pushed {run} two"
    deliver(second)
    shown = new_mail_notification(notifications, second)
    notifications.click(shown, action_key(shown, "Mark as read"))
    wait_for(
        lambda: "\\Seen" in (alice_flags(second) or ""), timeout=20, what="read on the server"
    )
    assert not window_shown()

    # So does "Archive".
    third = f"Pushed {run} three"
    deliver(third)
    shown = new_mail_notification(notifications, third)
    notifications.click(shown, action_key(shown, "Archive"))
    wait_for(lambda: alice_flags(third) is None, timeout=20, what="the message to leave the Inbox")
    assert alice_flags(third, "Archive") is not None, "not in Archive"

    # Read on another device: the app withdraws the notification at the next
    # sync (GreenMail doesn't report flag changes over IDLE).
    fourth = f"Pushed {run} four"
    deliver(fourth)
    shown = new_mail_notification(notifications, fourth)
    mark_read_elsewhere(fourth)
    driver.execute_script("window.__TAURI_INTERNALS__.invoke('sync_all')")
    notifications.wait_closed(shown["id"], timeout=30)
    assert not window_shown()


# --- reminders ---------------------------------------------------------------------


def invoke(driver, command: str, args: dict | None = None):
    """Calls one of the app's commands and returns its answer."""
    result = driver.execute_async_script(
        "const [command, args, done] = arguments;"
        "window.__TAURI_INTERNALS__.invoke(command, args)"
        ".then((ok) => done({ ok }), (err) => done({ err: String(err && err.message || err) }));",
        command,
        args or {},
    )
    assert "err" not in result, f"{command} failed: {result['err']}"
    return result["ok"]


def utc(at: datetime) -> str:
    return at.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def event_soon(driver, title: str) -> None:
    """An event in the local calendar starting in a minute, reminding a
    minute before: its reminder is due at once."""
    calendar = invoke(driver, "list_calendars")[0]
    start = datetime.now(timezone.utc) + timedelta(minutes=1)
    invoke(
        driver,
        "create_event",
        {
            "draft": {
                "calendarId": calendar["id"],
                "subject": title,
                "start": utc(start),
                "end": utc(start + timedelta(minutes=30)),
                "reminderMinutes": 1,
                "showAs": "busy",
            }
        },
    )


def reminder_notification(notifications: FakeNotificationServer, title: str) -> dict:
    return notifications.wait_for(
        lambda n: n["summary"] == title, timeout=20, what=f"the reminder for {title!r}"
    )


def reminders(driver, notifications: FakeNotificationServer) -> str:
    """Reminders go off as notifications with Snooze and Dismiss, which work
    without the window. Returns the title of the dismissed one."""
    run = uuid.uuid4().hex[:6]

    snoozed = f"Design review {run}"
    event_soon(driver, snoozed)
    shown = reminder_notification(notifications, snoozed)
    assert "Snooze 5 min" in shown["actions"] and "Dismiss" in shown["actions"], shown["actions"]
    assert "Join" not in shown["actions"], shown["actions"]
    hints = shown["hints"]
    assert hints.get("urgency") == 1, hints
    assert hints.get("sound-name") == "alarm-clock-elapsed", hints
    assert "category" not in hints, hints
    notifications.click(shown, action_key(shown, "Snooze 5 min"))
    wait_for(
        lambda: any(
            r["title"] == snoozed and r["snoozedUntil"] for r in invoke(driver, "list_active_reminders")
        ),
        timeout=15,
        what="the reminder snoozed",
    )

    dismissed = f"Standup {run}"
    event_soon(driver, dismissed)
    shown = reminder_notification(notifications, dismissed)
    notifications.click(shown, action_key(shown, "Dismiss"))
    wait_for(
        lambda: all(r["title"] != dismissed for r in invoke(driver, "list_active_reminders")),
        timeout=15,
        what="the reminder dismissed",
    )
    assert not window_shown()
    return dismissed


def bob_has(subject: str) -> bool:
    with imaplib.IMAP4("127.0.0.1", IMAP_PORT) as imap:
        imap.login("bob", "secret")
        imap.select("INBOX")
        _, data = imap.search(None, "SUBJECT", f'"{subject}"')
        return bool(data[0].split())


if __name__ == "__main__":
    test()

"""End-to-end test of the real desktop app (Tauri + Rust core) through
tauri-driver: add an IMAP account, read mail, send mail, restart.

Run via ./run.sh, which provides the mail server, keyring and display.
"""

import imaplib
import os
import subprocess
import time
from pathlib import Path

from selenium import webdriver
from selenium.common.exceptions import NoSuchElementException, StaleElementReferenceException
from selenium.webdriver.common.by import By
from selenium.webdriver.common.keys import Keys
from selenium.webdriver.common.options import ArgOptions
from selenium.webdriver.remote.webelement import WebElement
from selenium.webdriver.support.select import Select

APP = os.environ["CUMULAS_APP"]
OUT = Path(os.environ.get("E2E_OUTPUT", "e2e/.output"))
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


def test() -> None:
    driver = start()
    try:
        first_run(driver)
    except Exception:
        shot(driver, "failure")
        raise
    finally:
        driver.quit()

    # Restart: the account, cache and keyring password survive.
    driver = start()
    try:
        wait_for(lambda: len(message_rows(driver)) >= 2, timeout=60, what="messages after restart")
        shot(driver, "06-after-restart")
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
        assert "allow-scripts" not in (frame.get_attribute("sandbox") or "")
        button(driver, "Show blocked content")  # tracking pixel was blocked
        driver.switch_to.frame(frame)
        body = wait_for(lambda: driver.find_element(By.TAG_NAME, "body").text, what="message body")
        assert "quarterly numbers" in body.lower(), body
        assert not driver.find_elements(By.CSS_SELECTOR, "img[src^='https://tracker']")
        driver.switch_to.default_content()
        shot(driver, "03-reading")

        # Attachment metadata comes from the real MIME structure.
        by_text(driver, "Signed contract").click()
        by_text(driver, "contract.pdf")

        # Send a message through SMTP.
        button(driver, "New mail").click()
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


def bob_has(subject: str) -> bool:
    with imaplib.IMAP4("127.0.0.1", IMAP_PORT) as imap:
        imap.login("bob", "secret")
        imap.select("INBOX")
        _, data = imap.search(None, "SUBJECT", f'"{subject}"')
        return bool(data[0].split())


if __name__ == "__main__":
    test()

# Desktop end-to-end test

Drives the real desktop app (Tauri + Rust core, WebKitGTK webview) through
[tauri-driver](https://v2.tauri.app/develop/tests/webdriver/) against a local
[GreenMail](https://greenmail-mail-test.github.io/greenmail/) IMAP/SMTP server,
inside a private D-Bus session with GNOME Keyring as the Secret Service.

It adds an IMAP account through the UI, reads mail (checking the sandboxed
body and blocked tracking pixel), sends a message over SMTP and checks its
delivery and the copy in Sent. Then it closes the window (as a window manager
would, with `WM_DELETE_WINDOW`) and, with the app running in the background,
checks the desktop side against a fake notification server
(`fake_notifications.py`, on the test's session bus): mail delivered over SMTP
raises a notification within seconds (IMAP IDLE), clicking it shows the window
on the message, its Mark as read and Archive buttons work without the window,
and a notification goes once its message is read elsewhere. Finally it
restarts the app to check that the account, cache and keyring password persist.

Without a StatusNotifier host on the test's bus, the app shows no tray icon;
starting without crashing with libayatana-appindicator installed is part of the
test.

```sh
sudo apt install webkit2gtk-driver xvfb gnome-keyring libsecret-tools dbus-x11 openjdk-21-jre-headless
cargo install tauri-driver --locked
python3 -m pip install -r e2e/desktop/requirements.txt
e2e/desktop/run.sh   # screenshots in e2e/.output/desktop/
```

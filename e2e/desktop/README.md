# Desktop end-to-end test

Drives the real desktop app (Tauri + Rust core, WebKitGTK webview) through
[tauri-driver](https://v2.tauri.app/develop/tests/webdriver/) against a local
[GreenMail](https://greenmail-mail-test.github.io/greenmail/) IMAP/SMTP server,
inside a private D-Bus session with GNOME Keyring as the Secret Service.

It adds an IMAP account through the UI, reads mail (checking the sandboxed
body and blocked tracking pixel), sends a message over SMTP and checks its
delivery and the copy in Sent, then restarts the app to check that the
account, cache and keyring password persist.

```sh
sudo apt install webkit2gtk-driver xvfb gnome-keyring libsecret-tools dbus-x11 openjdk-21-jre-headless
cargo install tauri-driver --locked
python3 -m pip install -r e2e/desktop/requirements.txt
e2e/desktop/run.sh   # screenshots in e2e/.output/desktop/
```

# Desktop end-to-end test

Drives the real desktop app (Tauri + Rust core, WebKitGTK webview) through
[tauri-driver](https://v2.tauri.app/develop/tests/webdriver/) against a local
[GreenMail](https://greenmail-mail-test.github.io/greenmail/) IMAP/SMTP server,
inside a private D-Bus session with GNOME Keyring as the Secret Service.

It adds an IMAP account through the UI, reads mail (checking the sandboxed
body and blocked tracking pixel), and in a message's frame points at a link
(the reader shows where it goes), clicks a web link (`bin/xdg-open`, first on
the app's `PATH`, records it in `opened.log` instead of opening it), a mail
link (a new message), a link to further down (the reader scrolls) and presses
Ctrl+R (a reply); the message's own script never runs, neither as delivered
nor put into the frame's document past the sanitizer. With the reading font
set to the interface font in Settings → General, pointing at the link still
shows its address, and the frame keeps its document. It sends a message over
SMTP and checks its delivery and the copy in Sent. It types a signature into Settings →
Signatures (the signature editor is a sandboxed frame), adds a word to it in
a new message and checks that the delivered HTML has it, edited, in its
`cumulas-signature` block under the body. It searches mail: by the start of a
word, and with `from:`; and for a message from long before the sync window
(`seed_mail.py` puts it into the Inbox over IMAP with an arrival date 400 days
ago), which only a search of the whole mailbox on the server finds: it shows
labeled with its folder, and opens (and is never notified as new mail). Then it closes the window (as a window manager
would, with `WM_DELETE_WINDOW`) and, with the app running in the background,
checks the desktop side against a fake notification server
(`fake_notifications.py`, on the test's session bus): mail delivered over SMTP
raises a notification within seconds (IMAP IDLE), clicking it shows the window
on the message (where Ctrl+U and Ctrl+Q mark it unread and read, as in
Outlook, without quitting), its Mark as read and Archive buttons work without
the window, and a notification goes once its message is read elsewhere.
Events starting in a minute (made through the app's commands) remind at once:
the reminder's notification has Snooze and Dismiss, which work without the
window. Finally it restarts the app to check that the account, cache and
keyring password persist, that the message the server search found is still
on the device (the syncs since kept it), and that the dismissed reminder doesn't go off again.

Without a StatusNotifier host on the test's bus, the app shows no tray icon;
starting without crashing with libayatana-appindicator installed is part of the
test. `test_tray.py` then starts the app again and brings up a fake host
(`org.kde.StatusNotifierWatcher`) after it: the app adds its icon, the test
reads its menu over dbusmenu and clicks Quit, and the app exits cleanly.

```sh
sudo apt install webkit2gtk-driver xvfb gnome-keyring libsecret-tools dbus-x11 openjdk-21-jre-headless
cargo install tauri-driver --locked
python3 -m pip install -r e2e/desktop/requirements.txt
e2e/desktop/run.sh   # screenshots and logs in e2e/.output/desktop/
```

The tests start the app through `app.sh`, which adds its output to `app.log`
there (tauri-driver drops the app's stdout, where it logs). `tauri-driver.log`
and `greenmail.log` sit next to it. CI uploads the folder as the `desktop-e2e`
artifact.

# Privacy policy

Cumulas Mail is a desktop app. It keeps your mail on your computer and talks
to your mail providers directly. This page says what it keeps, what it sends
where, and what passes through the one service Cumulas runs for it: the push
service at `sync.cumulas.co`, which tells the app when the Inbox or the
calendar of a Microsoft account changes.

Cumulas Mail has no telemetry, no analytics, no crash reporting and no
advertising, and you don't need a Cumulas account.

_Last updated: October 15, 2026._

## What the app keeps on your computer

- **A copy of your mail, calendar, contacts and tasks**, so that lists and
  search work instantly and offline. It's in a database in the app's data
  folder, `~/.local/share/co.cumulas.Mail/`, along with your settings.
- **Attachments you open**, in the app's cache folder,
  `~/.cache/co.cumulas.Mail/`.
- **Passwords and sign-in tokens**, in your desktop's keyring (GNOME Keyring
  or KWallet), never in the app's database.
- **An installation ID**: a random string made once, which the push service
  uses to tell this installation's connections apart. It says nothing about
  you or your computer.
- If you choose to start the app at login, an autostart entry,
  `~/.config/autostart/co.cumulas.Mail.desktop`.

The Flatpak keeps its data and cache folders in `~/.var/app/co.cumulas.Mail/`
instead. It keeps passwords and sign-in tokens there too, encrypted, in a
keyring file in its data folder (`~/.var/app/co.cumulas.Mail/data/keyrings/`);
the key to that file is kept by the Secret portal, in your desktop's keyring.
Its autostart entry is the same file as above, which your desktop writes for
it.

## Removing your data

Removing an account in Settings deletes its data from the database, its
password or sign-in tokens from the keyring, and its subscriptions at
Microsoft (when the app can't reach Microsoft, those expire within three
days). Attachments you opened stay in the cache folder.

To remove everything from your computer, remove your accounts first. Then
uninstall the app and delete:

- the data folder, `~/.local/share/co.cumulas.Mail/`;
- the cache folder, `~/.cache/co.cumulas.Mail/`, with the attachments you
  opened;
- `~/.config/autostart/co.cumulas.Mail.desktop`, if you started the app at
  login.

For the Flatpak, `flatpak uninstall --delete-data co.cumulas.Mail` deletes
`~/.var/app/co.cumulas.Mail/`, with the database, the cache and the keyring
file. If you started it at login, also delete
`~/.config/autostart/co.cumulas.Mail.desktop`. The Secret portal's key for the
app stays in your desktop's keyring; without the keyring file, it unlocks
nothing.

Accounts you don't remove before uninstalling leave their password or sign-in
tokens in your desktop's keyring (except in the Flatpak, where they go with
the keyring file). Their subscriptions at Microsoft expire within three days;
until then, Microsoft keeps sending notifications to the push service, which
drops them.

## What the app sends, and to whom

**To your mail providers.** The app connects only to the servers your
accounts use:

- **Microsoft accounts** (Microsoft 365, Outlook.com, Hotmail, Live): sign-in
  happens in your web browser on Microsoft's own page, and the app then uses
  Microsoft Graph to read and change your mail, calendar, contacts and tasks,
  acting as you. Microsoft's privacy statement covers what Microsoft does
  with this.
- **IMAP accounts**: the IMAP and SMTP servers you set up, with the password
  you gave. To show new mail within seconds, the app keeps a second
  connection open to your IMAP server (IMAP IDLE).

Pictures and other content from the internet in messages are blocked unless
you allow them, because loading them can tell senders when and where you
read their message.

**To the Cumulas push service, for Microsoft accounts.** Microsoft can only
tell an app about changes by sending a notification to a public web address,
which an app on your computer doesn't have. So, while the app runs:

1. The app signs in to the push service with a token Microsoft issues for it.
   That token lets the service confirm which Microsoft account it's talking to;
   it does not let it read your mail. The service never receives a token for
   your mailbox.
2. The app asks Microsoft to send notifications about changes to your Inbox
   and calendar to the push service. It asks for notifications **without any
   content**: no subjects, senders or message text.
3. Microsoft sends each notification to the push service, which passes on
   only which subscription changed and how (created, updated or deleted) to
   your app. The app then fetches what changed from Microsoft itself.

What passes through, and what's kept:

- Microsoft's notifications carry no content, but they do name the user, the
  organization and the message or event. The service reads them in memory and
  passes on, or keeps, nothing but the subscription ID and the kind of change.
- Nothing is written to disk. While your app is connected, the service holds
  in memory scrambled values (HMACs, made with a key only the service has)
  that stand for your channel, installation, user and organization, your IP
  network (to enforce its limits), and when the connection ends. When your app
  disconnects, they go. Changes that arrive while no app is connected are
  dropped; the app catches up when it connects again.
- A few lifecycle events from Microsoft (for example, that a subscription
  needs renewing) are kept in memory for up to a day, for the next app to
  connect: the channel's HMAC, the subscription ID and the event's name.
- The service's logs contain counts only (connections opened, notifications
  passed on), never a token, organization, user, address, channel or
  subscription. Its web server logs nothing about it.

**To nobody else.** No usage statistics, no crash reports and no
identifiers are sent anywhere.

## Turning the push service off

Open **Settings → General → Sync** and turn off **Microsoft push service**.
The app then deletes its notification subscriptions at Microsoft, and checks
your Inbox about every 30 seconds instead (other folders, the calendar,
contacts and tasks keep their regular sync). Turning off **Instant updates**
stops both the push service and the open IMAP connections; new mail then
arrives with the regular sync.

Removing an account also deletes its subscriptions at Microsoft.

## Running in the background

By default, closing the window keeps the app running so that new mail still
raises a notification. This only keeps the connections above open; it sends
nothing more. You can change it, and starting at login, in **Settings →
General → Background**, and quit at any time with **Quit** in the main menu or
the tray icon's menu.

## Changes and questions

If this policy changes, the new version is published here, in the app's
source repository, with the date at the top. Questions:
[open an issue](https://github.com/cumulas-inc/email-client/issues).

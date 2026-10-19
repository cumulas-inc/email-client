# Privacy policy

Cumulas Mail is a desktop app. It keeps your mail on your computer and talks
to your mail providers directly. This page says what it keeps, what it sends
where, and what passes through the services Cumulas runs: the push service at
`sync.cumulas.co`, which tells the app when the Inbox or the calendar of a
Microsoft account changes, and, for Microsoft 365 work and school accounts,
Cumulas Signatures at `signature.cumulas.co`, which gives the app your company
signature when your organization sets one up there.

Cumulas Mail has no telemetry, no analytics, no crash reporting and no
advertising, and you don't need a Cumulas account.

_Last updated: October 19, 2026._

## What the app keeps on your computer

- **A copy of your mail, calendar, contacts and tasks**, so that lists and
  search work instantly and offline. It's in a database in the app's data
  folder, `~/.local/share/co.cumulas.Mail/`, along with your settings. Older
  messages that a search of your whole mailbox finds on the server are kept
  there too, so that they open like the rest, until 14 days after a search
  last found them or you last opened them (at most 2,000 per account). For
  reminders, the same database keeps which ones went off and which you
  snoozed or dismissed (forgotten after 30 days), and, for Microsoft
  accounts, the reminder times Microsoft reports.
- **Your signatures**: the one you write for each account and, for a
  Microsoft 365 work or school account, the last answer about your company
  signature (the signature itself, that your organization's mail server adds
  it, or that there's none for you), so that it works offline. They're in the
  same database.
- **For Microsoft 365 work and school accounts**, the sensitivity labels
  Microsoft 365 lists for you once you turn them on, and what the checks of
  encryption and labels in Settings found and what each check's test
  messages showed, in the same database. After an administrator sets up
  encryption and labels from the app, it also keeps when that last changed
  the organization's rules and policies, so that checks run soon after can
  say it's too soon to tell.
- **Attachments you open**, in the app's cache folder,
  `~/.cache/co.cumulas.Mail/`, readable only by you. They're removed when the
  app next starts, and any older than a day when you open another.
- **Your recent searches** (the last 8), so the search box can suggest them, in
  the app's data folder. "Clear recent searches" in the search box's suggestions
  deletes them.
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
days). Attachments you opened stay in the cache folder until the app next
starts.

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
  acting as you. That includes reading your calendar's reminders, and
  dismissing or snoozing one when you do so in the app, so that Outlook and
  your phone stop reminding you too. Microsoft's privacy statement covers
  what Microsoft does with this.
- **IMAP accounts**: the IMAP and SMTP servers you set up, with the password
  you gave. To show new mail within seconds, the app keeps a second
  connection open to your IMAP server (IMAP IDLE).

**When an administrator approves Cumulas Mail** for their organization with
the app's approval link, Microsoft then sends their browser to
`https://flatpak.cumulas.co/admin-consent/`, a static page on GitHub Pages,
with the result in the address: that it was approved or Microsoft's error,
the organization's tenant ID and the permissions they approved. The page
reads it in the browser to show the result and sends nothing anywhere, and
its links don't pass the address on; Cumulas doesn't receive it. GitHub,
which serves the page, may log the request, as its
[privacy statement](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement)
describes.

When you search your mail, the words you type and the filters you choose (such
as dates, or unread) go to the mail servers of your accounts (Microsoft, or your
IMAP server), so that each can search your whole mailbox, and never to Cumulas.

When you check encryption and sensitivity labels in Settings (Microsoft 365
work and school accounts), the app sends a few test messages from your
account to yourself through Microsoft 365, to see what your organization's
mail server does with them. Once it has read them, it moves them to Deleted
Items. **Copy diagnostic details** copies a description of a message's
protection to your clipboard; nothing is sent.

The checks keep, with each result, what Microsoft 365 said about how it
handled the test message: its Message-ID, whether it went through your
organization's signature relay, whether Microsoft 365 counted it as from
inside your organization, and what protection and labels arrived. **Copy
details for IT** copies that to your clipboard; nothing is sent.

**When an administrator sets up encryption and labels for their
organization** (Set up or check for your organization), the app signs them
in to a separate app, Cumulas Mail - Admin Setup, and talks to Microsoft 365
directly: Exchange Online and Security & Compliance, to read and change the
organization's mail flow rules and auto-labeling policies. It also reads
which licenses the work account itself has, with the access that account
already gave. The sign-in is kept in memory only while that window is open
and is never saved; Microsoft keeps the administrator's approval of the setup
app until they remove it. The app keeps a record of those calls in memory
until the next setup sign-in or until it quits, for **Copy setup details**;
nothing goes to Cumulas.

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

**To Cumulas Signatures, for Microsoft 365 work and school accounts.**
Organizations that use Cumulas Signatures set up their people's email
signatures there. The app finds out whether yours has one for you, with
nothing to set up:

1. The app asks Microsoft, with your sign-in, for a token for Cumulas
   Signatures, as it does for Microsoft Graph. Microsoft gives Cumulas Mail
   one only in organizations that have added the Cumulas Signatures app in
   Microsoft 365. That includes organizations that use it now, used it before
   and kept the app, or where someone signed in to the service. In other
   organizations Microsoft refuses and nothing is sent to Cumulas Signatures.
2. If Microsoft gives the token, the app sends it to `signature.cumulas.co`
   to get your signature. The token names you and your organization:
   Microsoft puts your account and your organization in it (and may add your
   name and sign-in address), so that the service can find the signature your
   organization set up for you. It doesn't let the service read your mail. No
   mail, contacts or recipients are sent. The service's request log records
   the IP address and the path of each request, never the token or an
   address.
3. The service answers with your signature and whether it goes on replies and
   forwards too, or that your organization's mail server adds it when you
   send (the app then adds none, so your mail never carries two), or that
   there's none for you right now.

The app keeps the answer on your computer. When it asks again depends on
that answer:

- **While your organization signs your mail**, and before its first answer:
  about 15 seconds after the app starts, every 6 hours, and in the
  background when you start a message more than an hour after the last
  answer.
- **While it doesn't** (your organization hasn't added the Cumulas
  Signatures app, or the service has no signature for you) **or hasn't
  allowed Cumulas Mail**: once a day, and not each time the app starts or
  you start a message.
- **After a check that failed** (no network, say): 30 minutes later (or when
  the service asks), when the network comes back, and when the app next
  starts.
- **Whatever the answer**: right after you sign in.
- **When you open Settings → Signatures**: if the last answer is more than
  10 minutes old or the last check failed, unless that answer was that your
  organization hasn't added the Cumulas Signatures app, or that the service
  has no signature for you: your organization is then asked again only once
  a day and when you sign in.

Pictures in a company signature, such as a logo, load from where your
organization keeps them (usually `signature.cumulas.co`) when the app shows
the signature. Your organization decides what your signature says, and
whether Cumulas Mail may fetch it. Personal Microsoft accounts (Outlook.com,
Hotmail, Live) and IMAP accounts never take part.

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

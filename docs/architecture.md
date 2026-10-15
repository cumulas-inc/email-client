# Architecture

Cumulas Mail is a Tauri 2 desktop app: a Rust backend that talks to mail
servers and keeps a local cache, and a React/TypeScript UI rendered in the
system webview (WebKitGTK on Linux).

```
┌──────────────────────────── UI (src/) ────────────────────────────┐
│  modules/mail · calendar · people · tasks · settings · onboarding  │
│        │ React Query (api/queries.ts)    ▲ "data-changed" /        │
│        ▼                                 │ "sync-status" events    │
│  api/backend.ts  ── Backend interface ───┤                         │
│     ├─ api/tauri.ts   (desktop: invoke Rust commands)              │
│     └─ api/mock/      (browser dev, tests, screenshots)            │
└────────┬───────────────────────────────────────────────────────────┘
         │ Tauri commands (src-tauri/src/commands.rs)
         │ src-tauri/src/desktop: notifications, tray, background
┌────────▼──────────────── crates/core ─────────────────────────────┐
│  service/   CoreService: every UI operation, sync engine           │
│  push/      IMAP IDLE/NOTIFY watchers, the push service client     │
│  reminders/ which reminders are due (a pure planner)               │
│  signatures/ company signatures (Cumulas Signatures client)        │
│  store/     SQLite cache (+ FTS5 search)                           │
│  providers/ MailProvider · CalendarProvider · ContactsProvider ·   │
│             TasksProvider                                          │
│     ├─ graph/  Microsoft 365 & Outlook.com (Microsoft Graph)       │
│     └─ imap/   IMAP + SMTP                                         │
│  auth/      Microsoft OAuth (PKCE, loopback redirect)              │
│  secrets.rs Secret Service / Flatpak secret portal (oo7)           │
│  html.rs    sanitizing email HTML and signatures                   │
└────────────────────────────────────────────────────────────────────┘
```

## Principles

- **The UI reads only from the local cache.** Lists and searches are instant
  and work offline. The sync engine (`service/sync.rs`) refreshes the cache in
  the background and emits `data-changed` events; the UI invalidates the
  matching React Query keys (every key starts with a `DataKind`).
- **Writes go to the server first**, then to the cache, so the cache never
  shows something the server rejected. The UI may update optimistically.
- **One provider trait per domain.** Microsoft accounts implement all four
  traits on Microsoft Graph. IMAP accounts implement mail only; their
  calendar, contacts and tasks are stored locally (CalDAV/CardDAV can be added
  later as more providers).
- **Local ids.** Every entity has a stable local id derived from
  `(account, kind, remote id)` (`model::local_id`). The UI only uses local ids.
  Graph requests use immutable ids (`Prefer: IdType="ImmutableId"`) so messages
  keep their id when moved.
- **Errors are typed.** Rust `Error { code, message, details }` is serialized
  to the UI as-is; the UI branches on `code` (e.g. `adminConsentRequired`).
- **The core does not depend on Tauri** and is tested headless with fakes
  (`service/tests.rs`) and mock HTTP servers (wiremock).

## Push, notifications and running in the background

New mail reaches the app within seconds, and the app keeps running when its
window closes so that it can say so. See [PRIVACY.md](../PRIVACY.md) for what
this sends where.

**In the core** (`crates/core/src/push`, `service/background.rs`):

- `CoreService::start_background` starts the periodic full sync (every
  `sync_interval_minutes`), a push supervisor per account and a resume
  detector (a jump between the wall clock and the monotonic clock means the
  computer slept: everything reconnects and catches up).
- IMAP accounts keep a second connection in IDLE on the Inbox (with NOTIFY
  for the other folders where the server offers it); servers without IDLE are
  polled every 2 minutes. Microsoft accounts follow a stream from the Cumulas
  push service (`push/relay.rs`, `push/graph.rs`), which passes on Microsoft
  Graph's change notifications for subscriptions the app keeps on the Inbox
  and the calendar; without it the Inbox is polled every 30 seconds. Each
  account's state is a `PushStatus` (mode, health, detail), sent as
  `push-status` events.
- Changes feed a per-account queue (`service/targeted.rs`) that syncs just
  the Inbox or the folder that changed. What it inserts goes through the alert
  policy (`service/alerts.rs`: Inbox only, not paused or muted, unread, recent),
  and the sink's `new_mail` gets one `MailAlert` per sync. Messages read,
  moved or deleted, here or elsewhere, come back as `mail_settled`.

**In the shell** (`src-tauri/src/desktop`):

- `notify/`: the Notification portal inside a Flatpak, `org.freedesktop.Notifications`
  otherwise. Toast ids are the app's (`mail:{message}`, `mail-summary:{account}`),
  so notifications are replaced and withdrawn (on `mail_settled`). Each has a
  click action (open the message) and buttons (Mark as read, Archive). No
  notification is urgent, so Do Not Disturb holds them all; none is shown
  while the main window is focused and visible, and messages read meanwhile
  are left out.
- `router.rs`: a click opens the window (with the activation token, for focus
  on Wayland) and sends the page a `navigate` event; when the window was just
  created for it, the page takes it with `take_pending_navigation` once it
  listens. Mark as read and Archive run without the window.
- `window.rs`, `lib.rs`: one instance per user (tauri-plugin-single-instance,
  registered first); `--background` (start at login) creates no window;
  closing the window hides it while running in the background is on and
  allowed (the first time, a notification says so); Quit in the main menu
  (`quit_app`) or the tray's menu exits, shutting push down. Ctrl+Q is Mail's
  Mark as read, as in Outlook.
- `tray.rs`: a tray icon only where the libayatana-appindicator library loads
  and a StatusNotifier host owns `org.kde.StatusNotifierWatcher` (followed as
  it comes and goes). Its menu opens the app, starts a message, checks mail,
  pauses mail notifications (reminders still show) and quits; its icon and
  first line show unread mail.
- `background.rs`: inside a Flatpak, the Background portal allows running
  without a window and starting at login (a refusal makes closing the window
  quit) and shows a status; a native install writes
  `~/.config/autostart/co.cumulas.Mail.desktop`. `get_background_state` and
  the `background-state` event tell Settings.
- `network.rs`: the NetworkMonitor portal tells the core when the network
  comes and goes (push pauses while offline).

## Reminders

Events and tasks remind, also while the window is closed (see above).

**In the core** (`crates/core/src/reminders`, `service/reminders.rs`):

- **Keys.** A reminder is known by `{event|task}:{local id}:{fire time}`.
  Moving an event (or changing a task's reminder) makes a new key, which goes
  off afresh. What became of each one (shown, missed, snoozed until a time,
  dismissed) is kept in `reminder_state`; rows of items that are gone, and
  rows more than 30 days old, are pruned on each sync.
- **The planner** (`reminders::plan`) is a pure function of the sources,
  those states, the time and the time zone:
  - Sources are events starting from a day ago to 15 days ahead, minus
    cancelled and declined ones, and open tasks with a reminder.
  - Timed events go off their reminder time before the start. All-day events
    go off before local midnight of their first day, worked out on every pass
    so daylight saving time and a computer in another time zone count.
  - It says which reminders are active (due, or snoozed), which just came
    due, which are no longer due (dismissed, snoozed, completed, moved,
    deleted) and when to look again.
  - A reminder comes due once. One missed while the app wasn't running or
    the computer slept goes off when it next looks, for up to 24 hours
    (`reminders_checked_until`, in `app_settings`, is when it last looked).
    An event already over is listed as missed rather than shown, and only
    when its reminder went off since that last look: an event that was over
    before the app heard of it isn't worth a reminder.
- **The scheduler** runs it whenever events, tasks or the reminder settings
  change, on wake, and at the next reminder time, sleeping a minute at most
  (timers stand still while the computer sleeps). Each pass saves what came
  due and tells the sink: `reminders_changed(active, newly_due, cleared)`.
  `reminders_enabled` off clears them all.
- **Microsoft 365** keeps reminders itself. Each sync of an account's
  calendars reads `/me/reminderView` for the same window into
  `server_reminders`; its `reminderFireTime` wins over the event's own
  reminder time (a reminder snoozed in Outlook goes off when the snooze
  ends), and an event it leaves out has no reminder (it was dismissed). A
  report that couldn't be read keeps the last one, for a day; then the
  events' own times count. Occurrences of a series are matched by id, or by
  start, end and subject.
- **Dismiss and snooze** (`dismiss_reminders`, `snooze_reminder`) save the
  state at once. For a Microsoft 365 event they also post `dismissReminder`
  or `snoozeReminder` (`newReminderTime` in UTC); what fails stays pending in
  `reminder_state` and is retried at the next sync of the calendar.
- **Tasks**: `reminder_at` maps To Do's `isReminderOn` and `reminderDateTime`
  (written in UTC); IMAP accounts' local tasks keep it with their other data.

**In the shell**: `reminders_changed` withdraws the notifications of
reminders no longer due and shows the new ones, `reminder:{key}`, at high
priority on the portal (never urgent), with the reminder sound when sounds
are on. Unlike mail they show while the window is in front, as they're about
now. Buttons: Snooze (for `default_snooze_minutes`), Dismiss, and Join for an
https meeting address (a task's: Mark complete); GNOME shows at most three.
More than three at once (after sleep, say) show as one notification that
opens the Reminders panel. A click opens the event in Calendar or the task in
Tasks (`navigate` with kind `event`, `task` or `reminders`). The page gets
`reminders-changed` for the bell in the header and its Reminders panel, and
reads out reminders as they come due (a live region).

## Signatures

Every account has a signature of its own, and Microsoft 365 work and school
accounts can have their organization's, from Cumulas Signatures (see
[microsoft-365.md](microsoft-365.md#company-signatures-cumulas-signatures)).

**In the core** (`crates/core/src/signatures`, `service/signatures.rs`):

- **Configuration** (`signatures/config.rs`): the service's address
  (`CUMULAS_SIGNATURE_URL`) and its shared app's client ID
  (`CUMULAS_SIGNATURE_CLIENT_ID`), from the environment or the build. The API
  scope is made from the two, never taken from an answer. Without a client ID
  the feature is off.
- **The check** (`OrganizationSignatures`, made by the `Connector` for work
  and school accounts only): a token for the API from the account's refresh
  token (`auth::microsoft::ApiTokenSource`, whose refusals never become
  account errors), then `GET /apps/signature` (`signatures/client.rs`), once
  more with a new token after a 401. The outcome is an answer (signature,
  added on send, not used, not allowed), "unchanged" (304), or a failure to
  try again later.
- **State** (`account_signatures`, one row per account): the own signature,
  and the organization's last answer with its ETag, when it came, when the
  app last asked, whether that failed (`stale`) and the service's
  `Retry-After`. Only answers change it; a failure only marks it stale. Each
  answer is kept for the scope it came for, so a build pointed at another
  service starts afresh. Both signatures are sanitized before they're kept
  (`html::sanitize_signature_html`).
- **The scheduler** asks about 15 seconds after start, then every 6 hours
  while the organization signs the mail, every 24 hours while it doesn't, 30
  minutes after a failure (or at the `Retry-After`), at once after a sign-in
  and when the network is back, by the wall clock and sleeping a minute at
  most. One check runs per account at a time, at most one a minute.
- **Compose** (`get_compose_signature`): the organization's signature (on
  replies and forwards only if it says so), nothing when its server adds it
  on send, else the account's own (on replies only if the person said so). An
  account never checked waits up to 4 seconds for its first answer; an
  answer over an hour old is refreshed in the background.
- Changes reach the UI as `data-changed` with `signatures`.

**In the UI**, the signature goes in the message as ordinary content that can
be changed, in a `cumulas-signature` block above the quoted original (in
`cumulas-quote`), as Outlook does.

## Security

- Email HTML is sanitized in Rust (`html.rs`: no scripts, event handlers,
  forms, frames or `javascript:`/`data:` links), then rendered in an
  `<iframe sandbox>` **without** `allow-scripts`, with a Content Security Policy
  that blocks remote images and styles unless the user allows them (tracking
  pixels are blocked by default).
- Signatures, which go into messages the person sends, are sanitized more
  strictly: pictures and CSS backgrounds only over HTTPS or as `data:` images,
  links only to `http`, `https`, `mailto` and `tel`, and no style block that
  imports or loads from elsewhere.
- Passwords and refresh tokens are kept in the Secret Service through `oo7`
  (inside Flatpak, in `oo7`'s encrypted keyring file in the app's data folder,
  keyed by the Secret portal), never in SQLite.
- Plain-text IMAP/SMTP connections are refused except to `localhost`
  (e.g. Proton Mail Bridge).
- Microsoft sign-in uses the system browser (authorization code + PKCE with a
  `http://localhost` loopback redirect), so passwords and MFA never pass
  through the app.

## Microsoft 365 permissions

The app requests only delegated permissions that users can grant themselves:
`User.Read`, `offline_access`, `Mail.ReadWrite`, `Mail.Send`,
`MailboxSettings.Read`, `Calendars.ReadWrite`, `Contacts.ReadWrite`,
`Tasks.ReadWrite`. Whether a user may consent is the organization's policy;
when it is blocked the app shows an admin-consent link to forward to IT. See
[microsoft-365.md](microsoft-365.md).

## Data on disk

| What               | Where                                                                                                    |
| ------------------ | -------------------------------------------------------------------------------------------------------- |
| Cache database     | `$XDG_DATA_HOME/co.cumulas.Mail/mail.db`                                                                 |
| Opened attachments | `$XDG_CACHE_HOME/co.cumulas.Mail/attachments/`                                                           |
| Passwords, tokens  | Secret Service (GNOME Keyring, KWallet); Flatpak: `$XDG_DATA_HOME/keyrings/`, keyed by the Secret portal |
| Start at login     | `~/.config/autostart/co.cumulas.Mail.desktop` (Flatpak: written by the Background portal)                |

## Adding a provider

1. Implement the relevant traits from `crates/core/src/providers/mod.rs`,
   deriving local ids with `model::local_id`.
2. Return a `ProviderSet` for the new account kind from
   `service/connector.rs`.
3. Add the account kind to `model::AccountSettings` and to `src/api/types.ts`.

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
│  store/     SQLite cache (+ FTS5 search)                           │
│  providers/ MailProvider · CalendarProvider · ContactsProvider ·   │
│             TasksProvider                                          │
│     ├─ graph/  Microsoft 365 & Outlook.com (Microsoft Graph)       │
│     └─ imap/   IMAP + SMTP                                         │
│  auth/      Microsoft OAuth (PKCE, loopback redirect)              │
│  secrets.rs Secret Service / Flatpak secret portal (oo7)           │
│  html.rs    sanitizing untrusted email HTML                        │
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
  pauses notifications and quits; its icon and first line show unread mail.
- `background.rs`: inside a Flatpak, the Background portal allows running
  without a window and starting at login (a refusal makes closing the window
  quit) and shows a status; a native install writes
  `~/.config/autostart/co.cumulas.Mail.desktop`. `get_background_state` and
  the `background-state` event tell Settings.
- `network.rs`: the NetworkMonitor portal tells the core when the network
  comes and goes (push pauses while offline).

## Security

- Email HTML is sanitized in Rust (`html.rs`: no scripts, event handlers,
  forms, frames or `javascript:`/`data:` links), then rendered in an
  `<iframe sandbox>` **without** `allow-scripts`, with a Content Security Policy
  that blocks remote images and styles unless the user allows them (tracking
  pixels are blocked by default).
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

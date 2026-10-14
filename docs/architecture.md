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
┌────────▼──────────────── crates/core ─────────────────────────────┐
│  service/   CoreService: every UI operation, sync engine           │
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

## Security

- Email HTML is sanitized in Rust (`html.rs`: no scripts, event handlers,
  forms, frames or `javascript:`/`data:` links), then rendered in an
  `<iframe sandbox>` **without** `allow-scripts`, with a Content Security Policy
  that blocks remote images and styles unless the user allows them (tracking
  pixels are blocked by default).
- Passwords and refresh tokens are kept in the Secret Service through `oo7`
  (the XDG secret portal inside Flatpak), never in SQLite.
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

| What               | Where                                                            |
| ------------------ | ---------------------------------------------------------------- |
| Cache database     | `$XDG_DATA_HOME/co.cumulas.Mail/mail.db`                         |
| Opened attachments | `$XDG_CACHE_HOME/co.cumulas.Mail/attachments/`                   |
| Passwords, tokens  | Secret Service (GNOME Keyring, KWallet) or Flatpak secret portal |

## Adding a provider

1. Implement the relevant traits from `crates/core/src/providers/mod.rs`,
   deriving local ids with `model::local_id`.
2. Return a `ProviderSet` for the new account kind from
   `service/connector.rs`.
3. Add the account kind to `model::AccountSettings` and to `src/api/types.ts`.

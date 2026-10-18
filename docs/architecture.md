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
│  store/     SQLite cache (+ FTS5 full-text indexes)                │
│  search/    the search language and ranking, for FTS5, Graph, IMAP │
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
  matching React Query keys (every key starts with a `DataKind`). The one
  call that asks a server is searching the whole mailbox
  ([Search](#search)), and even that stores what it finds in the cache, which
  the UI then reads like any other message.
- **Store reads stay off the main thread.** Commands that read the store are
  `async` and do it on the blocking pool (`blocking` in
  `src-tauri/src/commands.rs`): a sync holding the store's lock makes them
  wait, never the window.
- **Writes go to the server first**, then to the cache, so the cache never
  shows something the server rejected. The UI may update optimistically.
- **One provider trait per domain.** Microsoft accounts implement all four
  traits on Microsoft Graph. IMAP accounts implement mail only; their
  calendar, contacts and tasks are stored locally (CalDAV/CardDAV can be added
  later as more providers).
- **Local ids.** Every entity has a stable local id derived from
  `(account, kind, remote id)` (`model::local_id`). The UI only uses local ids.
  Graph requests use immutable ids (`Prefer: IdType="ImmutableId"`) so messages
  keep their id when moved. Graph's delta reports a move out of a folder as a
  removal, as it does a deletion, so the provider looks removals up (60 per
  sync at most) and reports the moved ones in `MessageBatch::moved_out`: the
  row moves with its fetched body. A removal only deletes a row still in the
  folder that reported it.
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
  Mark as read, as in Outlook. The tray's Quit asks the page
  (`quit-requested`), which asks first about an unsent message; its Restart
  to update asks the page the same way (`restart-requested`).
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
- `update.rs`: in a Flatpak, the Flatpak portal's update monitor says when a
  version other than the running one is installed; `get_update_state` and the
  `update-state` event tell the page, which shows a card once per version, a
  main menu item and a row in About, and the tray adds Restart to update. A
  desktop notification says it once per version while the window isn't open.
  `restart_app` starts the installed version through the portal (`Spawn` with
  `LatestVersion`, `--restarted-from=<bus name>`) and hands over through the
  single-instance name: the new instance checks its runtime against
  `/app/manifest.json` and opens the display, answers `--restart-ready` (or
  `--restart-refused`) through the single-instance callback, and waits for
  the old one to shut push down, release the name and exit. An early exit
  (`SpawnExited`), a refusal or 30 seconds without an answer leave the old one
  running, and Restart can be tried again.

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
  (`CUMULAS_SIGNATURE_CLIENT_ID`), from the environment, else the build, else
  Cumulas Signatures (`signature.cumulas.co`) and its published shared app.
  That client ID goes with that address only, since the scope names the
  host: a service elsewhere needs its own, and without one the feature is
  off, as it is when the client ID is `none`. The API scope is made from the
  two, never taken from an answer.
- **The check** (`OrganizationSignatures`, made by the `Connector` for work
  and school accounts only): a token for the API from the account's refresh
  token (`auth::microsoft::ApiTokenSource`, whose refusals never become
  account errors), then `GET /apps/signature` (`signatures/client.rs`), once
  more with a new token after a 401. Microsoft gives the token only in
  organizations that have added the service's app; elsewhere its refusal
  counts as "not used" and nothing reaches the service. The outcome is an
  answer (signature, added on send, each with whether replies are signed;
  not used; not allowed), "unchanged" (304), or a failure to try again
  later: anything else, including a 404 without the service's `not-managed`
  or `not-set-up` and an answer whose `version` is newer than 1.
- **State** (`account_signatures`, one row per account): the own signature,
  and the organization's last answer with its ETag, when it came, when the
  app last asked, whether that failed (`stale`) and the service's
  `Retry-After`. Only answers change it; a failure only marks it stale. Each
  answer is kept for the scope it came for, so a build pointed at another
  service starts afresh. Both signatures are sanitized before they're kept
  (`html::sanitize_signature_html`) and again whenever they're read
  (`sanitize_stored_signature_html`), so a stricter sanitizer applies at
  once. The kept ETag starts with the sanitizer's version
  (`html::SIGNATURE_SANITIZER_VERSION`, as `s<version>:`), and a check
  sends only one this sanitizer kept: an answer kept by another comes again
  in full.
- **The scheduler** asks about 15 seconds after start while the
  organization signs the mail, before its first answer and after a failure;
  then every 6 hours while it signs the mail, every 24 hours (by the last
  answer, also across restarts, never at start) while it doesn't or hasn't
  allowed the app, 30 minutes after a failure (or at the `Retry-After`), at
  once after a sign-in whatever was said and when the network is back, by
  the wall clock and sleeping a minute at most. One check runs per account
  at a time, at most one a minute; what the UI gets says when asking now
  would next reach the service (`checkableAt`). A removed account is
  remembered, so no check starts for it again, even from a list of
  accounts taken before.
- **Compose** (`get_compose_signature`): the organization's signature (on
  replies and forwards only if it says so); nothing when its server adds it
  on send, or on a reply or forward that the server leaves unsigned; else
  the account's own (on replies only if the person said so). An account
  never checked waits up to 4 seconds for its first answer; while the
  organization signs the mail, an answer over an hour old is refreshed in
  the background.
- Changes reach the UI as `data-changed` with `signatures`.

**In the UI**, the signature goes in the message as ordinary content that can
be changed, as Outlook does. The sent HTML is up to three blocks side by
side: the letter, in the letter font; the signature, in a `cumulas-signature`
block (an organization's in the letter font without its line height, so its
tables keep their layout); and, in a reply or forward, the original in a
`cumulas-quote` block. Only an organization's signature block carries
`data-signature-stamp`, which tells the service's relay the message is signed
already, so a kept company signature and the relay never make two. A relay
that signs on send puts its signature just before `cumulas-quote`, outside
the letter's block.

## Search

The search box looks in two places at once: the mail on this device, at once
and offline, and each account's whole mailbox on its server, whose finds come
in after.

**The query** (`crates/core/src/search`): one parser for words, `"phrases"`,
`from:`, `to:`, `cc:`, `subject:`, `body:` and `has:attachment`, and the
filters `is:unread`, `is:read`, `is:flagged`, `in:<folder>`,
`account:<account>`, `received:`, `after:` and `before:`. Anything else is a
word, a filter with a value it doesn't know is left out, and no input is an
error (at most 16 terms, and 8 dates, folders and accounts). Dates
(`dates.rs`) are a day (`2026-10-01`), a range for `received:`
(`2026-10-01..2026-10-15`, open at either end) or one of KQL's words
(`today`, `"this week"`, `"last month"`…), as days in the computer's time zone,
with weeks starting on the day the UI's locale says; `after:` includes its
day, `before:` doesn't, and clauses intersect. `in:` and `account:` values are
kept as scope keys (`scope.rs`: composed, without quotes, backslashes or
control characters, whitespace runs as one space, lowercased), and folder and
account names are keyed alike before they're compared, so what the filters
write always names its folder again; `in:` also takes a role (`inbox`, `sent`,
`spam`…). Where to look alone, or `is:read`, is too little to ask the servers
for. It becomes:

- for the device (`fts.rs`), FTS5 matches on two indexes keyed by
  `messages.fts_rowid`, a rowid each message row keeps, so indexing or
  removing a message is a rowid lookup, never a scan; a trigger on `messages`
  removes a message's index rows, also on cascades. `messages_fts` has the
  subject, sender, To, Cc and body (a message's text once it's been opened, its
  preview before) and matches words by their start; `messages_infix`, a
  trigram index of subjects and senders, matches inside words from 3
  characters ("voice" finds "Invoice"). The filters are conditions on the
  message rows;
- for Microsoft Graph (`kql.rs`), KQL for `$search` when there are words:
  words as prefixes (`invoi*`), matched in the sender, subject and body, and
  the dates as `received>=MM/DD/YYYY` and `received<=MM/DD/YYYY`, a day wider
  on each side (KQL compares dates in UTC). Without words, a `$filter` on
  `receivedDateTime`, `isRead`, `flag/flagStatus` and `hasAttachments`;
- for IMAP (`imap.rs`), `UID SEARCH` keys (`FROM`, `SUBJECT`…, which match
  substrings; a plain word is `OR`ed over sender, To, Cc, subject and body,
  where the device looks, not `TEXT`, which looks in every header too), with
  `CHARSET UTF-8` and literals only when a word isn't ASCII, in the case it
  was typed in; `FLAGGED`, `SEEN` and `UNSEEN`; and `SINCE` and `BEFORE`,
  which compare the day of `INTERNALDATE` in its own zone (anywhere from
  UTC−12 to UTC+14), so they are widened to hold for any.

The UI ports the parser to `src/lib/search/query.ts`, tested on the same
cases, and the browser demo's backend (`src/api/mock/searchQuery.ts`) adds
the local matching, so the UI's tests search the way the app does.

**On this device** (`Store::search`), under one hold of the store: the
messages that match and pass the filters, newest first, 5,000 at most, plus
the server hits the UI passes back (`includeIds`), which the filters apply to
too. A query with a plain, `subject:` or `body:` word is then ranked best
first (`rank.rs`; `from:`, `to:` and `cc:` alone only pick by address, and
list newest first, as "Newest" does):

- a term found in the subject or sender (one FTS5 probe per word,
  `{subject sender} : …`) counts 1, one found only elsewhere 0.3; a message's
  field share is the mean over its ranked terms;
- bm25 (columns weighted 10, 8, 3, 2 and 1) decides the last quarter of that,
  relative to the best match: relevance is `field × (0.75 + 0.25 × bm25/best)`;
  a match only inside words gets 0.6 (whether or not a server found it
  too), a server hit the words don't match here (it matched text the device
  hasn't seen) 0.225;
- recency decides all but 0.4 of the score, halving every 30 days.

So a match in the subject or sender (at least `0.75 × 0.4 = 0.3`) ranks above
one only in the text (at most 0.3) at any age, and within each, recent strong
matches lead. The first 200 come back, with the server hits, and the first 100
with a snippet of the text where it matched (FTS5's `snippet()`, cut with
private-use marks that indexing keeps out of mail, returned as runs of text
the UI never parses). Every statement that matches `messages_fts` and limits
rowids writes `+rowid IN (…)`: as `rowid IN (…)`, FTS5 would run the match
again for every rowid (4 s for 200 of 5,000 rows, against 19 ms), and a test
checks the plans. Since the match runs over every row first, none runs
without rows to keep. At 50,000 messages, two letters that match nearly every one
rank in about 0.3 s (an ignored test measures it; a prefix index is the next
step if that grows).

**On the server** (`MailProvider::search`, `service/search.rs`):
`search_server` asks one account for pages of 50. Graph searches every folder
with `GET /me/messages?$search=` (which takes no `$filter` or `$orderby`), or
one folder's messages when `in:` names exactly one there, and pages by its
own next links; filters alone go to `$filter` with
`$orderby=receivedDateTime desc` (`receivedDateTime` first, as Graph needs),
which finds old unread or flagged mail `$search` can't say. A mailbox that
answers a search with dates with 400 is asked again without them, and when
that works, the provider leaves dates to the device from then on (until the
app restarts). Servers are told dates from 1970 through 9998 only (IMAP's
must have four-digit years), a bound beyond dropped or moved out, so they
find more, never less.
IMAP asks about 25 folders per round trip (or the ones `in:` names),
pipelined (`EXAMINE` and `UID SEARCH` for each, then the arrival dates of
their newest 200 matches), and pages through what it found by date; a page
spends 20 seconds at most looking (less when the call has less left, keeping
10 for its summaries), and the cursor keeps the folders it
didn't get to, and the older matches of busier folders (`UID 1:n`), for the
pages after. Servers can't say every filter (Graph has no read or flagged
with words, nor several folders; IMAP's dates are in each server's zones), so
each message they find is checked against the query's filters, exactly,
before it is stored; since that can empty a page, one call reads pages until
25 messages pass, the server has no more, it read 5, or less than 30 seconds
are left (a page cut off would be lost, and an IMAP session with it). A search for another
account (`account:`), for no folder there or for no time asks nothing. A
search id names the search over every account: a new one cancels the one
before, whose pending calls come back `cancelled` and store nothing. A call
may take 45 seconds, all its pages included; a page that fails after others
were read keeps what they found and is asked for again by "Show more
results". An account's failure comes back in its result, never as an error,
so it doesn't hide the others' results or the device's.

**What it finds is stored**, as rows marked `search_only`, and returned as
local ids. The UI passes them to the local search (`includeIds`), so every
result is a stored message in its folder, which opens, moves and flags like
any other, and shows in its folder's list too. The sync window's pruning
(Graph's full resync, IMAP's `retain_only`) leaves these rows alone, but for
IMAP UIDs the incremental sync lists (from the window's oldest up, where new
mail a search found first sits); a removal the server reports deletes them,
and so does opening one the server no longer has. A page stored after a change made here while the server answered (a
deletion, a move, a flag) leaves that message as it is now. A sync that sees
one takes it over. Otherwise they go 14 days after a
search last found them or they were last opened, and beyond 2,000 per account
(after each sync of the account, even one that fails). Read and flag changes made elsewhere reach them when a
search next finds them.

**In the UI** (`modules/mail/search/`, `modules/mail/serverSearch.ts`,
`MessageList.tsx`): the search text is the only state. The filters above the
results (From, Date, Has attachment, Unread, Flagged, Folder, Account) are a
view of the parsed text and edit it, so typed and clicked filters never
disagree, and they show only while there is text; with an empty box, the
suggestions offer the same filters. While searching, the folder's
All/Unread/Flagged control is gone. Results are best first or newest first
("Sort results", kept per device); best first keeps the rows already shown in
their places while servers answer, and new ones join below them, best first,
until the text or the sort changes. The search box is a combobox: suggestions
offer recent searches, people (`suggestAddresses`, which without text gives
the people who wrote most lately), filters, folders, accounts and dates for
the operator being typed. Recent searches (the last 8) are kept in
localStorage, on this device only. The local search runs 150 ms after typing
stops, the servers are asked after 600 ms. The server search's state is kept
out of React Query, so the `data-changed` of every sync doesn't ask the
servers again; the local search's key carries how many server hits it
includes. "Searching your whole mailbox…" shows beside the count while servers
answer (never "No results"; "No results so far" while a server has more to
look through), a failed account gets a row above the results with Try again
(Sign in again when its sign-in failed), and "Show more results" asks for the
next pages, up to the 1,000 hits a local search includes. The words found are
highlighted in the rows (from the parsed text, for server hits too, and in
the snippet) and in the open message: in plain text by the app, in HTML by
the frame's bridge, which marks visible text by splitting text nodes; F3 and
Shift+F3 step through them.

## Security

- Email HTML is sanitized in Rust (`html.rs`: no scripts, event handlers,
  forms, frames or `javascript:`/`data:` links), then rendered in an
  `<iframe sandbox="allow-scripts">` **without** `allow-same-origin`: an opaque
  origin, which can't reach the app or its IPC (Tauri's scripts go into the
  main frame only). Its Content Security Policy blocks remote images and styles
  unless the user allows them (tracking pixels are blocked by default) and runs
  one script, by its exact URL: the app's bridge (`public/message-bridge.js`),
  right after the charset and policy tags, before any of the mail. WebKitGTK
  calls no event listener in a frame that may not run scripts, and the app
  can't reach into an opaque one, so the bridge reports what happens inside by
  `postMessage`: links clicked (opened through the app, in the browser or a
  new message) and pointed at (the reader shows where they really go), keys
  pressed (the mail shortcuts), and the document's size. Only real input is
  reported, and the app takes a message only from its own frame, carrying the
  secret channel that frame's document was given. Should a script in the mail
  run anyway, the bridge limits what it can change: its listeners are called
  first, it reads events and links through getters taken before the mail is
  parsed, and a link clicked with the pointer opens only if it still leads
  where it did when the pointer went down on it. To highlight search words,
  the bridge accepts only `highlight` and `reveal` from the app, trusted, from
  the parent, carrying a second token the frame never sends (so reading the
  frame's messages can't forge the app's), keeps them from other listeners in
  the frame, and marks text by splitting text nodes, never parsing markup.
- Signatures, which go into messages the person sends, are sanitized more
  strictly: pictures and CSS backgrounds only over HTTPS or as `data:` images,
  links only to `http`, `https`, `mailto` and `tel`, and no CSS that imports,
  runs code, or loads from elsewhere (`image-set()` included). Kept
  signatures are sanitized again whenever they're read.
- Passwords and refresh tokens are kept in the Secret Service through `oo7`
  (inside Flatpak, in `oo7`'s encrypted keyring file in the app's data folder,
  keyed by the Secret portal), never in SQLite.
- Plain-text IMAP/SMTP connections are refused except to `localhost`
  (e.g. Proton Mail Bridge).
- Microsoft sign-in uses the system browser (authorization code + PKCE with a
  `http://localhost` loopback redirect), so passwords and MFA never pass
  through the app.

## Microsoft 365 permissions

The app requests only delegated permissions: `User.Read`, `offline_access`,
`Mail.ReadWrite` and `Mail.Send` for mail (`CORE_SCOPES` in
`auth/microsoft.rs`), and one optional permission per feature
(`OPTIONAL_SCOPES`): `MailboxSettings.Read`, `Calendars.ReadWrite`,
`Contacts.ReadWrite`, `Tasks.ReadWrite`. Whether people may grant them is the
organization's policy, and under Microsoft's default since late 2025 an
administrator usually has to approve the app first; the app then shows an
approval link to forward to IT, and can sign in with mail only. What each
account was granted is recorded (`accounts.microsoft_grant`). A feature
without its permission, or one Microsoft 365 refuses, gets a provider that
fails with `permissionRequired`, and its cached data is dropped. See
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
   deriving local ids with `model::local_id`. `MailProvider::search` is
   optional: without it, an account's server search fails with a message,
   and its local search still works. It may return more than the query's
   filters let through (filters the server can't say): the service checks
   each message before storing it.
2. Return a `ProviderSet` for the new account kind from
   `service/connector.rs`.
3. Add the account kind to `model::AccountSettings` and to `src/api/types.ts`.

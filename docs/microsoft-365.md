# Microsoft 365 and Outlook.com

Cumulas Mail connects to Microsoft accounts through **Microsoft Graph** using
OAuth 2.0 in the system browser (authorization code flow with PKCE and a
`http://localhost` redirect). Passwords and MFA prompts never pass through the
app; it only stores a refresh token, in the system keyring.

Exchange ActiveSync and EWS are not used. ActiveSync would not avoid any of
the consent requirements below (it also needs an Entra app registration since
basic authentication was switched off in Exchange Online), and Microsoft is
retiring EWS in Exchange Online.

## Permissions

All permissions are **delegated**: the app acts as the signed-in user, never
on behalf of the whole organization, and reaches only that user's own data.

**Needed for mail.** An account can't be added without these
(`CORE_SCOPES` in `crates/core/src/auth/microsoft.rs`):

| Permission                                | Used for                                               |
| ----------------------------------------- | ------------------------------------------------------ |
| `openid`, `profile`, `email`, `User.Read` | Signing in and showing your name and address           |
| `offline_access`                          | Staying signed in (refresh token)                      |
| `Mail.ReadWrite`                          | Reading, searching, flagging, moving and deleting mail |
| `Mail.Send`                               | Sending mail                                           |

**Optional, one per feature** (`OPTIONAL_SCOPES`). The account works without
them. A feature whose permission is missing "needs permission": Settings →
Accounts and the Calendar, People and Tasks modules say so, with **Turn on**
(asks Microsoft for that permission) and **Ask IT** (the approval link and an
email for IT).

| Permission             | Used for                                           | Without it                                                          |
| ---------------------- | -------------------------------------------------- | ------------------------------------------------------------------- |
| `MailboxSettings.Read` | Time zone and mailbox preferences                  | All-day events and due dates use the computer's time zone           |
| `Calendars.ReadWrite`  | Calendar views, creating events, meeting responses | The account has no calendars, events or reminders in the app        |
| `Contacts.ReadWrite`   | Contacts and address autocomplete                  | The account's contacts aren't in the app or in address autocomplete |
| `Tasks.ReadWrite`      | Microsoft To Do lists and tasks                    | The account has no task lists                                       |

Adding an account asks for both groups at once, so one approval covers
everything; **Try with mail only** asks for the first group alone (see
["Need admin approval"](#need-admin-approval)).

One more scope is not a Graph permission but the app's own:
`api://<client ID>/Sync.Receive` lets the app open a stream to the Cumulas push
service (see [Instant updates](#instant-updates-the-cumulas-push-service)).
Work and school accounts also ask for a scope of Cumulas Signatures' shared
app, `api://<service host>/<its client ID>/access_as_user`, which the
service's operator pre-authorizes for Cumulas Mail (see
[Company signatures](#company-signatures-cumulas-signatures)).

### How permissions are kept

The app records which Graph permissions Microsoft says each account has: at
each sign-in, and at each token refresh (when the access token expires, about
every 60 to 90 minutes while the app runs). It turns on the features whose
permission is there.

- **Work and school accounts** refresh with
  `https://graph.microsoft.com/.default offline_access`, which returns every
  permission the account was granted, without a prompt. So when an
  administrator approves a permission later, its feature turns on by itself
  within a couple of hours. If Microsoft refuses `.default`, the app asks once
  more with the explicit list.
- **Personal accounts** refresh with an explicit list: the permissions needed
  for mail, plus the optional ones the account was granted.
- **New permissions in future versions are always optional**, so updating
  never signs anyone out: an account gets a new feature when someone selects
  **Turn on** (or, for work and school accounts, once an administrator has
  approved it).
- **Features Microsoft 365 refuses** (see "Refused" below) are checked again
  at the account's first sync a day later, or at once with **Check again**.

Accounts added before the app recorded permissions count as having all of them
until their first refresh records what Microsoft says.

## "Need admin approval"

Whether people may grant these permissions themselves is decided by **their
organization's consent settings**, not by the app. Since late 2025, in steps
until mid-2026, Microsoft's default setting ("Let Microsoft manage your
consent settings") blocks user consent to `Mail.*`, `Calendars.*`,
`MailboxSettings.*`, `Contacts.ReadWrite` and `Tasks.*` for apps that aren't
on Microsoft's own list of mail clients (Cumulas Mail isn't). `Mail.Send`,
`User.Read`, the OpenID Connect scopes and `Contacts.Read` remain ones people
can approve themselves, and approvals given before the change remain. So in
most organizations an administrator must approve Cumulas Mail once before
anyone can add a work or school account. Other organizations:

- let people approve any app (the old default);
- allow consent only for apps from **verified publishers**, and only for
  permissions the admins classified as low impact; or
- require an administrator to approve every app.

Personal Microsoft accounts (Outlook.com, Hotmail, Live) aren't affected and
never need an administrator.

When Microsoft says an administrator must approve, people see "Need admin
approval" (or "Approval required") in the browser, and the app helps:

- **The approval panel** shows the admin consent link, with **Email IT** (a
  ready-to-send request for IT that says what the app is and what it asks
  for) and **Open approval page** (for people who are administrators
  themselves). With the published registration, the link brings the
  administrator back to
  [the page for IT administrators](#approving-cumulas-mail-it-admins).
- **Request approval.** When the organization has the admin consent workflow
  on (AADSTS90095), **Request approval** is the main button: it opens
  Microsoft's page again, where people send the request with a reason.
  Microsoft usually emails them when their IT team responds; they then select
  **Try again**.
- **Try with mail only** signs in with the permissions needed for mail alone,
  for organizations that approved mail but not the rest, or let people
  approve only some permissions. The other features then need permission.
- **"Needs permission".** A feature whose permission wasn't granted offers
  **Turn on**, which asks Microsoft for that one permission on top of what the
  account has (incremental consent), and **Ask IT**, which shows the approval
  link for the account's organization. Approvals given later are picked up by
  themselves within a couple of hours (see
  [How permissions are kept](#how-permissions-are-kept)).
- **"Refused".** When the permission is granted but Microsoft 365 refuses the
  feature (Graph answers 403 for the account's calendars, contacts or task
  lists, for example because the mailbox's license doesn't include it), the
  feature shows as refused, with **Check again** and **Email IT**, not Turn
  on, which can't fix it.
- **When Microsoft's page doesn't send people back.** Microsoft shows some
  errors on its own page and never returns to the app. The waiting, timeout
  and cancel screens offer **Microsoft said I need admin approval**, which
  shows the approval panel for the address being signed in, and **Microsoft
  showed an error code**, which turns the AADSTS number into the matching
  screen (or opens Microsoft's explanation of codes the app doesn't know).

When only the push service's scope needs approval, the sign-in goes ahead
without it and the app shows no link yet (see
[What users see before it is set up](#what-users-see-before-it-is-set-up)).

## Approving Cumulas Mail (IT admins)

The page for administrators is <https://flatpak.cumulas.co/admin-consent/>.
It says what Cumulas Mail is and what it asks for, and how to:

- approve it for everyone, with the link below;
- approve it in the Microsoft Entra admin center (Enterprise apps → Cumulas
  Mail → Security → Permissions → Grant admin consent), once Cumulas Mail is
  listed there, which it is after someone in the organization approved it;
- approve the requests people send with the admin consent workflow
  (Enterprise apps → Activity → Admin consent requests);
- let people approve Cumulas Mail for themselves, with a custom app consent
  policy for its client ID (or Cumulas Inc's publisher ID, `7118869`), or by
  classifying its permissions as low impact;
- fix sign-ins that approving doesn't fix (the AADSTS codes in
  [Troubleshooting](#troubleshooting)).

The approval link is the one the app builds for the published registration,
for the tenant `organizations` (a Rust unit test and
`packaging/flatpak/repo/check-admin-consent.sh` keep the app, the page and
this link the same):

```text
https://login.microsoftonline.com/organizations/v2.0/adminconsent?client_id=10abb2b9-1a7a-4dbc-ba0f-c9a0de9088ad&scope=openid+profile+email+offline_access+https%3A%2F%2Fgraph.microsoft.com%2FUser.Read+https%3A%2F%2Fgraph.microsoft.com%2FMail.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FMail.Send+https%3A%2F%2Fgraph.microsoft.com%2FMailboxSettings.Read+https%3A%2F%2Fgraph.microsoft.com%2FCalendars.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FContacts.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FTasks.ReadWrite+api%3A%2F%2F10abb2b9-1a7a-4dbc-ba0f-c9a0de9088ad%2FSync.Receive&redirect_uri=https%3A%2F%2Fflatpak.cumulas.co%2Fadmin-consent%2F
```

A Global Administrator, Privileged Role Administrator, Cloud Application
Administrator or Application Administrator opens it and signs in. Microsoft
then brings them back to the page, which shows whether the approval went
through. The link names exactly the permissions in the tables above plus
`Sync.Receive`, unlike the plain `/adminconsent?client_id=…` link, which
grants whatever is listed on the registration.

The link asks for `Sync.Receive` while `ADMIN_LINK_INCLUDES_PUSH` in
`crates/core/src/auth/microsoft.rs` is `true`. If Microsoft ever refuses the
scope in the link, turn it off, drop the scope from the page's link and this
one, remove the page's "For instant updates" rows from its table, and drop
"plus `Sync.Receive`" above (`check-admin-consent.sh` fails until they all
match); people then approve instant updates through the app's own two-step
sign-in.

## Registering the app (publisher)

The published app has one multi-tenant app registration, Cumulas Mail, from
the verified publisher Cumulas Inc. Its **Application (client) ID**,
`10abb2b9-1a7a-4dbc-ba0f-c9a0de9088ad`, is built into the app, so builds sign
in with it without any build settings. The client ID is not a secret.

A build can name another registration instead, at build time:
`CUMULAS_MS_CLIENT_ID=<id> cargo build --release ...`, or an entry in the
Flatpak manifest's `build-options.env`. An empty value keeps the published
registration (the Flatpak manifest's is empty), and `none` builds in no client
ID: people must then enter one under **Settings → Microsoft app registration**
before signing in. Any other value that isn't a GUID is ignored, with a
warning in the log. A client ID entered in Settings takes precedence over the
built-in one (see
[Using your organization's own registration](#using-your-organizations-own-registration-it-admins)).

The published registration was set up as follows; another registration needs
the same steps. For company signatures, a build with another registration also
needs its client ID authorized on Cumulas Signatures' shared app and allowed
on the server (see
[Setting it up](#setting-it-up-signature-service-operators)); until it is, its
work accounts show company signatures as not allowed. Build it with
`CUMULAS_SIGNATURE_CLIENT_ID=none` to leave them out instead.

1. In the [Microsoft Entra admin center](https://entra.microsoft.com), go to
   **App registrations → New registration**.
2. Name: `Cumulas Mail`. Supported account types: **Accounts in any
   organizational directory and personal Microsoft accounts**.
3. Redirect URI: platform **Public client/native (mobile & desktop)**, value
   `http://localhost` (any port is accepted for localhost). Then
   **Authentication → Add a platform → Web**, redirect URI
   `https://flatpak.cumulas.co/admin-consent/`, exactly, with the trailing
   slash. The app uses it only as the approval link's `redirect_uri`
   (`ADMIN_CONSENT_LANDING_URI`): sign-in keeps `http://localhost`, and no
   client secret is involved. Without it, every approval link from the app
   fails with AADSTS50011.
4. **API permissions**: exactly the permissions in the tables above and
   `Sync.Receive`, nothing else. Add the Graph ones with **Add a permission →
   Microsoft Graph → Delegated**, and `Sync.Receive` with **Add a permission →
   My APIs → Cumulas Mail → Sync.Receive** once the registration exposes it
   (see
   [Setting up the app registration for it](#setting-up-the-app-registration-for-it-publisher)).
   No `.Shared` permissions and no `User-Mail.ReadWrite.All`. The list
   matters: the admin center's **Grant admin consent** button and `.default`
   requests use it, and Microsoft may refuse an approval link that names a
   scope of an API that isn't on it (AADSTS650057). Do not grant admin consent
   for other organizations; each organization decides for itself.
5. **Branding & properties**: set the publisher domain to `cumulas.co`, then
   complete **publisher verification** with a Microsoft AI Cloud Partner
   Program ID (Cumulas Inc's is `7118869`). Verification lets the app pass
   risk-based step-up consent and the "verified publishers, selected
   permissions" setting, but doesn't exempt it from Microsoft's managed default
   (see ["Need admin approval"](#need-admin-approval)).

## Instant updates (the Cumulas push service)

Microsoft Graph tells apps about new mail with change notifications, but only
by posting them to a public HTTPS address, which an app on someone's computer
doesn't have. Cumulas Mail therefore uses a small relay, **Cumulas Sync**, at
`https://sync.cumulas.co`:

1. The app gets a token for the relay's own scope,
   `api://<client ID>/Sync.Receive`, from the account's refresh token. The
   relay never receives a Graph token, so it can't read anyone's mail.
2. It asks the relay for the account's channel, and keeps a stream of
   server-sent events open to it.
3. In Graph it subscribes, without resource data, to the Inbox's messages and
   to calendar events, with the channel's URL and `clientState`.
4. Graph posts each change to the relay, which passes on only the
   subscription ID and the change type. The app then fetches what changed from
   Graph itself.

Each time the stream opens, the app reconciles the subscriptions (renewing,
recreating or replacing them as needed) and catches up on the Inbox and the
calendar, so nothing is missed while it was disconnected. While connected it
renews every subscription at least every 30 minutes. When the push service is
turned off in Settings, unavailable, or not set up, Microsoft accounts check
the Inbox about every 30 seconds instead (the calendar keeps its regular sync).
Turning the service off, or removing the account, deletes the account's
subscriptions.

### Setting up the app registration for it (publisher)

These steps are done on the published registration, and are needed once on
any other. Until they're done, everything works as before, with the 30-second
Inbox check (see [below](#what-users-see-before-it-is-set-up)).

1. **Expose an API → Application ID URI → Add**: keep the suggested
   `api://<client ID>` (the app asks for exactly this form) and save.
2. **Expose an API → Add a scope**:
   - Scope name: `Sync.Receive`
   - Who can consent: **Admins and users**
   - Admin consent display name: `Receive change notices for Cumulas Mail`
   - Admin consent description: `Lets Cumulas Mail receive notices from the
Cumulas Sync service that mail or events changed, so it can fetch them
right away. The notices don't contain mail or event content, and the
service gets no access to the mailbox.`
   - User consent display name: `Receive instant mail updates`
   - User consent description: `Lets Cumulas Mail hear within seconds that
your mail or calendar changed. The notices don't contain your mail.`
   - State: **Enabled**
3. **Expose an API → Authorized client applications → Add a client
   application**: enter the registration's own Application (client) ID and
   tick `api://<client ID>/Sync.Receive`. This pre-authorizes the app for its
   own scope, so users (including everyone already signed in) get the token
   without being asked to consent again.
4. **Manifest**: check that the requested access token version is 2. In the
   current manifest editor (Microsoft Graph format) that is
   `"api": { "requestedAccessTokenVersion": 2 }`; in the older Azure AD Graph
   format it is `"accessTokenAcceptedVersion": 2`. Registrations that include
   personal Microsoft accounts, like the published one, require version 2. The
   relay accepts version 1 and version 2 tokens alike: it takes the client ID
   (version 2) or `api://<client ID>` (version 1) as the audience, either
   version's issuer, and the app that asked for the token from `azp`
   (version 2) or `appid` (version 1), and never reads the version. A
   registration for work and school accounts only can keep version 1.
5. The relay must accept the client ID: its operator lists it in
   `SYNC_CLIENT_IDS` (for `sync.cumulas.co`, the published ID already is).

Organizations that use [their own registration](#using-your-organizations-own-registration-it-admins)
follow the same steps on it, and ask the relay's operator to add their client
ID to `SYNC_CLIENT_IDS`; until then their users get the 30-second Inbox check.

### What users see before it is set up

Each account's push status says why. **Settings → Accounts** shows it under
**New mail** (for example "Instant (Microsoft push)", or "Every 30 seconds —
push service unreachable"), with a **Sign in again** button where signing in
helps, and the approval link where an administrator must approve.

| Situation                                                       | What happens                                                                                                                                                                                                                                                                          |
| --------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The registration doesn't expose `Sync.Receive` yet              | The Inbox is checked every 30 seconds. Status "unsupported": "This app registration doesn't offer instant updates for Microsoft accounts yet." The app asks again every 6 hours and at each start.                                                                                    |
| The scope exists, but isn't pre-authorized and wasn't consented | The Inbox is checked every 30 seconds. Status "consent missing": "Sign in again to turn on instant updates." Signing in again as that account then asks consent for the scope too (only then, since a sign-in that asks for a scope the registration doesn't offer fails altogether). |
| The relay doesn't accept the client ID or the organization      | The Inbox is checked every 30 seconds; status "unsupported"; asked again every 6 hours.                                                                                                                                                                                               |
| The relay is unreachable or busy                                | The Inbox is checked every 30 seconds while the app reconnects, backing off up to 5 minutes (and honoring `Retry-After`).                                                                                                                                                             |
| Graph refuses to watch the mailbox                              | The Inbox is checked every 30 seconds; status "unsupported"; tried again every 6 hours.                                                                                                                                                                                               |

**Sign in again** (or signing in through **Add account** with the account's
address) asks consent for the scope too. If the scope needs an administrator's
approval, or the user declines it, the sign-in goes ahead without it: the
browser page says "Signing in without instant updates" and the sign-in opens
again in a new tab, for the Graph permissions alone. The Inbox is still checked
every 30 seconds. Settings then says why instead of offering **Sign in again**,
and when an administrator must approve, it shows the approval link (covering
the Graph permissions and `Sync.Receive`) with **Copy**, **Email IT** and
**Open approval page** buttons. Later sign-ins don't ask for the scope again
while the app runs.

Pre-authorizing the app for its own scope
([step 3](#setting-up-the-app-registration-for-it-publisher)) fixes it for
everyone. Until then, an administrator can approve the scope for their
organization with the published registration's approval link (see
[Approving Cumulas Mail](#approving-cumulas-mail-it-admins)), or, for any
registration, by opening
`https://login.microsoftonline.com/organizations/v2.0/adminconsent?client_id=<client ID>&scope=api://<client ID>/Sync.Receive&redirect_uri=http://localhost`
and signing in to their own tenant. The app turns instant updates on within 6
hours, or when it next starts.

None of these is an account error: mail, calendar, contacts and tasks keep
syncing through Graph as usual.

### Pointing a build at another relay

The relay's address is `https://sync.cumulas.co` unless `CUMULAS_SYNC_URL`
says otherwise:

- **At build time**, like the client ID:
  `CUMULAS_SYNC_URL=https://sync.example.org cargo build --release ...`, or an
  entry in the Flatpak manifest's `build-options.env`.
- **At run time**, which takes precedence:
  `CUMULAS_SYNC_URL=https://sync.example.org cumulas-mail`, or
  `flatpak run --env=CUMULAS_SYNC_URL=https://sync.example.org co.cumulas.Mail`.

The address must start with `https://`; `http://` is accepted only for
`localhost`, `127.0.0.1` and `[::1]`, for testing a relay on the same computer.
Any other value is ignored (with a warning in the log) and the default is used.
The relay must accept the build's client ID (`SYNC_CLIENT_IDS`), and Graph only
posts to an HTTPS address it can reach, so a relay on `localhost` serves for
testing the stream only.

Graph allows the app one subscription per mailbox and resource. If the same
account uses Cumulas Mail with two different relays, only one of them gets
instant updates; the other checks the Inbox every 30 seconds, and its push
status names the relay that has them.

## Company signatures (Cumulas Signatures)

Organizations that use Cumulas Signatures get their signature in Cumulas Mail
with nothing for their people to set up. The app adds it to new messages, and
to replies and forwards when the organization says so, as ordinary content the
person can change; when the organization's mail server adds signatures on send
(the service's relay), the app adds none, so mail never carries two, and a
reply or forward that the relay leaves unsigned (the organization's setting)
starts with no signature. Only Microsoft 365 work and school accounts take
part: personal Microsoft accounts (tenant
`9188040d-6c67-4c5b-b112-36a304b66dad`) and IMAP accounts never do. While an
organization's signature is in use, the person's own signature for that
account isn't used.

For each work or school account, the app:

1. Asks Microsoft, at the account's own tenant, for a token for the service's
   API from the account's refresh token: the scope
   `api://<service host>/<shared app's client ID>/access_as_user` (with
   `offline_access`). The app makes this scope itself from its build settings,
   never from a server's answer, and names the host's port only when it isn't
   the default.
2. Microsoft gives Cumulas Mail a token only in organizations that have
   added the Cumulas Signatures app in Microsoft 365. That includes
   organizations that use it now, used it before and kept the app, or where
   someone signed in to the service (its portal, sign-up and `/manage` sign
   in with the shared app). In other organizations Microsoft refuses
   (AADSTS500011, 650052, 650057 or 700016), nothing reaches the service, and
   the app counts the organization as not using it. A refusal for consent
   (AADSTS65001 or the other admin consent codes, 65004, or
   `consent_required`) means the organization has the app but hasn't allowed
   Cumulas Mail. Anything else (no network, MFA or Conditional Access wanting
   the person, an expired sign-in) is tried again later, and is never an
   account error: the Graph sync reports sign-in problems on its own.
3. With a token, calls `GET <service>/apps/signature`, sending the last
   answer's ETag in `If-None-Match` and following no redirects. The token
   names the person and their organization. No mail, contacts or recipients
   are sent. The app keeps the answer: the signature
   (`"source": "app"`), added on send (`"relay"`), or none (`"none"`, and 404
   `not-managed` or `not-set-up`), with `applyToReplies`, which says for
   `app` and `relay` alike whether replies and forwards are signed too. 403
   `app-not-allowed` means the service's operator hasn't allowed Cumulas Mail.
   Any other answer is a failure, including a 403 or 404 without the
   service's code (a proxy's, say) and an answer whose `version` is newer
   than 1, the format this app reads (an answer without one is version 1).

Signatures are sanitized before use, like message bodies, but stricter:
pictures and CSS backgrounds only over HTTPS or as `data:` images (no
`image-set()`, `image()`, `cross-fade()` or `src()`, whose plain strings are
addresses), links only to the web, email and phone numbers, and relative
links resolved against the service. Kept signatures are sanitized again
whenever they're read, so a stricter sanitizer in an update applies at once.
The kept ETag starts with the sanitizer's version (`s2:`): an answer kept by
another sanitizer is asked for without it, so it comes again in full. A check
that fails keeps the last answer, marked stale, so a signature keeps working
offline.

| When the app asks                |                                                                                                                                                                                                                                                                                |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| About 15 seconds after it starts | Each work or school account that was never checked, whose last check failed, or whose organization signs its mail.                                                                                                                                                             |
| Every 6 hours                    | While the organization signs the account's mail (the app adds the signature, or the relay does on send).                                                                                                                                                                       |
| Every 24 hours                   | While the organization doesn't use the service (Microsoft refused), has no signature for the person, or hasn't allowed the app; not at each start.                                                                                                                             |
| 30 minutes after a failure       | Or when the service's `Retry-After` says (at most 6 hours), or as soon as the network is back.                                                                                                                                                                                 |
| Right after signing in           | Whatever the last answer. Signing in again picks up consent the organization gave meanwhile.                                                                                                                                                                                   |
| When starting a message          | Waits up to 4 seconds for an account never checked; checks again in the background while the organization signs the mail and its last answer is over an hour old.                                                                                                              |
| When Settings → Signatures opens | Each account whose last answer is over 10 minutes old or whose last check failed, unless that answer was that the organization doesn't sign the person's mail (it doesn't use the service or has no signature for them): that one is asked only every 24 hours and at sign-in. |
| When asked to check now          | At most once a minute, and not before the service's `Retry-After`.                                                                                                                                                                                                             |

Times go by the wall clock, so a check due while the computer slept comes
within a minute of waking, and a day-long wait counts across restarts.

### Setting it up (signature service operators)

The service's shared multi-tenant app registration (the shared Outlook
add-in's, whose client ID is the server's `microsoft.clientId`) exposes
`api://<host>/<client ID>/access_as_user`, where `<host>` is the host of the
server's public address. For Cumulas Mail:

1. In the [Microsoft Entra admin center](https://entra.microsoft.com), on that
   registration: **Expose an API → Authorized client applications → Add a
   client application**. Enter Cumulas Mail's Application (client) ID and tick
   `api://<host>/<client ID>/access_as_user`. Pre-authorized, Cumulas Mail
   gets tokens in every organization that has the shared app, without asking
   anyone to consent again: clients, former clients that kept it, and
   organizations where someone signed in to the service. Their tokens reach
   the service, which answers `not-managed` where the organization isn't a
   client.
2. On the server, add Cumulas Mail's client ID under **Settings → Desktop mail
   apps**. Until then the service answers 403 `app-not-allowed`.

For `signature.cumulas.co`, both are done for the published registration.

Organizations that sign in with [their own registration](#using-your-organizations-own-registration-it-admins)
of Cumulas Mail need its client ID added in both places too.

### Building it in

Cumulas Signatures is built in: with nothing set, the app asks
`https://signature.cumulas.co` for signatures, with the scope of its published
shared app, whose client ID is `da27f705-3818-4ca0-9b5d-7781405ea54e`. Two
variables change that:

- `CUMULAS_SIGNATURE_URL`: the service's address, `https://signature.cumulas.co`
  unless set. As with `CUMULAS_SYNC_URL`, it must start with `https://`
  (`http://` only for `localhost`, `127.0.0.1` and `[::1]`) and carry no user
  name, query or fragment; any other value is ignored, with a warning in the
  log. The scope follows its host, so the shared app must expose
  `api://<that host>/<client ID>`.
- `CUMULAS_SIGNATURE_CLIENT_ID`: the shared app's client ID (a GUID). Unless
  set, the published shared app's is used, but only with Cumulas Signatures'
  own address: that app exposes its API at `signature.cumulas.co` alone, so
  with another address it would make a scope nobody exposes. A service at
  another address therefore needs its client ID here; without one, company
  signatures are off, with a warning in the log. `none` turns company
  signatures off: every account shows them as unsupported, and the app never
  asks Microsoft or the service for them. An empty value counts as not set,
  and any other value that isn't a GUID is ignored, with a warning in the
  log.

Both are read at build time (`CUMULAS_SIGNATURE_CLIENT_ID=<id> cargo build
--release ...`, or the Flatpak manifest's `build-options.env`, where the
client ID is empty, so the published one applies) and at run time, which
takes precedence. A client ID from the build goes with the build's address: a
run pointed at another service leaves it out, so give that run the service's
client ID too:
`flatpak run --env=CUMULAS_SIGNATURE_URL=https://signature.example.org --env=CUMULAS_SIGNATURE_CLIENT_ID=<its client ID> co.cumulas.Mail`,
or `flatpak run --env=CUMULAS_SIGNATURE_CLIENT_ID=none co.cumulas.Mail` to
turn company signatures off for that run.

## Using your organization's own registration (IT admins)

Organizations that prefer to control the app themselves can register it in
their own tenant (single-tenant, the same `http://localhost` redirect URI and
permissions as above; no Web redirect URI), grant admin consent once, and have
users enter the client ID and tenant ID under **Settings → Microsoft app
registration**.

With such a registration, the approval link the app shows still returns to
`http://localhost`, so after approving, the administrator's browser shows a
"can't connect" page. The approval is recorded by then: close it, and select
**Try again** in Cumulas Mail. The page for IT administrators at
flatpak.cumulas.co is for the published registration only and doesn't apply.

## Troubleshooting

| What the user sees                                                                                                                      | Meaning                                                                                                                                                                                                                                        |
| --------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| "Your organization needs to approve Cumulas Mail" (AADSTS65001, 90093, 90094, 900941, 900981)                                           | The organization requires an administrator to approve the app. The app shows the approval link with **Email IT** and **Open approval page**, and offers **Try with mail only**.                                                                |
| Same screen, with **Request approval** as the main button (AADSTS90095)                                                                 | The organization has the admin consent workflow on: the user can request approval on Microsoft's page, then select **Try again** once IT has approved.                                                                                         |
| "Cumulas Mail doesn't have permission yet" (AADSTS65004)                                                                                | The permission prompt was declined. Microsoft also sends this when the user selects "Back to app" after requesting approval, so try again once IT has approved.                                                                                |
| "You haven't been given access to Cumulas Mail" (AADSTS50105)                                                                           | The organization requires assignment to the app: an administrator assigns the user under Enterprise apps → Cumulas Mail → Users and groups.                                                                                                    |
| "Your organization has blocked Cumulas Mail" (AADSTS7000112)                                                                            | The app is disabled in the organization, for example after a consent request was blocked: Enterprise apps → Cumulas Mail → Properties → Enabled for users to sign in.                                                                          |
| "Your organization isn't set up for Cumulas Mail yet" (AADSTS65005)                                                                     | The organization is missing something the app needs; approving the app usually fixes it. If it doesn't, [let Cumulas know](https://github.com/cumulas-inc/email-client/issues).                                                                |
| "Your organization's Microsoft 365 is missing a service" (AADSTS650052)                                                                 | The organization hasn't subscribed to or enabled a service the app needs, usually Exchange Online. Approving the app doesn't help.                                                                                                             |
| "Set up your sign-in security info first" (AADSTS53004, 53010)                                                                          | The user must finish setting up multifactor authentication or other security info, which Microsoft couldn't do during this sign-in.                                                                                                            |
| "Your organization's sign-in rules …" or "Your organization only allows apps it has approved" (AADSTS53000–53003, 53011, 530000–530999) | A Conditional Access policy blocked the sign-in; the Entra sign-in logs show which. A sign-in through the browser on Linux may not meet policies that require a compliant or managed device or an approved client app.                         |
| "Calendar needs permission", or "Needs permission: …" in Settings → Accounts                                                            | The feature's optional permission wasn't granted. **Turn on** asks for it; **Ask IT** shows the approval link. An approval given later is picked up within a couple of hours.                                                                  |
| "Microsoft 365 refused access to your calendar" (or contacts, tasks)                                                                    | The permission is granted, but Graph refused the feature (403), often because the mailbox's license doesn't include it. The app checks again daily, or at once with **Check again**; **Email IT** asks IT to look.                             |
| "Sign-in timed out"                                                                                                                     | Microsoft's page didn't send the browser back within 5 minutes, often because it showed an error of its own. **Microsoft said I need admin approval** and **Microsoft showed an error code** lead to the matching screen.                      |
| "Signing in without instant updates" (in the browser)                                                                                   | The push service's scope needs an administrator's approval or was declined. The sign-in continues in a new tab without it; see [Instant updates](#what-users-see-before-it-is-set-up).                                                         |
| "Sign in again to keep … up to date"                                                                                                    | The refresh token expired or was revoked (password change, admin action, or a Conditional Access policy requiring interaction). When Microsoft says it's one of the blocks above, the account shows that reason instead, with its AADSTS code. |
| Sign-in never returns to the app                                                                                                        | The browser could not reach `http://localhost:<port>`; check that nothing blocks loopback connections. Cancel and try again. If Microsoft's page shows an error, use **Microsoft showed an error code**.                                       |

After an administrator approves Cumulas Mail with the published registration's
link, Microsoft brings their browser back to
<https://flatpak.cumulas.co/admin-consent/>, which says whether it worked and
what to tell people. With an organization's own registration the link returns
to `http://localhost` instead, which shows a "can't connect" page; the approval
is recorded at that point, and the user can select **Try again**.

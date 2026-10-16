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

All permissions are **delegated** (the app acts as the signed-in user, never
on behalf of the whole organization) and are ones Microsoft lets users grant
themselves by default:

| Permission                                | Used for                                           |
| ----------------------------------------- | -------------------------------------------------- |
| `openid`, `profile`, `email`, `User.Read` | Signing in and showing your name and address       |
| `offline_access`                          | Staying signed in (refresh token)                  |
| `Mail.ReadWrite`                          | Reading, flagging, moving and deleting mail        |
| `Mail.Send`                               | Sending mail                                       |
| `MailboxSettings.Read`                    | Time zone and mailbox preferences                  |
| `Calendars.ReadWrite`                     | Calendar views, creating events, meeting responses |
| `Contacts.ReadWrite`                      | Contacts and address autocomplete                  |
| `Tasks.ReadWrite`                         | Microsoft To Do lists and tasks                    |

One more scope is not a Graph permission but the app's own:
`api://<client ID>/Sync.Receive` lets the app open a stream to the Cumulas push
service (see [Instant updates](#instant-updates-the-cumulas-push-service)).
Work and school accounts also ask for a scope of Cumulas Signatures' shared
app, `api://<service host>/<its client ID>/access_as_user`, which the
service's operator pre-authorizes for Cumulas Mail (see
[Company signatures](#company-signatures-cumulas-signatures)).

## "Need admin approval"

Whether a user may grant these permissions is decided by **their
organization's consent settings**, not by the app. Organizations commonly:

- allow users to consent to any app;
- allow consent only for apps from **verified publishers**, and only for
  permissions the admins classified as low impact; or
- require an administrator to approve every app.

In the last two cases the user sees "Need admin approval" in the browser. The
app detects this and shows a screen with an **admin consent link** and a
ready-to-send email to IT. An administrator opening the link can approve the
app once for the whole organization; when the sign-in also asked consent for
the push service's scope, the link covers that too. If the organization has
the admin consent workflow enabled, users can also request approval directly
from Microsoft's page.

When only the push service's scope needs approval, the sign-in goes ahead
without it and the app shows no link yet (see
[What users see before it is set up](#what-users-see-before-it-is-set-up)).

Personal Microsoft accounts (Outlook.com, Hotmail, Live) never need admin
approval.

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
   `http://localhost` (any port is accepted for localhost).
4. **API permissions → Add a permission → Microsoft Graph → Delegated**: add
   the permissions in the table above. Do not grant admin consent for other
   organizations; each organization decides for itself.
5. **Branding & properties**: set the publisher domain to `cumulas.co`, then
   complete **publisher verification** with a Microsoft AI Cloud Partner
   Program ID. Verification is what lets users in organizations that only
   allow verified publishers sign in without an administrator.

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
the Graph permissions and `Sync.Receive`) with **Copy** and **Email IT**
buttons. Later sign-ins don't ask for the scope again while the app runs.

Pre-authorizing the app for its own scope
([step 3](#setting-up-the-app-registration-for-it-publisher)) fixes it for
everyone. Until then, an administrator can approve the scope for their
organization by opening
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
their own tenant (single-tenant, same redirect URI and permissions as above),
grant admin consent once, and have users enter the client ID and tenant ID
under **Settings → Microsoft app registration**.

## Troubleshooting

| What the user sees                                                                            | Meaning                                                                                                                                                                                |
| --------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| "Your organization needs to approve Cumulas Mail" (AADSTS65001, 90093, 90094, 900941, 900981) | The organization requires an administrator to approve the app. The app shows the admin consent link and an email to send to IT.                                                        |
| Same screen, with "Request approval" mentioned (AADSTS90095)                                  | The organization has the admin consent workflow enabled: the user can also request approval on Microsoft's page.                                                                       |
| "Cumulas Mail needs your permission" (AADSTS65004)                                            | The permission prompt was declined. Microsoft also sends this when the user clicks "Back to app" after requesting approval, so retry once IT has approved.                             |
| "Signing in without instant updates" (in the browser)                                         | The push service's scope needs an administrator's approval or was declined. The sign-in continues in a new tab without it; see [Instant updates](#what-users-see-before-it-is-set-up). |
| "Sign in again to keep … up to date"                                                          | The refresh token expired or was revoked (password change, admin action, or a Conditional Access policy requiring interaction).                                                        |
| Sign-in never returns to the app                                                              | The browser could not reach `http://localhost:<port>`; check that nothing blocks loopback connections. Cancel and try again.                                                           |

After an administrator grants consent through the link, Microsoft redirects
their browser to `http://localhost`, which shows a "can't connect" page. The
consent is recorded at that point; the user can select **Try again**. A
friendlier landing page needs an extra redirect URI on the app registration
(for example `https://cumulas.co/mail/consent-granted`) and the
`ADMIN_CONSENT_REDIRECT_URI` constant in `crates/core/src/auth/microsoft.rs`
changed to match.

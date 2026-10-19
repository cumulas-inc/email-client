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

| Permission                         | Used for                                                                                                               | Without it                                                          |
| ---------------------------------- | ---------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| `MailboxSettings.Read`             | Time zone and mailbox preferences                                                                                      | All-day events and due dates use the computer's time zone           |
| `Calendars.ReadWrite`              | Calendar views, creating events, meeting responses                                                                     | The account has no calendars, events or reminders in the app        |
| `Contacts.ReadWrite`               | Contacts and address autocomplete                                                                                      | The account's contacts aren't in the app or in address autocomplete |
| `Tasks.ReadWrite`                  | Microsoft To Do lists and tasks                                                                                        | The account has no task lists                                       |
| `InformationProtectionPolicy.Read` | The sensitivity labels people may apply to mail, and the label policy (work and school accounts; Microsoft Graph beta) | No sensitivity labels when writing mail                             |

Adding an account asks for both groups at once, so one approval covers everything, except
`InformationProtectionPolicy.Read`: it's asked for only when someone turns sensitivity labels on,
because personal accounts can't have it and a new permission in the first sign-in would make
organizations that approved Cumulas Mail approve it again before anyone else could sign in.
**Try with mail only** asks for the first group alone (see
["Need admin approval"](#need-admin-approval)).

One more scope is not a Graph permission but the app's own:
`api://<client ID>/Sync.Receive` lets the app open a stream to the Cumulas push
service (see [Instant updates](#instant-updates-the-cumulas-push-service)).
Work and school accounts also ask for a scope of Cumulas Signatures' shared
app, `api://<service host>/<its client ID>/access_as_user`, which the
service's operator pre-authorizes for Cumulas Mail (see
[Company signatures](#company-signatures-cumulas-signatures)).

Administrators who set up encryption and labels from the app sign in to a
separate app, **Cumulas Mail - Admin Setup**, which has permissions of its
own; Cumulas Mail itself never asks for them (see
[Set it up from Cumulas Mail](#set-it-up-from-cumulas-mail)).

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
(but `InformationProtectionPolicy.Read`, which no earlier version asked for)
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
  policy limited to its client ID and the permissions it asks for, or by
  classifying its permissions as low impact (the page also says how to
  narrow a consent policy made from its earlier version, which could let
  people approve any permission for Cumulas Mail, or any app from Cumulas
  Inc);
- fix sign-ins that approving doesn't fix (the AADSTS codes in
  [Troubleshooting](#troubleshooting));
- set up **Cumulas Mail - Admin Setup**, the app administrators sign in to
  for [Set up or check for your organization](#set-it-up-from-cumulas-mail),
  for only the administrators they name, with a script.

The approval link is the one the app builds for the published registration,
for the tenant `organizations` (a Rust unit test and
`packaging/flatpak/repo/check-admin-consent.sh` keep the app, the page and
this link the same):

```text
https://login.microsoftonline.com/organizations/v2.0/adminconsent?client_id=10abb2b9-1a7a-4dbc-ba0f-c9a0de9088ad&scope=openid+profile+email+offline_access+https%3A%2F%2Fgraph.microsoft.com%2FUser.Read+https%3A%2F%2Fgraph.microsoft.com%2FMail.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FMail.Send+https%3A%2F%2Fgraph.microsoft.com%2FMailboxSettings.Read+https%3A%2F%2Fgraph.microsoft.com%2FCalendars.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FContacts.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FTasks.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FInformationProtectionPolicy.Read+api%3A%2F%2F10abb2b9-1a7a-4dbc-ba0f-c9a0de9088ad%2FSync.Receive&redirect_uri=https%3A%2F%2Fflatpak.cumulas.co%2Fadmin-consent%2F
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
   for other organizations; each organization decides for itself. Never add
   Office 365 Exchange Online or Microsoft Exchange Online Protection
   permissions here; they belong to the
   [setup app](#registering-the-setup-app-publisher).
5. **Branding & properties**: set the publisher domain to `cumulas.co`, then
   complete **publisher verification** with a Microsoft AI Cloud Partner
   Program ID (Cumulas Inc's is `7118869`). Verification lets the app pass
   risk-based step-up consent and the "verified publishers, selected
   permissions" setting, but doesn't exempt it from Microsoft's managed default
   (see ["Need admin approval"](#need-admin-approval)).

## Registering the setup app (publisher)

[Set up or check for your organization](#set-it-up-from-cumulas-mail) signs
administrators in to a second multi-tenant app registration, **Cumulas Mail -
Admin Setup**, from the same verified publisher. It was set up as follows;
another setup registration needs the same steps:

1. In the [Microsoft Entra admin center](https://entra.microsoft.com), go to
   **App registrations → New registration**. Name:
   `Cumulas Mail - Admin Setup`, which is what Cumulas Mail calls it.
   Supported account types: **Accounts in any organizational directory
   (Multitenant)** only, not personal Microsoft accounts. If the portal won't
   change it on an existing registration, set
   `"signInAudience": "AzureADMultipleOrgs"` in **Manifest**.
2. **Authentication**: one platform, **Mobile and desktop applications**,
   redirect URI exactly `http://localhost`. No Web platform, no implicit
   grant, no front-channel logout URL.
3. **API permissions**: exactly **Office 365 Exchange Online → Delegated →
   Exchange.Manage**, and **Microsoft Graph → Delegated →** `openid`,
   `profile` and `offline_access`; remove `User.Read` if it's listed. No
   application permissions, no `Exchange.ManageAsApp`, no
   `Exchange.ManageV2`, no `LicenseAssignment.Read.All`. Don't select
   **Grant admin consent**. Exchange.Manage must be listed: an approval link
   for an API that isn't listed fails with AADSTS650057.
4. **Certificates & secrets**, **Expose an API** and **Token configuration**:
   nothing.
5. **Branding & properties**: the same logo, terms of service and privacy
   statement URLs as Cumulas Mail, the publisher domain `cumulas.co`, and
   **publisher verification** with the Microsoft AI Cloud Partner Program ID
   `7118869`. Without it, risk-based step-up consent keeps administrators
   from approving a multi-tenant app for themselves.
6. Open its approval link, once, in a tenant you administer:

   ```text
   https://login.microsoftonline.com/organizations/v2.0/adminconsent?client_id=b4f17c55-b9f6-4a84-958f-afe3842da327&scope=openid+profile+offline_access+https%3A%2F%2Foutlook.office365.com%2FExchange.Manage&redirect_uri=http%3A%2F%2Flocalhost
   ```

   Microsoft must end at `http://localhost` (a page that can't connect),
   without AADSTS50011 or AADSTS700016. That approval is for everyone in that
   tenant: if you don't want to keep it, remove it with the command for an
   approval for everyone, or delete the enterprise app (see
   [Approval and security](#approval-and-security)).

Its client ID, `b4f17c55-b9f6-4a84-958f-afe3842da327`, is built in as
`PUBLISHED_SETUP_CLIENT_ID` (`crates/core/src/auth/admin_setup.rs`).
`CUMULAS_MS_SETUP_CLIENT_ID=<id>` at build time names another, and `none`
builds without automatic setup: the dialog then offers only the PowerShell
script. Any other value that isn't a GUID is ignored, with a warning in the
log. It must never be the mail app's registration, and the app refuses a
setup client ID that is the mail app's (published, built in, or entered in
Settings): Exchange.Manage on the mail app would let every stored sign-in of
an administrator, and with approval for everyone every person's, reach
Exchange's admin interface. The other way round too: Settings refuses the
setup app's ID as the mail app's, sign-in won't use one saved earlier, and a
build that names it as the mail app's gets the published mail app instead.
Tests in `crates/core/src/auth/` and
`packaging/flatpak/repo/check-admin-consent.sh` keep Exchange and Security &
Compliance scopes out of the mail app: out of its scopes, its approval links
and [its registration's steps](#registering-the-app-publisher).

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

## Sensitivity labels and encryption (IT admins)

### Receiving protected mail

Cumulas Mail can't open mail protected with Microsoft Purview (a sensitivity label that encrypts,
**Encrypt**, **Do Not Forward**) or encrypted with S/MIME: Microsoft Graph gives apps the protected
wrapper, not its content, and the only supported way to decrypt it is Microsoft's closed-source
SDK, which an open-source app can't ship. So Cumulas Mail:

- recognizes protected mail (a `message.rpmsg` attachment or its `_v2`–`_v4` variants,
  `Content-Class: rpmsg.message`, the S/MIME message classes and files, and links to Microsoft's
  encrypted message portal), says so with the sender's address, and offers **Open in Outlook on
  the web**, or a button to Microsoft's encrypted message portal when the message links to it;
- doesn't list those files as attachments, since nothing on the computer can open them;
- shows the attachments of S/MIME-signed mail, which Exchange keeps inside the signature file;
- opens Outlook on the web for **Reply**, **Reply all** and **Forward** on encrypted mail, so it
  never sends an unprotected copy;
- shows a message's sensitivity label when its `msip_labels` header (or Exchange's copy of it)
  names a label of the account's own organization. Any sender can write that header, with any
  name and your tenant ID, which is public, so Cumulas Mail never shows the name it gives. Once
  people have turned sensitivity labels on, only labels Microsoft 365 lists for them are shown,
  with their names from that list; until then, the message shows "Sensitivity label
  (unverified)".

Outlook on the web shows protected mail only once Exchange Online uses Azure Rights Management:
`Get-IRMConfiguration` shows `AzureRMSLicensingEnabled : True`, and
`Test-IRMConfiguration -Sender you@contoso.com -Recipient you@contoso.com` ends with PASS. A mail
flow rule with **Remove Office 365 Message Encryption and rights protection** makes your
organization's protected mail arrive readable in every app, Cumulas Mail included, but removes
its protection; we don't recommend it.

**More actions → Copy diagnostic details** copies what Cumulas Mail found about a message (the
signals above, file types and sizes), never its subject, addresses, text or links, for a bug
report.

### Sending protected mail

Microsoft Graph gives apps no way to encrypt or label a message. Cumulas Mail marks the message
with a header, and your mail server does the rest with rules you set up:

| Header              | Values                      | Set up with                                               |
| ------------------- | --------------------------- | --------------------------------------------------------- |
| `x-cumulas-protect` | `encrypt`, `do-not-forward` | [Mail flow rules](#encrypt-and-do-not-forward)            |
| `x-cumulas-label`   | the label's ID, a GUID      | [One auto-labeling policy per label](#sensitivity-labels) |

The fastest way to set this up is **Set up or check for your organization** in Cumulas Mail
([below](#set-it-up-from-cumulas-mail)). It makes the same rules and auto-labeling policies as the
PowerShell in this section, and can check and repair them later.

Cumulas Mail sets these on the message's draft (as Exchange Internet-header properties), checks
that the draft kept them, and only then sends it; otherwise nothing is sent. Only Microsoft 365
work and school accounts can send protected mail; IMAP accounts can't.

#### Set it up from Cumulas Mail

In **Settings → Accounts →** (a work or school account) **→ Sensitivity labels and encryption**,
select **Set up or check for your organization…**. The dialog starts with **Cumulas Mail's
permissions**: whether reading sensitivity labels is approved for that account, and the approval
link for everyone (**Approve again for everyone**), with what approving can't fix. That part needs
no other sign-in.

**Who:** an Exchange Administrator for the mail flow rules, a Compliance Administrator (or a member
of the Information Protection Admins role group) for the auto-labeling policies, or a Global
Administrator for both. Activate a role you get through Privileged Identity Management before
signing in: Exchange can take a while to apply it. Not an administrator? Select **Show the
PowerShell script** and send it to your IT team.

What happens:

1. **Sign in as an administrator** opens Microsoft's sign-in in your browser for **Cumulas Mail -
   Admin Setup**, a separate app from the same verified publisher. It asks for Exchange.Manage, to
   run Exchange Online and Security & Compliance commands as you, within your admin roles, and for
   `openid`, `profile` and `offline_access`, to sign you in and keep the sign-in while the window
   is open. Cumulas Mail itself never gets these permissions. If Microsoft shows _Consent on
   behalf of your organization_, leave it unchecked; if signing in then needs an administrator's
   approval, see [Approval and security](#approval-and-security).
2. Cumulas Mail reads what the feature needs and shows its status, without changing anything:
   Message Encryption and its templates, the three mail flow rules, each label's auto-labeling
   policy, whether your license includes automatic labeling, and how your mail is routed (the
   Cumulas Signatures relay, or another service).
3. You choose Encrypt and Do Not Forward, and which labels to set up: none is chosen at first,
   except labels that already have an auto-labeling policy of Cumulas Mail's. **Set up** (or
   **Repair**) then lists each change with its PowerShell, for you to confirm.
4. **Make these changes** reads everything again, makes the changes only if they are still the
   ones you confirmed, and reads them back.

Only rules and policies whose names start with `Cumulas Mail: ` change, and nothing is ever
deleted. The routing connector, the Message Encryption (IRM) settings and other rules never
change, except that other rules can move down a place: Cumulas Mail's encryption rules go above
rules that stop processing or send mail out through another service, and after the Cumulas
Signatures relay's rule only when mail coming back from it counts as internal. Moving a rule keeps
the others in their order. A rule or policy of Cumulas Mail's with settings Cumulas Mail doesn't
set (another action, condition or exception; for an auto-labeling policy, another rule or
location) is left alone, and the status names those settings. No auto-labeling policy is created,
repaired or turned on until "Cumulas Mail: ignore label requests from outside" works.

Auto-labeling policies are created in simulation and turn themselves on an hour after it
completes. Anyone in your organization can apply a label set up this way from any mail app that
can set the header, whoever the label is published to. Labels need a license that includes
automatic labeling (see [Sensitivity labels](#sensitivity-labels)). Cumulas Mail can't read your
organization's plans, only whether the license of the account you opened the dialog from includes
it; when it doesn't, labels are set up only after you select **Set up labels anyway**.

Cumulas Mail forgets the sign-in when the window closes, after 60 minutes without use, or when it
quits; it's never stored. **Copy setup details**, available after any sign-in attempt, copies what
setup did, for support: each sign-in attempt with Microsoft's error codes, and each call to
Microsoft 365 with its status and errors. It names your tenant, the administrator and Cumulas
Mail's own rules and policies, other rules by name and order only, and no passwords or tokens.
Cumulas Mail keeps it in memory until the next setup sign-in or until it quits.

It uses Microsoft 365's PowerShell interface over HTTPS (`InvokeCommand`), which Microsoft doesn't
document for other apps, so it may stop working. When it fails, the dialog gives a PowerShell
script, filled in for your organization, that does the same, and the filled-in Microsoft Purview
portal steps for administrators without Windows (Security & Compliance PowerShell runs only on
Windows). Setup and its script work only with Microsoft's worldwide cloud (GCC included), not
GCC High, DoD or 21Vianet.

##### Approval and security

Each administrator approves Cumulas Mail - Admin Setup for themselves when they first sign in,
where your consent settings allow it. When they don't, Microsoft says an administrator must
approve it, and the dialog shows the app's approval link. A Global Administrator, Privileged Role
Administrator, Cloud Application Administrator or Application Administrator can approve it with
that link (the browser then shows a page that can't connect: the approval is recorded by then, so
return to Cumulas Mail and select **Sign in again**). A Global Administrator or Privileged Role
Administrator can instead select **Sign in again** and check _Consent on behalf of your
organization_, which goes straight on. Approving it for everyone lets anyone in your organization
use it with their own Exchange rights, so use the setup script on the
[page for IT administrators](https://flatpak.cumulas.co/admin-consent/#setup-app) instead when you
can: it requires assignment and assigns the administrators you name before it approves the app.

An approval stays in your organization until you remove it, and someone who tricks an
administrator into signing in with this app's ID could get that administrator's Exchange rights.
After a one-off setup, remove your approval (**Done with setup?** at the end of the dialog shows
this with your object ID filled in). Removing approvals takes a Global Administrator, Cloud
Application Administrator or Application Administrator. This removes the approval you gave for
yourself, and says how many approvals for everyone there are:

```powershell
Connect-MgGraph -Scopes 'DelegatedPermissionGrant.ReadWrite.All','Application.Read.All'
$sp = Get-MgServicePrincipal -Filter "appId eq 'b4f17c55-b9f6-4a84-958f-afe3842da327'"
$mine = @(Get-MgOauth2PermissionGrant -Filter "clientId eq '$($sp.Id)' and principalId eq '<your object ID>'")
$mine | ForEach-Object { Remove-MgOauth2PermissionGrant -OAuth2PermissionGrantId $_.Id }
$everyone = @(Get-MgOauth2PermissionGrant -Filter "clientId eq '$($sp.Id)' and consentType eq 'AllPrincipals'")
"Removed your approvals: $($mine.Count). Approvals for everyone: $($everyone.Count)."
```

An approval for everyone (with the approval link, _Consent on behalf of your organization_ or the
IT page's setup script) isn't anyone's own, so that command leaves it. Remove it too:

```powershell
Get-MgOauth2PermissionGrant -Filter "clientId eq '$($sp.Id)' and consentType eq 'AllPrincipals'" |
  ForEach-Object { Remove-MgOauth2PermissionGrant -OAuth2PermissionGrantId $_.Id }
```

Or delete the app, with every approval of it: Entra admin center → Enterprise apps →
Cumulas Mail - Admin Setup → Properties → Delete. To keep it for later repairs, set **Assignment
required** to Yes, assign only your IT administrators under **Users and groups**, and add a
Conditional Access policy for it that requires phishing-resistant MFA or a compliant device.

#### How the check works

Nothing is offered until it's checked. In **Settings → Accounts →** (the account) **→ Sensitivity
labels and encryption**, people select **Check**. Cumulas Mail sends them a control message
without any header and a test message for each option with its header, all with the same kind of
subject (`Cumulas Mail setup check [k7q2m9xa]`, with a random code) and the same short text, which
names no option or label. An option is offered when writing mail only when its test message
arrives encrypted (or with the label) and the control doesn't, and, for **Encrypt** and **Do Not
Forward**, when the arrived message no longer has `x-cumulas-protect` (your rule removes it).
Otherwise the check says why, with a note under it:

- **Arrived without it:** nothing was applied. The note says which case it is:
  - the request reached your mail server (the header was still on the test message), but no rule
    or auto-labeling policy acted on it: missing, turned off, in test mode or simulation, not
    including the sender, too new, or without the license it needs;
  - the request was removed on the way: for a label, usually by "Cumulas Mail: ignore label
    requests from outside", because the message didn't count as from inside your organization;
    for Encrypt or Do Not Forward, usually by that rule while its template couldn't be applied;
  - the message went through the Cumulas Signatures relay and came back as outside mail (see
    [With the Cumulas Signatures relay](#with-the-cumulas-signatures-relay));
  - your organization's setup was changed from this computer too recently to tell.
- **Arrived with another label:** a different label of your organization arrived instead; the note
  also says when the control had it too, as with a label applied to all mail.
- **Couldn't tell:** another rule already encrypts or labels the control too (for example a rule
  that encrypts all internal mail, or a default label applied to all mail); the encryption rule
  doesn't remove the header; the test message arrived protected another way (S/MIME, or as a link
  to Microsoft's encrypted message portal); or it arrived encrypted, but its label couldn't be
  read.
- **Couldn't check:** the test messages didn't arrive in the Inbox within 15 minutes (an Inbox
  rule that moves them elsewhere has this effect too, and so does a message held for another try
  or sent to Junk). An earlier pass that's still valid stays, with a note that the latest test
  didn't arrive.
- **Couldn't send the test:** Microsoft 365 didn't keep the request on the test message's draft.
- **Check again to use it:** the pass is more than 90 days old, or the account's mail now goes
  through the Cumulas Signatures relay and didn't when it was checked.

Most notes for **Arrived without it** end with "Your IT team can fix this. Copy details for IT
gives them what the check found." See [Troubleshooting the check](#troubleshooting-the-check).

**How long it takes:** Microsoft says to wait at least 30 minutes after creating or changing a mail
flow rule before testing it; auto-labeling policies turn on after their simulation (several hours)
and can take up to a day more. Checks run sooner can say _Arrived without it_; when this computer
made the change, the check says so.

**Copy details for IT** copies what the last checks found: each test message's Message-ID,
whether it went through the Cumulas Signatures relay, whether Microsoft 365 counted it as
internal, what arrived, and the message trace commands to look further. It includes no message
text.

Once Cumulas Mail has read the test messages, the control included, it moves them to Deleted
Items. A passed check lasts 90 days; then the option isn't offered until people select **Check
again**, and Settings says when it was last checked. While a check runs again, the earlier pass
still counts (and a required label stays required) until the new test message is judged. The copy
in the sender's Sent Items is saved before your rules run, so it usually isn't encrypted or
labeled; Cumulas Mail shows it as "requested". A check sends at most 20 messages, under Exchange
Online's 30 messages a minute.

#### Encrypt and Do Not Forward

In Exchange Online PowerShell:

```powershell
Connect-ExchangeOnline

# The template names can differ; Get-RMSTemplate lists them.
New-TransportRule -Name "Cumulas Mail: Encrypt" `
  -ExceptIfFromScope NotInOrganization `
  -HeaderMatchesMessageHeader "x-cumulas-protect" `
  -HeaderMatchesPatterns "^encrypt$" `
  -ApplyRightsProtectionTemplate "Encrypt" `
  -RemoveHeader "x-cumulas-protect" `
  -RuleErrorAction Defer

New-TransportRule -Name "Cumulas Mail: Do Not Forward" `
  -ExceptIfFromScope NotInOrganization `
  -HeaderMatchesMessageHeader "x-cumulas-protect" `
  -HeaderMatchesPatterns "^do-not-forward$" `
  -ApplyRightsProtectionTemplate "Do Not Forward" `
  -RemoveHeader "x-cumulas-protect" `
  -RuleErrorAction Defer

# Check each rule's template: the check can't tell Do Not Forward from Encrypt.
Get-TransportRule "Cumulas Mail: Do Not Forward" | Format-List State,Mode,ExceptIfFromScope,HeaderMatchesMessageHeader,HeaderMatchesPatterns,ApplyRightsProtectionTemplate,RemoveHeader,RuleErrorAction
Get-TransportRule "Cumulas Mail: Encrypt" | Format-List State,Mode,ExceptIfFromScope,HeaderMatchesMessageHeader,HeaderMatchesPatterns,ApplyRightsProtectionTemplate,RemoveHeader,RuleErrorAction

# New rules go last. Put them above any rule that stops processing or sends
# mail out through another service (with the Cumulas Signatures relay, see
# below), Do Not Forward right after Encrypt:
Get-TransportRule | Sort-Object Priority | Format-Table Priority,Name,State,Mode,StopRuleProcessing,RouteMessageOutboundConnector -AutoSize
$n = 1   # the priority of the first such rule
Set-TransportRule "Cumulas Mail: Encrypt" -Priority $n
Set-TransportRule "Cumulas Mail: Do Not Forward" -Priority ($n + 1)
```

Or in the [Exchange admin center](https://admin.exchange.microsoft.com):

1. **Mail flow → Rules → Add a rule → Create a new rule.** Name: `Cumulas Mail: Encrypt`.
2. **Apply this rule if:** **A message header → matches these text patterns**: header
   `x-cumulas-protect`, pattern `^encrypt$`.
3. **Do the following:** **Modify the message security → Apply Office 365 Message Encryption and
   rights protection → Encrypt**. Then add an action: **Modify the message properties → remove a
   message header → `x-cumulas-protect`**.
4. **Except if:** **The sender → is external/internal → Outside the organization**.
5. Keep **Enforce**; under **Properties of this rule**, select **Defer the message if rule
   processing doesn't complete**. Finish, and turn the rule on.
6. Repeat for `Cumulas Mail: Do Not Forward`, with the pattern `^do-not-forward$` and the **Do Not
   Forward** template.
7. Move both rules above any rule that stops processing or sends mail out through another service,
   Do Not Forward right after Encrypt; after the Cumulas Signatures relay's rule only when mail
   coming back from it counts as internal (see
   [With the Cumulas Signatures relay](#with-the-cumulas-signatures-relay)).

- **Mail from your organization's addresses** (`-ExceptIfFromScope NotInOrganization`): the header
  in mail from anyone else does nothing. The rules remove the header, so recipients never see it,
  and the check relies on that. Microsoft documents this scope by the sender's address alone; the
  earlier version of this guide used `-FromScope InOrganization` (mail Microsoft 365 authenticated
  as from inside), which mail handed back by a relay doesn't always meet. Whether Exchange counts
  mail that only claims one of your addresses as from outside isn't documented, so where the
  rules go doesn't depend on it: they act on mail as it's sent, or, after the Cumulas Signatures
  relay's rule, on mail that counts as internal. If the documented meaning holds, the cost is
  that someone outside who forges one of your addresses and adds the header gets Encrypt or Do
  Not Forward applied to a message to your people; spoof filtering and DMARC still act on such
  mail.
- **Defer** (`-RuleErrorAction Defer`): when encryption fails, Exchange holds the message and
  tries again, instead of sending it unencrypted as the default, `Ignore`, does. A message that
  keeps failing waits until Exchange gives up and returns it to its sender. The usual cause is
  Message Encryption being off: see `Test-IRMConfiguration` in
  [Receiving protected mail](#receiving-protected-mail).
- **Don't add a recipient condition or exception to these rules, in either direction** (inside or
  outside the organization): people's checks send the test message to themselves, so a rule
  limited to some recipients would pass the check and miss other mail.
- Mail that another service routes only for outside recipients isn't covered by people's checks:
  send a test to such an address.
- People can confirm Do Not Forward with **Open test message** next to the check: Outlook on the
  web shows that the message can't be forwarded.
- **Licensing:** Purview Message Encryption comes with Microsoft 365 E3, E5 and Business Premium,
  among others; check your plan.

#### Sensitivity labels

1. **Create this rule first:** auto-labeling acts on mail by sender address, so without it someone
   outside could ask for a label. It removes the label request from every message that isn't
   authenticated mail from inside your organization (including mail that only pretends to come
   from one of your domains). In Exchange Online PowerShell:

   ```powershell
   Connect-ExchangeOnline

   New-TransportRule -Name "Cumulas Mail: ignore label requests from outside" `
     -ExceptIfFromScope InOrganization `
     -RemoveHeader "x-cumulas-label" `
     -Priority 0
   ```

   Or in the Exchange admin center: a new rule `Cumulas Mail: ignore label requests from outside`,
   **Apply this rule if: Apply to all messages**, **Do the following: Modify the message
   properties → remove a message header → `x-cumulas-label`**, **Except if: The sender → is
   external/internal → Inside the organization**, and move it to the top of the list. Keep it
   there: a rule above it that stops processing would hide it from the mail it matches (only
   routing rules for mail from inside, `-FromScope InOrganization`, may come before it). **Set up
   or check for your organization** keeps it first.

2. **Approve `InformationProtectionPolicy.Read`.** The [approval link](#approving-cumulas-mail-it-admins)
   includes it, or people select **Turn on** in Settings → Accounts if your settings let them
   approve it. Work and school accounts only. Cumulas Mail then lists the labels your label
   policies publish to each person (Microsoft Graph beta), with the policy's default label and
   whether a label is required. Graph gives one default and one "required" setting, not separate
   ones for email, so Cumulas Mail may apply your document defaults to mail.
3. **Who may label mail from Cumulas Mail:** everyone in your organization by default. To limit
   it, use a distribution group or mail-enabled security group (not a Microsoft 365 group). Anyone
   in scope can apply a label you set up from any mail app that can set the header, whoever the
   label is published to. **Set up or check for your organization** creates auto-labeling
   policies for everyone, and keeps the group of a policy you limited.
4. **Create an auto-labeling policy for each label** people should be able to choose in Cumulas
   Mail (each auto-labeling policy applies one label). In Security & Compliance PowerShell, which
   runs only on Windows:

   ```powershell
   Connect-IPPSSession

   # The labels and their IDs:
   Get-Label | Format-Table DisplayName, Name, Guid, ImmutableId

   $label = Get-Label -Identity "Confidential"   # the label's Name
   $sent  = ""   # optional: the ID Cumulas Mail sends (Copy details for IT), if neither of these
   $group = ""   # optional: the group from step 3
   $ids = @("$($label.Guid)", "$($label.ImmutableId)", $sent) | Where-Object { $_ } |
     ForEach-Object { $_.ToLowerInvariant() } | Select-Object -Unique
   $pattern = '(?i)^(' + ($ids -join '|') + ')$'
   $name = "Cumulas Mail: $($label.Name)"

   $policy = @{ Name = $name; ApplySensitivityLabel = "$($label.Guid)"; ExchangeLocation = "All"
     OverwriteLabel = $true; Mode = "TestWithoutNotifications" }
   if ($group) { $policy.ExchangeSenderMemberOf = $group }
   New-AutoSensitivityLabelPolicy @policy
   New-AutoSensitivityLabelRule -Policy $name -Name "$name (header)" `
     -Workload Exchange `
     -HeaderMatchesPatterns @{ "x-cumulas-label" = $pattern }

   # Adding the rule is an edit, and Microsoft says every edit needs the simulation started again:
   Set-AutoSensitivityLabelPolicy -Identity $name -StartSimulation $true `
     -AutoEnableAfter (New-TimeSpan -Hours 1)
   ```

   A policy in simulation labels nothing; this one turns itself on an hour after its simulation
   completes. To turn it on yourself once the portal shows _Simulation complete_:
   `Set-AutoSensitivityLabelPolicy -Identity $name -Mode Enable`. Names can be at most 64
   characters: for a label with a long name, Cumulas Mail names the policy
   `Cumulas Mail: label <the label's Guid>`, and its rule `Cumulas Mail: <the label's Guid> (header)`
   when `(header)` doesn't fit.

   Or in the [Microsoft Purview portal](https://purview.microsoft.com), which also works from Linux
   and macOS:
   1. **Solutions → Information Protection → Policies → Auto-labeling policies → Create
      auto-labeling policy → Custom policy**.
   2. Name: `Cumulas Mail: <the label's Name>`.
   3. Locations: **Exchange email** only. Included: **All users** (or your group from step 3).
   4. Rules: **Advanced rules → Create rule**, name `Cumulas Mail: <the label's Name> (header)`,
      condition **Header matches patterns**: header `x-cumulas-label`, pattern
      `(?i)^(<the label's Guid>|<its ImmutableId>|<the ID Cumulas Mail sends>)$`, each in
      lowercase and only once (the IDs from `Get-Label | Format-Table DisplayName,Guid,ImmutableId`
      and from **Copy details for IT**). No other conditions.
   5. Label: the label. Under **Additional settings for email**, select **Replace existing labels
      that have the same or lower priority**.
   6. Policy mode: **Run policy in simulation mode**, with **Automatically turn on policy if not
      edited** after the shortest time offered, or turn it on yourself once the simulation is
      complete.

   **Set up or check for your organization** shows these steps filled in for each label, with
   **Copy** for the names and the pattern, next to its script.

   `-OverwriteLabel $true` (the portal's "Replace existing labels") matters: a reply or forward
   can carry the original message's label, which counts as chosen by a person and isn't replaced
   by default. Cumulas Mail also refuses to send a reply whose copied label differs from the one
   the person picked.

Microsoft doesn't document the order in which mail flow rules and auto-labeling act on a message,
so watch the auto-labeling policies' simulation results for mail from outside your organization.

- **Don't add a recipient condition or exception to these policies**: people's checks send the
  test message to themselves.
- **Licensing:** sending with a label needs automatic labeling: Microsoft 365 E5, or Microsoft 365
  E3 or Business Premium with an add-on such as Microsoft Purview Suite (formerly E5 Compliance),
  for the people who send labeled mail and the administrator who manages the auto-labeling
  policies. Encrypt and Do Not Forward work with E3 and Business Premium.
- **The label's ID:** Cumulas Mail sends the label ID Microsoft Graph lists; the pattern matches
  that and both of the label's IDs in Purview, Guid and ImmutableId.
- **How long it takes:** auto-labeling policies turn on after their simulation (several hours) and
  can take up to a day more to start working; people's checks say "Arrived without it" until
  then. A default label applied to all mail makes the check for that same label say "Couldn't
  tell".
- Labels configured for S/MIME can't be applied this way.
- Replies and forwards keep the original's label when it's offered. When it isn't and your label
  policy requires a label, Cumulas Mail sends the reply or forward only once the person has chosen
  one. When your label policy asks for a reason to lower a label, people lower it in Outlook:
  Cumulas Mail can't record a reason.
- An organization's own app registration needs `InformationProtectionPolicy.Read` (delegated)
  added too.

#### With the Cumulas Signatures relay

When your organization's mail goes through the Cumulas Signatures relay to get its signature,
Microsoft 365 handles each message twice: when it's sent, where the relay's rule sends it to the
relay and stops the rules after it, and again when the relay hands it back.

**Encrypt and Do Not Forward** work either way. Until mail coming back from the relay counts as
internal, they come before the relay's rule: the message is encrypted when it's sent, and the relay
passes encrypted mail on unchanged, without a signature. Once it counts as internal, they come
after the relay's rule, so mail is signed, then encrypted. **Set up or check for your
organization** moves them when that changes.

**Sensitivity labels** need mail coming back from the relay to count as mail from inside your
organization: otherwise "Cumulas Mail: ignore label requests from outside" removes the label
requests. Other Microsoft 365 features that rely on internal mail don't work for it either (for
example, room mailboxes accepting meetings). In Cumulas Signatures, **Finish setting up** offers
"Count mail coming back from the signature server as mail from inside your organization" once your
signature service's operator allows it. It sets the inbound connector _Email signatures (from
signature server)_ to `TreatMessagesAsInternal $true` and `CloudServicesMailEnabled $false`. What
that means: Microsoft 365 doesn't spam-filter mail coming back from the signature server, the same
as mail between your own people; only mail in your organization's own domains counts as internal
this way; and anyone holding the signature server's certificate, which every organization using it
trusts, could send mail that counts as internal in your organization. To check:

```powershell
Get-InboundConnector | Where-Object Name -eq 'Email signatures (from signature server)' | Format-List ConnectorType,TreatMessagesAsInternal,CloudServicesMailEnabled
```

Microsoft doesn't document whether auto-labeling acts before the relay or after it, so a label that
encrypts may be applied before the relay sees the message, and that mail goes out unsigned.
People's checks go through the relay like any other mail, so they show whether labels work; **Copy
details for IT** says whether the test messages went through the relay and whether they counted as
internal.

The earlier version of this guide had the rules wait for `X-Signature-Stamped` and added a
"<label> after the signature" rule per label. Neither is needed now, and with the relay's earlier
connector settings they couldn't work. **Set up or check for your organization** removes the
conditions from the Encrypt and Do Not Forward rules and points each auto-labeling rule at
`x-cumulas-label` again; delete the "after the signature" rules when convenient, and "Cumulas Mail:
ignore stamped label requests from outside" once no auto-labeling rule of yours uses
`x-cumulas-label-stamped` any more (`Remove-TransportRule '<name>'`).

**Another signature or security service** in the mail path: Cumulas Mail's encryption rules come
before its rule, so it receives encrypted mail and can't sign it. Mail it routes only for outside
recipients isn't covered by people's checks: send a test to such an address. For labels, mail it
hands back must count as internal (for Exchange Online, Microsoft's guidance for add-on services
keeps internal headers on both connectors).

#### Troubleshooting the check

Work through these in order; most cases are settled by the first two.

1. **Start from Copy details for IT** (Settings → Accounts → the account → Sensitivity labels and
   encryption). Its _Path_ line says whether the test messages went through the Cumulas Signatures
   relay and whether Microsoft 365 counted them as from inside your organization when they came
   back; each item gives its Message-ID, whether the request was still on it when it arrived, and
   what arrived. It ends with the commands of step 6, filled in, and a link here.
2. **Read the note under the check:**
   - _Treats it as outside mail when it comes back_ (`relayNotInternal`): mail goes through the
     Cumulas Signatures relay and comes back as outside mail. For Encrypt and Do Not Forward, **Set
     up or check for your organization** puts the rules before the relay's rule; for labels, see
     [With the Cumulas Signatures relay](#with-the-cumulas-signatures-relay).
   - _Got the request, but no rule acted on it_ (`requestIgnored`): the rule or auto-labeling
     policy is missing, turned off, in test mode or simulation, doesn't include the sender, is too
     new, or lacks a license. Steps 3 to 5.
   - _The request was removed on the way_ (`requestRemoved`): a rule removed the header. For a
     label, usually "Cumulas Mail: ignore label requests from outside" because the message didn't
     count as from inside your organization (a relay or another service handed it back as outside
     mail); for Encrypt or Do Not Forward, usually that rule while its template couldn't be
     applied; or a rule of yours that removes headers. The message trace (step 6) shows which.
   - _Your organization's setup changed … before this check_ (`tooSoon`): wait 30 minutes or more
     for rules, a day for auto-labeling policies, then check again.
   - _It arrived labeled … instead_ (`otherLabel`): another policy or a default label applied a
     different label; with "puts that label on all mail", one applies to every message. Look at
     your label policies' default labels and other auto-labeling policies.
   - _Arrived protected another way_ (`otherProtection`): S/MIME, or a link to Microsoft's
     encrypted message portal (Office 365 Message Encryption without rights protection). Open the
     test message to see; check the rule's template.
   - _Couldn't read its label_ (`labelUnreadable`): it arrived encrypted, but without a label of
     your organization that Cumulas Mail could read. Open the test message in Outlook on the web
     to see its label.
3. **Mail flow rules**, in Exchange Online PowerShell:

   ```powershell
   Connect-ExchangeOnline
   Get-TransportRule | Sort-Object Priority | Format-Table Priority,Name,State,Mode,StopRuleProcessing,FromScope,ExceptIfFromScope,RouteMessageOutboundConnector,WhenChanged -AutoSize
   Get-TransportRule "Cumulas Mail: Encrypt" | Format-List ExceptIfFromScope,HeaderMatchesMessageHeader,HeaderMatchesPatterns,HeaderContainsMessageHeader,ApplyRightsProtectionTemplate,RemoveHeader,RuleErrorAction
   ```

   "Cumulas Mail: Encrypt" and "Cumulas Mail: Do Not Forward" must be `Enabled` and `Enforce`,
   changed at least 30 minutes before the check, with no rule above them that stops processing or
   sends mail out (`RouteMessageOutboundConnector`), except the Cumulas Signatures relay's rule
   when mail coming back from it counts as internal. "Cumulas Mail: ignore label requests from
   outside" must come before any rule that stops processing.

4. **Message Encryption**: `Get-IRMConfiguration | Format-List AzureRMSLicensingEnabled,InternalLicensingEnabled`
   (both `True`), `Test-IRMConfiguration -Sender you@contoso.com -Recipient you@contoso.com` (it
   must end with `OVERALL RESULT: PASS`), and `Get-RMSTemplate | Format-Table Name` (the templates
   your rules use must be listed).
5. **Auto-labeling policies**, in Security & Compliance PowerShell:

   ```powershell
   Connect-IPPSSession
   Get-AutoSensitivityLabelPolicy -Identity "Cumulas Mail: Confidential" -IncludeTestModeResults $true | Format-List Name,Mode,Enabled,*TestMode*
   Get-AutoSensitivityLabelRule | Where-Object Name -like 'Cumulas Mail*' | Format-List Name,ParentPolicyName,Disabled,Workload,HeaderMatchesPatterns
   Get-Label | Format-Table DisplayName, Guid, ImmutableId
   ```

   The policy must be in `Enable` mode after a completed simulation, and on (`Enabled` `True`);
   the pattern must match the ID in "Sent: x-cumulas-label: …" in **Copy details for IT**. A
   policy limited to a group must include the sender. The people who send labeled mail need a
   license that includes automatic labeling (see [Sensitivity labels](#sensitivity-labels)).

6. **Message trace**, if it's still unclear, with the commands from **Copy details for IT**:

   ```powershell
   $me = "you@contoso.com"
   Get-MessageTraceV2 -SenderAddress $me -RecipientAddress $me -Subject 'Cumulas Mail setup check' -SubjectFilterType StartsWith -StartDate (Get-Date).AddDays(-2) -EndDate (Get-Date) |
     ForEach-Object { Get-MessageTraceDetailV2 -MessageTraceId $_.MessageTraceId -RecipientAddress $me } | Format-List Date,Event,Detail
   Get-MailDetailTransportRuleReport -MessageId '<Message-ID>' -EventType TransportRuleHits,TransportRuleActionHits -StartDate (Get-Date).AddDays(-2) -EndDate (Get-Date) | Format-List
   ```

   Look at which rules matched on each pass, whether the message went out through the connector
   _Email signatures (to signature server)_ and came back, whether "Cumulas Mail: Encrypt"
   matched, and whether "Cumulas Mail: ignore label requests from outside" matched (the message
   came back as outside mail). No rule hits anywhere: check that the rules' `SetAuditSeverity`
   isn't `DoNotAudit`.

7. **A check that never arrives** (_Couldn't check_): look in Junk; in the message trace for a
   `Defer` event (an Encrypt or Do Not Forward rule holds the message while encryption fails: see
   step 4); and for a non-delivery report in the sender's Inbox.
8. **With the Cumulas Signatures relay**, the connectors:
   `Get-InboundConnector | Where-Object Name -eq 'Email signatures (from signature server)' | Format-List ConnectorType,TreatMessagesAsInternal,CloudServicesMailEnabled,TlsSenderCertificateName`.
   `TreatMessagesAsInternal` `True` means mail coming back counts as internal.
9. **Rarely:** `Get-TransportConfig | Format-List JournalingReportNdrTo` must not be the person's
   own mailbox.
10. After any change, wait (30 minutes or more for rules, up to a day for auto-labeling policies),
    then select **Check again**.

#### Removing Cumulas Mail's rules and policies

Remove the auto-labeling policies first, so that nothing from outside can ask for a label while
the rule that removes those requests is gone. In Security & Compliance PowerShell:

```powershell
Connect-IPPSSession
Get-AutoSensitivityLabelPolicy | Where-Object Name -like 'Cumulas Mail: *' |
  ForEach-Object { Remove-AutoSensitivityLabelPolicy -Identity $_.Name }
```

Microsoft 365 takes a while to delete them: until then they show `PendingDeletion` as their mode.
Once `Get-AutoSensitivityLabelPolicy` lists none of them, in Exchange Online PowerShell:

```powershell
Connect-ExchangeOnline
Remove-TransportRule 'Cumulas Mail: Encrypt'
Remove-TransportRule 'Cumulas Mail: Do Not Forward'
# Only now that the auto-labeling policies are gone:
Remove-TransportRule 'Cumulas Mail: ignore label requests from outside'
```

Rules left from the earlier relay recipe ("Cumulas Mail: … after the signature" and "Cumulas
Mail: ignore stamped label requests from outside") go the same way. In the portals: Microsoft
Purview → **Solutions → Information Protection → Policies → Auto-labeling policies**, then the
Exchange admin center → **Mail flow → Rules**, deleting each one whose name starts with
`Cumulas Mail:`. Then remove the approvals of Cumulas Mail - Admin Setup, yours and any for
everyone, or the app (see [Approval and security](#approval-and-security)).

Cumulas Mail's encryption and label options then stop working for everyone, but people's passed
checks keep offering them until their next check or for 90 days, so tell them.

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

**Set up or check for your organization** still signs administrators in to
the published **Cumulas Mail - Admin Setup**, whichever registration people
use for mail. Organizations that don't want to approve it use the PowerShell
script the dialog shows, or a build with their own setup registration
(`CUMULAS_MS_SETUP_CLIENT_ID`, see
[Registering the setup app](#registering-the-setup-app-publisher)).

## Troubleshooting

| What the user sees                                                                                                                      | Meaning                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| --------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| "Your organization needs to approve Cumulas Mail" (AADSTS65001, 90093, 90094, 900941, 900981)                                           | The organization requires an administrator to approve the app. The app shows the approval link with **Email IT** and **Open approval page**, and offers **Try with mail only**.                                                                                                                                                                                                                                                                       |
| Same screen, with **Request approval** as the main button (AADSTS90095)                                                                 | The organization has the admin consent workflow on: the user can request approval on Microsoft's page, then select **Try again** once IT has approved.                                                                                                                                                                                                                                                                                                |
| "Cumulas Mail doesn't have permission yet" (AADSTS65004)                                                                                | The permission prompt was declined. Microsoft also sends this when the user selects "Back to app" after requesting approval, so try again once IT has approved.                                                                                                                                                                                                                                                                                       |
| "You haven't been given access to Cumulas Mail" (AADSTS50105)                                                                           | The organization requires assignment to the app: an administrator assigns the user under Enterprise apps → Cumulas Mail → Users and groups.                                                                                                                                                                                                                                                                                                           |
| "Your organization has blocked Cumulas Mail" (AADSTS7000112)                                                                            | The app is disabled in the organization, for example after a consent request was blocked: Enterprise apps → Cumulas Mail → Properties → Enabled for users to sign in.                                                                                                                                                                                                                                                                                 |
| "Your organization isn't set up for Cumulas Mail yet" (AADSTS65005)                                                                     | The organization is missing something the app needs; approving the app usually fixes it. If it doesn't, [let Cumulas know](https://github.com/cumulas-inc/email-client/issues).                                                                                                                                                                                                                                                                       |
| "Your organization's Microsoft 365 is missing a service" (AADSTS650052)                                                                 | The organization hasn't subscribed to or enabled a service the app needs, usually Exchange Online. Approving the app doesn't help.                                                                                                                                                                                                                                                                                                                    |
| "Set up your sign-in security info first" (AADSTS53004, 53010)                                                                          | The user must finish setting up multifactor authentication or other security info, which Microsoft couldn't do during this sign-in.                                                                                                                                                                                                                                                                                                                   |
| "Your organization's sign-in rules …" or "Your organization only allows apps it has approved" (AADSTS53000–53003, 53011, 530000–530999) | A Conditional Access policy blocked the sign-in; the Entra sign-in logs show which. A sign-in through the browser on Linux may not meet policies that require a compliant or managed device or an approved client app.                                                                                                                                                                                                                                |
| "Calendar needs permission", or "Needs permission: …" in Settings → Accounts                                                            | The feature's optional permission wasn't granted. **Turn on** asks for it; **Ask IT** shows the approval link. An approval given later is picked up within a couple of hours.                                                                                                                                                                                                                                                                         |
| "Microsoft 365 refused access to your calendar" (or contacts, tasks)                                                                    | The permission is granted, but Graph refused the feature (403), often because the mailbox's license doesn't include it. The app checks again daily, or at once with **Check again**; **Email IT** asks IT to look.                                                                                                                                                                                                                                    |
| "Sign-in timed out"                                                                                                                     | Microsoft's page didn't send the browser back within 5 minutes, often because it showed an error of its own. **Microsoft said I need admin approval** and **Microsoft showed an error code** lead to the matching screen.                                                                                                                                                                                                                             |
| "Signing in without instant updates" (in the browser)                                                                                   | The push service's scope needs an administrator's approval or was declined. The sign-in continues in a new tab without it; see [Instant updates](#what-users-see-before-it-is-set-up).                                                                                                                                                                                                                                                                |
| "Sign in again to keep … up to date"                                                                                                    | The refresh token expired or was revoked (password change, admin action, or a Conditional Access policy requiring interaction). When Microsoft says it's one of the blocks above, the account shows that reason instead, with its AADSTS code.                                                                                                                                                                                                        |
| Sign-in never returns to the app                                                                                                        | The browser could not reach `http://localhost:<port>`; check that nothing blocks loopback connections. Cancel and try again. If Microsoft's page shows an error, use **Microsoft showed an error code**.                                                                                                                                                                                                                                              |
| "Protected message" in the reading pane                                                                                                 | The message is encrypted with Microsoft Purview (a sensitivity label, Encrypt or Do Not Forward). Cumulas Mail can't open it; **Open in Outlook on the web** does. See [Sensitivity labels and encryption](#sensitivity-labels-and-encryption-it-admins).                                                                                                                                                                                             |
| A check in Settings says _Arrived without it_, _Arrived with another label_ or _Couldn't tell_                                          | See [Troubleshooting the check](#troubleshooting-the-check). The note under the check says which case it is.                                                                                                                                                                                                                                                                                                                                          |
| A check in Settings says _Couldn't check_                                                                                               | The test messages didn't arrive in the Inbox within 15 minutes. See [Troubleshooting the check](#troubleshooting-the-check), step 7.                                                                                                                                                                                                                                                                                                                  |
| "Microsoft 365 refused access to your sensitivity labels"                                                                               | The permission is granted, but Graph refused the label list (403), often for licensing.                                                                                                                                                                                                                                                                                                                                                               |
| **Set up or check for your organization** says an administrator must approve Cumulas Mail - Admin Setup                                 | Your consent settings don't let administrators approve it for themselves. A Global Administrator, Privileged Role Administrator, Cloud Application Administrator or Application Administrator can approve it with the link the dialog shows; the [setup script on the page for IT administrators](https://flatpak.cumulas.co/admin-consent/#setup-app) limits it to the administrators you name. See [Approval and security](#approval-and-security). |
| It says Cumulas Mail - Admin Setup isn't fully registered, or that Microsoft 365 didn't accept the sign-in                              | Use the PowerShell script the dialog shows, and send **Copy setup details** to Cumulas.                                                                                                                                                                                                                                                                                                                                                               |
| It says your account can't read or change mail flow rules, or auto-labeling policies                                                    | Sign in as an Exchange Administrator (mail flow rules), a Compliance Administrator or member of Information Protection Admins (auto-labeling policies), or a Global Administrator. Activate a role from Privileged Identity Management first; Exchange can take a while to apply it.                                                                                                                                                                  |
| It says a license is missing                                                                                                            | Sending with a label needs automatic labeling, for the people who send labeled mail and the administrator: see [Sensitivity labels](#sensitivity-labels). Encrypt and Do Not Forward don't need it.                                                                                                                                                                                                                                                   |
| It says Microsoft 365 treats relay mail as outside mail                                                                                 | Finish setting up in Cumulas Signatures: see [With the Cumulas Signatures relay](#with-the-cumulas-signatures-relay).                                                                                                                                                                                                                                                                                                                                 |
| It leaves a rule or policy alone because of settings Cumulas Mail doesn't set                                                           | Remove those settings in the Exchange admin center or the Microsoft Purview portal (or delete the rule or policy), then select **Read again**.                                                                                                                                                                                                                                                                                                        |

After an administrator approves Cumulas Mail with the published registration's
link, Microsoft brings their browser back to
<https://flatpak.cumulas.co/admin-consent/>, which says whether it worked and
what to tell people. With an organization's own registration the link returns
to `http://localhost` instead, which shows a "can't connect" page; the approval
is recorded at that point, and the user can select **Try again**.

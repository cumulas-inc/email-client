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
on behalf of the whole organisation) and are ones Microsoft lets users grant
themselves by default:

| Permission                                | Used for                                           |
| ----------------------------------------- | -------------------------------------------------- |
| `openid`, `profile`, `email`, `User.Read` | Signing in and showing your name and address       |
| `offline_access`                          | Staying signed in (refresh token)                  |
| `Mail.ReadWrite`                          | Reading, flagging, moving and deleting mail        |
| `Mail.Send`                               | Sending mail                                       |
| `MailboxSettings.Read`                    | Time zone and mailbox preferences                  |
| `Calendars.ReadWrite`                     | Calendar views, creating events, meeting responses |
| `Contacts.ReadWrite`                      | People and address autocomplete                    |
| `Tasks.ReadWrite`                         | Microsoft To Do lists and tasks                    |

## "Need admin approval"

Whether a user may grant these permissions is decided by **their
organisation's consent settings**, not by the app. Organisations commonly:

- allow users to consent to any app;
- allow consent only for apps from **verified publishers**, and only for
  permissions the admins classified as low impact; or
- require an administrator to approve every app.

In the last two cases the user sees "Need admin approval" in the browser. The
app detects this and shows a screen with an **admin consent link** and a
ready-to-send email to IT. An administrator opening the link can approve the
app once for the whole organisation. If the organisation has the admin consent
workflow enabled, users can also request approval directly from Microsoft's
page.

Personal Microsoft accounts (Outlook.com, Hotmail, Live) never need admin
approval.

## Registering the app (publisher)

The published app needs one multi-tenant app registration, whose
**Application (client) ID** is compiled in with
`CUMULAS_MS_CLIENT_ID=<id>` (see the Flatpak manifest). The client ID is not a
secret.

1. In the [Microsoft Entra admin center](https://entra.microsoft.com), go to
   **App registrations → New registration**.
2. Name: `Cumulas Mail`. Supported account types: **Accounts in any
   organizational directory and personal Microsoft accounts**.
3. Redirect URI: platform **Public client/native (mobile & desktop)**, value
   `http://localhost` (any port is accepted for localhost).
4. **API permissions → Add a permission → Microsoft Graph → Delegated**: add
   the permissions in the table above. Do not grant admin consent for other
   organisations; each organisation decides for itself.
5. **Branding & properties**: set the publisher domain to `cumulas.co`, then
   complete **publisher verification** with a Microsoft AI Cloud Partner
   Program ID. Verification is what lets users in organisations that only
   allow verified publishers sign in without an administrator.

## Using your organisation's own registration (IT admins)

Organisations that prefer to control the app themselves can register it in
their own tenant (single-tenant, same redirect URI and permissions as above),
grant admin consent once, and have users enter the client ID and tenant ID
under **Settings → Microsoft app registration**.

## Troubleshooting

| Message                                        | Meaning                                                                                                   |
| ---------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| Need admin approval / AADSTS65001, AADSTS90094 | The organisation requires an administrator to approve the app; use the admin consent link the app shows.  |
| AADSTS65004                                    | The permission prompt was declined. Sign in again and accept.                                             |
| "Sign in again to keep … up to date"           | The refresh token expired or was revoked (password change, admin action, or a Conditional Access policy). |
| Sign-in window never returns to the app        | The browser could not reach `http://localhost:<port>`; check that nothing blocks loopback connections.    |

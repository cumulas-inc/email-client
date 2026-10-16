# Flatpak packaging

App ID: `co.cumulas.Mail` (runtime `org.gnome.Platform` 50, which provides
WebKitGTK 4.1 for Tauri).

## Build locally

```sh
flatpak install flathub org.gnome.Sdk//50 org.gnome.Platform//50 \
  org.freedesktop.Sdk.Extension.rust-stable//25.08 org.freedesktop.Sdk.Extension.node22//25.08
./generate-sources.sh
git clone https://github.com/flathub/shared-modules   # the tray library's module
flatpak-builder --user --install --force-clean build-dir co.cumulas.Mail.yml
flatpak run co.cumulas.Mail
```

`shared-modules/` (Flathub's [shared modules](https://github.com/flathub/shared-modules),
for libayatana-appindicator) is a git submodule in the Flathub repository; here
it's cloned next to the manifest and ignored by git. CI clones a pinned commit.

CI builds the same manifest on every pull request (see `.github/workflows/ci.yml`).

## Sandbox permissions

| Permission                                                                 | Why                                                                    |
| -------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| `--share=network`                                                          | Mail, calendar and Microsoft sign-in servers, and the push service     |
| `--socket=wayland`, `--socket=fallback-x11`, `--share=ipc`, `--device=dri` | Display and GPU-accelerated WebKit rendering                           |
| `--talk-name=org.kde.StatusNotifierWatcher`                                | The tray icon, where the desktop shows tray icons (StatusNotifierItem) |

No filesystem, keyring or notification-server access is requested: `oo7`
keeps secrets encrypted in a keyring file in the app's data folder, with a key
from the Secret portal; notifications go through the Notification portal,
running in the background and starting at login through the Background portal
(which writes the autostart entry on the host), file pickers through the
FileChooser portal and links through the OpenURI portal. The
single-instance check owns `co.cumulas.Mail.SingleInstance`, which the app id
allows without a permission.

### For the Flathub review

- **Push service.** Microsoft accounts hear about changes to the Inbox and the
  calendar through the Cumulas push service at `sync.cumulas.co` (on by
  default; Settings → General → Sync turns it off). The metainfo description
  and [PRIVACY.md](../../PRIVACY.md) disclose it.
- **Background.** Closing the window keeps the app running (on by default,
  through the Background portal); start at login is off until the user turns
  it on.
- **AI disclosure.** Flathub's Generative AI policy requires one; see
  [Submitting to Flathub](#submitting-to-flathub).

## Submitting to Flathub

This app was written with an AI agent (Claude): its code, documentation,
packaging and artwork. That includes `co.cumulas.Mail.yml`, the metainfo, the
desktop file, the app and tray icons (`public/icon.svg` and
`src-tauri/icons/`), and the screenshots in `docs/screenshots/`, which an
AI-written script takes of the app with demo data. Flathub's
[Generative AI policy](https://docs.flathub.org/docs/for-app-authors/requirements#generative-ai-policy)
sets three rules for the submission:

- **A person writes the Flathub manifest.** Flathub manifests must not contain
  AI-generated or AI-assisted content, even when it's disclosed.
  `co.cumulas.Mail.yml` is for local and CI builds only: read it for the build
  steps and permissions, but don't copy it, and don't have an AI tool write or
  edit the manifest for Flathub.
- **The submission discloses the AI-generated material**: the code,
  documentation, packaging and artwork (the app and tray icons and the
  screenshots) in this repository, naming the parts and their approximate
  extent (the commit history shows which commits an AI agent wrote).
- **No AI agent opens the pull request** or writes its commit messages,
  description, review comments or replies, and nobody asks for an AI-agent
  review.

Following these rules doesn't guarantee acceptance. Reviewers decide on
disclosed AI-generated material at their discretion, and may reject a
submission, even without further review, for the extent or role of that
material or for concerns about its review, quality or maintainability. Since
an AI agent wrote all of this app, that is the most likely reason for a
rejection; know it before you start.

Then, by hand:

1. Make the screenshot URLs in the metainfo publicly reachable.
2. Register the Microsoft Entra app (see `docs/microsoft-365.md`) and get
   publisher verification.
3. Tag a release, and write the manifest for Flathub yourself: a `git` source
   pinned to the tag and commit, the generated `cargo-sources.json` and
   `node-sources.json`, and the app's client ID in `CUMULAS_MS_CLIENT_ID`. It
   needs Flathub's shared modules (for libayatana-appindicator) as a git
   submodule: `git submodule add https://github.com/flathub/shared-modules`,
   checked out at the commit `.github/workflows/ci.yml` pins.
4. Open a pull request against
   [flathub/flathub](https://github.com/flathub/flathub) (branch `new-pr`) with
   the manifest, the two sources files and the submodule, and the AI
   disclosure in its description.
5. To get the "verified" badge, publish the token Flathub gives you at
   `https://cumulas.co/.well-known/org.flathub.VerifiedApps.txt`.

Before submitting, validate your manifest (in your flathub/flathub checkout)
and the metainfo (here):

```sh
flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest co.cumulas.Mail.yml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder appstream co.cumulas.Mail.metainfo.xml
```

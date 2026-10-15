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

No filesystem, keyring or notification-server access is requested: secrets go
through the Secret portal (`oo7`), notifications through the Notification
portal, running in the background and starting at login through the
Background portal (which writes the autostart entry on the host), file pickers
through the FileChooser portal and links through the OpenURI portal. The
single-instance check owns `co.cumulas.Mail.SingleInstance`, which the app id
allows without a permission.

### For the Flathub review

- **Push service.** Microsoft accounts get new-mail alerts through the Cumulas
  push service at `sync.cumulas.co` (on by default; Settings → General → Sync
  turns it off). The metainfo description and [PRIVACY.md](../../PRIVACY.md)
  disclose it.
- **Background.** Closing the window keeps the app running (on by default,
  through the Background portal); start at login is off until the user turns
  it on.

## Submitting to Flathub

1. Make the screenshot URLs in the metainfo publicly reachable.
2. Register the Microsoft Entra app (see `docs/microsoft-365.md`), get publisher
   verification, and set `CUMULAS_MS_CLIENT_ID` in the manifest.
3. Tag a release, then open a pull request against
   [flathub/flathub](https://github.com/flathub/flathub) (branch `new-pr`) with
   this manifest using a `git` source pinned to the tag and commit, plus the
   generated `cargo-sources.json` and `node-sources.json`.
4. To get the "verified" badge, publish the token Flathub gives you at
   `https://cumulas.co/.well-known/org.flathub.VerifiedApps.txt`.

Validate before submitting:

```sh
flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest co.cumulas.Mail.yml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder appstream co.cumulas.Mail.metainfo.xml
```

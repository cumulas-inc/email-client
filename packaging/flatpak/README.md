# Flatpak packaging

App ID: `co.cumulas.Mail` (runtime `org.gnome.Platform` 50, which provides
WebKitGTK 4.1 for Tauri).

## Build locally

```sh
flatpak install flathub org.gnome.Sdk//50 org.gnome.Platform//50 \
  org.freedesktop.Sdk.Extension.rust-stable//25.08 org.freedesktop.Sdk.Extension.node22//25.08
./generate-sources.sh
flatpak-builder --user --install --force-clean build-dir co.cumulas.Mail.yml
flatpak run co.cumulas.Mail
```

CI builds the same manifest on every pull request (see `.github/workflows/ci.yml`).

## Sandbox permissions

| Permission                                                                 | Why                                          |
| -------------------------------------------------------------------------- | -------------------------------------------- |
| `--share=network`                                                          | Mail, calendar and Microsoft sign-in servers |
| `--socket=wayland`, `--socket=fallback-x11`, `--share=ipc`, `--device=dri` | Display and GPU-accelerated WebKit rendering |
| `--talk-name=org.freedesktop.Notifications`                                | New-mail notifications                       |

No filesystem or keyring access is requested: secrets go through the Secret
portal (`oo7`), file pickers through the FileChooser portal and links through
the OpenURI portal.

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

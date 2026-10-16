# Flatpak packaging

App ID: `co.cumulas.Mail` (runtime `org.gnome.Platform` 50, which provides
WebKitGTK 4.1 for Tauri).

## Build locally

```sh
flatpak install flathub org.gnome.Sdk//50 org.gnome.Platform//50 \
  org.freedesktop.Sdk.Extension.rust-stable//25.08 org.freedesktop.Sdk.Extension.node22//25.08
./generate-sources.sh
./fetch-shared-modules.sh   # the tray library's module
flatpak-builder --user --install --force-clean build-dir co.cumulas.Mail.yml
flatpak run co.cumulas.Mail
```

`shared-modules/` (Flathub's [shared modules](https://github.com/flathub/shared-modules),
for libayatana-appindicator) is a git submodule in the Flathub repository; here
`fetch-shared-modules.sh` fetches it next to the manifest (ignored by git), at
the commit it pins.

CI builds the same manifest on every pull request (see `.github/workflows/ci.yml`),
with `build-repo.sh`; the [self-hosted repository](#self-hosted-repository-flatpakcumulasco)
is built the same way.

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
2. Tag a release, and write the manifest for Flathub yourself: a `git` source
   pinned to the tag and commit, and the generated `cargo-sources.json` and
   `node-sources.json`. It needs no client IDs: the app has the published
   Microsoft registration's and Cumulas Signatures' built in (see
   `docs/microsoft-365.md`). It needs Flathub's shared modules (for
   libayatana-appindicator) as a git submodule:
   `git submodule add https://github.com/flathub/shared-modules`, checked out
   at the commit `fetch-shared-modules.sh` pins.
3. Open a pull request against
   [flathub/flathub](https://github.com/flathub/flathub) (branch `new-pr`) with
   the manifest, the two sources files and the submodule, and the AI
   disclosure in its description.
4. To get the "verified" badge, publish the token Flathub gives you at
   `https://cumulas.co/.well-known/org.flathub.VerifiedApps.txt`.

Before submitting, validate your manifest (in your flathub/flathub checkout)
and the metainfo (here):

```sh
flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest co.cumulas.Mail.yml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder appstream co.cumulas.Mail.metainfo.xml
```

## Self-hosted repository (flatpak.cumulas.co)

`.github/workflows/flatpak-repo.yml` publishes Cumulas Mail to its own Flatpak
repository at <https://flatpak.cumulas.co>, served by GitHub Pages: this
manifest built for x86_64 and aarch64, branch `stable`, signed with the
Cumulas key. Flathub's Generative AI policy covers submissions to Flathub, not
this repository, so it uses `co.cumulas.Mail.yml` as it is.

### What's published, and when

- **Every `v*` tag** publishes. Add the release to
  `co.cumulas.Mail.metainfo.xml` first: software centers show its version and
  notes, and the workflow warns when they don't match the tag. A manual run
  (Actions → Flatpak repository → Run workflow) publishes the default branch
  only with **Publish** ticked.
- **Dry runs**: pull requests that touch `packaging/flatpak/`,
  `.github/actions/` or the workflow, and manual runs without Publish, run the
  same steps with a throwaway key and deploy nothing. A manual dry run of the
  default branch also checks the signing key: it imports the secret into a
  scratch keyring, checks it against the committed public key with a test
  signature, and removes it, signing nothing.
- **The signing key** is a secret of the `flatpak-signing` environment, which
  only the default branch and release tags can use: the job that signs a
  publish and the key check above. Pull requests, other branches and dry runs
  never get it.
- **Each run** builds both architectures (`build-repo.sh`), then:
  1. mirrors the published repository with its history (`repo/mirror.sh`);
  2. adds the new builds on top of it, signed (`repo/add-builds.sh`);
  3. generates the appstream data software centers read and static deltas,
     prunes old builds and signs the summary (`repo/update-repo.sh`);
  4. assembles the site (`repo/assemble-site.sh`);
  5. installs the app from a local copy of the site on both architectures, from
     the `.flatpakrepo` and from the `.flatpakref` (`repo/smoke-test.sh`);
  6. and only then deploys the site to Pages.

  `repo/test.sh` runs steps 1–5 against fake builds, as part of every run.

- **Kept**: the last 3 builds of each architecture (`KEEP_BUILDS` in
  `repo/common.sh`), so people can go back to an earlier one:
  `flatpak remote-info --log cumulas co.cumulas.Mail` lists them and
  `flatpak update --commit=COMMIT co.cumulas.Mail` installs one. Debug info
  (`.Debug`) isn't published, to stay well under GitHub Pages' 1 GB limit.

| On the site                  | What it is                                   |
| ---------------------------- | -------------------------------------------- |
| `repo/`                      | The Flatpak (ostree) repository              |
| `cumulas.flatpakrepo`        | Adds the remote (`flatpak remote-add`)       |
| `co.cumulas.Mail.flatpakref` | Installs the app in one click                |
| `cumulas-flatpak.gpg`        | The public key the repository is signed with |
| `index.html`, `logo.svg`     | The install page (`repo/index.html`)         |

### Installing

People open <https://flatpak.cumulas.co> and click Install, which opens
`co.cumulas.Mail.flatpakref` in their software center, or run:

```sh
flatpak remote-add --if-not-exists cumulas https://flatpak.cumulas.co/cumulas.flatpakrepo
flatpak install cumulas co.cumulas.Mail
```

The GNOME runtime comes from Flathub. The `.flatpakref` names it
(`RuntimeRepo`), so Flatpak offers to add Flathub when it's missing; the
`.flatpakrepo` names it too, but Flatpak reads that key from `.flatpakref`
files only, so the commands need Flathub set up already. Most distributions do
that with Flatpak; otherwise:

```sh
flatpak remote-add --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
```

### Alongside Flathub

If Flathub accepts the app, both will offer `co.cumulas.Mail`, branch
`stable`, as separate remotes (`cumulas` and `flathub`) with separate builds
and keys: Flathub builds from the manifest a person writes for it, this
repository from `co.cumulas.Mail.yml`. People install from one or the other
(`flatpak install` asks which remote when both have the app), and updates come
from the remote it was installed from (`flatpak info co.cumulas.Mail` shows it
as the origin). To switch, uninstall and install from the other remote; the
app's data in `~/.var/app/co.cumulas.Mail` stays.

### Setting it up

Once, in this order, by an owner of the repository and of cumulas.co's DNS:

1. **Generate the signing key** on a trusted computer:

   ```sh
   export GNUPGHOME="$(mktemp -d)"
   gpg --batch --passphrase '' --quick-generate-key \
     'Cumulas Flatpak Repository <flatpak@cumulas.co>' rsa4096 sign never
   gpg --export >packaging/flatpak/repo/cumulas-flatpak.gpg
   gpg --armor --export-secret-keys >cumulas-flatpak-private.asc
   ```

   No passphrase, since the workflow can't type one: the secret store protects
   it. No expiry, since an expired key stops updates for everyone (if you set
   one, extend it and commit the public key again before it lapses). Keep
   `cumulas-flatpak-private.asc` and the revocation certificate in
   `$GNUPGHOME/openpgp-revocs.d/` in a password manager or offline, then delete
   them and `$GNUPGHOME`.

2. **Create the `flatpak-signing` environment, and store the private key in
   it**: repository Settings → Environments → New environment,
   `flatpak-signing`. Under Deployment branches and tags, choose **Selected
   branches and tags** and add two rules: the branch `main`, and the tag
   `v*.*.*`. Then Environment secrets → Add environment secret:
   `FLATPAK_GPG_PRIVATE_KEY`, with the whole `.asc` file as the value, or:

   ```sh
   gh secret set FLATPAK_GPG_PRIVATE_KEY --env flatpak-signing \
     --repo cumulas-inc/email-client <cumulas-flatpak-private.asc
   ```

   Only jobs that name the environment get its secret, and GitHub runs them
   only for the default branch and release tags, whatever a pull request or
   another branch's workflow says.

   > **You already have a repository secret `FLATPAK_GPG_PRIVATE_KEY`: move
   > it.** GitHub never shows a secret's value, so add the environment secret
   > from your saved `cumulas-flatpak-private.asc` as above, then delete the
   > repository secret: Settings → Secrets and variables → Actions →
   > Repository secrets, or
   > `gh secret delete FLATPAK_GPG_PRIVATE_KEY --repo cumulas-inc/email-client`.
   > The workflow reads only the environment's, and a repository secret left
   > behind is readable by any workflow run that anyone with write access can
   > start.

3. **Protect release tags**: repository Settings → Rules → Rulesets → New
   ruleset → New tag ruleset. Name it `Release tags`, set Enforcement status
   to Active, add the target pattern `v*`, tick **Restrict creations**,
   **Restrict updates** and **Restrict deletions**, and add the Repository
   admin and Maintain roles to the bypass list. Then only maintainers can push, move or
   delete the tags that publish and that may use the signing key. The
   environment trusts the default branch too: keep `main` protected, with
   changes through reviewed pull requests.
4. **Commit the public key**, `packaging/flatpak/repo/cumulas-flatpak.gpg`, in
   a pull request: its dry run checks the file. Publishing fails while the file
   is missing, empty or doesn't match the secret.
5. **Verify cumulas.co for GitHub Pages** for the organization, so that no
   other GitHub account can serve flatpak.cumulas.co (say, while Pages is off
   here): organization Settings → Pages → Add a domain → `cumulas.co`, add the
   TXT record it shows (`_github-pages-challenge-cumulas-inc.cumulas.co`) and
   click Verify.
6. **DNS**: a CNAME record from `flatpak.cumulas.co` to `cumulas-inc.github.io`.
7. **Pages**: repository Settings → Pages → Build and deployment → Source:
   **GitHub Actions**. Custom domain: `flatpak.cumulas.co`, Save; once the DNS
   check passes and the certificate is issued, tick **Enforce HTTPS**. With
   GitHub Actions as the source, this setting is the custom domain: the site
   has no `CNAME` file.
8. **Let tags deploy**: repository Settings → Environments → `github-pages` →
   Deployment branches and tags → add a tag rule, `v*.*.*`. Pages allows the
   default branch only at first, which would block every tag's deploy.
9. **Check the key, then try it**: Actions → Flatpak repository → Run
   workflow, on `main`, without Publish. That dry run's "Check the signing
   key" job tells you whether the secret in `flatpak-signing` is the committed
   key and signs as clients will verify, before a tag depends on it (run it
   again whenever you change the secret). Then push the next release tag.
   Check <https://flatpak.cumulas.co> and install from it.

### Rotating the signing key

Rotate it if it may have leaked, or to replace it. Flatpak can't hand people a
new key: each of them has to import it, so plan for it.

1. Generate a new key as in [Setting it up](#setting-it-up), step 1.
2. Commit the new `cumulas-flatpak.gpg` in a pull request, and when it's merged,
   replace the `FLATPAK_GPG_PRIVATE_KEY` secret of the `flatpak-signing`
   environment. (Publishing fails while the two don't match.)
3. Publish. From then on the summary and new builds are signed with the new
   key, and new installs get it from the `.flatpakrepo` and `.flatpakref`.
4. Tell people (release notes, cumulas.co): until they import the new key,
   updates from this repository fail with a GPG signature error, in software
   centers too. To import it, checking its fingerprint against the one on
   <https://flatpak.cumulas.co>:

   ```sh
   curl -fsSLO https://flatpak.cumulas.co/cumulas-flatpak.gpg
   gpg --show-keys cumulas-flatpak.gpg
   flatpak remote-modify --gpg-import=cumulas-flatpak.gpg cumulas   # add --user for a per-user remote
   ```

   That adds the new key to the old one. If the old key leaked, remove and add
   the remote instead, so it's trusted no more (installed apps stay):

   ```sh
   flatpak remote-delete --force cumulas
   flatpak remote-add cumulas https://flatpak.cumulas.co/cumulas.flatpakrepo
   ```

The builds signed with the old key leave the repository after 3 more
publishes. Publishing doesn't check old signatures, so a rotation doesn't
block it.

### When a publish fails

Nothing is deployed until every step has passed, the install tests included,
so the site stays as it was. Fix the cause and run the workflow again ("Re-run
failed jobs" reuses the builds for 3 days).

- **The published repository can't be read** (DNS, TLS, an HTTP error, a
  missing object): the publish stops rather than drop the published history.
  Check the site.
- **It answers 404**: that means nothing is published yet, so the publish
  starts a new repository, with a warning. After the first publish, a 404
  means Pages or its custom domain is misconfigured: fix that before
  publishing again, or the older builds are lost for rollback (installed apps
  keep updating).
- **GitHub Pages caches files for up to 10 minutes**: people may get the
  previous build for a few minutes after a publish, and a publish that starts
  within minutes of the last one may mirror the cached copy and leave that
  build out of the history (it's still installed and updated normally).
- **A newer tag** waiting behind a running publish replaces an older waiting
  one (GitHub keeps one waiting run per group): run that one again if it
  should still go out.

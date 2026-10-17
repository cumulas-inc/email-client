# Flatpak packaging

App ID: `co.cumulas.Mail` (runtime `org.gnome.Platform` 50, which provides
WebKitGTK 4.1 for Tauri).

## Build locally

```sh
flatpak install flathub org.gnome.Sdk//50 org.gnome.Platform//50 \
  org.freedesktop.Sdk.Extension.rust-stable//25.08 org.freedesktop.Sdk.Extension.node22//25.08
pip install --require-hashes --only-binary :all: \
  -r ../../.github/actions/flatpak-sources/requirements.txt   # in a virtualenv
./generate-sources.sh
./fetch-shared-modules.sh   # the tray library's module
flatpak-builder --user --install --force-clean build-dir co.cumulas.Mail.yml
flatpak run co.cumulas.Mail
```

`shared-modules/` (Flathub's [shared modules](https://github.com/flathub/shared-modules),
for libayatana-appindicator) is a git submodule in the Flathub repository; here
`fetch-shared-modules.sh` fetches it next to the manifest (ignored by git), at
the commit it pins.

`generate-sources.sh` runs the generators of
[flatpak-builder-tools](https://github.com/flatpak/flatpak-builder-tools) at the
commit it pins, with the Python packages that
`.github/actions/flatpak-sources/requirements.txt` pins with their hashes
(regenerate it from `requirements.in` with the command at its top). To move to
a newer flatpak-builder-tools, change the commit and compare the generated
files.

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
allows without a permission. Update notices and restarting into a new version
use the Flatpak portal, which every Flatpak app can reach, and that
single-instance name: no permission either.

## Updates

People get updates from their software center, which installs Flatpak updates
on its own, or with `flatpak update`. The app doesn't install them. It keeps
running in the background, though, so it says when a new version is installed
and restarts into it (`src-tauri/src/desktop/update.rs`).

- **Noticing.** The Flatpak portal's update monitor (`CreateUpdateMonitor` on
  `org.freedesktop.portal.Flatpak`) checks every 30 minutes, the first time 30
  minutes after the app starts, and not at all on a metered connection (unless
  the portal runs with `--poll-when-metered`). When the installed version
  isn't the running one, a card in the window's corner says so, once per
  version (Restart, or Later); the main menu (with a dot on its button), the
  tray menu (above Quit) and Settings → About keep offering **Restart to
  update**; and while the window isn't open, one desktop notification per
  version says so, with a Restart button. Outside a Flatpak, or with a portal
  without the monitor, nothing shows; a portal that can't be reached is tried
  again after 1, 5 and then every 30 minutes, and at once when it starts.
- **Restarting.** It asks first about an unsent message, as Quit does. Then
  the app asks the portal to start the installed version (`Spawn` with
  `LatestVersion`, `--restarted-from=<its bus name>`) and hands over to it
  through the single-instance name: the new instance checks that it can run
  and says so (`--restart-ready`); the old one shuts push down, releases
  `co.cumulas.Mail.SingleInstance` and exits; the new one takes the name,
  opens its window and says "Updated to version …". If the new instance
  refuses (`--restart-refused`), exits early (`SpawnExited`) or doesn't answer
  within 30 seconds, the old one keeps running and says it couldn't restart
  (with a notification when its window isn't open). Restart can be tried
  again, and quitting and opening the app always works.
- **The runtime.** `Spawn` starts the new version on the running one's
  runtime, whatever it was built for. So the new instance compares the runtime
  its manifest names (`runtime` and `runtime-version` in `/app/manifest.json`,
  which flatpak-builder writes into every build, Flathub's included; or `sdk`,
  which `flatpak run --devel` runs it on) with the one it was started on
  (`[Application] runtime` in `/.flatpak-info`), and refuses when they differ:
  after a release that moves to another runtime branch, the app asks for a
  quit and reopen instead. A build without `/app/manifest.json` isn't checked.
- **flatpak before 1.15.6** (Ubuntu 24.04, Debian 12) starts the new version
  with the portal's environment rather than the app's. The app passes its
  locale and input method on (`LANG`, `LANGUAGE`, `LC_*`, `GTK_IM_MODULE`,
  `QT_IM_MODULE`, `XMODIFIERS`), and the new instance checks that it can open
  the display before it says it's ready.
- **Keep `enableGTKAppId` off** in `src-tauri/tauri.conf.json` (a test checks
  it): with it on, GTK would make the new instance a remote of the old one's
  application, and it would exit at once.

### Testing updates by hand

This needs a desktop session with the portal (or `dbus-run-session` and Xvfb).

1. Build into a local repository and install from it:

   ```sh
   flatpak-builder --user --repo=/tmp/cm-repo --force-clean build-dir co.cumulas.Mail.yml
   flatpak --user remote-add --no-gpg-verify cm-local /tmp/cm-repo
   flatpak --user install cm-local co.cumulas.Mail
   ```

2. Make the portal check every 15 seconds, then start the app:
   `/usr/libexec/flatpak-portal --replace --verbose --poll-timeout=15 --poll-when-metered &`
   (the path differs between distributions), then `flatpak run co.cumulas.Mail`.
3. Change anything, build into the same repository and run
   `flatpak --user update co.cumulas.Mail`. Within 15 seconds: the card, the
   dot, the menu and tray items and the About row. With the window closed, one
   notification, which doesn't come again when `flatpak-portal --replace`
   reports again.
4. Restart: the window goes, and the new one comes within a few seconds with
   "Updated to version …". `flatpak ps --columns=instance,application,commit`
   shows one instance, on the new commit, and
   `busctl --user status co.cumulas.Mail.SingleInstance` one owner. Try it from
   the tray and from the notification's button too, with the app started with
   `--background`, and with an unsent message (it asks first).
5. Build with another `runtime-version` (another installed GNOME branch),
   update, and Restart: "Couldn't restart…" within a second or two, and the app
   keeps running; quit and open it, and `flatpak ps` shows it on its runtime.
6. Build with `command:` set to a missing binary: Restart fails at once (the
   new version exited), not after 30 seconds.
7. Click the launcher several times during a restart: one instance remains.
8. Run `flatpak-portal --replace` while the app runs: its log says it watches
   for updates again, and step 3 still works.

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

- **Release tags** publish: `vMAJOR.MINOR.PATCH`, such as `v1.2.0`
  (pre-releases such as `v1.2.0-rc.1` don't start the workflow). The tag must
  be the highest release tag, so that an older one never replaces newer
  builds, and the newest `<release>` in `co.cumulas.Mail.metainfo.xml` must be
  its version, since software centers show that release's version and notes:
  add the release there before tagging. Otherwise the run stops before it
  builds (`repo/check-release.sh`). A manual run (Actions → Flatpak repository
  → Run workflow) publishes the default branch only with **Publish** ticked.
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
  1. mirrors the published repository with its history (`repo/mirror.sh`),
     after checking that its summary is signed with the committed key, so that
     nothing unsigned joins a signed publish (say, from someone else serving
     the domain), and stops if a build in that history can't be fetched;
  2. adds the new builds on top of it, signed (`repo/add-builds.sh`);
  3. generates the appstream data software centers read and static deltas,
     prunes old builds and signs the summary (`repo/update-repo.sh`);
  4. assembles the site (`repo/assemble-site.sh`);
  5. installs the app from a local copy of the site on both architectures, from
     the `.flatpakrepo` and from the `.flatpakref` (`repo/smoke-test.sh`);
  6. and only then, when publishing, deploys the site to Pages, if the
     published repository is still the one it mirrored
     (`repo/check-live-summary.sh`);
  7. then waits until Pages serves the new summary, adds the remote from the
     live `.flatpakrepo` and finds the app on both architectures
     (`repo/verify-live.sh`), and reports the type the `.flatpakref` is served
     as.

  `repo/test.sh` runs these steps against fake builds, all but the deploy
  itself, as part of every run.

- **Kept**: up to the last 3 builds of each architecture (`KEEP_BUILDS` in
  `repo/common.sh`), fewer after the first releases or if the history ever has
  to start over (see [When a publish fails](#when-a-publish-fails)). After
  `KEEP_BUILDS` is raised, the history grows by a build with each publish until
  it gets there: each publish records how many builds of each ref it keeps in
  the repository's config, and the next one expects that many. While an
  earlier build is there, people can go back to it:
  `flatpak remote-info --log cumulas co.cumulas.Mail` lists them and
  `flatpak update --commit=COMMIT co.cumulas.Mail` installs one. Debug info
  (`.Debug`) isn't published, to stay well under GitHub Pages' 1 GB limit.
- **Hosting**: GitHub Pages publishes sites of up to 1 GB, with a soft
  bandwidth limit of about 100 GB a month; a site that keeps going over it may
  be slowed down or asked to move. If downloads outgrow it, move the site to
  another static host or a CDN (it's plain files) and point
  `flatpak.cumulas.co` at it: people's remotes keep working, since the URL and
  the key stay the same. The deploy and live check jobs are what would change.

| On the site                  | What it is                                                 |
| ---------------------------- | ---------------------------------------------------------- |
| `repo/`                      | The Flatpak (ostree) repository                            |
| `cumulas.flatpakrepo`        | Adds the remote (`flatpak remote-add`)                     |
| `co.cumulas.Mail.flatpakref` | Installs the app in one click                              |
| `cumulas-flatpak.gpg`        | The public key the repository is signed with               |
| `index.html`, `logo.svg`     | The install page (`repo/index.html`)                       |
| `admin-consent/`             | The page for IT administrators (`repo/admin-consent.html`) |

Microsoft brings administrators back to `admin-consent/` after they approve
Cumulas Mail, so its address is registered on the app registration and built
into the app (see [Microsoft 365](../../docs/microsoft-365.md#approving-cumulas-mail-it-admins)).
`repo/check-admin-consent.sh` checks the page against the app's approval link
and permissions in CI and in `repo/test.sh`.

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
   admin and Maintain roles to the bypass list. Then only maintainers can
   push, move or delete the tags that publish and that may use the signing
   key. The environment trusts the default branch too: keep `main` protected,
   with changes through reviewed pull requests.
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

Flatpak checks updates against the keys each person's remote trusts, so a new
key has to reach people before it signs anything. A planned rotation does that
in two phases, with nothing for people to do; a leaked key can't wait for
that.

**To replace a key that hasn't leaked:**

1. Generate the new key as in [Setting it up](#setting-it-up), step 1, but
   export its public half to `packaging/flatpak/repo/cumulas-flatpak-next.gpg`.
   Keep its private half as you keep the current one's.
2. **Phase 1**: commit `cumulas-flatpak-next.gpg` in a pull request, and
   publish. While the file exists, every publish is still signed with the
   current key, and its summary carries the next one (`update-repo.sh` passes
   it to `flatpak build-update-repo --gpg-import`). Flatpak, software centers
   included, adds it to the remote's keys when it checks for updates. Leave
   phase 1 in place for long enough that most people have checked, over
   several releases if you can.
3. **Phase 2**: in a pull request, rename `cumulas-flatpak.gpg` to
   `cumulas-flatpak-previous.gpg`, and `cumulas-flatpak-next.gpg` to
   `cumulas-flatpak.gpg`. (The next publish checks the published summary,
   still signed with the old key, with the previous one.) When it's merged,
   replace the `FLATPAK_GPG_PRIVATE_KEY` secret of the `flatpak-signing`
   environment with the new private key and check it with a manual dry run of
   `main` (Setting it up, step 9); publishing fails while the two don't match.
   From the next publish on, the summary and new builds are signed with the
   new key only, and new installs get it from the `.flatpakrepo` and
   `.flatpakref`.
4. Once a publish signed with the new key is out, delete
   `cumulas-flatpak-previous.gpg`. The builds signed with the old key leave the
   repository once 3 publishes (`KEEP_BUILDS`) have been signed with the new
   key: that first one and the 2 after it.

People who didn't check for updates during phase 1 get a GPG signature error
from phase 2 on, and have to remove and add the remote as below.

**If the key may have leaked**, stop trusting it at once: generate a new key,
rename the old `cumulas-flatpak.gpg` to `cumulas-flatpak-previous.gpg` (only
until the first publish with the new key, which needs it to read the published
history), commit the new one as `cumulas-flatpak.gpg`, replace the secret, and
publish. Then tell people (release notes, cumulas.co) to remove and add the
remote, so that the old key is trusted no more (installed apps stay):

```sh
flatpak remote-delete --force cumulas
flatpak remote-add cumulas https://flatpak.cumulas.co/cumulas.flatpakrepo   # add --user for a per-user remote
```

### When a publish fails

Nothing is deployed until every step has passed, the install tests included,
so the site stays as it was. Fix the cause and run the workflow again ("Re-run
failed jobs" reuses the builds for 3 days). Only the newest release tag
publishes: once a higher tag exists, an older tag's run stops at its release
check, re-runs included.

- **The published repository can't be read** (DNS, TLS, an HTTP error, a
  missing object, a history that stops short of the builds it keeps): the
  publish stops rather than drop the published history. Check the site.
- **Its summary isn't signed with a committed key**: the publish stops rather
  than sign on top of a history it can't trust. Someone else may be serving
  flatpak.cumulas.co, or a key rotation skipped a step: check the domain's
  Pages settings and the committed keys.
- **It answers 404**: that means nothing is published yet, so the publish
  starts a new repository, with a warning. After the first publish, a 404
  means Pages or its custom domain is misconfigured: fix that before
  publishing again, or the older builds are lost for rollback (installed apps
  keep updating).
- **The published repository changed since the run mirrored it**: something
  was deployed in between (a run of another tag, or an older run re-run), and
  deploying would drop it, so the deploy stops. Run the newest release tag's
  workflow again, all jobs.
- **GitHub Pages caches files for up to 10 minutes**: people may get the
  previous build for a few minutes after a publish. The run's last job waits
  for that, so a publish queued behind it mirrors the new site; one that
  mirrors a cached copy anyway finds, at its deploy, that the published
  repository changed, and stops (above).
- **The live site check fails** after the deploy: the site is out, but Pages
  didn't serve the new summary within 10 minutes, or the app can't be found
  through the live `.flatpakrepo`. Check Pages and the site, then re-run that
  job.
- **A newer tag** waiting behind a running publish replaces an older waiting
  one (GitHub keeps one waiting run per group), which couldn't publish anyway.

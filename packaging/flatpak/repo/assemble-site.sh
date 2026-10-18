#!/usr/bin/env bash
# Makes the site published at https://flatpak.cumulas.co:
#
#   repo/                         the Flatpak repository
#   cumulas.flatpakrepo           adds the remote (flatpak remote-add)
#   co.cumulas.Mail.flatpakref    installs the app in one click
#   cumulas-flatpak.gpg           the public key the repository is signed with
#   index.html, logo.svg          the install page
#   admin-consent/index.html      the page for IT administrators, where
#                                 Microsoft brings them back after they
#                                 approve Cumulas Mail
#
# Usage: assemble-site.sh REPO PUBLIC-KEY SITE
#   REPO        the repository, after update-repo.sh
#   PUBLIC-KEY  the public half of the key it was signed with
#   SITE        the directory to create; it must not exist
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
# shellcheck source=SCRIPTDIR/common.sh
. "$here/common.sh"

# GitHub Pages publishes sites of up to 1 GB.
size_limit=$((1000 * 1000 * 1000))
size_warning=$((800 * 1000 * 1000))

if [ $# != 3 ]; then
  echo "usage: $0 REPO PUBLIC-KEY SITE" >&2
  exit 2
fi
repo=$1
public_key=$2
site=$3
[ ! -e "$site" ] || die "$site already exists."
[ -s "$repo/summary.sig" ] || die "$repo has no signed summary; run update-repo.sh first."

fingerprint=$("$here/signing-key.sh" check "$public_key")

# A GnuPG home in /tmp whatever TMPDIR is: gpg starts gpg-agent even to
# import a public key, and fails when the agent's socket path is too long.
scratch=$(mktemp -d /tmp/assemble-site.XXXXXXXX)
cleanup() {
  gpgconf --homedir "$scratch" --kill all 2>/dev/null || true
  rm -rf -- "$scratch"
}
trap cleanup EXIT
chmod 700 "$scratch"
mkdir -p "$site"

# The key as clients get it: binary, without third-party signatures.
errors=$(gpg --homedir "$scratch" --batch --import "$public_key" 2>&1 >/dev/null) ||
  die "Importing $public_key failed: $errors"
gpg --homedir "$scratch" --batch --export --export-options export-minimal "$fingerprint" \
  >"$site/cumulas-flatpak.gpg"
gpg_key=$(base64 -w 0 "$site/cumulas-flatpak.gpg")

# The repository, without ostree's working files.
mkdir "$site/repo"
cp -a "$repo/." "$site/repo/"
rm -rf -- "$site/repo/tmp" "$site/repo/state" "$site/repo/.lock"

cat >"$site/$REMOTE_NAME.flatpakrepo" <<EOF
[Flatpak Repo]
Title=$REPO_TITLE
Url=$REPO_URL
Homepage=$REPO_HOMEPAGE
Comment=$REPO_COMMENT
Description=$REPO_DESCRIPTION
Icon=$REPO_ICON
DefaultBranch=$BRANCH
RuntimeRepo=$FLATHUB_REPO_FILE
GPGKey=$gpg_key
EOF

cat >"$site/$APP_ID.flatpakref" <<EOF
[Flatpak Ref]
Name=$APP_ID
Branch=$BRANCH
Title=$APP_NAME
Comment=$APP_SUMMARY
Url=$REPO_URL
SuggestRemoteName=$REMOTE_NAME
Homepage=$REPO_HOMEPAGE
Icon=$REPO_ICON
RuntimeRepo=$FLATHUB_REPO_FILE
IsRuntime=false
GPGKey=$gpg_key
EOF

cp "$root/public/icon.svg" "$site/logo.svg"
grouped=$(printf '%s\n' "$fingerprint" | sed 's/.\{4\}/& /g; s/ $//')
sed "s/@FINGERPRINT@/$grouped/g" "$here/index.html" >"$site/index.html"
mkdir "$site/admin-consent"
cp "$here/admin-consent.html" "$site/admin-consent/index.html"
for page in index.html admin-consent/index.html; do
  if grep -q '@[A-Z_]*@' "$site/$page"; then
    die "$page has a placeholder left: $(grep -o '@[A-Z_]*@' "$site/$page" | head -1)"
  fi
done

bytes=$(du -sb "$site" | cut -f1)
if [ "$bytes" -ge "$size_limit" ]; then
  die "The site is $((bytes / 1000000)) MB; GitHub Pages publishes up to $((size_limit / 1000000)) MB. Keep fewer builds (KEEP_BUILDS in common.sh)."
elif [ "$bytes" -ge "$size_warning" ]; then
  warn "The site is $((bytes / 1000000)) MB, close to GitHub Pages' limit of $((size_limit / 1000000)) MB."
fi
log "Assembled $site ($((bytes / 1000000)) MB), signed with $fingerprint"

# shellcheck shell=bash
# shellcheck disable=SC2034 # The scripts that source this use its settings.
# Settings and helpers shared by the scripts that publish the Flatpak
# repository at https://flatpak.cumulas.co. Sourced, not run.

APP_ID=co.cumulas.Mail
APP_NAME="Cumulas Mail"
APP_SUMMARY="Email, calendar, contacts and tasks"
BRANCH=stable
SITE_URL=https://flatpak.cumulas.co
REPO_URL="$SITE_URL/repo/"
# The name users give the remote (the .flatpakref suggests it).
REMOTE_NAME=cumulas
# Builds kept per architecture: the newest and the ones before it.
KEEP_BUILDS=3
# The architectures the workflow builds and publishes.
ARCHES="x86_64 aarch64"
FLATHUB_REPO_FILE=https://dl.flathub.org/repo/flathub.flatpakrepo

REPO_TITLE=Cumulas
REPO_COMMENT="Cumulas Mail, built and signed by Cumulas"
REPO_DESCRIPTION="Releases of Cumulas Mail, built from https://github.com/cumulas-inc/email-client and signed by Cumulas. The GNOME runtime it needs comes from Flathub."
REPO_HOMEPAGE=https://cumulas.co
REPO_ICON="$SITE_URL/logo.svg"

# The committed public keys (packaging/flatpak/README.md, "Rotating the
# signing key"): the one the repository is signed with and, during a
# rotation, the next one (phase 1: the summary carries it, so that clients
# import it before it signs) and then the previous one (phase 2: it signed
# what's published until the first publish with the new key).
KEYS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PUBLIC_KEY="$KEYS_DIR/cumulas-flatpak.gpg"
NEXT_PUBLIC_KEY="$KEYS_DIR/cumulas-flatpak-next.gpg"
PREVIOUS_PUBLIC_KEY="$KEYS_DIR/cumulas-flatpak-previous.gpg"

# The refs these scripts publish, as an extended regular expression: the app
# and, when a build has translations, its Locale extension. Debug info
# (.Debug) stays out, to keep the site well under GitHub Pages' size limit.
MANAGED_REFS="^(app/${APP_ID//./\\.}|runtime/${APP_ID//./\\.}\\.Locale)/[^/]+/$BRANCH\$"

log() {
  printf '%s\n' "$*" >&2
}

# Annotations in GitHub Actions (commands are read from stderr too), plain
# messages elsewhere.
warn() {
  if [ "${GITHUB_ACTIONS:-}" = true ]; then
    printf '::warning::%s\n' "$*" >&2
  else
    printf 'warning: %s\n' "$*" >&2
  fi
}

die() {
  if [ "${GITHUB_ACTIONS:-}" = true ]; then
    printf '::error::%s\n' "$*" >&2
  else
    printf 'error: %s\n' "$*" >&2
  fi
  exit 1
}

# The commits in REF's history that REPO still has, newest first.
ref_history() {
  local repo=$1 ref=$2 commit
  commit=$(ostree --repo="$repo" rev-parse "$ref")
  while [ -n "$commit" ] && ostree --repo="$repo" show "$commit" >/dev/null 2>&1; do
    printf '%s\n' "$commit"
    commit=$(ostree --repo="$repo" rev-parse "$commit^" 2>/dev/null) || commit=
  done
}

# The refs in REPO that these scripts publish.
managed_refs() {
  ostree --repo="$1" refs | grep -E "$MANAGED_REFS" || true
}

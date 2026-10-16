#!/usr/bin/env bash
# Checks the live site after a deploy, as people get it. Waits until
# SITE/repo/summary is the one deployed (GitHub Pages caches files for up to
# 10 minutes), then, in a temporary Flatpak user installation, adds the
# remote from SITE/cumulas.flatpakrepo and finds the app, branch stable, for
# each architecture with remote-ls and remote-info (which check the summary's
# signature with the key the .flatpakrepo carries). Prints a short report in
# Markdown, with the Content-Type the .flatpakref is served with.
#
# Usage: verify-live.sh [--site URL] [--timeout SECONDS] SUMMARY-SHA256
#   SUMMARY-SHA256  the SHA-256 of the deployed repo/summary
#   --site          SITE_URL (common.sh) by default
#   --timeout       how long to wait for the deployed summary; 600 by default
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=SCRIPTDIR/common.sh
. "$here/common.sh"

usage() {
  sed -n '2,/^set -euo/{/^set -euo/d;s/^# \{0,1\}//;p}' "$0" >&2
  exit 2
}

site=$SITE_URL timeout=600 interval=15
while [ $# -gt 0 ]; do
  case "$1" in
    --site) site=$2; shift 2 ;;
    --timeout) timeout=$2; shift 2 ;;
    --) shift; break ;;
    -*) usage ;;
    *) break ;;
  esac
done
[ $# = 1 ] || usage
expected=$1
site=${site%/}

scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT

deadline=$((SECONDS + timeout))
while :; do
  live=$(curl --silent --show-error --fail --location --connect-timeout 20 --max-time 60 \
    "$site/repo/summary" | sha256sum | cut -d ' ' -f 1) || live=unreadable
  [ "$live" != "$expected" ] || break
  if [ "$SECONDS" -ge "$deadline" ]; then
    die "After ${timeout}s, $site/repo/summary still isn't the one deployed ($expected; it's $live). Check GitHub Pages, and whether something else was deployed since."
  fi
  log "$site/repo/summary isn't the one deployed yet ($live); waiting for GitHub Pages' cache."
  sleep "$(((deadline - SECONDS) < interval ? (deadline - SECONDS) : interval))"
done
log "$site/repo/summary is the one deployed."

export FLATPAK_USER_DIR="$scratch/flatpak"
mkdir -p "$FLATPAK_USER_DIR"
flatpak remote-add --user --from "$REMOTE_NAME" "$site/$REMOTE_NAME.flatpakrepo" ||
  die "Adding the remote from $site/$REMOTE_NAME.flatpakrepo failed."

echo "| Architecture | \`$APP_ID//$BRANCH\` |"
echo "| --- | --- |"
for arch in $ARCHES; do
  listed=$(flatpak remote-ls --user --app --arch="$arch" --columns=application,branch "$REMOTE_NAME") ||
    die "Listing the remote's apps for $arch failed."
  printf '%s\n' "$listed" | grep -qx "$APP_ID"$'\t'"$BRANCH" ||
    die "The remote doesn't list $APP_ID//$BRANCH for $arch."
  commit=$(flatpak remote-info --user --arch="$arch" --show-commit "$REMOTE_NAME" "$APP_ID//$BRANCH") ||
    die "Reading $APP_ID//$BRANCH for $arch from the remote failed."
  echo "| $arch | commit \`${commit:0:12}\` |"
done

content_type=$(curl --silent --show-error --fail --location --head --output /dev/null \
  --write-out '%{content_type}' "$site/$APP_ID.flatpakref") ||
  die "Fetching $site/$APP_ID.flatpakref failed."
echo
echo "\`$APP_ID.flatpakref\` is served as \`${content_type:-nothing}\`."
log "OK: $site serves the deployed repository, with $APP_ID//$BRANCH for $ARCHES; $APP_ID.flatpakref is ${content_type:-without a Content-Type}."

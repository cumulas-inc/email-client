#!/usr/bin/env bash
# Brings the repository's metadata up to date after add-builds.sh: the
# appstream branches that software centers read, static deltas for quick
# updates, pruning to the last KEEP_BUILDS builds of each ref, and a signed
# summary. Records KEEP_BUILDS in the repository's config for the next
# publish's mirror.sh, then checks the result.
#
# Usage: update-repo.sh --gpg-homedir DIR --gpg-key FINGERPRINT
#          [--next-key PUBLIC-KEY] REPO
#   --next-key  during a key rotation, the key that signs from phase 2 on:
#               the summary carries it, and Flatpak clients that check for
#               updates import it. NEXT_PUBLIC_KEY (common.sh) by default, and
#               nothing when there's no such file.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=SCRIPTDIR/common.sh
. "$here/common.sh"

usage() {
  sed -n '2,/^set -euo/{/^set -euo/d;s/^# \{0,1\}//;p}' "$0" >&2
  exit 2
}

gpg_homedir='' gpg_key='' next_key=$NEXT_PUBLIC_KEY
while [ $# -gt 0 ]; do
  case "$1" in
    --gpg-homedir) gpg_homedir=$2; shift 2 ;;
    --gpg-key) gpg_key=$2; shift 2 ;;
    --next-key) next_key=$2; shift 2 ;;
    --) shift; break ;;
    -*) usage ;;
    *) break ;;
  esac
done
[ -n "$gpg_homedir" ] && [ -n "$gpg_key" ] && [ $# = 1 ] || usage
repo=$1

# Each publish starts from a new repository (mirror.sh), so the next key goes
# in again every time, for as long as the file exists.
next_args=()
if [ -e "$next_key" ]; then
  next_fingerprint=$("$here/signing-key.sh" check "$next_key")
  if [ "${next_fingerprint^^}" = "${gpg_key^^}" ]; then
    die "$next_key is the key that signs: it's the next key no more. Delete it (packaging/flatpak/README.md, \"Rotating the signing key\")."
  fi
  next_args=(--gpg-import="$next_key")
  log "The summary carries the next signing key, $next_fingerprint: clients import it when they check for updates."
fi

# --prune-depth counts the parents kept besides each ref's newest commit.
flatpak build-update-repo \
  --title="$REPO_TITLE" \
  --comment="$REPO_COMMENT" \
  --description="$REPO_DESCRIPTION" \
  --homepage="$REPO_HOMEPAGE" \
  --icon="$REPO_ICON" \
  --default-branch="$BRANCH" \
  --generate-static-deltas \
  --prune --prune-depth=$((KEEP_BUILDS - 1)) \
  --gpg-homedir="$gpg_homedir" --gpg-sign="$gpg_key" \
  "${next_args[@]}" \
  "$repo"
# How many builds the next publish's mirror.sh should find for each ref.
ostree --repo="$repo" config set cumulas.keep-builds "$KEEP_BUILDS"

# Check what clients and the next publish rely on.
[ -s "$repo/summary" ] && [ -s "$repo/summary.sig" ] || die "$repo has no signed summary."
if [ ${#next_args[@]} -gt 0 ]; then
  ostree --repo="$repo" summary --view | grep -q '^xa\.gpg-keys:' ||
    die "The summary of $repo doesn't carry the next key."
fi
mapfile -t refs < <(managed_refs "$repo")
[ ${#refs[@]} -gt 0 ] || die "$repo has none of the app's refs."
deltas=$(ostree --repo="$repo" static-delta list)
for ref in "${refs[@]}"; do
  head=$(ostree --repo="$repo" rev-parse "$ref")
  builds=$(ref_history "$repo" "$ref" | wc -l)
  [ "$builds" -le "$KEEP_BUILDS" ] || die "$ref keeps $builds builds after pruning; expected at most $KEEP_BUILDS."
  # A from-scratch delta is named after the commit alone.
  printf '%s\n' "$deltas" | grep -qx "$head" || die "$ref has no static delta for its newest commit $head."
  log "$ref: $head, $builds build(s) kept"
done
for arch in $(printf '%s\n' "${refs[@]}" | grep "^app/" | cut -d/ -f3 | sort -u); do
  ostree --repo="$repo" rev-parse "appstream2/$arch" >/dev/null 2>&1 ||
    die "$repo has no appstream data for $arch."
done

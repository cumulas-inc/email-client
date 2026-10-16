#!/usr/bin/env bash
# Brings the repository's metadata up to date after add-builds.sh: the
# appstream branches that software centers read, static deltas for quick
# updates, pruning to the last KEEP_BUILDS builds of each ref, and a signed
# summary. Then checks the result.
#
# Usage: update-repo.sh --gpg-homedir DIR --gpg-key FINGERPRINT REPO
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=SCRIPTDIR/common.sh
. "$here/common.sh"

usage() {
  sed -n '2,/^set -euo/{/^set -euo/d;s/^# \{0,1\}//;p}' "$0" >&2
  exit 2
}

gpg_homedir='' gpg_key=''
while [ $# -gt 0 ]; do
  case "$1" in
    --gpg-homedir) gpg_homedir=$2; shift 2 ;;
    --gpg-key) gpg_key=$2; shift 2 ;;
    --) shift; break ;;
    -*) usage ;;
    *) break ;;
  esac
done
[ -n "$gpg_homedir" ] && [ -n "$gpg_key" ] && [ $# = 1 ] || usage
repo=$1

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
  "$repo"

# Check what clients and the next publish rely on.
[ -s "$repo/summary" ] && [ -s "$repo/summary.sig" ] || die "$repo has no signed summary."
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

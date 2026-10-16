#!/usr/bin/env bash
# Adds new builds to the repository: for each build repository (what
# flatpak-builder --repo --default-branch=stable wrote), a signed commit of
# the app and its Locale extension, on top of the ref's current commit in
# REPO, so the history carries on from the published one.
#
# Usage: add-builds.sh --gpg-homedir DIR --gpg-key FINGERPRINT \
#          --subject TEXT [--body TEXT] REPO BUILD-REPO...
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=SCRIPTDIR/common.sh
. "$here/common.sh"

usage() {
  sed -n '2,/^set -euo/{/^set -euo/d;s/^# \{0,1\}//;p}' "$0" >&2
  exit 2
}

gpg_homedir='' gpg_key='' subject='' body=''
while [ $# -gt 0 ]; do
  case "$1" in
    --gpg-homedir) gpg_homedir=$2; shift 2 ;;
    --gpg-key) gpg_key=$2; shift 2 ;;
    --subject) subject=$2; shift 2 ;;
    --body) body=$2; shift 2 ;;
    --) shift; break ;;
    -*) usage ;;
    *) break ;;
  esac
done
[ -n "$gpg_homedir" ] && [ -n "$gpg_key" ] && [ -n "$subject" ] && [ $# -ge 2 ] || usage
repo=$1
shift

declare -A seen=()
for build in "$@"; do
  mapfile -t refs < <(managed_refs "$build")
  printf '%s\n' "${refs[@]}" | grep -q "^app/$APP_ID/" ||
    die "$build has no app/$APP_ID/*/$BRANCH ref. Was it built with --default-branch=$BRANCH?"
  for ref in "${refs[@]}"; do
    [ -z "${seen[$ref]:-}" ] || die "$ref comes from two builds ($build and ${seen[$ref]})."
    seen[$ref]=$build
  done

  log "Adding ${refs[*]} from $build"
  # --untrusted checks every object's checksum on the way in: the build
  # repository comes from another job.
  flatpak build-commit-from --untrusted --no-update-summary \
    --src-repo="$build" \
    --gpg-homedir="$gpg_homedir" --gpg-sign="$gpg_key" \
    --subject="$subject" ${body:+--body="$body"} \
    "$repo" "${refs[@]}"
done

for ref in "${!seen[@]}"; do
  log "$ref: now $(ostree --repo="$repo" rev-parse "$ref"), $(ref_history "$repo" "$ref" | wc -l) build(s)"
done

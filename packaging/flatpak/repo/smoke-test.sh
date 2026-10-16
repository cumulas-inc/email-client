#!/usr/bin/env bash
# Installs the app from an assembled site the way users will, before the
# site is deployed. In a temporary Flatpak user installation, with SITE
# served on 127.0.0.1:
#
#  1. adds Flathub, for the runtime;
#  2. adds the remote from SITE's cumulas.flatpakrepo (its Url pointed at the
#     local server), installs the app from it and checks the installed ref,
#     branch, origin and commit, and that the app shows in the remote's
#     appstream data (what software centers list);
#  3. removes both, installs again from co.cumulas.Mail.flatpakref and checks
#     the same.
#
# Usage: smoke-test.sh SITE
# Downloads the GNOME runtime from Flathub (about 1 GB) unless
# SMOKE_FLATHUB_REPO_FILE names another .flatpakrepo to take it from
# (test.sh uses a local one).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=SCRIPTDIR/common.sh
. "$here/common.sh"

if [ $# != 1 ]; then
  echo "usage: $0 SITE" >&2
  exit 2
fi
site=$(cd "$1" && pwd)
flathub=${SMOKE_FLATHUB_REPO_FILE:-$FLATHUB_REPO_FILE}
arch=$(flatpak --default-arch)
ref="app/$APP_ID/$arch/$BRANCH"
[ -f "$site/repo/refs/heads/$ref" ] || die "$site/repo has no $ref."
expected_commit=$(cat "$site/repo/refs/heads/$ref")

scratch=$(mktemp -d)
server=
cleanup() {
  if [ -n "$server" ]; then
    kill "$server" 2>/dev/null || true
    wait "$server" 2>/dev/null || true
  fi
  rm -rf -- "$scratch"
}
trap cleanup EXIT

export FLATPAK_USER_DIR="$scratch/flatpak"
mkdir -p "$FLATPAK_USER_DIR"

port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
python3 -m http.server --bind 127.0.0.1 --directory "$site" "$port" >"$scratch/server.log" 2>&1 &
server=$!
local_url="http://127.0.0.1:$port"
for _ in $(seq 1 50); do
  curl -sf -o /dev/null "$local_url/$REMOTE_NAME.flatpakrepo" && break
  sleep 0.2
done
curl -sf -o /dev/null "$local_url/$REMOTE_NAME.flatpakrepo" || die "The local server didn't start: $(cat "$scratch/server.log")"

# The published files, pointed at the local server. Everything else in them
# (the key above all) is used as published.
for file in "$REMOTE_NAME.flatpakrepo" "$APP_ID.flatpakref"; do
  grep -qx "Url=$REPO_URL" "$site/$file" || die "$file doesn't point at $REPO_URL."
  sed -e "s|^Url=.*|Url=$local_url/repo/|" -e "s|^RuntimeRepo=.*|RuntimeRepo=$flathub|" \
    "$site/$file" >"$scratch/$file"
done

# An option of a remote of the temporary installation.
remote_option() {
  awk -v section="[remote \"$1\"]" -v key="$2" '
    /^\[/ { in_section = ($0 == section); next }
    in_section && index($0, key "=") == 1 { print substr($0, length(key) + 2); exit }
  ' "$FLATPAK_USER_DIR/repo/config"
}

check_installed() {
  local how=$1 info_ref origin commit appstream option
  # Without the key, Flatpak would install unsigned builds without a word.
  for option in gpg-verify gpg-verify-summary; do
    [ "$(remote_option "$REMOTE_NAME" "$option")" = true ] ||
      die "$how: the remote '$REMOTE_NAME' was added without $option=true."
  done
  info_ref=$(flatpak info --user --show-ref "$APP_ID")
  origin=$(flatpak info --user --show-origin "$APP_ID")
  commit=$(flatpak info --user --show-commit "$APP_ID")
  [ "$info_ref" = "$ref" ] || die "$how: installed $info_ref, expected $ref."
  [ "$origin" = "$REMOTE_NAME" ] || die "$how: installed from the remote '$origin', expected '$REMOTE_NAME'."
  [ "$commit" = "$expected_commit" ] || die "$how: installed commit $commit, expected $expected_commit."
  flatpak info --user "$APP_ID"

  flatpak update --user --appstream "$REMOTE_NAME" >/dev/null
  appstream="$FLATPAK_USER_DIR/appstream/$REMOTE_NAME/$arch/active/appstream.xml"
  if [ -f "$appstream.gz" ]; then
    gzip -dc "$appstream.gz" >"$scratch/appstream.xml"
  elif [ -f "$appstream" ]; then
    cp "$appstream" "$scratch/appstream.xml"
  else
    die "$how: no appstream data for $arch from '$REMOTE_NAME'."
  fi
  grep -q "<id>$APP_ID</id>" "$scratch/appstream.xml" ||
    die "$how: $APP_ID isn't in the remote's appstream data, so software centers won't list it."
  log "$how: OK ($ref $commit from '$REMOTE_NAME', listed in its appstream data)"
}

flatpak remote-add --user --if-not-exists flathub "$flathub"

log "Installing from $REMOTE_NAME.flatpakrepo"
flatpak remote-add --user --from "$REMOTE_NAME" "$scratch/$REMOTE_NAME.flatpakrepo"
flatpak remote-ls --user "$REMOTE_NAME"
flatpak install --user --noninteractive "$REMOTE_NAME" "$APP_ID"
check_installed "$REMOTE_NAME.flatpakrepo"

log "Installing from $APP_ID.flatpakref"
flatpak uninstall --user --noninteractive "$APP_ID"
flatpak remote-delete --user "$REMOTE_NAME"
flatpak install --user --noninteractive --from "$scratch/$APP_ID.flatpakref"
check_installed "$APP_ID.flatpakref"

#!/usr/bin/env bash
# Tests the repository scripts end to end without building the app, with
# fake builds and a fake Flathub served from 127.0.0.1:
#
#  - signing-key.sh imports the key whose public half is committed, and
#    refuses a missing, empty, expired or different public key, a private key
#    with a passphrase, and more than one key;
#  - mirror.sh starts empty when nothing is published (404), and fails on a
#    server error, an unreachable host (unless it's a dry run), a published
#    repository with a missing object or summary, one without the app, one
#    whose summary isn't signed with a key it trusts, and one whose history
#    stops short (a warning in a dry run), unless its config says it keeps
#    fewer builds (after KEEP_BUILDS is raised);
#  - five publishes in a row (mirror, add-builds, update-repo, assemble-site,
#    then "deploy") keep the last KEEP_BUILDS builds of each ref, with their
#    history, static deltas and appstream data, and leave .Debug out;
#  - smoke-test.sh installs the last one from the .flatpakrepo and the
#    .flatpakref.
#
# Usage: test.sh
# Needs flatpak, ostree, gpg, python3 and curl. Your own Flatpak
# installations are left alone: it uses a temporary FLATPAK_USER_DIR. The
# test keys' GnuPG homes go in /tmp whatever TMPDIR is: gpg-agent puts its
# sockets there, and a socket's path can't be much longer than 100 bytes.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=SCRIPTDIR/common.sh
. "$here/common.sh"

# Expected failures shouldn't show up as error annotations in GitHub Actions.
unset GITHUB_ACTIONS FLATPAK_GPG_PRIVATE_KEY

work=$(mktemp -d)
keys=$(mktemp -d /tmp/flatpak-test-keys.XXXXXX)
servers=()
gnupg_homes=()
cleanup() {
  local pid home
  for pid in "${servers[@]}"; do
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  done
  for home in "${gnupg_homes[@]}"; do
    "$here/signing-key.sh" remove "$home" 2>/dev/null || true
  done
  rm -rf -- "$work" "$keys"
}
trap cleanup EXIT
export FLATPAK_USER_DIR="$work/flatpak-user"
native=$(flatpak --default-arch)
if [ "$native" = x86_64 ]; then other=aarch64; else other=x86_64; fi

failures=0
pass() {
  log "ok - $1"
}
fail() {
  log "FAIL - $1"
  failures=$((failures + 1))
}
# expect_success|expect_failure DESCRIPTION COMMAND...: the command's output
# shows only when the expectation isn't met.
expect_success() {
  local what=$1
  shift
  if "$@" >"$work/output" 2>&1; then pass "$what"; else
    fail "$what"
    cat "$work/output" >&2
  fi
}
expect_failure() {
  local what=$1
  shift
  if "$@" >"$work/output" 2>&1; then
    fail "$what (it succeeded)"
    cat "$work/output" >&2
  else pass "$what"; fi
}
# expect_error DESCRIPTION PATTERN COMMAND...: the command fails, and its
# output matches PATTERN (grep -E), so that it fails for the reason tested.
expect_error() {
  local what=$1 pattern=$2
  shift 2
  if "$@" >"$work/output" 2>&1; then
    fail "$what (it succeeded)"
    cat "$work/output" >&2
  elif ! grep -Eq -- "$pattern" "$work/output"; then
    fail "$what (but not with /$pattern/)"
    cat "$work/output" >&2
  else pass "$what"; fi
}
check() {
  local what=$1
  shift
  if "$@"; then pass "$what"; else fail "$what"; fi
}

free_port() {
  python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
}
wait_for() {
  local _
  for _ in $(seq 1 50); do
    curl -s -o /dev/null "$1" && return 0
    sleep 0.2
  done
  die "Nothing answers at $1."
}

# --- Signing keys -------------------------------------------------------------

# Runs gpg, and stops the test with gpg's messages if it fails: a key the
# test needs that can't be made would only show up as checks failing later.
gpg_or_die() { # WHAT GPG-ARGUMENT...
  local what=$1
  shift
  gpg "$@" 2>"$work/gpg.log" || die "$what failed: $(cat "$work/gpg.log")"
}
new_key() { # NAME → $keys/NAME (GNUPGHOME), NAME.gpg (public key), NAME.fpr
  local name=$1
  gnupg_homes+=("$keys/$name")
  "$here/signing-key.sh" throwaway "$keys/$name" "$keys/$name.gpg" >"$keys/$name.fpr" 2>"$work/gpg.log" ||
    die "Generating the test key '$name' failed: $(cat "$work/gpg.log")"
}
secret_of() {
  gpg_or_die "Exporting the test key '$1'" --homedir "$keys/$1" --batch --armor --export-secret-keys
}
import_key() { # SECRET PUBLIC-KEY → imports into a new GNUPGHOME
  local home="$keys/import.$RANDOM$RANDOM"
  gnupg_homes+=("$home")
  FLATPAK_GPG_PRIVATE_KEY=$1 "$here/signing-key.sh" import "$home" "$2"
}

new_key signer
new_key other
signer=$(cat "$keys/signer.fpr")
signer_secret=$(secret_of signer)

expect_success "check: a committed public key" "$here/signing-key.sh" check "$keys/signer.gpg"
if import_key "$signer_secret" "$keys/signer.gpg" >"$work/imported" 2>"$work/output"; then
  check "import: the key that matches the committed public key, printing its fingerprint" \
    test "$(cat "$work/imported")" = "$signer"
else
  fail "import: the key that matches the committed public key"
  cat "$work/output" >&2
fi
expect_failure "import: a different committed public key" \
  import_key "$signer_secret" "$keys/other.gpg"
expect_failure "import: no committed public key" \
  import_key "$signer_secret" "$keys/missing.gpg"
: >"$keys/empty.gpg"
expect_failure "import: an empty committed public key" \
  import_key "$signer_secret" "$keys/empty.gpg"
expect_failure "import: no private key" import_key "" "$keys/signer.gpg"
expect_failure "import: something that isn't a key" import_key "not a key" "$keys/signer.gpg"
cat "$keys/signer.gpg" "$keys/other.gpg" >"$keys/both.gpg"
expect_failure "check: two public keys" "$here/signing-key.sh" check "$keys/both.gpg"
expect_failure "import: two private keys" \
  import_key "$signer_secret"$'\n'"$(secret_of other)" "$keys/signer.gpg"
printf '%s\n' "$signer_secret" >"$keys/secret.asc"
expect_failure "check: a private key committed as the public key" \
  "$here/signing-key.sh" check "$keys/secret.asc"

mkdir -m 700 "$keys/protected"
gnupg_homes+=("$keys/protected")
gpg_or_die "Generating a key with a passphrase" --homedir "$keys/protected" --batch \
  --pinentry-mode loopback --passphrase secret \
  --quick-generate-key "Protected <protected@example.com>" rsa2048 sign 1d
gpg_or_die "Exporting the key with a passphrase" --homedir "$keys/protected" --batch \
  --export >"$keys/protected.gpg"
protected_secret=$(gpg_or_die "Exporting the private key with a passphrase" \
  --homedir "$keys/protected" --batch --pinentry-mode loopback --passphrase secret \
  --armor --export-secret-keys)
expect_failure "import: a private key with a passphrase" import_key \
  "$protected_secret" "$keys/protected.gpg"

mkdir -m 700 "$keys/expired"
gnupg_homes+=("$keys/expired")
gpg_or_die "Generating an expiring key" --homedir "$keys/expired" --batch --passphrase '' \
  --quick-generate-key "Expired <expired@example.com>" rsa2048 sign seconds=1
gpg_or_die "Exporting the expiring key" --homedir "$keys/expired" --batch \
  --export >"$keys/expired.gpg"
sleep 2
expect_failure "check: an expired public key" "$here/signing-key.sh" check "$keys/expired.gpg"

mkdir -p "$keys/not-empty"
touch "$keys/not-empty/file"
expect_failure "import: into a GNUPGHOME that isn't empty" \
  env FLATPAK_GPG_PRIVATE_KEY="$signer_secret" "$here/signing-key.sh" import \
  "$keys/not-empty" "$keys/signer.gpg"

# --- Fake builds, a fake Flathub and the "published" site ------------------------

# mirror.sh, trusting the test signing key.
mirror() {
  "$here/mirror.sh" --gpg-import "$keys/signer.gpg" "$@"
}

# A build as flatpak-builder --repo --default-branch=stable leaves it: the
# app, its Locale extension and its debug info.
make_build() { # REPO ARCH VERSION
  local repo=$1 arch=$2 version=$3 dir
  dir=$(mktemp -d "$work/build.XXXXXX")
  mkdir -p "$dir/app/files/bin" "$dir/app/files/share/app-info/xmls" "$dir/app/export" \
    "$dir/locale/files" "$dir/debug/files"
  cat >"$dir/app/metadata" <<EOF
[Application]
name=$APP_ID
runtime=org.gnome.Platform/$arch/50
sdk=org.gnome.Sdk/$arch/50
command=cumulas-mail
EOF
  printf '#!/bin/sh\necho %s\n' "$version" >"$dir/app/files/bin/cumulas-mail"
  chmod +x "$dir/app/files/bin/cumulas-mail"
  # Something that changes with every build, like the real binary.
  head -c 100000 /dev/urandom >"$dir/app/files/bin/data"
  printf '<?xml version="1.0"?>\n<components version="0.8" origin="%s"><component type="desktop-application"><id>%s</id><name>%s</name><summary>%s</summary></component></components>\n' \
    "$APP_ID" "$APP_ID" "$APP_NAME" "$APP_SUMMARY" | gzip >"$dir/app/files/share/app-info/xmls/$APP_ID.xml.gz"
  printf '[Runtime]\nname=%s.Locale\n' "$APP_ID" >"$dir/locale/metadata"
  printf '%s\n' "$version" >"$dir/locale/files/version"
  printf '[Runtime]\nname=%s.Debug\n' "$APP_ID" >"$dir/debug/metadata"
  printf '%s\n' "$version" >"$dir/debug/files/version"
  flatpak build-export --arch="$arch" --no-update-summary "$repo" "$dir/app" "$BRANCH" >/dev/null
  flatpak build-export --runtime --files=files --arch="$arch" --no-update-summary \
    "$repo" "$dir/locale" "$BRANCH" >/dev/null
  flatpak build-export --runtime --files=files --arch="$arch" --no-update-summary \
    "$repo" "$dir/debug" "$BRANCH" >/dev/null
}

mkdir -p "$work/www/flathub"
start_port=$(free_port)
python3 -m http.server --bind 127.0.0.1 --directory "$work/www" "$start_port" >"$work/http.log" 2>&1 &
servers+=("$!")
www="http://127.0.0.1:$start_port"
wait_for "$www/"
live="$www/live/repo"

# The runtime the fake app needs, signed like Flathub's.
mkdir -p "$work/runtime/files" "$work/runtime/usr"
printf '[Runtime]\nname=org.gnome.Platform\nruntime=org.gnome.Platform/%s/50\nsdk=org.gnome.Sdk/%s/50\n' \
  "$native" "$native" >"$work/runtime/metadata"
printf 'fake runtime\n' >"$work/runtime/usr/README"
flatpak build-export --runtime --arch="$native" --gpg-homedir="$keys/signer" \
  --gpg-sign="$signer" "$work/www/flathub/repo" "$work/runtime" 50 >/dev/null
flatpak build-update-repo --gpg-homedir="$keys/signer" --gpg-sign="$signer" \
  "$work/www/flathub/repo" >/dev/null
printf '[Flatpak Repo]\nTitle=Fake Flathub\nUrl=%s/flathub/repo/\nGPGKey=%s\n' \
  "$www" "$(base64 -w 0 "$keys/signer.gpg")" >"$work/www/flathub/flathub.flatpakrepo"

# --- Five publishes in a row ------------------------------------------------------

# The subject of a commit: the first line after the headers of ostree log.
subject() {
  ostree --repo="$1" log "$2" | sed -n '/^$/{n;s/^ *//;p;q}'
}
has_commit() { # REPO COMMIT
  [ -n "$2" ] && [ -e "$1/objects/${2:0:2}/${2:2}.commit" ]
}

declare -A commits=()
for round in 1 2 3 4 5; do
  dir="$work/round$round"
  mkdir -p "$dir"
  if ! summary=$(mirror "$dir/repo" "$live" 2>"$dir/mirror.log"); then
    fail "publish $round: mirror"
    cat "$dir/mirror.log" >&2
    break
  fi
  mirrored=$((round - 1 < KEEP_BUILDS ? round - 1 : KEEP_BUILDS))
  if [ "$round" = 1 ]; then
    check "publish 1: nothing published yet, so the mirror starts empty" \
      test -z "$(ostree --repo="$dir/repo" refs)"
    check "publish 1: mirror.sh says there was no summary" test "$summary" = none
  else
    check "publish $round: the mirror has the last $mirrored build(s)" \
      test "$(ref_history "$dir/repo" "app/$APP_ID/$other/$BRANCH" | wc -l)" = "$mirrored"
    check "publish $round: mirror.sh prints the SHA-256 of the summary it mirrored" \
      test "$summary" = "$(sha256sum <"$work/www/live/repo/summary" | cut -d ' ' -f 1)"
  fi

  make_build "$dir/build-$native" "$native" "$round"
  make_build "$dir/build-$other" "$other" "$round"
  expect_success "publish $round: add-builds" "$here/add-builds.sh" \
    --gpg-homedir "$keys/signer" --gpg-key "$signer" --subject "Test build $round" \
    "$dir/repo" "$dir/build-$native" "$dir/build-$other"
  expect_success "publish $round: update-repo" "$here/update-repo.sh" \
    --gpg-homedir "$keys/signer" --gpg-key "$signer" "$dir/repo"
  expect_success "publish $round: assemble-site" "$here/assemble-site.sh" \
    "$dir/repo" "$keys/signer.gpg" "$dir/site"
  [ -d "$dir/site" ] || break
  # (Before anything opens the copy with ostree, which makes them again.)
  check "publish $round: no ostree working files in the site" \
    test -z "$(ls -d "$dir/site/repo/tmp" "$dir/site/repo/state" 2>/dev/null || true)"

  kept=$((round < KEEP_BUILDS ? round : KEEP_BUILDS))
  for arch in "$native" "$other"; do
    for ref in "app/$APP_ID/$arch/$BRANCH" "runtime/$APP_ID.Locale/$arch/$BRANCH"; do
      check "publish $round: $ref keeps $kept build(s)" \
        test "$(ref_history "$dir/site/repo" "$ref" | wc -l)" = "$kept"
    done
    head=$(ostree --repo="$dir/site/repo" rev-parse "app/$APP_ID/$arch/$BRANCH")
    commits[$arch.$round]=$head
    check "publish $round: the $arch head is this build" \
      test "$(subject "$dir/site/repo" "$head")" = "Test build $round"
    check "publish $round: no debug info for $arch" \
      test -z "$(ostree --repo="$dir/site/repo" refs | grep -F ".Debug/" || true)"
  done
  # "Deploy".
  ln -sfn "$dir/site" "$work/www/live"
done

site="$work/round5/site"
for arch in "$native" "$other"; do
  for round in 1 2; do
    if [ -n "${commits[$arch.$round]:-}" ] && ! has_commit "$site/repo" "${commits[$arch.$round]}"; then
      pass "pruned: the $arch build of publish $round is gone"
    else
      fail "pruned: the $arch build of publish $round is gone"
    fi
  done
  check "kept: the $arch build of publish 3 is there" has_commit "$site/repo" "${commits[$arch.3]:-}"
done

# --- The site ---------------------------------------------------------------------

key_line() {
  sed -n "s/^$2=//p" "$1"
}
check "site: the .flatpakrepo points at $REPO_URL" \
  test "$(key_line "$site/$REMOTE_NAME.flatpakrepo" Url)" = "$REPO_URL"
for file in "$REMOTE_NAME.flatpakrepo" "$APP_ID.flatpakref"; do
  key_line "$site/$file" GPGKey | base64 -d >"$work/published.gpg"
  check "site: the key in $file is the signing key" \
    test "$("$here/signing-key.sh" check "$work/published.gpg" 2>/dev/null)" = "$signer"
  check "site: $file names Flathub for the runtime" \
    test "$(key_line "$site/$file" RuntimeRepo)" = "$FLATHUB_REPO_FILE"
done
check "site: cumulas-flatpak.gpg is the signing key" \
  test "$("$here/signing-key.sh" check "$site/cumulas-flatpak.gpg" 2>/dev/null)" = "$signer"
for line in "Name=$APP_ID" "Branch=$BRANCH" "IsRuntime=false" "SuggestRemoteName=$REMOTE_NAME" "Url=$REPO_URL"; do
  check "site: the .flatpakref has $line" grep -qx "$line" "$site/$APP_ID.flatpakref"
done
check "site: index.html shows the fingerprint" grep -q "${signer:0:4} ${signer:4:4}" "$site/index.html"
check "site: index.html has no scripts" test -z "$(grep -i '<script' "$site/index.html" || true)"
check "site: index.html loads nothing from elsewhere" \
  test -z "$(grep -Ei '(src|srcset)="(https?:)?//|<link[^>]*href="(https?:)?//|@import|url\(' "$site/index.html" || true)"
check "site: the icon is there" test -s "$site/logo.svg"
check "site: the repository's config records KEEP_BUILDS" \
  grep -qx "keep-builds=$KEEP_BUILDS" "$site/repo/config"

# --- What clients see -----------------------------------------------------------

sed "s|^Url=.*|Url=$www/live/repo/|" "$site/$REMOTE_NAME.flatpakrepo" >"$work/client.flatpakrepo"
expect_success "client: adds the remote from the .flatpakrepo" \
  flatpak remote-add --user --from "$REMOTE_NAME" "$work/client.flatpakrepo"
for arch in "$native" "$other"; do
  check "client: lists the $arch build (the summary's signature verifies)" \
    test "$(flatpak remote-ls --user --app --arch="$arch" --columns=application "$REMOTE_NAME" 2>/dev/null)" = "$APP_ID"
  check "client: sees $KEEP_BUILDS $arch builds in the history" \
    test "$(flatpak remote-info --user --log --arch="$arch" "$REMOTE_NAME" "$APP_ID" 2>/dev/null | grep -c 'Commit:')" = "$KEEP_BUILDS"
done

# --- Mirror failures ----------------------------------------------------------------

expect_failure "mirror: an existing destination" mirror "$work/round5" "$live"

mkdir -p "$work/www/missing-object"
cp -a "$site/." "$work/www/missing-object/"
object=$(find "$work/www/missing-object/repo/objects" -name '*.filez' -print -quit)
rm -- "$object"
expect_failure "mirror: a published repository with a missing object" \
  mirror "$work/mirror-missing-object" "$www/missing-object/repo"

mkdir -p "$work/www/corrupt-summary"
cp -a "$site/." "$work/www/corrupt-summary/"
head -c 4096 /dev/urandom >"$work/www/corrupt-summary/repo/summary"
expect_failure "mirror: a published repository with a corrupt summary" \
  mirror "$work/mirror-corrupt-summary" "$www/corrupt-summary/repo"

mkdir -p "$work/www/no-summary/repo"
cp "$site/repo/config" "$work/www/no-summary/repo/"
expect_failure "mirror: a published repository with a config but no summary" \
  mirror "$work/mirror-no-summary" "$www/no-summary/repo"

mkdir -p "$work/www/other-app"
ostree --repo="$work/www/other-app/repo" init --mode=archive
flatpak build-update-repo --gpg-homedir="$keys/signer" --gpg-sign="$signer" \
  "$work/www/other-app/repo" >/dev/null
expect_failure "mirror: a published repository without the app" \
  mirror "$work/mirror-other-app" "$www/other-app/repo"

mkdir -p "$work/www/other-key"
cp -a "$site/." "$work/www/other-key/"
flatpak build-update-repo --gpg-homedir="$keys/other" --gpg-sign="$(cat "$keys/other.fpr")" \
  "$work/www/other-key/repo" >/dev/null
expect_error "mirror: a published summary signed with a key it isn't given" "isn't signed with" \
  mirror "$work/mirror-other-key" "$www/other-key/repo"
expect_error "mirror: a published summary signed with a key it isn't given, in a dry run" \
  "isn't signed with" mirror --dry-run "$work/mirror-other-key-dry-run" "$www/other-key/repo"
expect_success "mirror: a published summary signed with one of the keys it's given" \
  mirror --gpg-import "$keys/other.gpg" "$work/mirror-two-keys" "$www/other-key/repo"
mkdir -p "$work/www/unsigned"
cp -a "$site/." "$work/www/unsigned/"
rm -- "$work/www/unsigned/repo/summary.sig"
expect_error "mirror: a published summary without a signature" "isn't signed with" \
  mirror "$work/mirror-unsigned" "$www/unsigned/repo"
expect_error "mirror: a public key that isn't there" "doesn't exist" \
  "$here/mirror.sh" --gpg-import "$keys/missing.gpg" "$work/mirror-missing-key" "$live"

# A commit missing in the middle of the published history: ostree takes it
# for the start of the history, and would quietly drop the builds before it.
mkdir -p "$work/www/missing-commit"
cp -a "$site/." "$work/www/missing-commit/"
middle=$(ostree --repo="$work/www/missing-commit/repo" rev-parse "app/$APP_ID/$native/$BRANCH^")
rm -- "$work/www/missing-commit/repo/objects/${middle:0:2}/${middle:2}.commit"
expect_error "mirror: a published history with a commit missing" "history stops after 1 build" \
  mirror "$work/mirror-missing-commit" "$www/missing-commit/repo"
if mirror --dry-run "$work/mirror-missing-commit-dry-run" "$www/missing-commit/repo" \
  >"$work/output" 2>&1; then
  check "mirror: a published history with a commit missing, in a dry run, is a warning" \
    grep -q "^warning: .*history stops after 1 build" "$work/output"
else
  fail "mirror: a published history with a commit missing, in a dry run, is a warning"
  cat "$work/output" >&2
fi

# After KEEP_BUILDS is raised, the published refs have fewer builds until
# enough are published, and the published config says so.
mkdir -p "$work/www/raised"
cp -a "$site/." "$work/www/raised/"
ostree --repo="$work/www/raised/repo" prune --refs-only --depth=$((KEEP_BUILDS - 2)) >/dev/null
ostree --repo="$work/www/raised/repo" config set cumulas.keep-builds $((KEEP_BUILDS - 1))
expect_success "mirror: a published history as long as its config says, shorter than KEEP_BUILDS" \
  mirror "$work/mirror-raised" "$www/raised/repo"
check "mirror: ... all of that history" \
  test "$(ref_history "$work/mirror-raised" "app/$APP_ID/$native/$BRANCH" | wc -l)" = $((KEEP_BUILDS - 1))
ostree --repo="$work/www/raised/repo" config set cumulas.keep-builds "$KEEP_BUILDS"
expect_error "mirror: the same history, when its config says it keeps KEEP_BUILDS" \
  "history stops after $((KEEP_BUILDS - 1)) build" mirror "$work/mirror-raised-2" "$www/raised/repo"

error_port=$(free_port)
python3 -c '
import http.server, sys
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_error(500)
    do_HEAD = do_GET
    def log_message(self, *args):
        pass
http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
' "$error_port" &
servers+=("$!")
wait_for "http://127.0.0.1:$error_port/"
expect_failure "mirror: a server error" \
  mirror "$work/mirror-500" "http://127.0.0.1:$error_port/repo"
expect_failure "mirror: a server error, in a dry run" \
  mirror --dry-run "$work/mirror-500-dry-run" "http://127.0.0.1:$error_port/repo"

unused_port=$(free_port)
expect_failure "mirror: an unreachable host" \
  mirror "$work/mirror-unreachable" "http://127.0.0.1:$unused_port/repo"
if summary=$(mirror --dry-run "$work/mirror-unreachable-dry-run" \
  "http://127.0.0.1:$unused_port/repo" 2>"$work/output"); then
  check "mirror: an unreachable host, in a dry run, starts empty" test "$summary" = none
else
  fail "mirror: an unreachable host, in a dry run, starts empty"
  cat "$work/output" >&2
fi

# --- Installing -----------------------------------------------------------------

expect_success "smoke-test.sh installs the last publish" \
  env SMOKE_FLATHUB_REPO_FILE="$www/flathub/flathub.flatpakrepo" "$here/smoke-test.sh" "$site"

if [ "$failures" != 0 ]; then
  die "$failures check(s) failed."
fi
log "All checks passed."

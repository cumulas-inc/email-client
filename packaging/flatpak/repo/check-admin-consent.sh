#!/usr/bin/env bash
# Checks the page for IT administrators, published at
# https://flatpak.cumulas.co/admin-consent/, against the app:
#
#  - its approval link is the one the app builds for the published app
#    registration (crates/core/src/auth/microsoft.rs and
#    service/connector.rs): the client ID, every permission, Sync.Receive
#    when ADMIN_LINK_INCLUDES_PUSH says so, and this page as the redirect
#    URI, which must be ADMIN_CONSENT_LANDING_URI on this site; rebuilt
#    here from those, and the same as PUBLISHED_LINK_WITH_PUSH (or
#    _WITHOUT_PUSH), which the app's tests hold its builder to; and
#    docs/microsoft-365.md shows the same link; the page's table and the docs
#    name Sync.Receive as approved by it only when it asks for it;
#  - it names every permission the app asks for, the roles that can
#    approve, the admin consent workflow, the app and publisher IDs, and has
#    a <noscript> note;
#  - it loads nothing from elsewhere, its one script is inline and never
#    writes HTML, and the script shows each answer Microsoft can bring an
#    administrator back with, and nothing else of the address
#    (check-admin-consent.mjs);
#  - the install page links to it.
#
# CI runs it on every pull request (ci.yml) and test.sh on the assembled
# site, so a change to the app's permissions fails until the page matches.
#
# Usage: check-admin-consent.sh [PAGE]
#   PAGE  the page; admin-consent.html next to this script unless given
# Needs node.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
# shellcheck source=SCRIPTDIR/common.sh
. "$here/common.sh"

if [ $# -gt 1 ]; then
  echo "usage: $0 [PAGE]" >&2
  exit 2
fi
page=${1:-$here/admin-consent.html}
microsoft=$root/crates/core/src/auth/microsoft.rs
connector=$root/crates/core/src/service/connector.rs
docs=$root/docs/microsoft-365.md
landing=https://flatpak.cumulas.co/admin-consent/
publisher_id=7118869
oidc_scopes=(openid profile email offline_access)

[ -s "$page" ] || die "$page is missing or empty."
for file in "$microsoft" "$connector" "$docs"; do
  [ -s "$file" ] || die "$file is missing."
done

failures=0
pass() {
  log "ok - $1"
}
fail() {
  log "FAIL - $1"
  failures=$((failures + 1))
}
check() {
  local what=$1
  shift
  if "$@"; then pass "$what"; else fail "$what"; fi
}

# rust_const FILE NAME: the declaration of `pub const NAME`, joined into one
# line, from its first line to the ";" that ends it.
rust_const() {
  awk -v name="$2" '
    $0 ~ "^pub const " name ":" { found = 1 }
    found {
      line = line " " $0
      if ($0 ~ /;[[:space:]]*(\/\/.*)?$/) { print line; exit }
    }
  ' "$1"
}
# The string literals in a declaration, one per line.
strings() {
  grep -o '"[^"]*"' | tr -d '"'
}
# form_encode TEXT: as url::form_urlencoded, which the app builds the link
# with, encodes a query value (ASCII): a space becomes "+".
form_encode() {
  local text=$1 out="" c i
  for ((i = 0; i < ${#text}; i++)); do
    c=${text:i:1}
    case $c in
      [A-Za-z0-9*._-]) out+=$c ;;
      " ") out+=+ ;;
      *) out+=$(printf '%%%02X' "'$c") ;;
    esac
  done
  printf '%s' "$out"
}
is_oidc() {
  local scope
  for scope in "${oidc_scopes[@]}"; do
    [ "$1" = "$scope" ] && return 0
  done
  return 1
}

client_id=$(rust_const "$connector" PUBLISHED_MICROSOFT_CLIENT_ID | strings)
landing_uri=$(rust_const "$microsoft" ADMIN_CONSENT_LANDING_URI | strings)
includes_push=$(rust_const "$microsoft" ADMIN_LINK_INCLUDES_PUSH | sed -n 's/.*=[[:space:]]*\(true\|false\);.*/\1/p')
mapfile -t core_scopes < <(rust_const "$microsoft" CORE_SCOPES | strings)
mapfile -t optional_scopes < <(rust_const "$microsoft" OPTIONAL_SCOPES | strings)
[ -n "$client_id" ] || die "No PUBLISHED_MICROSOFT_CLIENT_ID in $connector."
[ -n "$landing_uri" ] || die "No ADMIN_CONSENT_LANDING_URI in $microsoft."
[ -n "$includes_push" ] || die "No ADMIN_LINK_INCLUDES_PUSH (true or false) in $microsoft."
[ "${#core_scopes[@]}" -gt 0 ] || die "No CORE_SCOPES in $microsoft."
[ "${#optional_scopes[@]}" -gt 0 ] || die "No OPTIONAL_SCOPES in $microsoft."

# --- Where Microsoft brings administrators back -----------------------------------

repo_host=${REPO_URL#https://}
repo_host=${repo_host%%/*}
check "ADMIN_CONSENT_LANDING_URI is $landing (it is $landing_uri)" test "$landing_uri" = "$landing"
check "the landing page is on the repository's site, $repo_host" \
  test "$landing_uri" = "https://$repo_host/admin-consent/"

# --- The approval link --------------------------------------------------------------

# As the app builds it for the published registration and the tenant
# "organizations": the OpenID Connect scopes, then each Graph permission in
# CORE_SCOPES and OPTIONAL_SCOPES order, qualified, then the push service's.
scopes=("${oidc_scopes[@]}")
for scope in "${core_scopes[@]}" "${optional_scopes[@]}"; do
  is_oidc "$scope" || scopes+=("https://graph.microsoft.com/$scope")
done
if [ "$includes_push" = true ]; then
  scopes+=("api://$client_id/Sync.Receive")
fi
endpoint=https://login.microsoftonline.com/organizations/v2.0/adminconsent
client_param="client_id=$(form_encode "$client_id")"
scope_param="scope=$(form_encode "${scopes[*]}")"
redirect_param="redirect_uri=$(form_encode "$landing_uri")"
# As the docs show it, and as it is in the page's HTML.
plain_link="$endpoint?$client_param&$scope_param&$redirect_param"
link="$endpoint?$client_param&amp;$scope_param&amp;$redirect_param"

# The link the app's test holds admin_consent_url to, so that the page and
# the docs (checked against $plain_link below) follow the real builder, not
# only this rebuild of it.
published=PUBLISHED_LINK_WITHOUT_PUSH
if [ "$includes_push" = true ]; then
  published=PUBLISHED_LINK_WITH_PUSH
fi
app_link=$(sed -n "s/^[[:space:]]*const $published: &str = \"\([^\"]*\)\";.*/\1/p" "$microsoft")
[ -n "$app_link" ] || die "No $published in $microsoft."
check "the link is $published, which the app's tests build" test "$plain_link" = "$app_link"

links=$(grep -o 'https://login\.microsoftonline\.com/[^"<[:space:]]*adminconsent[^"<[:space:]]*' "$page" | sort -u || true)
check "the page has the approval link (ADMIN_LINK_INCLUDES_PUSH = $includes_push)" \
  grep -qF -- "href=\"$link\"" "$page"
check "every approval link in the page is that one" test "$links" = "$link"
if [ "$includes_push" = false ]; then
  check "no approval link asks for Sync.Receive" test -z "$(grep Sync.Receive <<<"$links" || true)"
fi
check "docs/microsoft-365.md shows the same link" grep -qxF -- "$plain_link" "$docs"
# Whether the page's table and the docs say the link approves Sync.Receive,
# with line breaks as single spaces, since Prettier may wrap a phrase
# anywhere.
names_push() {
  if tr -s '[:space:]' ' ' <"$1" | grep -qF -- "$2"; then echo true; else echo false; fi
}
check "the page's table lists Sync.Receive only when the link asks for it" \
  test "$(names_push "$page" "<code>Sync.Receive</code>")" = "$includes_push"
check "docs/microsoft-365.md says the link asks for Sync.Receive only when it does" \
  test "$(names_push "$docs" "above plus \`Sync.Receive\`")" = "$includes_push"
check "every client ID in the page is the published one, $client_id" \
  test "$(grep -oiE '[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}' "$page" | tr 'A-F' 'a-f' | sort -u)" \
  = "$(tr 'A-F' 'a-f' <<<"$client_id")"

# --- What the page says ---------------------------------------------------------------

# The page with its line breaks and indentation as single spaces, since
# Prettier may wrap a phrase anywhere.
flat=$(tr -s '[:space:]' ' ' <"$page")
says() {
  grep -qF -- "$1" <<<"$flat"
}
for scope in "${core_scopes[@]}" "${optional_scopes[@]}"; do
  check "the page names $scope" says "<code>$scope</code>"
done
for role in "Global Administrator" "Privileged Role Administrator" "Cloud Application Administrator"; do
  check "the page names the role $role" says "$role"
done
check "the page names the role Application Administrator" \
  grep -qF "Application Administrator" <<<"${flat//Cloud Application Administrator/}"
check "the page says where Admin consent requests are" says "Admin consent requests"
check "the page shows the app ID" says "<code>$client_id</code>"
check "the page shows the publisher ID" says "<code>$publisher_id</code>"
check "the page has a <noscript> note" says "<noscript>"
# Without the script, a failure is only in the address: Microsoft shows none.
check "the <noscript> note says an error in the address means it wasn't approved" \
  says "contains <code>error=</code>, Cumulas Mail wasn't approved"

# --- Nothing from elsewhere, no HTML from the address ---------------------------------

check "the page has one inline script" test "$(grep -oi '<script[^>]*>' "$page")" = "<script>"
check "the page loads nothing from elsewhere" \
  test -z "$(grep -Ei '(src|srcset)="(https?:)?//|<link[^>]*href="(https?:)?//|@import|url\(' "$page" || true)"
check "the page never writes HTML or runs text as code" \
  test -z "$(grep -E 'innerHTML|outerHTML|insertAdjacentHTML|document\.write|eval\(|Function\(' "$page" || true)"
check "the install page links to the page" grep -qF 'href="admin-consent/"' "$here/index.html"

if node "$here/check-admin-consent.mjs" "$page"; then
  pass "the page's script shows each answer from Microsoft"
else
  fail "the page's script shows each answer from Microsoft"
fi

if [ "$failures" -gt 0 ]; then
  die "$failures check(s) of $page failed."
fi
log "$page matches the app."

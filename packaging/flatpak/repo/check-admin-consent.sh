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
#  - the install page links to it;
#  - the setup app, Cumulas Mail - Admin Setup (crates/core/src/auth/
#    admin_setup.rs), stays apart from the mail app: no SETUP_ONLY_MARKERS
#    string (its API, scope and resource apps) in the mail app's scopes,
#    its approval links, the page's approval link or the docs' steps for
#    registering it, and the two client IDs differ;
#  - with PUBLISHED_SETUP_CLIENT_ID set, the page has the setup app's
#    section (id="setup-app"): its client ID, the permissions it asks for
#    (SETUP_CONSENT_SCOPES), and a setup script that requires assignment and
#    assigns admins before it approves the app for the organization, with
#    exactly those permissions; no link to approve it for everyone; the
#    notice for consent policies made from the page's earlier version
#    (id="consent-policy-notice"), which keeps the setup app out; and the
#    setup app's approval link, built here from the constants, is
#    PUBLISHED_SETUP_LINK, which the app's tests build, and the docs show
#    it. Unset: no such section;
#  - the client IDs in the page are the mail app's and the setup app's, and
#    the app IDs it looks up or names as an API (appId eq '…',
#    -ResourceApplication) are those, Microsoft Graph's, and, in the setup
#    app's section only, Office 365 Exchange Online's;
#  - its consent policy script (id="consent-policy-script"), and the
#    notice's commands that narrow an earlier policy (id="consent-policy-fix"),
#    name the API of each include; exactly one include is Microsoft Graph's,
#    and it alone carries -Permissions $ids, built from Graph's permissions
#    and limited to exactly those in CORE_SCOPES and OPTIONAL_SCOPES, with
#    a stop when one isn't found; and nothing outside the notice includes
#    apps by publisher (ClientApplicationPublisherIds);
#  - the setup app's script stops at the first error, checks that
#    assignment is required before it approves the app, and approves, for
#    the setup app ($sp) and for everyone, exactly Exchange.Manage on Office
#    365 Exchange Online and the OpenID Connect scopes on Microsoft Graph.
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
admin_setup=$root/crates/core/src/auth/admin_setup.rs
connector=$root/crates/core/src/service/connector.rs
docs=$root/docs/microsoft-365.md
landing=https://flatpak.cumulas.co/admin-consent/
publisher_id=7118869
oidc_scopes=(openid profile email offline_access)
graph_app=00000003-0000-0000-c000-000000000000
exchange_app=00000002-0000-0ff1-ce00-000000000000
guid='[0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}'

[ -s "$page" ] || die "$page is missing or empty."
for file in "$microsoft" "$admin_setup" "$connector" "$docs"; do
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

# The setup app: PUBLISHED_SETUP_CLIENT_ID is None, or Some("<GUID>").
setup_decl=$(rust_const "$admin_setup" PUBLISHED_SETUP_CLIENT_ID)
if grep -qE '=[[:space:]]*None[[:space:]]*;' <<<"$setup_decl"; then
  setup_id=
elif grep -qE '=[[:space:]]*Some\([[:space:]]*"[^"]*"[[:space:]]*,?[[:space:]]*\)[[:space:]]*;' <<<"$setup_decl"; then
  setup_id=$(strings <<<"$setup_decl")
  grep -qxE "$guid" <<<"$setup_id" ||
    die "PUBLISHED_SETUP_CLIENT_ID in $admin_setup isn't a GUID: $setup_id"
else
  die "No PUBLISHED_SETUP_CLIENT_ID (None or Some(\"<GUID>\")) in $admin_setup."
fi
mapfile -t setup_scopes < <(rust_const "$admin_setup" SETUP_CONSENT_SCOPES | strings)
mapfile -t setup_markers < <(rust_const "$admin_setup" SETUP_ONLY_MARKERS | strings)
setup_redirect=$(rust_const "$admin_setup" SETUP_REDIRECT_URI | strings)
[ "${#setup_scopes[@]}" -gt 0 ] || die "No SETUP_CONSENT_SCOPES in $admin_setup."
[ "${#setup_markers[@]}" -gt 0 ] || die "No SETUP_ONLY_MARKERS in $admin_setup."
[ -n "$setup_redirect" ] || die "No SETUP_REDIRECT_URI in $admin_setup."

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

# --- The setup app stays apart from the mail app -------------------------------------

# Whether TEXT names none of SETUP_ONLY_MARKERS, ASCII case ignored, as the
# app's test does (form-encoding leaves them as they are).
lacks_setup_markers() {
  local marker
  for marker in "${setup_markers[@]}"; do
    if grep -qiF -- "$marker" <<<"$1"; then return 1; fi
  done
}
# The docs' "## Registering the app (publisher)", up to the next "## ".
mail_registration=$(awk '
  /^## / { inside = ($0 == "## Registering the app (publisher)") }
  inside
' "$docs")
[ -n "$mail_registration" ] || die "No \"## Registering the app (publisher)\" in $docs."
mail_links=$(sed -n 's/^[[:space:]]*const PUBLISHED_LINK_WITH\(OUT\)\{0,1\}_PUSH: &str = "\([^"]*\)";.*/\2/p' "$microsoft")
check "CORE_SCOPES names nothing of the setup app's" lacks_setup_markers "${core_scopes[*]}"
check "OPTIONAL_SCOPES names nothing of the setup app's" lacks_setup_markers "${optional_scopes[*]}"
check "PUBLISHED_LINK_WITH_PUSH and _WITHOUT_PUSH name nothing of the setup app's" \
  lacks_setup_markers "$mail_links"
check "the page's approval link names nothing of the setup app's" lacks_setup_markers "$links"
check "the docs' steps for registering the mail app name nothing of the setup app's" \
  lacks_setup_markers "$mail_registration"
if [ -n "$setup_id" ]; then
  check "the setup app's client ID isn't the mail app's" \
    test "$(tr 'A-F' 'a-f' <<<"$setup_id")" != "$(tr 'A-F' 'a-f' <<<"$client_id")"
fi

# --- The setup app ----------------------------------------------------------------------

# The page's section for it, and the parts of the page outside it.
setup_section=$(awk '
  /<section id="setup-app"/ { inside = 1 }
  inside { print }
  inside && /<\/section>/ { exit }
' "$page")
outside_setup=$(awk '
  /<section id="setup-app"/ { inside = 1 }
  !inside { print }
  inside && /<\/section>/ { inside = 0 }
' "$page")
# The text of a <pre> by its id, from the line with the id to its </pre>.
pre_by_id() {
  awk -v id="id=\"$1\"" '
    index($0, id) { inside = 1 }
    inside { print }
    inside && /<\/pre>/ { exit }
  '
}
# The line number of the first line with TEXT in the input, or a large one.
first_line() {
  local n
  n=$(grep -nF -- "$1" | head -n 1 | cut -d: -f1 || true)
  echo "${n:-999999}"
}
# Whether the first line of the input with $1 comes before the first with $2.
comes_before() {
  local text
  text=$(cat)
  [ "$(first_line "$1" <<<"$text")" -lt "$(first_line "$2" <<<"$text")" ]
}
# Each PowerShell command of the input on one line: its lines continued
# with a backtick joined.
commands() {
  awk '{
    line = $0
    if (sub(/`[[:space:]]*$/, "", line)) { joined = joined line " "; next }
    print joined line
    joined = ""
  }'
}
# The commands of the input that run CMDLET.
runs() {
  grep -E -- "(^|[[:space:](])$1( |$)" || true
}

if [ -n "$setup_id" ]; then
  setup_flat=$(tr -s '[:space:]' ' ' <<<"$setup_section")
  in_setup() {
    grep -qF -- "$1" <<<"$setup_flat"
  }
  check "the page has the setup app's section (id=\"setup-app\")" test -n "$setup_section"
  check "the setup app's section shows its client ID" in_setup "<code>$setup_id</code>"
  # Its permissions, as people approve them: the API's name only.
  approved=()
  for scope in "${setup_scopes[@]}"; do
    approved+=("${scope##*/}")
    check "the setup app's section names ${scope##*/}" in_setup "<code>${scope##*/}</code>"
  done
  setup_script=$(pre_by_id setup-app-script <<<"$setup_section")
  check "the setup app's section has its setup script (id=\"setup-app-script\")" \
    test -n "$setup_script"
  check "the setup script names the setup app's client ID" grep -qF -- "$setup_id" <<<"$setup_script"
  check "the setup script approves the app for the organization" \
    grep -qF -- "New-MgOauth2PermissionGrant" <<<"$setup_script"
  check "the setup script requires assignment before it approves the app" \
    comes_before "AppRoleAssignmentRequired:\$true" "New-MgOauth2PermissionGrant" <<<"$setup_script"
  check "the setup script assigns admins before it approves the app" \
    comes_before "New-MgServicePrincipalAppRoleAssignedTo" "New-MgOauth2PermissionGrant" <<<"$setup_script"
  granted=$(grep -oE -- '-Scope "[^"]*"' <<<"$setup_script" | sed 's/^-Scope "\(.*\)"$/\1/' |
    tr -s ' ' '\n' | sort -u)
  check "the setup script approves exactly SETUP_CONSENT_SCOPES" \
    test "$granted" = "$(printf '%s\n' "${approved[@]}" | sort -u)"
  check "the setup script stops at its first error, before it changes anything" \
    comes_before "\$ErrorActionPreference = 'Stop'" "Update-MgServicePrincipal" <<<"$setup_script"
  check "the setup script stops at its first error, before it creates the app" \
    comes_before "\$ErrorActionPreference = 'Stop'" "New-MgServicePrincipal " <<<"$setup_script"
  # A check that assignment is required, which stops it, before any approval.
  rechecked=$(grep -nE -- 'if \(-not .*\.AppRoleAssignmentRequired\)' <<<"$setup_script" | head -n 1 | cut -d: -f1 || true)
  check "the setup script checks that assignment is required before it approves the app" \
    test "${rechecked:-999999}" -lt "$(first_line "New-MgOauth2PermissionGrant" <<<"$setup_script")"
  # Each approval: for the setup app, for everyone, and what for which API,
  # by the app ID the variable it names was looked up with.
  setup_commands=$(commands <<<"$setup_script")
  declare -A looked_up=()
  while read -r var app; do
    looked_up[$var]=$app
  done < <(sed -nE "s/^\\\$([A-Za-z0-9_]+) = Get-MgServicePrincipal +-Filter \"appId eq '([^'\$]*)'\".*/\1 \2/p" <<<"$setup_commands")
  exchange_granted=()
  graph_granted=()
  grants_ok=true
  while IFS= read -r grant; do
    [ -n "$grant" ] || continue
    # shellcheck disable=SC2016 # a PowerShell variable
    var=$(sed -nE 's/.*-ResourceId \$([A-Za-z0-9_]+)\.Id( .*|$)/\1/p' <<<"$grant")
    scope=$(sed -nE 's/.*-Scope "([^"]*)".*/\1/p' <<<"$grant")
    grep -qF -- "-ClientId \$sp.Id" <<<"$grant" || grants_ok=false
    grep -qF -- "-ConsentType AllPrincipals" <<<"$grant" || grants_ok=false
    case "${looked_up[$var]:-}" in
      "$exchange_app") read -ra words <<<"$scope" && exchange_granted+=("${words[@]}") ;;
      "$graph_app") read -ra words <<<"$scope" && graph_granted+=("${words[@]}") ;;
      *) grants_ok=false ;;
    esac
  done < <(runs New-MgOauth2PermissionGrant <<<"$setup_commands")
  want_exchange=()
  want_graph=()
  for scope in "${setup_scopes[@]}"; do
    case $scope in
      https://outlook.office365.com/*) want_exchange+=("${scope#https://outlook.office365.com/}") ;;
      */*) want_exchange+=("unexpected:$scope") ;;
      *) want_graph+=("$scope") ;;
    esac
  done
  sorted() { printf '%s\n' "$@" | sort -u; }
  check "the setup script approves only for the setup app, for everyone, on Exchange or Graph" \
    "$grants_ok"
  check "the setup script approves on Office 365 Exchange Online exactly its SETUP_CONSENT_SCOPES" \
    test "$(sorted "${exchange_granted[@]}")" = "$(sorted "${want_exchange[@]}")"
  check "the setup script approves on Microsoft Graph exactly its SETUP_CONSENT_SCOPES" \
    test "$(sorted "${graph_granted[@]}")" = "$(sorted "${want_graph[@]}")"

  notice=$(awk '
    /<aside id="consent-policy-notice"/ { inside = 1 }
    inside { print }
    inside && /<\/aside>/ { exit }
  ' "$page")
  check "the page has the notice for earlier consent policies (id=\"consent-policy-notice\")" \
    test -n "$notice"
  # Its exclude: the command, with its continued lines.
  exclude=$(awk '
    /New-MgPolicyPermissionGrantPolicyExclude/ { inside = 1 }
    inside { print; if ($0 !~ /`[[:space:]]*$/) exit }
  ' <<<"$notice" | tr -s '[:space:]`' ' ')
  check "the notice keeps the setup app out of the consent policy" \
    grep -qF -- "-ClientApplicationIds \"$setup_id\"" <<<"$exclude"

  # The link the app shows administrators of a tenant: here for the tenant
  # "organizations", with the OpenID Connect scopes as they are.
  setup_link="$endpoint?client_id=$(form_encode "$setup_id")"
  setup_link+="&scope=$(form_encode "${setup_scopes[*]}")"
  setup_link+="&redirect_uri=$(form_encode "$setup_redirect")"
  setup_app_link=$(sed -n 's/^[[:space:]]*const PUBLISHED_SETUP_LINK: &str = "\([^"]*\)";.*/\1/p' "$admin_setup")
  [ -n "$setup_app_link" ] || die "No PUBLISHED_SETUP_LINK in $admin_setup."
  check "the setup app's approval link is PUBLISHED_SETUP_LINK, which the app's tests build" \
    test "$setup_link" = "$setup_app_link"
  check "docs/microsoft-365.md shows the setup app's approval link" \
    grep -qxF -- "$setup_link" <<<"$(sed 's/^[[:space:]]*//' "$docs")"
else
  check "without a setup app, the page has no section for it" \
    test -z "$(grep -F 'id="setup-app"' "$page" || true)"
fi

# --- App IDs in the page -----------------------------------------------------------------

lower_ids() {
  grep -oE -- "$guid" | tr 'A-F' 'a-f' | sort -u
}
# Lines that look an app up or name the API a permission is of: their IDs
# are resource apps, not apps that sign in.
resource_line='appId eq|-ResourceApplication'
clients=$(grep -vE -- "$resource_line" "$page" | lower_ids || true)
expected=$client_id
which="the published one, $client_id"
if [ -n "$setup_id" ]; then
  expected+=$'\n'$setup_id
  which+=", or the setup app's, $setup_id"
fi
check "every client ID in the page is $which" test "$clients" = "$(lower_ids <<<"$expected")"
# Allowed: Microsoft Graph, the mail app (its own API) and the setup app;
# Office 365 Exchange Online only in the setup app's section.
only_ids() {
  local found id
  found=$(grep -E -- "$resource_line" | lower_ids || true)
  for id in $found; do
    grep -qxF -- "$id" <<<"$(lower_ids <<<"$*")" || return 1
  done
}
check "the page looks up only Microsoft Graph, the mail app and the setup app outside the setup app's section" \
  only_ids "$graph_app" "$client_id" "$setup_id" <<<"$outside_setup"
check "the setup app's section looks up only those and Office 365 Exchange Online" \
  only_ids "$graph_app" "$exchange_app" "$client_id" "$setup_id" <<<"$setup_section"

# --- The consent policy -------------------------------------------------------------------

# The policy's includes as the script ($1, the <pre>'s id) adds them. Its
# patterns name PowerShell variables, which stay as they are.
# shellcheck disable=SC2016
check_consent_includes() {
  local id=$1 script cmds includes graph_includes others named
  script=$(pre_by_id "$id" <"$page")
  check "the page has the consent policy commands (id=\"$id\")" test -n "$script"
  cmds=$(commands <<<"$script")
  includes=$(runs New-MgPolicyPermissionGrantPolicyInclude <<<"$cmds")
  check "$id: every include of the consent policy names its API (-ResourceApplication)" \
    test -n "$includes" -a -z "$(grep -vF -- "-ResourceApplication" <<<"$includes" || true)"
  graph_includes=$(grep -F -- "-ResourceApplication \"$graph_app\"" <<<"$includes" || true)
  others=$(grep -vF -- "-ResourceApplication \"$graph_app\"" <<<"$includes" || true)
  check "$id: exactly one include is Microsoft Graph's" \
    test "$(grep -c . <<<"$graph_includes" || true)" = 1
  check "$id: Microsoft Graph's include is limited to -Permissions \$ids" \
    grep -qE -- '-Permissions \$ids( |$)' <<<"$graph_includes"
  check "$id: no other include has -Permissions" \
    test -z "$(grep -F -- "-Permissions" <<<"$others" || true)"
  check "$id: \$ids are Microsoft Graph's permissions" \
    grep -qE -- '^\$ids = .*\$graph\.Oauth2PermissionScopes' <<<"$cmds"
  check "$id: it stops at its first error" \
    comes_before '$ErrorActionPreference = "Stop"' "New-MgPolicyPermissionGrantPolicyInclude" <<<"$script"
  # Without every permission, -Permissions would be empty: all of Graph's.
  check "$id: it stops when a permission isn't found, before the includes" \
    comes_before 'if ($ids.Count -ne $names.Count)' "New-MgPolicyPermissionGrantPolicyInclude" <<<"$script"
  # The permissions it names: its single-quoted names, but app IDs.
  named=$(grep -oE "'[^']*'" <<<"$script" | tr -d "'" | grep -vxE -- "$guid" | sort -u || true)
  check "$id: the consent policy names exactly the permissions in CORE_SCOPES and OPTIONAL_SCOPES" \
    test "$named" = "$(printf '%s\n' "${core_scopes[@]}" "${optional_scopes[@]}" | sort -u)"
}
check_consent_includes consent-policy-script
check_consent_includes consent-policy-fix
without_notice=$(awk '
  /<aside id="consent-policy-notice"/ { inside = 1 }
  !inside { print }
  inside && /<\/aside>/ { inside = 0 }
' "$page")
check "nothing outside the notice includes apps by publisher (ClientApplicationPublisherIds)" \
  test -z "$(grep -F ClientApplicationPublisherIds <<<"$without_notice" || true)"

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

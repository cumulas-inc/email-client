#!/usr/bin/env bash
# Copies the published repository, with the history it still has, into a new
# local repository, so that new builds are added on top of it.
#
# Usage: mirror.sh [--dry-run] [--gpg-import PUBLIC-KEY]... DEST [URL]
#   DEST          the local repository to create; it must not exist
#   URL           the published repository; REPO_URL (common.sh) by default
#   --gpg-import  a public key the published summary may be signed with;
#                 repeatable. By default, the committed keys: PUBLIC_KEY and,
#                 after a rotation, PREVIOUS_PUBLIC_KEY (common.sh).
#
# - URL/summary answers 200: its signature must verify with one of the keys,
#   and then the app's refs (MANAGED_REFS in common.sh) and their history are
#   mirrored. Any failure on the way fails the script.
# - URL/summary and URL/config both answer 404: nothing has been published
#   yet, and DEST starts empty.
# - Anything else (another HTTP status, a TLS or DNS error, a repository with
#   none of the app's refs) fails the script, so that a publish never drops
#   the published history.
# - With --dry-run, a host that doesn't resolve or doesn't answer (the site
#   isn't set up yet) also starts DEST empty, with a warning: nothing is
#   published from a dry run.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=SCRIPTDIR/common.sh
. "$here/common.sh"

usage() {
  echo "usage: $0 [--dry-run] [--gpg-import PUBLIC-KEY]... DEST [URL]" >&2
  exit 2
}

dry_run=false
keys=()
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) dry_run=true; shift ;;
    --gpg-import) [ $# -ge 2 ] || usage; keys+=("$2"); shift 2 ;;
    --) shift; break ;;
    -*) usage ;;
    *) break ;;
  esac
done
if [ $# -lt 1 ] || [ $# -gt 2 ]; then
  usage
fi
if [ ${#keys[@]} = 0 ]; then
  for key in "$PUBLIC_KEY" "$PREVIOUS_PUBLIC_KEY"; do
    [ ! -e "$key" ] || keys+=("$key")
  done
fi
dest=$1
url=${2:-$REPO_URL}
url=${url%/}
[ ! -e "$dest" ] || die "$dest already exists."

# The HTTP status of URL, or "unreachable" when no HTTP answer came (DNS,
# connection refused or timed out). Other curl failures (TLS...) fail.
http_status() {
  local status rc=0
  status=$(curl --silent --show-error --location --output /dev/null --write-out '%{http_code}' \
    --connect-timeout 20 --max-time 120 --retry 3 --retry-delay 5 "$1") || rc=$?
  case "$rc" in
    0) printf '%s\n' "$status" ;;
    6 | 7 | 28) printf 'unreachable\n' ;;
    *) die "Fetching $1 failed (curl exit status $rc)." ;;
  esac
}

start_empty() {
  ostree --repo="$dest" init --mode=archive
  log "Started an empty repository in $dest."
}

summary_status=$(http_status "$url/summary")
case "$summary_status" in
  200) ;;
  404)
    config_status=$(http_status "$url/config")
    if [ "$config_status" != 404 ]; then
      die "$url/summary answers 404 but $url/config answers $config_status: that isn't an unpublished repository. Check the site before publishing."
    fi
    warn "Nothing is published at $url yet (404): starting a new repository. Expected until the first publish; after that, check GitHub Pages and its custom domain."
    start_empty
    exit 0
    ;;
  unreachable)
    if $dry_run; then
      warn "$url can't be reached (DNS or connection failure): this dry run starts from an empty repository. A publish would fail until the site is set up."
      start_empty
      exit 0
    fi
    die "$url can't be reached (DNS or connection failure). Publishing needs the site set up (packaging/flatpak/README.md, \"Self-hosted repository\") or reachable again."
    ;;
  *)
    die "$url/summary answers HTTP $summary_status; publishing would drop the published history. Check the site and run again."
    ;;
esac

[ ${#keys[@]} -gt 0 ] || die "No public key to check the summary of $url with: commit cumulas-flatpak.gpg (packaging/flatpak/README.md, \"Setting it up\")."
for key in "${keys[@]}"; do
  [ -s "$key" ] || die "$key doesn't exist or is empty."
done

ostree --repo="$dest" init --mode=archive
# The summary must be signed with one of the keys: it names each ref's
# commit, and each commit names its parent by checksum, so nothing that
# isn't signed can join the history a publish adds to and signs (say, from
# someone serving the domain). The commits' own signatures aren't checked:
# after a key rotation, the older builds keep the old key's.
ostree --repo="$dest" remote add --set=gpg-verify=false --set=gpg-verify-summary=true \
  published "$url"
import_args=()
for key in "${keys[@]}"; do
  import_args+=(--keyring="$key")
done
ostree --repo="$dest" remote gpg-import published "${import_args[@]}" >&2
published_refs=$(ostree --repo="$dest" remote refs published) ||
  die "Reading the summary of $url failed, or it isn't signed with ${keys[*]}. Publishing would drop the published history, or add to one that isn't signed: check the site and the committed keys."
mapfile -t refs < <(printf '%s\n' "$published_refs" | sed 's/^published://' | grep -E "$MANAGED_REFS" || true)
if [ ${#refs[@]} = 0 ]; then
  die "$url has none of the refs these scripts publish ($MANAGED_REFS); refusing to replace it."
fi
log "Mirroring ${refs[*]} from $url"
ostree --repo="$dest" pull --mirror --depth=-1 published "${refs[@]}" ||
  die "Mirroring $url failed; publishing would drop the published history. Run again, or check the site."
ostree --repo="$dest" remote delete published
ostree --repo="$dest" fsck --quiet >&2 || die "The mirror of $url is corrupt."

for ref in "${refs[@]}"; do
  log "$ref: $(ref_history "$dest" "$ref" | wc -l) build(s) mirrored"
done

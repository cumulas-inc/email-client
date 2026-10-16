#!/usr/bin/env bash
# Checks, just before a deploy, that the published repository is still the
# one this run mirrored. The site adds to the history mirror.sh found: if
# anything was deployed since (a run of another tag, an older run re-run),
# deploying the site would drop it.
#
# Usage: check-live-summary.sh [--url URL] MIRRORED-SUMMARY
#   MIRRORED-SUMMARY  what mirror.sh printed: the SHA-256 of the summary it
#                     mirrored, or "none" when nothing was published
#   --url             the published repository; REPO_URL (common.sh) by default
#
# - URL/summary answers 200: it must be the summary mirrored.
# - It answers 404: nothing must have been mirrored either ("none").
# - Anything else (another HTTP status, no answer) fails: the deploy waits
#   until the site can be checked.
# A new query string each time gets past GitHub Pages' cache.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=SCRIPTDIR/common.sh
. "$here/common.sh"

usage() {
  sed -n '2,/^set -euo/{/^set -euo/d;s/^# \{0,1\}//;p}' "$0" >&2
  exit 2
}

url=$REPO_URL
while [ $# -gt 0 ]; do
  case "$1" in
    --url) [ $# -ge 2 ] || usage; url=$2; shift 2 ;;
    --) shift; break ;;
    -*) usage ;;
    *) break ;;
  esac
done
if [ $# != 1 ]; then
  usage
fi
mirrored=$1
url=${url%/}

scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT

status=$(curl --silent --show-error --location --connect-timeout 20 --max-time 120 \
  --retry 3 --retry-delay 5 --output "$scratch/summary" --write-out '%{http_code}' \
  "$url/summary?check=${GITHUB_RUN_ID:-local}.${GITHUB_RUN_ATTEMPT:-1}.$RANDOM$RANDOM") ||
  die "Fetching $url/summary failed; not deploying over what may be there. Check the site and run the deploy again."
case "$status" in
  200) live=$(sha256sum "$scratch/summary" | cut -d ' ' -f 1) ;;
  404) live=none ;;
  *) die "$url/summary answers HTTP $status; not deploying over it. Check the site and run the deploy again." ;;
esac

if [ -z "$mirrored" ] || [ "$live" != "$mirrored" ]; then
  die "The published repository changed since this run mirrored it (summary ${mirrored:-unknown}, now $live): deploying would drop what was published in between. Run the newest release tag's workflow again, all jobs."
fi
if [ "$live" = none ]; then
  echo "Nothing is published at $url yet, as when this run mirrored it."
else
  echo "The published summary is still $live, the one this run added to."
fi

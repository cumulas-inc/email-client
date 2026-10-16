#!/usr/bin/env bash
# Checks that a release tag can be published: it's vMAJOR.MINOR.PATCH (no
# pre-releases), the highest such tag in the repository, and the version of
# the newest <release> in the metainfo, the one software centers show.
# Publishing a lower tag would replace newer builds with older ones.
#
# Usage: check-release.sh TAG [METAINFO] <TAGS
#   TAG       the tag to publish
#   METAINFO  co.cumulas.Mail.metainfo.xml, next to the manifest, by default
#   TAGS      the repository's tags, one per line (the workflow gets them from
#             the GitHub API; locally, git tag)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=SCRIPTDIR/common.sh
. "$here/common.sh"

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
  echo "usage: $0 TAG [METAINFO] <TAGS" >&2
  exit 2
fi
tag=$1
metainfo=${2:-$here/../$APP_ID.metainfo.xml}
release='^v[0-9]+\.[0-9]+\.[0-9]+$'

printf '%s\n' "$tag" | grep -Eq "$release" ||
  die "$tag isn't a release tag (vMAJOR.MINOR.PATCH, without a pre-release part)."

highest=$({ cat; printf '%s\n' "$tag"; } | grep -E "$release" | sort -V | tail -n 1)
if [ "$highest" != "$tag" ]; then
  die "$tag isn't the newest release: $highest is. Publishing $tag would replace newer builds with older ones. To publish again, run the workflow of $highest."
fi

version=$(sed -n '/<release version="/{s/.*<release version="\([^"]*\)".*/\1/p;q}' "$metainfo")
[ -n "$version" ] || die "$metainfo has no <release version=\"...\">."
if [ "$version" != "${tag#v}" ]; then
  die "The newest release in $(basename "$metainfo") is $version, not ${tag#v}: software centers would show $version and its notes. Add a <release> for ${tag#v} first, then tag that commit."
fi
log "$tag is the newest release tag and the newest release in $(basename "$metainfo")."

#!/usr/bin/env bash
# Builds the app into an ostree repository with the flags CI uses, so that
# the CI build (.github/workflows/ci.yml) and the published one
# (flatpak-repo.yml) are built the same way.
#
# Usage: build-repo.sh REPO [FLATPAK-BUILDER-OPTION...]
# build-dir/ and .flatpak-builder/ go in the current directory. Needs
# flatpak-builder, the Flathub remote in the user installation, the
# generated *-sources.json (generate-sources.sh) and shared-modules/
# (fetch-shared-modules.sh).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
if [ $# -lt 1 ]; then
  echo "usage: $0 REPO [FLATPAK-BUILDER-OPTION...]" >&2
  exit 2
fi
repo=$1
shift

flatpak-builder --user --install-deps-from=flathub --disable-rofiles-fuse --force-clean \
  --repo="$repo" "$@" build-dir "$here/co.cumulas.Mail.yml"

#!/usr/bin/env bash
# Regenerates the offline dependency sources Flatpak builds need
# (cargo-sources.json and node-sources.json next to this script).
# Run after changing Cargo.lock or package-lock.json.
#
# The generators come from flatpak-builder-tools at the commit below, fetched
# into FLATPAK_BUILDER_TOOLS (a cache directory by default) unless it's there.
# They need git, and python3 with the packages pinned, with their hashes, in
# .github/actions/flatpak-sources/requirements.txt; in a virtualenv:
#   pip install --require-hashes --only-binary :all: -r .github/actions/flatpak-sources/requirements.txt
set -euo pipefail

# flatpak-builder-tools' master on 2026-09-30 ("node: Support pnpm v12").
commit=74697c75b630d7330e77250fc13cb5ea688d9479
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
tools="${FLATPAK_BUILDER_TOOLS:-${XDG_CACHE_HOME:-$HOME/.cache}/flatpak-builder-tools-$commit}"

if [ ! -d "$tools" ]; then
  # Into a new directory first, so that a failed fetch leaves nothing behind.
  rm -rf -- "$tools.new"
  git init -q "$tools.new"
  git -C "$tools.new" fetch -q --depth 1 https://github.com/flatpak/flatpak-builder-tools "$commit"
  git -C "$tools.new" checkout -q FETCH_HEAD
  mv -- "$tools.new" "$tools"
fi
actual=$(git -C "$tools" rev-parse HEAD 2>/dev/null || true)
if [ "$actual" != "$commit" ]; then
  echo "warning: $tools is at ${actual:-an unknown commit}, not $commit" >&2
fi

python3 "$tools/cargo/flatpak-cargo-generator.py" "$root/Cargo.lock" -o "$here/cargo-sources.json"
PYTHONPATH="$tools/node${PYTHONPATH:+:$PYTHONPATH}" \
  python3 -m flatpak_node_generator npm "$root/package-lock.json" -o "$here/node-sources.json"

echo "Wrote $here/cargo-sources.json and $here/node-sources.json"

#!/usr/bin/env bash
# Regenerates the offline dependency sources Flatpak builds need
# (cargo-sources.json and node-sources.json next to this script).
# Run after changing Cargo.lock or package-lock.json.
#
# Needs: git, python3 with aiohttp and tomlkit, pipx (or pip).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
tools="${FLATPAK_BUILDER_TOOLS:-${XDG_CACHE_HOME:-$HOME/.cache}/flatpak-builder-tools}"

if [ ! -d "$tools" ]; then
  git clone --depth 1 https://github.com/flatpak/flatpak-builder-tools "$tools"
fi

python3 "$tools/cargo/flatpak-cargo-generator.py" "$root/Cargo.lock" -o "$here/cargo-sources.json"

if command -v flatpak-node-generator >/dev/null; then
  generator=(flatpak-node-generator)
else
  generator=(pipx run --spec "$tools/node" flatpak-node-generator)
fi
"${generator[@]}" npm "$root/package-lock.json" -o "$here/node-sources.json"

echo "Wrote $here/cargo-sources.json and $here/node-sources.json"

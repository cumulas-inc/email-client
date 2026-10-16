#!/usr/bin/env bash
# Fetches Flathub's shared modules (libayatana-appindicator, for the tray)
# into shared-modules/ next to the manifest, at the commit builds use. In the
# Flathub repository they're a git submodule checked out at this commit.
set -euo pipefail

commit=cb9ec602a1ece1c76d5a4f8aa1d87c4a6bf99c3e
here="$(cd "$(dirname "$0")" && pwd)"

git init -q "$here/shared-modules"
git -C "$here/shared-modules" fetch -q --depth 1 https://github.com/flathub/shared-modules "$commit"
git -C "$here/shared-modules" checkout -q FETCH_HEAD

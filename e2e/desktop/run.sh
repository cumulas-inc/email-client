#!/usr/bin/env bash
# Runs the desktop end-to-end test against a local GreenMail server, inside a
# private D-Bus session with GNOME Keyring and a virtual display.
#
# Needs: Java 17+, Xvfb, webkit2gtk-driver (WebKitWebDriver), gnome-keyring,
# libsecret-tools, tauri-driver (`cargo install tauri-driver --locked`),
# python3 with the packages in requirements.txt.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
work="$(mktemp -d)"
greenmail_pid=""
cleanup() {
  local status=$?
  if [ -n "$greenmail_pid" ]; then kill "$greenmail_pid" 2>/dev/null || true; fi
  for job in $(jobs -p); do kill "$job" 2>/dev/null || true; done
  rm -rf "$work" 2>/dev/null || true
  exit "$status"
}
trap cleanup EXIT
export E2E_OUTPUT="${E2E_OUTPUT:-$root/e2e/.output/desktop}"
# Screenshots and logs (GreenMail's, tauri-driver's and the app's, which
# app.sh adds to), for when a test fails.
mkdir -p "$E2E_OUTPUT"
rm -f "$E2E_OUTPUT/app.log"

greenmail_jar="${GREENMAIL_JAR:-${XDG_CACHE_HOME:-$HOME/.cache}/cumulas/greenmail-standalone-2.1.14.jar}"
if [ ! -f "$greenmail_jar" ]; then
  mkdir -p "$(dirname "$greenmail_jar")"
  curl -fsSL -o "$greenmail_jar" \
    https://repo1.maven.org/maven2/com/icegreen/greenmail-standalone/2.1.14/greenmail-standalone-2.1.14.jar
fi

echo "Building the app with the bundled UI…"
npm run build >/dev/null
cargo build -p cumulas-mail --features tauri/custom-protocol
export CUMULAS_APP="$root/target/debug/cumulas-mail"
# The shell's debug lines too: the network monitor, notifications, the tray.
export RUST_LOG="${RUST_LOG:-info,cumulas_core=debug,cumulas_mail_lib=debug}"

java -Dgreenmail.setup.test.smtp -Dgreenmail.setup.test.imap -Dgreenmail.hostname=127.0.0.1 \
  -Dgreenmail.users=alice:secret@example.com,bob:secret@example.com \
  -jar "$greenmail_jar" >"$E2E_OUTPUT/greenmail.log" 2>&1 &
greenmail_pid=$!
until (exec 3<>/dev/tcp/127.0.0.1/3143) 2>/dev/null; do sleep 0.5; done
python3 e2e/desktop/seed_mail.py 127.0.0.1 3025

if [ -z "${DISPLAY:-}" ]; then
  Xvfb :97 -screen 0 1440x900x24 >/dev/null 2>&1 &
  export DISPLAY=:97
fi

# Fresh app data for every run.
export XDG_DATA_HOME="$work/data" XDG_CACHE_HOME="$work/cache" XDG_CONFIG_HOME="$work/config"
export XDG_RUNTIME_DIR="$work/runtime"
mkdir -m 700 "$XDG_RUNTIME_DIR"

if (exec 3<>/dev/tcp/127.0.0.1/4444) 2>/dev/null; then
  echo "Port 4444 is in use (a tauri-driver left over from another run?)" >&2
  exit 1
fi

dbus-run-session -- bash -c '
  set -euo pipefail
  trap "kill \$(jobs -p) 2>/dev/null || true" EXIT
  echo -n e2e | gnome-keyring-daemon --unlock --components=secrets --daemonize >/dev/null
  tauri-driver >"$E2E_OUTPUT/tauri-driver.log" 2>&1 &
  until (exec 3<>/dev/tcp/127.0.0.1/4444) 2>/dev/null; do sleep 0.2; done
  python3 e2e/desktop/test_desktop.py
  python3 e2e/desktop/test_tray.py
'

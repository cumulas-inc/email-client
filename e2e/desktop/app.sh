#!/bin/sh
# Starts the app ($CUMULAS_APP) with its output added to app.log in
# $E2E_OUTPUT, which CI keeps: the app logs to stdout, and tauri-driver drops
# the stdout of the apps it starts. The tests start the app through this.
log="${E2E_OUTPUT:-e2e/.output}/app.log"
mkdir -p "$(dirname "$log")"
echo "=== $(date +%T) cumulas-mail $*" >>"$log"
exec "${CUMULAS_APP:?the app binary}" "$@" >>"$log" 2>&1

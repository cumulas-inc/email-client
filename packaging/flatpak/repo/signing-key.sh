#!/usr/bin/env bash
# The key that signs the Flatpak repository.
#
#   signing-key.sh import GNUPGHOME PUBLIC-KEY
#       Imports the private key in $FLATPAK_GPG_PRIVATE_KEY (ASCII-armored,
#       without a passphrase) into GNUPGHOME, an empty directory, after
#       checking that it is the key published in PUBLIC-KEY (the committed
#       cumulas-flatpak.gpg): same fingerprint, and a signature made with it
#       verifies against PUBLIC-KEY alone, as clients will verify it.
#       Prints the fingerprint.
#   signing-key.sh throwaway GNUPGHOME PUBLIC-KEY-OUT
#       Generates a throwaway key in GNUPGHOME for dry runs and tests, and
#       writes its public key to PUBLIC-KEY-OUT. Prints the fingerprint.
#   signing-key.sh check PUBLIC-KEY
#       Checks that PUBLIC-KEY holds exactly one public key, usable for
#       signing. Prints its fingerprint.
#   signing-key.sh remove GNUPGHOME
#       Stops GNUPGHOME's gpg-agent and deletes the directory.
#
# Only the fingerprint goes to stdout; messages go to stderr. The private key
# is never written anywhere but GNUPGHOME.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=SCRIPTDIR/common.sh
. "$here/common.sh"

usage() {
  sed -n '2,/^set -euo/{/^set -euo/d;s/^# \{0,1\}//;p}' "$0" >&2
  exit 2
}

# Keyrings for reading and verifying public keys; they never hold the
# private key.
scratch=$(mktemp -d)
cleanup() {
  rm -rf -- "$scratch"
}
trap cleanup EXIT

scratch_home() {
  local dir
  dir=$(mktemp -d "$scratch/gnupg.XXXXXX")
  chmod 700 "$dir"
  printf '%s\n' "$dir"
}

# The fingerprint of the one key in FILE, after checking that FILE holds
# exactly one valid public key that can sign.
public_key_fingerprint() {
  local file=$1 home listing count fpr
  [ -e "$file" ] || die "$file doesn't exist. Commit the repository's public signing key there (packaging/flatpak/README.md, \"Self-hosted repository\")."
  [ -s "$file" ] || die "$file is empty: it must hold the repository's public signing key."
  home=$(scratch_home)
  listing=$(gpg --homedir "$home" --batch --with-colons --show-keys "$file" 2>/dev/null) ||
    die "$file isn't an OpenPGP public key."
  if printf '%s\n' "$listing" | grep -q '^sec:'; then
    die "$file holds a private key. Commit only the public key (gpg --export)."
  fi
  count=$(printf '%s\n' "$listing" | grep -c '^pub:' || true)
  [ "$count" = 1 ] || die "$file must hold exactly one public key; it holds $count."
  printf '%s\n' "$listing" | awk -F: '$1 == "pub" && ($2 == "e" || $2 == "r" || $2 == "d" || $2 == "i") { exit 1 }' ||
    die "The key in $file is expired, revoked or otherwise invalid."
  printf '%s\n' "$listing" | awk -F: '$1 == "pub" && $12 ~ /S/ { found = 1 } END { exit !found }' ||
    die "The key in $file can't sign (it has no usable signing key or subkey)."
  fpr=$(printf '%s\n' "$listing" | awk -F: '$1 == "pub" { want = 1; next } want && $1 == "fpr" { print $10; exit }')
  [ -n "$fpr" ] || die "Couldn't read the fingerprint of the key in $file."
  printf '%s\n' "$fpr"
}

new_gnupghome() {
  local home=$1
  [ -n "$home" ] || die "No GNUPGHOME given."
  mkdir -p -- "$home"
  [ -z "$(ls -A -- "$home")" ] || die "$home isn't empty."
  chmod 700 "$home"
}

cmd_check() {
  [ $# = 1 ] || usage
  public_key_fingerprint "$1"
}

cmd_import() {
  [ $# = 2 ] || usage
  local home=$1 public_key=$2 secret expected errors listing count actual verify_home data status primary
  # Keep the key out of the environment of everything this starts (gpg-agent
  # outlives this script).
  secret=${FLATPAK_GPG_PRIVATE_KEY:-}
  unset FLATPAK_GPG_PRIVATE_KEY
  [ -n "$secret" ] || die "The secret FLATPAK_GPG_PRIVATE_KEY is empty or not set."
  if [ "${GITHUB_ACTIONS:-}" = true ]; then
    # Secrets are masked in the log already; this also masks each line on
    # its own.
    printf '%s\n' "$secret" | while IFS= read -r line; do
      line=${line%$'\r'}
      case "$line" in
        "" | -----* | Version:* | Comment:*) ;;
        *) printf '::add-mask::%s\n' "$line" >&2 ;;
      esac
    done
  fi

  expected=$(public_key_fingerprint "$public_key")
  new_gnupghome "$home"
  # (gpg's messages name keys, never their material.)
  errors=$(printf '%s\n' "$secret" | gpg --homedir "$home" --batch --pinentry-mode error --import 2>&1 >/dev/null) ||
    die "Importing FLATPAK_GPG_PRIVATE_KEY failed: it must be an ASCII-armored OpenPGP private key without a passphrase. $errors"
  secret=
  listing=$(gpg --homedir "$home" --batch --with-colons --list-secret-keys 2>/dev/null)
  count=$(printf '%s\n' "$listing" | grep -c '^sec:' || true)
  [ "$count" = 1 ] || die "FLATPAK_GPG_PRIVATE_KEY must hold exactly one private key; it holds $count."
  actual=$(printf '%s\n' "$listing" | awk -F: '$1 == "sec" { want = 1; next } want && $1 == "fpr" { print $10; exit }')
  if [ "$actual" != "$expected" ]; then
    die "FLATPAK_GPG_PRIVATE_KEY is the key $actual, but $public_key is $expected. Update one of them (packaging/flatpak/README.md, \"Rotating the signing key\")."
  fi

  # Sign as ostree will, then verify with nothing but the committed public
  # key, as a client that added the remote will.
  verify_home=$(scratch_home)
  data="$verify_home/data"
  printf 'Cumulas Flatpak repository signing check\n' >"$data"
  errors=$(gpg --homedir "$home" --batch --pinentry-mode error --local-user "$expected" \
    --detach-sign --output "$data.sig" "$data" 2>&1 >/dev/null) ||
    die "Signing with FLATPAK_GPG_PRIVATE_KEY failed: it must have no passphrase and a signing key that hasn't expired. $errors"
  gpg --homedir "$verify_home" --batch --import "$public_key" 2>/dev/null
  status=$(gpg --homedir "$verify_home" --batch --status-fd 1 --verify "$data.sig" "$data" 2>/dev/null) || true
  primary=$(printf '%s\n' "$status" | awk '$1 == "[GNUPG:]" && $2 == "VALIDSIG" { print $NF }')
  if ! printf '%s\n' "$status" | grep -q '^\[GNUPG:\] GOODSIG ' || [ "$primary" != "$expected" ]; then
    die "A signature made with FLATPAK_GPG_PRIVATE_KEY doesn't verify with $public_key alone (an expired or missing signing subkey?). Export the public key again and commit it."
  fi
  log "Signing key: $expected"
  printf '%s\n' "$expected"
}

cmd_throwaway() {
  [ $# = 2 ] || usage
  local home=$1 out=$2 errors listing fpr
  new_gnupghome "$home"
  errors=$(gpg --homedir "$home" --batch --passphrase '' --quick-generate-key \
    "Cumulas Flatpak dry run (throwaway) <noreply@cumulas.co>" rsa4096 sign 2d 2>&1 >/dev/null) ||
    die "Couldn't generate a throwaway key in $home: $errors"
  listing=$(gpg --homedir "$home" --batch --with-colons --list-secret-keys 2>/dev/null)
  fpr=$(printf '%s\n' "$listing" | awk -F: '$1 == "sec" { want = 1; next } want && $1 == "fpr" { print $10; exit }')
  gpg --homedir "$home" --batch --export "$fpr" >"$out"
  log "Throwaway signing key: $fpr"
  printf '%s\n' "$fpr"
}

cmd_remove() {
  [ $# = 1 ] || usage
  local home=$1
  case "$home" in
    /*/*) ;;
    *) die "Refusing to remove '$home': not an absolute path below /." ;;
  esac
  [ "$home" != "${HOME:-}" ] || die "Refusing to remove the home directory."
  [ -d "$home" ] || return 0
  gpgconf --homedir "$home" --kill all 2>/dev/null || true
  rm -rf -- "$home"
  log "Removed $home"
}

[ $# -ge 1 ] || usage
command=$1
shift
case "$command" in
  import) cmd_import "$@" ;;
  throwaway) cmd_throwaway "$@" ;;
  check) cmd_check "$@" ;;
  remove) cmd_remove "$@" ;;
  *) usage ;;
esac

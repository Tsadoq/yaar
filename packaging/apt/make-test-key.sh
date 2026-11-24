#!/bin/bash
# Makes a THROWAWAY signing key for CI and tests (no passphrase, expires in 7 days), never the
# repository's.
# make-test-key.sh [<gnupg-home>]; without an argument the keyring is a new temporary folder.
# Stdout: GNUPGHOME and YAAR_APT_KEY assignments for `eval`; gpg's messages go to stderr.
set -euo pipefail
umask 077
if [ $# -ge 1 ]; then
  HOME_DIR=$1; mkdir -p "$HOME_DIR"
else
  HOME_DIR=$(mktemp -d "${TMPDIR:-/tmp}/yaar-test-key.XXXXXX")
fi
HOME_DIR=$(cd "$HOME_DIR" && pwd)
chmod 700 "$HOME_DIR"
cat > "$HOME_DIR/params" <<'PARAMS'
%no-protection
Key-Type: EDDSA
Key-Curve: ed25519
Key-Usage: sign
Name-Real: YAAR TEST KEY (throwaway, not for releases)
Name-Email: test@example.invalid
Expire-Date: 7d
%commit
PARAMS
gpg --homedir "$HOME_DIR" --batch --generate-key "$HOME_DIR/params" >&2
rm -f "$HOME_DIR/params"
FPR=$(gpg --homedir "$HOME_DIR" --batch --with-colons --list-secret-keys | awk -F: '$1=="fpr"{print $10; exit}')
[ -n "$FPR" ] || { echo "make-test-key.sh: no key was made" >&2; exit 1; }
gpg --homedir "$HOME_DIR" --batch --list-keys >&2
printf 'GNUPGHOME=%q\n' "$HOME_DIR"
printf 'YAAR_APT_KEY=%s\n' "$FPR"

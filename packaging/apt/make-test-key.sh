#!/bin/bash
# Makes a THROWAWAY signing key for CI and tests. It has no passphrase and expires after
# seven days. It is NOT the key of the published repository and must never sign a release.
#
#   make-test-key.sh [<gnupg-home>]
#
# Without an argument the keyring is a new temporary folder. Standard output is two shell
# assignments and nothing else (gpg's own messages go to standard error), so a caller can
# take them over:
#
#   eval "$(packaging/apt/make-test-key.sh)"
#   export GNUPGHOME YAAR_APT_KEY
#   packaging/apt/make-apt-repo.sh <repo-dir> <deb>...
#
# Delete the folder afterwards:  gpgconf --homedir "$GNUPGHOME" --kill gpg-agent; rm -rf "$GNUPGHOME"
#
# Needs: gpg (gnupg).
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

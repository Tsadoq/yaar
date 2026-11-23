#!/bin/sh
# Prints the one version number of the app, the `version` of `[workspace.package]` in the
# Cargo.toml at the root of the repository, and nothing else.
#
#   packaging/version.sh
#
# Everything else follows from it: the tag of a release is `v<version>`; the .deb's
# Version, the bundle's CFBundleShortVersionString and CFBundleVersion, and the name of
# every release file carry it as it is; `yaar-review --version` prints it, since every
# crate takes the workspace's version.
#
# A release version is three numbers, `X.Y.Z`, each without a leading zero: what the two
# version strings of a Mac bundle may hold, and what apt orders as a user expects. Anything
# else is refused, a pre-release such as `0.2.0-rc.1` among them.
#
# Exit codes: 0 the version printed; 1 none found, or not X.Y.Z (a sentence on standard
# error says which).
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
VERSION=$(awk '
  /^\[/ { inside = ($0 == "[workspace.package]") ; next }
  inside && /^version[ \t]*=/ {
    sub(/^version[ \t]*=[ \t]*"/, ""); sub(/".*$/, ""); print; exit
  }' "$ROOT/Cargo.toml")
if [ -z "$VERSION" ]; then
  echo "version.sh: Cargo.toml has no version under [workspace.package]" >&2
  exit 1
fi
case $VERSION in
  *[!0-9.]* | .* | *. | *..* | 0[0-9]* | *.0[0-9]*)
    echo "version.sh: a release version is three numbers X.Y.Z, not '$VERSION'" >&2
    exit 1 ;;
esac
# Exactly two dots.
rest=${VERSION#*.}; rest=${rest#*.}
case $rest in
  *.* | "") echo "version.sh: a release version is three numbers X.Y.Z, not '$VERSION'" >&2; exit 1 ;;
esac
case $VERSION in
  *.*.*) ;;
  *) echo "version.sh: a release version is three numbers X.Y.Z, not '$VERSION'" >&2; exit 1 ;;
esac
printf '%s\n' "$VERSION"

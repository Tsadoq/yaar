#!/bin/sh
# Prints the app version: `version` under [workspace.package] in the root Cargo.toml.
# Only X.Y.Z without leading zeros is accepted (no 0.2.0-rc.1): bundles and apt need it. Exit 1:
# none or invalid.
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
rest=${VERSION#*.}; rest=${rest#*.}
case $rest in
  *.* | "") echo "version.sh: a release version is three numbers X.Y.Z, not '$VERSION'" >&2; exit 1 ;;
esac
case $VERSION in
  *.*.*) ;;
  *) echo "version.sh: a release version is three numbers X.Y.Z, not '$VERSION'" >&2; exit 1 ;;
esac
printf '%s\n' "$VERSION"

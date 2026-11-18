#!/bin/sh
# Rebuilds vendor/gpui-pre-apple-<version> from the crate as published and reapplies
# patches/gpui-frame-source.patch. Run it whenever the GPUI packages are raised.
#
#   patches/vendor-gpui.sh <version> [path to a downloaded .crate]
#
# The sha256 of the published crate must be listed in patches/gpui-pre-apple.sha256.
# Take it from the `checksum` line of gpui-pre-apple in a Cargo.lock that was resolved
# without the [patch] entry, or from crates.io.
set -eu
VERSION=${1:?usage: vendor-gpui.sh <version> [path to a downloaded .crate]}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
NAME=gpui-pre-apple-$VERSION
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT

if [ $# -ge 2 ]; then
  cp "$2" "$tmp/$NAME.crate"
else
  curl --proto '=https' --tlsv1.2 -sSfL -A "yaar-vendor-script" \
    -o "$tmp/$NAME.crate" "https://static.crates.io/crates/gpui-pre-apple/$NAME.crate"
fi

if command -v sha256sum >/dev/null; then got=$(sha256sum "$tmp/$NAME.crate" | cut -d' ' -f1)
else got=$(shasum -a 256 "$tmp/$NAME.crate" | cut -d' ' -f1); fi
want=$(awk -v v="$VERSION" '$1 == v { print $2 }' "$ROOT/patches/gpui-pre-apple.sha256")
if [ -z "$want" ]; then
  echo "No checksum for $VERSION in patches/gpui-pre-apple.sha256." >&2
  echo "This download has sha256 $got. Check it against crates.io, then add the line:" >&2
  echo "  $VERSION $got" >&2
  exit 1
fi
if [ "$got" != "$want" ]; then
  echo "Checksum mismatch for $NAME.crate: got $got, want $want." >&2
  exit 1
fi

tar -xzf "$tmp/$NAME.crate" -C "$tmp"
if ! ( cd "$tmp/$NAME" && patch -p1 --no-backup-if-mismatch < "$ROOT/patches/gpui-frame-source.patch" ); then
  echo "The patch no longer applies to $NAME. Read patches/README.md before changing it." >&2
  exit 1
fi

rm -rf "$ROOT"/vendor/gpui-pre-apple-*
mkdir -p "$ROOT/vendor"
mv "$tmp/$NAME" "$ROOT/vendor/$NAME"
# Cargo's own marker of an unpacked registry crate; meaningless in a vendored copy.
rm -f "$ROOT/vendor/$NAME/.cargo-ok"

# Point the [patch] entry of the workspace at the new folder.
sed -i.bak -E "s|(gpui-pre-apple = \{ path = \"vendor/)gpui-pre-apple-[^\"]+|\1$NAME|" "$ROOT/Cargo.toml"
rm -f "$ROOT/Cargo.toml.bak"

echo "vendor/$NAME is the published crate plus the frame-source patch."
echo "Next: cargo update -p gpui-pre-apple, the tests, and the speed check on the reference Mac."

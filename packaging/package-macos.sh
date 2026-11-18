#!/bin/bash
# Builds YAAR.app, a .zip and a .dmg from release programs that are already built.
# Opens nothing and mounts nothing.
#
#   package-macos.sh <version> <out-dir> <window-program>... -- <keeper-program>...
#
# One program per processor on each side of `--`; two are merged into a universal file.
#
# Environment:
#   SIGN_IDENTITY   "-" (default) signs ad hoc, which Gatekeeper rejects on another Mac.
#                   "Developer ID Application: Name (TEAMID)" signs for distribution.
#   NOTARY_PROFILE  A keychain profile made with `xcrun notarytool store-credentials`.
#                   Set together with a Developer ID identity, it UPLOADS the app and the
#                   disk image to Apple, waits for the verdict and staples the tickets.
#                   Unset, nothing leaves this machine.
#   NOTICES         The third-party notices to put inside the app, from packaging/notices.sh.
#                   Required: a package without them may not be handed to anyone.
#   MIN_MACOS       LSMinimumSystemVersion (default 12.0, the declared floor).
#   BUILD_NUMBER    CFBundleVersion (default 1).
#
# Layout:
#   YAAR.app/Contents/MacOS/yaar-review      the Window (CFBundleExecutable)
#   YAAR.app/Contents/Helpers/yaar-keeper    the Keeper, signed with its own identifier
#   YAAR.app/Contents/Resources/AppIcon.icns
#   YAAR.app/Contents/Resources/LICENSE, THIRD-PARTY-NOTICES.txt
set -euo pipefail
VER=$1; OUT=$2; shift 2
WINDOW=(); KEEPER=(); side=window
for a in "$@"; do
  if [ "$a" = "--" ]; then side=keeper
  elif [ "$side" = window ]; then WINDOW+=("$a")
  else KEEPER+=("$a"); fi
done
[ ${#WINDOW[@]} -gt 0 ] && [ ${#KEEPER[@]} -gt 0 ] || { echo "usage: $0 <version> <out-dir> <window>... -- <keeper>..." >&2; exit 2; }

[ -n "${NOTICES:-}" ] && [ -s "$NOTICES" ] || { echo "$0: set NOTICES to the file packaging/notices.sh wrote" >&2; exit 2; }

HERE=$(cd "$(dirname "$0")" && pwd)
ID=io.github.tsadoq.yaar
SIGN_IDENTITY=${SIGN_IDENTITY:--}
MIN_MACOS=${MIN_MACOS:-12.0}
mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
APP="$OUT/YAAR.app"
ZIP="$OUT/yaar-review-$VER-macos.zip"
DMG="$OUT/yaar-review-$VER-macos.dmg"
rm -rf "$APP" "$ZIP" "$DMG"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Helpers" "$APP/Contents/Resources"

merge() { # <out> <in>...
  local out=$1; shift
  if [ $# -gt 1 ]; then lipo -create "$@" -output "$out"; else cp "$1" "$out"; fi
  chmod 755 "$out"
}
merge "$APP/Contents/MacOS/yaar-review" "${WINDOW[@]}"
merge "$APP/Contents/Helpers/yaar-keeper" "${KEEPER[@]}"

# The icon: one PNG scaled into an icon set. sips and iconutil ship with macOS.
WORK=$(mktemp -d); trap 'rm -rf "$WORK"' EXIT
I="$WORK/AppIcon.iconset"; mkdir -p "$I"
for s in 16 32 128 256 512; do
  sips -z $s $s "$HERE/macos/icon-1024.png" --out "$I/icon_${s}x${s}.png" > /dev/null
  sips -z $((s*2)) $((s*2)) "$HERE/macos/icon-1024.png" --out "$I/icon_${s}x${s}@2x.png" > /dev/null
done
iconutil -c icns "$I" -o "$APP/Contents/Resources/AppIcon.icns"

cp "$HERE/../LICENSE" "$APP/Contents/Resources/LICENSE"
cp "$NOTICES" "$APP/Contents/Resources/THIRD-PARTY-NOTICES.txt"

sed -e "s/@VERSION@/$VER/" -e "s/@BUILD@/${BUILD_NUMBER:-1}/" -e "s/@MINOS@/$MIN_MACOS/" \
  "$HERE/macos/Info.plist.in" > "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist" > /dev/null
printf 'APPL????' > "$APP/Contents/PkgInfo"

# Sign from the inside out: the Keeper first, with its own identifier, then the bundle,
# which seals the Keeper. Hardened runtime, no entitlement.
TS=(--timestamp); [ "$SIGN_IDENTITY" = "-" ] && TS=(--timestamp=none)
codesign --force --sign "$SIGN_IDENTITY" --options runtime "${TS[@]}" \
  --identifier "$ID.keeper" "$APP/Contents/Helpers/yaar-keeper"
codesign --force --sign "$SIGN_IDENTITY" --options runtime "${TS[@]}" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"

NOTARIZE=no
if [ "$SIGN_IDENTITY" != "-" ] && [ -n "${NOTARY_PROFILE:-}" ]; then NOTARIZE=yes; fi

if [ "$NOTARIZE" = yes ]; then
  # Apple takes an archive, not a folder. The ticket is then stapled to the app itself,
  # so that the .zip and the .dmg made below both carry an app that opens offline.
  ditto -c -k --keepParent "$APP" "$WORK/submit.zip"
  xcrun notarytool submit "$WORK/submit.zip" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$APP"
fi

ditto -c -k --keepParent "$APP" "$ZIP"

S="$WORK/dmg"; mkdir -p "$S"; cp -R "$APP" "$S/"; ln -s /Applications "$S/Applications"
hdiutil create -quiet -volname "YAAR" -srcfolder "$S" -fs HFS+ -format UDZO -imagekey zlib-level=9 "$DMG"
codesign --force --sign "$SIGN_IDENTITY" "${TS[@]}" "$DMG"

if [ "$NOTARIZE" = yes ]; then
  xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$DMG"
fi

( cd "$OUT" && shasum -a 256 "$(basename "$ZIP")" "$(basename "$DMG")" > SHA256SUMS-macos && cat SHA256SUMS-macos )
ls -l "$OUT"

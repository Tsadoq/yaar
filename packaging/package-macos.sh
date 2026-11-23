#!/bin/bash
# Builds YAAR.app, a .zip and a .dmg from release programs that are already built.
# Opens nothing and mounts nothing.
#
#   package-macos.sh <version> <out-dir> <programs-dir>...
#
# Each <programs-dir> holds the five programs of one processor as cargo builds them
# (target/<triple>/release): yaar-review, yaar-keeper, yaar-claude-hook, yaar-claude-tools,
# yaar-pi-report. Two folders are merged into universal programs with lipo; one is copied. A
# downloaded artifact keeps no execute bit: each program is made executable here. Any other
# program of the folder (standin-lsp, standin-agent, yaar-script) is never taken.
#
# Writes into <out-dir>, and nothing else:
#   YAAR.app
#   yaar-review-<version>-macos-universal.zip   (two folders; -macos-<arm64|x86_64> for one)
#   yaar-review-<version>-macos-universal.dmg
# No checksum file: the release's one checksum file is packaging/release/assemble.sh's.
#
# Environment:
#   SIGN_IDENTITY   "-" (default) signs ad hoc, which Gatekeeper rejects on another Mac.
#                   Otherwise a Developer ID Application identity, by its SHA-1 hash or name.
#   SIGN_KEYCHAIN   the keychain that holds that identity (codesign --keychain); unset, the
#                   user's search list.
#   NOTARY_KEY, NOTARY_KEY_ID, NOTARY_ISSUER
#                   an App Store Connect API key: the path of its .p8 file, its id, the
#                   issuer id. Set together with a Developer ID identity, they UPLOAD the app
#                   and the disk image to Apple (`xcrun notarytool submit --key --key-id
#                   --issuer --wait --timeout 30m`), and staple the tickets to both. Unset,
#                   nothing leaves this machine. Set with SIGN_IDENTITY "-" (or unset), or only
#                   some of the three set: refused before anything is built (Apple would
#                   refuse an ad hoc app after the upload).
#   NOTARY_PROFILE  a keychain profile of `xcrun notarytool store-credentials`, in place of
#                   the three above, for a run by hand; the same refusal with "-".
#   NOTICES         The third-party notices to put inside the app, from packaging/notices.sh.
#                   Required: a package without them may not be handed to anyone.
#   MIN_MACOS       LSMinimumSystemVersion (default 12.0, the declared floor).
#
# <version> is X.Y.Z (packaging/version.sh refuses anything else) and is both
# CFBundleShortVersionString and CFBundleVersion.
#
# Layout and signing identifiers:
#   YAAR.app                                   io.github.tsadoq.yaar
#   YAAR.app/Contents/MacOS/yaar-review        the Window (CFBundleExecutable)
#   YAAR.app/Contents/Helpers/yaar-keeper      io.github.tsadoq.yaar.keeper
#   YAAR.app/Contents/Helpers/yaar-claude-hook io.github.tsadoq.yaar.claude-hook
#   YAAR.app/Contents/Helpers/yaar-claude-tools io.github.tsadoq.yaar.claude-tools
#   YAAR.app/Contents/Helpers/yaar-pi-report   io.github.tsadoq.yaar.pi-report
#   YAAR.app/Contents/Resources/AppIcon.icns, LICENSE, THIRD-PARTY-NOTICES.txt
# The Keeper finds the other three beside its own program. Signed from the inside out, each
# under the hardened runtime with no entitlement, with a secure timestamp unless ad hoc; then
# the bundle, which seals them; then the disk image.
#
# The order, which a notarised release depends on (a .zip cannot be stapled, so the
# published .zip must hold the stapled app, and the .dmg must be made from the stapled app
# and then notarised and stapled itself). Steps 3 to 6 and 10 to 11 run only when notarising:
#    1. the four helpers signed, each with its identifier;
#    2. the bundle signed; `codesign --verify --deep --strict` on it;
#    3. a .zip of the bundle made with `ditto -c -k --keepParent` into the scratch: the upload;
#    4. that .zip submitted with `--wait --timeout 30m --output-format json`; the status read
#       from the output must be "Accepted", else `notarytool log <id>` is printed and the
#       script fails (notarytool exits 0 on "Invalid");
#    5. `xcrun stapler staple` on YAAR.app;
#    6. `xcrun stapler validate` on YAAR.app;
#    7. the published .zip made again from the stapled bundle (`ditto -c -k --keepParent`);
#    8. the .dmg made with hdiutil from a folder holding a copy of the stapled bundle and a
#       link Applications to /Applications;
#    9. the .dmg signed;
#   10. the .dmg submitted as in 4, status "Accepted";
#   11. `xcrun stapler staple` and `xcrun stapler validate` on the .dmg.
# Without notarising, 3 to 6 and 10 to 11 are skipped and 7 is the only .zip.
#
# The log says which signature it made, in one line starting "signature: ":
#   "signature: ad hoc, not notarised (for tests; Gatekeeper refuses it on another Mac)"
#   "signature: Developer ID <the identity's name>, notarised and stapled"
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

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
#   "signature: Developer ID <the identity's name>, not notarised (Gatekeeper refuses it on
#   another Mac)" (a Developer ID identity with no NOTARY_* set: a signed package that was
#   not sent to Apple)
set -euo pipefail

SELF=$(basename "$0")
HERE=$(cd "$(dirname "$0")" && pwd)

# Exit 2: a wrong call or a refused configuration, before anything is built. Exit 1: a step
# of the build failed.
usage() { echo "usage: $SELF <version> <out-dir> <programs-dir>..." >&2; exit 2; }
refuse() { echo "$SELF: $*" >&2; exit 2; }
fail() { echo "$SELF: $*" >&2; exit 1; }
# Each step below says in its own words what failed. This is for a command that is not a step:
# it still ends the run with a sentence naming the line.
trap 'fail "line $LINENO: $BASH_COMMAND failed"' ERR

[ $# -ge 3 ] || usage
VER=$1; OUT=$2; shift 2
PROGDIRS=("$@")

VERSION_RE='^[0-9]+\.[0-9]+\.[0-9]+$'
[[ $VER =~ $VERSION_RE ]] || refuse "the version \"$VER\" is not X.Y.Z"
[ ${#PROGDIRS[@]} -le 2 ] || refuse "at most two folders of programs (one per processor) are taken, not ${#PROGDIRS[@]}"
if [ -z "${NOTICES:-}" ] || [ ! -s "${NOTICES:-}" ]; then
  refuse "set NOTICES to the file packaging/notices.sh wrote; a package without notices may not be handed to anyone"
fi
MIN_MACOS=${MIN_MACOS:-12.0}
VERSION_RE='^[0-9]+(\.[0-9]+)*$'
[[ $MIN_MACOS =~ $VERSION_RE ]] || refuse "MIN_MACOS \"$MIN_MACOS\" is not a version"

# The five programs, and the identifier each helper is signed with.
PROGRAMS=(yaar-review yaar-keeper yaar-claude-hook yaar-claude-tools yaar-pi-report)
helper_identifier() {
  case $1 in
    yaar-keeper) echo io.github.tsadoq.yaar.keeper ;;
    yaar-claude-hook) echo io.github.tsadoq.yaar.claude-hook ;;
    yaar-claude-tools) echo io.github.tsadoq.yaar.claude-tools ;;
    yaar-pi-report) echo io.github.tsadoq.yaar.pi-report ;;
    *) fail "no identifier for $1" ;;
  esac
}
for d in "${PROGDIRS[@]}"; do
  [ -d "$d" ] || refuse "$d is not a folder of programs"
  for p in "${PROGRAMS[@]}"; do
    [ -f "$d/$p" ] || refuse "$d/$p is missing: each folder holds the five programs"
  done
done

# Signing and notarising, decided and refused before anything is built.
SIGN_IDENTITY=${SIGN_IDENTITY:--}
NOTARY_SET=""
for v in NOTARY_KEY NOTARY_KEY_ID NOTARY_ISSUER NOTARY_PROFILE; do
  if [ -n "${!v:-}" ]; then NOTARY_SET="$NOTARY_SET $v"; fi
done
NOTARY_SET=${NOTARY_SET# }
NOTARIZE=no
NOTARY_AUTH=()
if [ -n "$NOTARY_SET" ]; then
  if [ "$SIGN_IDENTITY" = "-" ]; then
    refuse "$NOTARY_SET set with an ad hoc signature: Apple refuses an ad hoc app after the upload; set SIGN_IDENTITY to a Developer ID Application identity or unset NOTARY_*"
  fi
  if [ -n "${NOTARY_PROFILE:-}" ]; then
    [ "$NOTARY_SET" = "NOTARY_PROFILE" ] ||
      refuse "NOTARY_PROFILE is set together with $NOTARY_SET: name the notary credentials one way"
    NOTARY_AUTH=(--keychain-profile "$NOTARY_PROFILE")
  else
    [ "$NOTARY_SET" = "NOTARY_KEY NOTARY_KEY_ID NOTARY_ISSUER" ] ||
      refuse "only $NOTARY_SET set: NOTARY_KEY, NOTARY_KEY_ID and NOTARY_ISSUER are set together"
    # shellcheck disable=SC2153 # NOTARY_KEY is the variable of the header, set by the caller
    [ -f "$NOTARY_KEY" ] || refuse "NOTARY_KEY names $NOTARY_KEY, which is not a file"
    NOTARY_AUTH=(--key "$NOTARY_KEY" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER")
  fi
  command -v xcrun > /dev/null || refuse "xcrun is not here: notarisation needs the Xcode command line tools"
  NOTARIZE=yes
fi
TS=(--timestamp)
if [ "$SIGN_IDENTITY" = "-" ]; then TS=(--timestamp=none); fi
KEYCHAIN=()
if [ -n "${SIGN_KEYCHAIN:-}" ]; then KEYCHAIN=(--keychain "$SIGN_KEYCHAIN"); fi

sign() { # codesign with the identity and the keychain: <codesign arguments>
  codesign --force --sign "$SIGN_IDENTITY" ${KEYCHAIN[@]+"${KEYCHAIN[@]}"} "$@" ||
    fail "codesign could not sign ${*: -1}"
}

mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
APP="$OUT/YAAR.app"
WORK=$(mktemp -d); trap 'rm -rf "$WORK"' EXIT
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Helpers" "$APP/Contents/Resources"

# One folder is copied, two are merged into one program for both processors. Either way the
# program is made executable here: a downloaded artifact keeps no execute bit.
merge() { # <program> <where it goes>
  local ins=() d
  for d in "${PROGDIRS[@]}"; do ins+=("$d/$1"); done
  if [ ${#ins[@]} -gt 1 ]; then
    lipo -create "${ins[@]}" -output "$2" || fail "lipo could not join $1 of the two folders"
  else
    cp "${ins[0]}" "$2" || fail "$1 could not be copied from ${PROGDIRS[0]}"
  fi
  chmod 755 "$2" || fail "$2 could not be made executable"
}
merge yaar-review "$APP/Contents/MacOS/yaar-review"
for p in "${PROGRAMS[@]}"; do
  if [ "$p" != yaar-review ]; then merge "$p" "$APP/Contents/Helpers/$p"; fi
done

archs=$(lipo -archs "$APP/Contents/MacOS/yaar-review") || fail "lipo cannot read the processors of yaar-review"
case $archs in
  *" "*) SUFFIX=universal ;;
  *) SUFFIX=$archs ;;
esac
ZIP="$OUT/yaar-review-$VER-macos-$SUFFIX.zip"
DMG="$OUT/yaar-review-$VER-macos-$SUFFIX.dmg"
rm -f "$ZIP" "$DMG"

# The icon: one PNG scaled into an icon set. sips and iconutil ship with macOS.
I="$WORK/AppIcon.iconset"; mkdir -p "$I"
for s in 16 32 128 256 512; do
  sips -z $s $s "$HERE/macos/icon-1024.png" --out "$I/icon_${s}x${s}.png" > /dev/null ||
    fail "sips could not scale the icon to ${s}x${s}"
  sips -z $((s*2)) $((s*2)) "$HERE/macos/icon-1024.png" --out "$I/icon_${s}x${s}@2x.png" > /dev/null ||
    fail "sips could not scale the icon to $((s*2))x$((s*2))"
done
iconutil -c icns "$I" -o "$APP/Contents/Resources/AppIcon.icns" || fail "iconutil could not make AppIcon.icns"

cp "$HERE/../LICENSE" "$APP/Contents/Resources/LICENSE" || fail "the LICENSE of the repository could not be copied into the app"
cp "$NOTICES" "$APP/Contents/Resources/THIRD-PARTY-NOTICES.txt" || fail "$NOTICES could not be copied into the app"

sed -e "s/@VERSION@/$VER/" -e "s/@BUILD@/$VER/" -e "s/@MINOS@/$MIN_MACOS/" \
  "$HERE/macos/Info.plist.in" > "$APP/Contents/Info.plist" || fail "Info.plist could not be made from $HERE/macos/Info.plist.in"
plutil -lint "$APP/Contents/Info.plist" > /dev/null || fail "the Info.plist made from Info.plist.in is not a property list"
printf 'APPL????' > "$APP/Contents/PkgInfo" || fail "PkgInfo could not be written"

notarise() { # <file> <what it is called in a sentence>
  local file=$1 what=$2 json="$WORK/notary.json" status id
  echo "$SELF: sending the $what to Apple and waiting for the verdict (up to 30 minutes)"
  # notarytool exits 0 on "Invalid": the verdict is read from its output.
  if ! xcrun notarytool submit "$file" "${NOTARY_AUTH[@]}" --wait --timeout 30m --output-format json > "$json"; then
    cat "$json" >&2
    fail "notarytool did not finish for the $what"
  fi
  status=$(sed -n 's/.*"status"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$json" | tail -n 1) ||
    fail "the answer of notarytool for the $what could not be read"
  id=$(sed -n 's/.*"id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$json" | sed -n '1p') ||
    fail "the answer of notarytool for the $what could not be read"
  if [ "$status" != "Accepted" ]; then
    if [ -z "$id" ]; then
      cat "$json" >&2
    elif ! xcrun notarytool log "$id" "${NOTARY_AUTH[@]}" >&2; then
      echo "$SELF: the log of submission $id could not be fetched" >&2
    fi
    fail "Apple's verdict on the $what is \"$status\", not \"Accepted\""
  fi
}

# 1. The four helpers, each with its identifier, hardened runtime, no entitlement. The
#    bundle is signed after them, never with --deep, which is for verifying only.
for p in "${PROGRAMS[@]}"; do
  if [ "$p" != yaar-review ]; then
    sign --options runtime "${TS[@]}" --identifier "$(helper_identifier "$p")" "$APP/Contents/Helpers/$p"
  fi
done
# 2. The bundle, which seals the helpers and signs the Window with the bundle's identifier.
sign --options runtime "${TS[@]}" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP" || fail "the signed bundle does not pass codesign --verify --deep --strict"

if [ "$NOTARIZE" = yes ]; then
  # 3, 4. Apple takes an archive, not a folder.
  ditto -c -k --keepParent "$APP" "$WORK/submit.zip" || fail "ditto could not make the archive of the app to send to Apple"
  notarise "$WORK/submit.zip" "app"
  # 5, 6. The ticket goes into the app itself, so that the .zip and the .dmg made below both
  #       carry an app that opens offline.
  xcrun stapler staple "$APP" || fail "stapler could not staple the ticket to the app"
  xcrun stapler validate "$APP" || fail "stapler finds no valid ticket on the app"
  codesign --verify --deep --strict --verbose=2 "$APP" || fail "the app no longer passes codesign --verify --deep --strict after stapling"
fi

# 7. The published .zip: the stapled app when there is a ticket.
ditto -c -k --keepParent "$APP" "$ZIP" || fail "ditto could not make $ZIP"

# 8. The disk image, made from that app and a link to /Applications.
S="$WORK/dmg"; mkdir -p "$S"
ditto "$APP" "$S/YAAR.app" || fail "ditto could not copy the app into the folder of the disk image"
ln -s /Applications "$S/Applications" || fail "the link Applications could not be made in the folder of the disk image"
hdiutil create -quiet -volname "YAAR" -srcfolder "$S" -fs HFS+ -format UDZO -imagekey zlib-level=9 "$DMG" ||
  fail "hdiutil could not make $DMG"
# 9. The image is signed (no runtime option: it holds no code of its own).
sign "${TS[@]}" "$DMG"

if [ "$NOTARIZE" = yes ]; then
  # 10, 11.
  notarise "$DMG" "disk image"
  xcrun stapler staple "$DMG" || fail "stapler could not staple the ticket to the disk image"
  xcrun stapler validate "$DMG" || fail "stapler finds no valid ticket on the disk image"
fi

if [ "$SIGN_IDENTITY" = "-" ]; then
  echo "signature: ad hoc, not notarised (for tests; Gatekeeper refuses it on another Mac)"
else
  name=$(codesign -dvv "$APP" 2>&1 | sed -n 's/^Authority=Developer ID Application: //p' | sed -n '1p')
  if [ -z "$name" ]; then name=$SIGN_IDENTITY; fi
  if [ "$NOTARIZE" = yes ]; then
    echo "signature: Developer ID $name, notarised and stapled"
  else
    echo "signature: Developer ID $name, not notarised (Gatekeeper refuses it on another Mac)"
  fi
fi
ls -l "$OUT"

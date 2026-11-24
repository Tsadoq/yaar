#!/bin/bash
# Builds YAAR.app, a .zip and a .dmg from release programs that are already built.
#
# package-macos.sh <version> <out-dir> <programs-dir>...
#
# Notarising order: a .zip cannot be stapled, so the .zip is made after stapling the app; the
# .dmg is made from the stapled app, then notarised and stapled itself.
set -euo pipefail

SELF=$(basename "$0")
HERE=$(cd "$(dirname "$0")" && pwd)

# Exit 2: a wrong call or refused configuration, before anything is built. Exit 1: a build step
# failed.
usage() { echo "usage: $SELF <version> <out-dir> <programs-dir>..." >&2; exit 2; }
refuse() { echo "$SELF: $*" >&2; exit 2; }
fail() { echo "$SELF: $*" >&2; exit 1; }
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
    # shellcheck disable=SC2153 # NOTARY_KEY is set by the caller
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

# The bundle is signed after the helpers, never with --deep, which is for verifying only.
for p in "${PROGRAMS[@]}"; do
  if [ "$p" != yaar-review ]; then
    sign --options runtime "${TS[@]}" --identifier "$(helper_identifier "$p")" "$APP/Contents/Helpers/$p"
  fi
done
sign --options runtime "${TS[@]}" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP" || fail "the signed bundle does not pass codesign --verify --deep --strict"

if [ "$NOTARIZE" = yes ]; then
  ditto -c -k --keepParent "$APP" "$WORK/submit.zip" || fail "ditto could not make the archive of the app to send to Apple"
  notarise "$WORK/submit.zip" "app"
  xcrun stapler staple "$APP" || fail "stapler could not staple the ticket to the app"
  xcrun stapler validate "$APP" || fail "stapler finds no valid ticket on the app"
  codesign --verify --deep --strict --verbose=2 "$APP" || fail "the app no longer passes codesign --verify --deep --strict after stapling"
fi

ditto -c -k --keepParent "$APP" "$ZIP" || fail "ditto could not make $ZIP"

S="$WORK/dmg"; mkdir -p "$S"
ditto "$APP" "$S/YAAR.app" || fail "ditto could not copy the app into the folder of the disk image"
ln -s /Applications "$S/Applications" || fail "the link Applications could not be made in the folder of the disk image"
hdiutil create -quiet -volname "YAAR" -srcfolder "$S" -fs HFS+ -format UDZO -imagekey zlib-level=9 "$DMG" ||
  fail "hdiutil could not make $DMG"
# The image is signed with no runtime option: it holds no code of its own.
sign "${TS[@]}" "$DMG"

if [ "$NOTARIZE" = yes ]; then
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

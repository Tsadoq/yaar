#!/bin/bash
# The step "What the Mac bundle holds" of the jobs macos and release-sign-macos: the app as
# it ships, read from the bundle, and the two files users download, read from the files.
#
#   .github/scripts/macos-bundle-check.sh <YAAR.app> <version> <notices> [<packages-dir>]
#
# Fails with a sentence naming what is wrong when any of these does not hold:
#   - Contents/Info.plist: CFBundleIdentifier io.github.tsadoq.yaar, CFBundleExecutable
#     yaar-review, CFBundlePackageType APPL, CFBundleShortVersionString and CFBundleVersion
#     both <version>, LSMinimumSystemVersion 12.0, and neither LSUIElement nor
#     LSBackgroundOnly set (the bundle is an application with a Dock icon: PKG-08);
#   - Contents/MacOS holds yaar-review only; Contents/Helpers holds exactly yaar-keeper,
#     yaar-claude-hook, yaar-claude-tools and yaar-pi-report; each of the five is executable
#     and `lipo -archs` gives the processors YAAR_EXPECT_ARCHS names (default "x86_64 arm64";
#     a builder on a Mac with one target sets "arm64");
#   - each of the five is signed under the hardened runtime (`codesign -dv` shows
#     flags=0x10000(runtime)) with no entitlement (`codesign -d --entitlements -` prints
#     none), and with its identifier, in every processor's slice (`codesign -a <processor>`:
#     without it codesign reports the slice of the processor it runs on only):
#     io.github.tsadoq.yaar (the bundle),
#     io.github.tsadoq.yaar.keeper, io.github.tsadoq.yaar.claude-hook,
#     io.github.tsadoq.yaar.claude-tools, io.github.tsadoq.yaar.pi-report;
#   - no file named standin-lsp, standin-agent or yaar-script anywhere in the bundle;
#   - Contents/Resources/THIRD-PARTY-NOTICES.txt has the bytes of <notices>, and
#     Contents/Resources/LICENSE is the LICENSE of the repository.
# With <packages-dir>, which holds yaar-review-<version>-macos-universal.zip and .dmg made
# from that app, also:
#   - the .zip unpacked with `ditto -x -k` into a temporary folder holds YAAR.app and nothing
#     else, and that app is <YAAR.app> file for file and byte for byte (`diff -r`, which also
#     names a file one side lacks);
#   - the .dmg mounted read-only (`hdiutil attach -nobrowse -readonly -mountpoint <temp>`,
#     detached at the end whatever happens) holds YAAR.app and a link Applications to
#     /Applications, and that app is <YAAR.app> in the same way; so the notices and the
#     Info.plist users get by either file are those checked above.
# "The same way" also covers the modes: `diff -r` compares names and bytes and not modes, so
# each of the five is executable in the app of the .zip and of the .dmg, and every file of
# either copy has the mode it has in <YAAR.app>.
#
# Exit codes: 0 all hold; 1 one does not; 2 usage error.
#
# Three readings of the list above:
#   - An app signed ad hoc shows flags=0x10002(adhoc,runtime), so the runtime is taken from
#     the word in the brackets, not from the number.
#   - The Window is read from the bundle, whose signature seals it: `codesign -dv` of the
#     bundle names the identifier and the flags.
#   - A build on one processor (YAAR_EXPECT_ARCHS names one) is packaged by package-macos.sh
#     as yaar-review-<version>-macos-<that processor>.zip and .dmg, and those are read.
set -euo pipefail

SELF=$(basename "$0")
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)

usage() { echo "usage: $SELF <YAAR.app> <version> <notices> [<packages-dir>]" >&2; exit 2; }
[ $# -eq 3 ] || [ $# -eq 4 ] || usage
APP=$1; VER=$2; NOTICES=$3; PKGS=${4:-}

fail() { echo "$SELF: $*" >&2; exit 1; }
# A command that fails with no sentence of its own still ends the run with one.
trap 'fail "line $LINENO: $BASH_COMMAND failed"' ERR

[ -d "$APP" ] || fail "$APP is not a folder: no app to check"
[ -f "$NOTICES" ] || fail "$NOTICES is not a file: no notices to compare the app's with"
[ -z "$PKGS" ] || [ -d "$PKGS" ] || fail "$PKGS is not a folder: no .zip and .dmg to check"
APP=$(cd "$APP" && pwd -P)

WORK=$(mktemp -d)
WORK=$(cd "$WORK" && pwd -P)
MNT=""
cleanup() {
  if [ -n "$MNT" ]; then
    # A volume can be busy for a moment after the last read: ask again until it lets go.
    local deadline=$((SECONDS + 15)) last forced
    until last=$(hdiutil detach -quiet "$MNT" 2>&1); do
      if [ "$SECONDS" -ge "$deadline" ]; then
        forced=$(hdiutil detach -quiet -force "$MNT" 2>&1) ||
          echo "$SELF: could not detach $MNT: hdiutil last said \"$last\", and when forced \"$forced\"" >&2
        break
      fi
      sleep 0.2
    done
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

PLIST="$APP/Contents/Info.plist"
[ -f "$PLIST" ] || fail "$PLIST is missing"

expect_plist() { # <key> <wanted>
  local got
  got=$(plutil -extract "$1" raw -o - "$PLIST" 2> /dev/null) || got="(not set)"
  [ "$got" = "$2" ] || fail "Info.plist of the app has $1 \"$got\", not \"$2\""
}
expect_plist CFBundleIdentifier io.github.tsadoq.yaar
expect_plist CFBundleExecutable yaar-review
expect_plist CFBundlePackageType APPL
expect_plist CFBundleShortVersionString "$VER"
expect_plist CFBundleVersion "$VER"
expect_plist LSMinimumSystemVersion 12.0
for key in LSUIElement LSBackgroundOnly; do
  if plutil -extract "$key" raw -o - "$PLIST" > /dev/null 2>&1; then
    fail "Info.plist of the app sets $key: the app must be an application with a Dock icon"
  fi
done

# What a folder holds, by name, sorted, hidden files included.
names_in() { (cd "$1" && find . -mindepth 1 -maxdepth 1 -print | sed 's|^\./||' | LC_ALL=C sort | tr '\n' ' ' | sed 's/ $//'); }
[ -d "$APP/Contents/MacOS" ] || fail "$APP/Contents/MacOS is missing"
[ -d "$APP/Contents/Helpers" ] || fail "$APP/Contents/Helpers is missing"
got=$(names_in "$APP/Contents/MacOS")
[ "$got" = "yaar-review" ] || fail "Contents/MacOS holds \"$got\", not yaar-review alone"
got=$(names_in "$APP/Contents/Helpers")
want="yaar-claude-hook yaar-claude-tools yaar-keeper yaar-pi-report"
[ "$got" = "$want" ] || fail "Contents/Helpers holds \"$got\", not \"$want\""

# Processors, as a sorted set, so that the order lipo prints them in does not matter.
sorted_words() { printf '%s\n' "$1" | tr -s ' ' '\n' | LC_ALL=C sort | tr '\n' ' ' | sed 's/ $//'; }
EXPECT_ARCHS=${YAAR_EXPECT_ARCHS:-x86_64 arm64}
WANT_ARCHS=$(sorted_words "$EXPECT_ARCHS")
read -ra ARCH_LIST <<< "$WANT_ARCHS"

# <path under Contents>|<what `codesign -dv` reads>|<identifier>
PROGRAMS=(
  "MacOS/yaar-review|$APP|io.github.tsadoq.yaar"
  "Helpers/yaar-keeper|$APP/Contents/Helpers/yaar-keeper|io.github.tsadoq.yaar.keeper"
  "Helpers/yaar-claude-hook|$APP/Contents/Helpers/yaar-claude-hook|io.github.tsadoq.yaar.claude-hook"
  "Helpers/yaar-claude-tools|$APP/Contents/Helpers/yaar-claude-tools|io.github.tsadoq.yaar.claude-tools"
  "Helpers/yaar-pi-report|$APP/Contents/Helpers/yaar-pi-report|io.github.tsadoq.yaar.pi-report"
)
for entry in "${PROGRAMS[@]}"; do
  rel=${entry%%|*}; rest=${entry#*|}; signed=${rest%%|*}; ident=${rest#*|}
  prog="$APP/Contents/$rel"
  [ -f "$prog" ] || fail "$rel is missing from the app"
  [ -x "$prog" ] || fail "$rel is not executable (mode $(stat -f %Lp "$prog"))"
  archs=$(lipo -archs "$prog" 2> /dev/null) || fail "lipo cannot read $rel: it is not a program"
  got=$(sorted_words "$archs")
  [ "$got" = "$WANT_ARCHS" ] || fail "$rel holds the processors \"$archs\", not \"$EXPECT_ARCHS\""

  # codesign reports the slice of the processor it runs on unless it is told which: ask for
  # each processor the program holds.
  for arch in "${ARCH_LIST[@]}"; do
    info=$(codesign -dv -a "$arch" "$signed" 2>&1) || fail "$rel is not signed in its $arch slice: codesign says: $info"
    got=$(printf '%s\n' "$info" | sed -n 's/^Identifier=//p')
    [ "$got" = "$ident" ] || fail "$rel is signed with the identifier \"$got\" in its $arch slice, not \"$ident\""
    flags=$(printf '%s\n' "$info" | sed -n 's/^CodeDirectory .*flags=0x[0-9a-f]*(\([^)]*\)).*/\1/p')
    case ",$flags," in
      *,runtime,*) ;;
      *) fail "$rel is signed without the hardened runtime in its $arch slice (flags \"$flags\"): sign with --options runtime" ;;
    esac
    ents=$(codesign -d -a "$arch" --entitlements - "$signed" 2> /dev/null) || fail "codesign cannot read the entitlements of $rel in its $arch slice"
    [ -z "$ents" ] || fail "$rel is signed with entitlements in its $arch slice, and the app takes none"
  done
done

for name in standin-lsp standin-agent yaar-script; do
  found=$(find "$APP" -name "$name")
  [ -z "$found" ] || fail "the app holds $name (${found#"$APP"/}): a test program that no package ships"
done

cmp -s "$NOTICES" "$APP/Contents/Resources/THIRD-PARTY-NOTICES.txt" ||
  fail "Contents/Resources/THIRD-PARTY-NOTICES.txt is missing or is not the bytes of $NOTICES"
cmp -s "$REPO/LICENSE" "$APP/Contents/Resources/LICENSE" ||
  fail "Contents/Resources/LICENSE is missing or is not the LICENSE of the repository"

if [ -z "$PKGS" ]; then
  echo "$SELF: the app holds what it should"
  exit 0
fi

# "<mode> <path>" of every file of a tree, sorted: diff -r does not read modes.
modes_in() { (cd "$1" && find . -type f -exec stat -f '%Lp %N' {} + | LC_ALL=C sort); }
APP_MODES=$(modes_in "$APP")

# A file users download holds this very app, byte for byte, with the modes the app has.
same_app() { # <the file's name> <the app in it>
  local diffs rel got entry only_app only_copy
  if ! diffs=$(diff -r "$APP" "$2" 2>&1); then
    fail "the app in $1 is not the app checked above: $(printf '%s\n' "$diffs" | sed -n '1,3p' | tr '\n' ';')"
  fi
  for entry in "${PROGRAMS[@]}"; do
    rel=${entry%%|*}
    [ -x "$2/Contents/$rel" ] || fail "$rel is not executable in the app of $1 (mode $(stat -f %Lp "$2/Contents/$rel")): the app would not start"
  done
  got=$(modes_in "$2")
  if [ "$got" != "$APP_MODES" ]; then
    only_app=$(comm -23 <(printf '%s\n' "$APP_MODES") <(printf '%s\n' "$got") | sed -n '1,3p' | tr '\n' ';')
    only_copy=$(comm -13 <(printf '%s\n' "$APP_MODES") <(printf '%s\n' "$got") | sed -n '1,3p' | tr '\n' ';')
    fail "the app in $1 has other file modes than the app checked above: the app has \"$only_app\" where the copy has \"$only_copy\""
  fi
}

SUFFIX=universal
case "$WANT_ARCHS" in *" "*) ;; *) SUFFIX=$WANT_ARCHS ;; esac
ZIP="$PKGS/yaar-review-$VER-macos-$SUFFIX.zip"
DMG="$PKGS/yaar-review-$VER-macos-$SUFFIX.dmg"
[ -f "$ZIP" ] || fail "$ZIP is missing"
[ -f "$DMG" ] || fail "$DMG is missing"

mkdir "$WORK/zip"
ditto -x -k "$ZIP" "$WORK/zip" || fail "$ZIP cannot be unpacked"
got=$(names_in "$WORK/zip")
[ "$got" = "YAAR.app" ] || fail "$(basename "$ZIP") holds \"$got\", not YAAR.app alone"
same_app "$(basename "$ZIP")" "$WORK/zip/YAAR.app"

mkdir "$WORK/mnt"
hdiutil attach -quiet -nobrowse -readonly -mountpoint "$WORK/mnt" "$DMG" > /dev/null ||
  fail "$(basename "$DMG") cannot be mounted"
MNT="$WORK/mnt"
# The volume may carry the system's own hidden bookkeeping (.fseventsd); nothing else shows.
shown=$(cd "$MNT" && find . -mindepth 1 -maxdepth 1 ! -name '.*' -print | sed 's|^\./||' | LC_ALL=C sort | tr '\n' ' ' | sed 's/ $//')
[ "$shown" = "Applications YAAR.app" ] || fail "$(basename "$DMG") holds \"$shown\", not YAAR.app and Applications"
[ -L "$MNT/Applications" ] || fail "Applications in $(basename "$DMG") is not a link"
link=$(readlink "$MNT/Applications")
[ "$link" = "/Applications" ] || fail "Applications in $(basename "$DMG") links to $link, not /Applications"
same_app "$(basename "$DMG")" "$MNT/YAAR.app"

echo "$SELF: the app, the .zip and the .dmg hold what they should"

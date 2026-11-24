#!/bin/sh
# The step "Package check" of package-linux: what the .deb and the archive of one processor
# hold, read from the files, before any install.
#
#   .github/scripts/linux-package-check.sh <dist> <version> <notices>
#
# <dist> holds yaar-review_<version>_<arm64|amd64>.deb and
# yaar-review-<version>-linux-<aarch64|x86_64>.tar.gz for the processor of this machine.
# Fails with a sentence naming the file and the line when any of these does not hold:
#   - the .deb's Package is yaar-review, Version is <version>, Architecture the machine's;
#   - its Depends names, besides what dpkg-shlibdeps found: libwayland-client0, libvulkan1,
#     "mesa-vulkan-drivers | vulkan-icd", fonts-dejavu-core, git and ca-certificates; and what
#     dpkg-shlibdeps found is whole: after the install, the package of every library each of
#     the five programs links directly (its NEEDED entries) is named in Depends, whatever
#     else the container already holds;
#   - it holds exactly these programs, each of mode 0755 and, in the .deb, owned by root (an
#     execute bit for root alone is not "executable": the user who runs the app could not
#     start them): /usr/bin/yaar-review,
#     /usr/libexec/yaar-review/yaar-keeper, /usr/libexec/yaar-review/yaar-claude-hook,
#     /usr/libexec/yaar-review/yaar-claude-tools, /usr/libexec/yaar-review/yaar-pi-report;
#     and no file named standin-lsp, standin-agent or yaar-script anywhere;
#   - it holds the desktop entry /usr/share/applications/io.github.tsadoq.yaar.desktop, the
#     icon, and usr/share/doc/yaar-review/{copyright,NOTICE,THIRD-PARTY-NOTICES.txt.gz},
#     the last one gunzipping to the bytes of <notices>;
#   - the archive holds the same programs under yaar-review/bin and yaar-review/libexec/
#     yaar-review, the same desktop entry and icon under yaar-review/share, LICENSE, NOTICE
#     and THIRD-PARTY-NOTICES.txt (the bytes of <notices>) under yaar-review/share/doc/
#     yaar-review;
#   - in the container of the job: `apt-get install ./<the .deb>` installs it, each of the
#     five installed programs' `ldd` lists no "not found", and `/usr/bin/yaar-review --version`
#     prints "yaar-review <version>". Needs readelf (binutils) and dpkg in that container.
#
# Exit codes: 0 all hold; 1 one does not; 2 usage error.
#
# This is a POSIX shell script without pipefail, so no command here sits on the left of a
# pipe: each writes a file under the scratch folder and the next one reads it.
set -eu
LC_ALL=C; export LC_ALL
ME=linux-package-check.sh
fail() { echo "$ME: $1" >&2; exit 1; }

[ $# -eq 3 ] || { echo "usage: $ME <dist> <version> <notices>" >&2; exit 2; }
DIST=$1; VER=$2; NOTICES=$3
[ -d "$DIST" ] || { echo "$ME: $DIST is not a folder" >&2; exit 2; }
[ -s "$NOTICES" ] || { echo "$ME: $NOTICES is missing or empty: give the notices file packaging/notices.sh wrote" >&2; exit 2; }
DIST=$(cd "$DIST" && pwd)
NOTICES=$(cd "$(dirname "$NOTICES")" && pwd)/$(basename "$NOTICES")
REPO=$(cd "$(dirname "$0")/../.." && pwd)

case $(uname -m) in
  aarch64) DEBARCH=arm64; ARCH=aarch64 ;;
  x86_64) DEBARCH=amd64; ARCH=x86_64 ;;
  *) fail "unsupported processor: $(uname -m)" ;;
esac
PKG=yaar-review
ID=io.github.tsadoq.yaar
DEB=$DIST/${PKG}_${VER}_${DEBARCH}.deb
TAR=$DIST/${PKG}-${VER}-linux-${ARCH}.tar.gz
[ -f "$DEB" ] || fail "$DEB is missing: package-linux.sh wrote no .deb for version $VER on $DEBARCH"
[ -f "$TAR" ] || fail "$TAR is missing: package-linux.sh wrote no archive for version $VER on $ARCH"
[ ! -e "$DIST/SHA256SUMS" ] || fail "$DIST/SHA256SUMS exists: the one checksum file of a release is assemble.sh's, package-linux.sh writes none"

W=$(mktemp -d); trap 'rm -rf "$W"' EXIT

# --- the .deb's control fields
got=$(dpkg-deb --field "$DEB" Package) || fail "$DEB: dpkg-deb cannot read its control fields"
[ "$got" = "$PKG" ] || fail "$DEB: Package is '$got', not $PKG"
got=$(dpkg-deb --field "$DEB" Version) || fail "$DEB: dpkg-deb cannot read its Version"
[ "$got" = "$VER" ] || fail "$DEB: Version is '$got', not $VER"
got=$(dpkg-deb --field "$DEB" Architecture) || fail "$DEB: dpkg-deb cannot read its Architecture"
[ "$got" = "$DEBARCH" ] || fail "$DEB: Architecture is '$got', not $DEBARCH"
DEPENDS=$(dpkg-deb --field "$DEB" Depends) || fail "$DEB: dpkg-deb cannot read its Depends"
printf '%s\n' "$DEPENDS" > "$W/depends.line" || fail "$W/depends.line: cannot write the Depends of $DEB"
tr ',' '\n' < "$W/depends.line" > "$W/depends.split" || fail "$DEB: cannot split its Depends"
sed 's/^ *//; s/ *$//' "$W/depends.split" > "$W/depends" || fail "$DEB: cannot read its Depends entries"
# The package names any entry holds, an alternative "a | b" giving both and a version or an
# architecture dropped: what a library's package is looked for in after the install.
tr '|' '\n' < "$W/depends" > "$W/depends.alternatives" || fail "$DEB: cannot split the alternatives of its Depends"
sed 's/^ *//; s/ .*$//; s/:[a-z0-9]*$//' "$W/depends.alternatives" > "$W/depends.names" || fail "$DEB: cannot read the package names of its Depends"
# libc6 is what dpkg-shlibdeps always finds in a program built here: without it the
# automatic part of Depends is gone.
for want in libc6 libwayland-client0 libvulkan1 "mesa-vulkan-drivers | vulkan-icd" fonts-dejavu-core git ca-certificates; do
  case $want in
    libc6) grep -q '^libc6 ' "$W/depends" || fail "$DEB: Depends has no libc6 entry, which dpkg-shlibdeps puts first (Depends: $DEPENDS)" ;;
    *) grep -qxF -- "$want" "$W/depends" || fail "$DEB: Depends lacks '$want' (Depends: $DEPENDS)" ;;
  esac
done

# --- what the .deb holds
# mode_in <listing> <path as the listing writes it> <the package file, for the message>
# sets MODE and OWNER from the listing of dpkg-deb -c or tar -tv (mode, owner/group, size,
# date, time, path).
mode_in() {
  MODE=; OWNER=; listed=0
  while read -r m o _size _date _time p _rest; do
    if [ "$p" = "$2" ]; then MODE=$m; OWNER=$o; listed=$((listed + 1)); fi
  done < "$1"
  [ "$listed" -eq 1 ] || fail "$3: lists $listed entries named $2, not one"
}
mkdir "$W/deb"
dpkg-deb -x "$DEB" "$W/deb" || fail "$DEB: dpkg-deb cannot unpack it"
dpkg-deb -c "$DEB" > "$W/deb.list" || fail "$DEB: dpkg-deb cannot list it"
cat > "$W/programs" <<LIST
usr/bin/yaar-review
usr/libexec/yaar-review/yaar-claude-hook
usr/libexec/yaar-review/yaar-claude-tools
usr/libexec/yaar-review/yaar-keeper
usr/libexec/yaar-review/yaar-pi-report
LIST
while read -r f; do
  [ -f "$W/deb/$f" ] || fail "$DEB: holds no /$f"
  # Exactly 0755 and root's: with an execute bit for the owner alone, the user who runs the
  # app could not start it, and [ -x ] as root would not tell.
  mode_in "$W/deb.list" "./$f" "$DEB"
  [ "$MODE" = "-rwxr-xr-x" ] || fail "$DEB: /$f has mode $MODE, not -rwxr-xr-x"
  [ "$OWNER" = "root/root" ] || fail "$DEB: /$f is owned by $OWNER, not root/root"
done < "$W/programs"
( cd "$W/deb" && find usr/bin usr/libexec \( -type f -o -type l \) > "$W/found.unsorted" ) || fail "$DEB: find cannot list usr/bin and usr/libexec of its unpacked tree"
sort "$W/found.unsorted" > "$W/found" || fail "$DEB: cannot sort the list of its programs"
if ! cmp -s "$W/programs" "$W/found"; then
  comm -13 "$W/programs" "$W/found" > "$W/extra" || fail "$DEB: cannot compare its programs with the five"
  fail "$DEB: holds more than the five programs in usr/bin and usr/libexec: $(tr '\n' ' ' < "$W/extra")"
fi
( cd "$W/deb" && find . \( -name standin-lsp -o -name standin-agent -o -name yaar-script \) > "$W/stray" ) || fail "$DEB: find cannot search its unpacked tree for test programs"
[ ! -s "$W/stray" ] || fail "$DEB: holds a test program, which no package ships: $(tr '\n' ' ' < "$W/stray")"
for f in "usr/share/applications/$ID.desktop" "usr/share/icons/hicolor/256x256/apps/$ID.png" \
    usr/share/doc/yaar-review/copyright usr/share/doc/yaar-review/NOTICE \
    usr/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt.gz; do
  [ -s "$W/deb/$f" ] || fail "$DEB: holds no /$f, or it is empty"
done
gzip -dc "$W/deb/usr/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt.gz" > "$W/deb-notices" \
  || fail "$DEB: /usr/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt.gz does not gunzip"
cmp -s "$W/deb-notices" "$NOTICES" || fail "$DEB: /usr/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt.gz does not gunzip to the bytes of $NOTICES"
cmp -s "$W/deb/usr/share/doc/yaar-review/NOTICE" "$REPO/NOTICE" || fail "$DEB: /usr/share/doc/yaar-review/NOTICE differs from $REPO/NOTICE"

# --- what the archive holds
mkdir "$W/tar"
tar -xzf "$TAR" -C "$W/tar" --no-same-owner || fail "$TAR: tar cannot unpack it"
tar -tvzf "$TAR" > "$W/tar.list" || fail "$TAR: tar cannot list it"
top=$(find "$W/tar" -mindepth 1 -maxdepth 1 -printf '%f ') || fail "$TAR: find cannot list its top"
[ "$top" = "yaar-review " ] || fail "$TAR: holds '$top' at its top, not only yaar-review/"
while read -r f; do
  a=yaar-review/${f#usr/}
  [ -f "$W/tar/$a" ] || fail "$TAR: holds no $a"
  mode_in "$W/tar.list" "$a" "$TAR"
  [ "$MODE" = "-rwxr-xr-x" ] || fail "$TAR: $a has mode $MODE, not -rwxr-xr-x"
  cmp -s "$W/tar/$a" "$W/deb/$f" || fail "$TAR: $a differs from /$f of $DEB"
done < "$W/programs"
( cd "$W/tar" && find yaar-review/bin yaar-review/libexec \( -type f -o -type l \) > "$W/found.unsorted" ) || fail "$TAR: find cannot list bin and libexec of its unpacked tree"
sort "$W/found.unsorted" > "$W/found" || fail "$TAR: cannot sort the list of its programs"
sed 's|^usr/|yaar-review/|' "$W/programs" > "$W/programs-tar" || fail "$W/programs-tar: cannot write the list of the archive's programs"
if ! cmp -s "$W/programs-tar" "$W/found"; then
  comm -13 "$W/programs-tar" "$W/found" > "$W/extra" || fail "$TAR: cannot compare its programs with the five"
  fail "$TAR: holds more than the five programs in bin and libexec: $(tr '\n' ' ' < "$W/extra")"
fi
( cd "$W/tar" && find . \( -name standin-lsp -o -name standin-agent -o -name yaar-script \) > "$W/stray" ) || fail "$TAR: find cannot search its unpacked tree for test programs"
[ ! -s "$W/stray" ] || fail "$TAR: holds a test program, which no package ships: $(tr '\n' ' ' < "$W/stray")"
for f in "share/applications/$ID.desktop" "share/icons/hicolor/256x256/apps/$ID.png"; do
  [ -s "$W/tar/yaar-review/$f" ] || fail "$TAR: holds no yaar-review/$f, or it is empty"
  cmp -s "$W/tar/yaar-review/$f" "$W/deb/usr/$f" || fail "$TAR: yaar-review/$f differs from /usr/$f of $DEB"
done
for f in LICENSE NOTICE THIRD-PARTY-NOTICES.txt; do
  [ -s "$W/tar/yaar-review/share/doc/yaar-review/$f" ] || fail "$TAR: holds no yaar-review/share/doc/yaar-review/$f, or it is empty"
done
cmp -s "$W/tar/yaar-review/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt" "$NOTICES" || fail "$TAR: yaar-review/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt is not the bytes of $NOTICES"
cmp -s "$W/tar/yaar-review/share/doc/yaar-review/NOTICE" "$REPO/NOTICE" || fail "$TAR: yaar-review/share/doc/yaar-review/NOTICE differs from $REPO/NOTICE"
cmp -s "$W/tar/yaar-review/share/doc/yaar-review/LICENSE" "$REPO/LICENSE" || fail "$TAR: yaar-review/share/doc/yaar-review/LICENSE differs from $REPO/LICENSE"

# --- the install, in the container of the job
[ "$(id -u)" -eq 0 ] || fail "the install needs root, and this runs as $(id -un): run it in the container of the job"
if ! DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "$DEB" > "$W/apt.log" 2>&1; then
  tail -n 20 "$W/apt.log" >&2
  fail "$DEB: apt-get install did not install it"
fi
command -v readelf > /dev/null || fail "readelf is not installed in this container (package binutils): the libraries each program links cannot be read"
# needs_package <program> <soname> <ldd output of the program>: the package that owns the
# file the soname resolves to is named in Depends.
needs_package() {
  found_at=
  while read -r a b c _rest; do
    if [ "$a" = "$2" ] && [ "$b" = "=>" ]; then found_at=$c; fi
    case $a in */"$2") found_at=$a ;; esac
  done < "$3"
  case $found_at in
    /*) ;;
    *) fail "/$1 links $2, which ldd resolves to no file on this system" ;;
  esac
  # The file and the file it resolves to are both asked, since a merged /usr gives dpkg's
  # list the one and the loader's answer the other; a diversion line names no package.
  real=$(readlink -f "$found_at") || fail "$found_at: readlink cannot resolve it"
  : > "$W/owner.out"
  for candidate in "$found_at" "$real"; do
    if dpkg -S "$candidate" > "$W/owner.try" 2>/dev/null; then cat "$W/owner.try" >> "$W/owner.out"; fi
  done
  grep -v '^diversion by' "$W/owner.out" > "$W/owner.packages" || fail "/$1 links $2, found at $found_at, which no installed package owns"
  sed -n 's/: \/.*$//p' "$W/owner.packages" > "$W/owner.line" || fail "dpkg -S $found_at: cannot read its answer"
  tr ',' '\n' < "$W/owner.line" > "$W/owner.split" || fail "dpkg -S $found_at: cannot split its answer"
  sed 's/^ *//; s/:[a-z0-9]*$//' "$W/owner.split" > "$W/owner.names" || fail "dpkg -S $found_at: cannot read the package names"
  grep -qxFf "$W/owner.names" "$W/depends.names" || fail "$DEB: /$1 links $2, which package $(tr '\n' ' ' < "$W/owner.names") provides, and Depends names none of it: dpkg-shlibdeps did not see this program (Depends: $DEPENDS)"
}
while read -r f; do
  [ -f "/$f" ] || fail "/$f is not installed by $DEB"
  got=$(stat -c '%a %U:%G' "/$f") || fail "/$f: stat cannot read it"
  [ "$got" = "755 root:root" ] || fail "/$f installed by $DEB has mode and owner '$got', not '755 root:root'"
  # ldd exits 0 for a program with a missing library, so its output is the test.
  ldd "/$f" > "$W/ldd.out" 2>&1 || { cat "$W/ldd.out" >&2; fail "/$f: ldd cannot read it"; }
  if grep -q 'not found' "$W/ldd.out"; then
    grep 'not found' "$W/ldd.out" >&2
    fail "/$f: ldd reports a library or a version of one that is not found on this system"
  fi
  # ldd cannot see a library missing from Depends when the container already holds it, so
  # each library the program links directly is followed to its package.
  readelf -d "/$f" > "$W/dynamic.out" 2>&1 || { cat "$W/dynamic.out" >&2; fail "/$f: readelf cannot read its dynamic section"; }
  sed -n 's/^.*(NEEDED).*Shared library: \[\(.*\)\].*$/\1/p' "$W/dynamic.out" > "$W/needed" || fail "/$f: cannot read its NEEDED libraries"
  [ -s "$W/needed" ] || fail "/$f: readelf lists no NEEDED library, which a program of this app always has"
  while read -r so; do
    needs_package "$f" "$so" "$W/ldd.out"
  done < "$W/needed"
done < "$W/programs"
got=$(/usr/bin/yaar-review --version 2>&1) || fail "/usr/bin/yaar-review --version failed: $got"
[ "$got" = "yaar-review $VER" ] || fail "/usr/bin/yaar-review --version prints '$got', not 'yaar-review $VER'"
echo "$ME: $DEB and $TAR hold what they should, and the .deb installs"

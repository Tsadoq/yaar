#!/bin/sh
# Package check: what the .deb and the tar.gz of one processor hold, read before any install.
# .github/scripts/linux-package-check.sh <dist> <version> <notices>
# POSIX sh without pipefail: no command sits on the left of a pipe, each writes a file for the next.
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
tr '|' '\n' < "$W/depends" > "$W/depends.alternatives" || fail "$DEB: cannot split the alternatives of its Depends"
sed 's/^ *//; s/ .*$//; s/:[a-z0-9]*$//' "$W/depends.alternatives" > "$W/depends.names" || fail "$DEB: cannot read the package names of its Depends"
# libc6 is what dpkg-shlibdeps always finds in a program built here: without it the automatic part
# of Depends is gone.
for want in libc6 libwayland-client0 libvulkan1 "mesa-vulkan-drivers | vulkan-icd" fonts-dejavu-core git ca-certificates; do
  case $want in
    libc6) grep -q '^libc6 ' "$W/depends" || fail "$DEB: Depends has no libc6 entry, which dpkg-shlibdeps puts first (Depends: $DEPENDS)" ;;
    *) grep -qxF -- "$want" "$W/depends" || fail "$DEB: Depends lacks '$want' (Depends: $DEPENDS)" ;;
  esac
done

mode_in() {
  MODE=; OWNER=; listed=0
  while read -r m o _size _date _time p _rest; do
    if [ "$p" = "$2" ]; then MODE=$m; OWNER=$o; listed=$((listed + 1)); fi
  done < "$1"
  [ "$listed" -eq 1 ] || fail "$3: lists $listed entries named $2, not one"
}
# modes_ok <listing> <package> <path prefix> <programs file> <owner or "">: folders and programs
# must be rwxr-xr-x, every other entry rw-r--r--, each of the owner when one is given.
modes_ok() {
  while read -r m o _size _date _time p _rest; do
    [ -n "$p" ] || fail "$2: its listing has a line without a path"
    rel=${p#"$3"}
    if [ -n "$5" ] && [ "$o" != "$5" ]; then fail "$2: $p is owned by $o, not $5"; fi
    case $m in
      d*) [ "$m" = "drwxr-xr-x" ] || fail "$2: the folder $p has mode $m, not drwxr-xr-x" ;;
      *)
        if grep -qxF -- "$rel" "$4"; then
          [ "$m" = "-rwxr-xr-x" ] || fail "$2: $p has mode $m, not -rwxr-xr-x"
        else
          [ "$m" = "-rw-r--r--" ] || fail "$2: $p has mode $m, not -rw-r--r--: only the five programs are executable"
        fi ;;
    esac
  done < "$1"
}
mkdir "$W/deb"
dpkg-deb -x "$DEB" "$W/deb" || fail "$DEB: dpkg-deb cannot unpack it"
dpkg-deb -c "$DEB" > "$W/deb.list" || fail "$DEB: dpkg-deb cannot list it"
# dpkg-deb --build writes `control` here and nothing else: any other member (a postinst runs as
# root) is named.
dpkg-deb --ctrl-tarfile "$DEB" > "$W/control.tar" || fail "$DEB: dpkg-deb cannot read its control archive"
tar -tf "$W/control.tar" > "$W/control.list" || fail "$DEB: tar cannot list its control archive"
sed 's|^\./||; /^$/d' "$W/control.list" > "$W/control.unsorted" || fail "$DEB: cannot read the members of its control archive"
sort "$W/control.unsorted" > "$W/control.members" || fail "$DEB: cannot sort the members of its control archive"
[ "$(cat "$W/control.members")" = "control" ] \
  || fail "$DEB: its control archive holds '$(tr '\n' ' ' < "$W/control.list")', not only ./ and ./control: a maintainer script or another member is not allowed"
cat > "$W/programs" <<LIST
usr/bin/yaar-review
usr/libexec/yaar-review/yaar-claude-hook
usr/libexec/yaar-review/yaar-claude-tools
usr/libexec/yaar-review/yaar-keeper
usr/libexec/yaar-review/yaar-pi-report
LIST
while read -r f; do
  [ -f "$W/deb/$f" ] || fail "$DEB: holds no /$f"
  # Exactly 0755 and root's: with an execute bit for the owner alone, the app's user could not start
  # it, and [ -x ] as root would not tell.
  mode_in "$W/deb.list" "./$f" "$DEB"
  [ "$MODE" = "-rwxr-xr-x" ] || fail "$DEB: /$f has mode $MODE, not -rwxr-xr-x"
  [ "$OWNER" = "root/root" ] || fail "$DEB: /$f is owned by $OWNER, not root/root"
done < "$W/programs"
# The whole unpacked tree, not only usr/: a stray file anywhere is named (say /etc/xdg/autostart,
# which would start the app at every login).
cp "$W/programs" "$W/files.deb" || fail "$W/files.deb: cannot write the list of the files the .deb should hold"
cat >> "$W/files.deb" <<LIST
usr/share/applications/$ID.desktop
usr/share/doc/yaar-review/NOTICE
usr/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt.gz
usr/share/doc/yaar-review/copyright
usr/share/icons/hicolor/256x256/apps/$ID.png
LIST
sort "$W/files.deb" > "$W/expected" || fail "$DEB: cannot sort the list of the files it should hold"
( cd "$W/deb" && find . ! -type d > "$W/found.dotted" ) || fail "$DEB: find cannot list its unpacked tree"
sed 's|^\./||' "$W/found.dotted" > "$W/found.unsorted" || fail "$DEB: cannot read the list of its files"
sort "$W/found.unsorted" > "$W/found" || fail "$DEB: cannot sort the list of its files"
if ! cmp -s "$W/expected" "$W/found"; then
  comm -13 "$W/expected" "$W/found" > "$W/extra" || fail "$DEB: cannot compare its files with the expected ones"
  comm -23 "$W/expected" "$W/found" > "$W/lacking" || fail "$DEB: cannot compare its files with the expected ones"
  fail "$DEB: holds files it should not: $(tr '\n' ' ' < "$W/extra") and lacks: $(tr '\n' ' ' < "$W/lacking")"
fi
modes_ok "$W/deb.list" "$DEB" "./" "$W/programs" root/root
( cd "$W/deb" && find . \( -name standin-lsp -o -name standin-agent -o -name yaar-script \) > "$W/stray" ) || fail "$DEB: find cannot search its unpacked tree for test programs"
[ ! -s "$W/stray" ] || fail "$DEB: holds a test program, which no package ships: $(tr '\n' ' ' < "$W/stray")"
for f in "usr/share/applications/$ID.desktop" "usr/share/icons/hicolor/256x256/apps/$ID.png" \
    usr/share/doc/yaar-review/copyright usr/share/doc/yaar-review/NOTICE \
    usr/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt.gz; do
  [ -s "$W/deb/$f" ] || fail "$DEB: holds no /$f, or it is empty"
done
cmp -s "$W/deb/usr/share/applications/$ID.desktop" "$REPO/packaging/linux/$ID.desktop" || fail "$DEB: /usr/share/applications/$ID.desktop differs from $REPO/packaging/linux/$ID.desktop"
cmp -s "$W/deb/usr/share/icons/hicolor/256x256/apps/$ID.png" "$REPO/packaging/linux/icon-256.png" || fail "$DEB: /usr/share/icons/hicolor/256x256/apps/$ID.png differs from $REPO/packaging/linux/icon-256.png"
gzip -dc "$W/deb/usr/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt.gz" > "$W/deb-notices" \
  || fail "$DEB: /usr/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt.gz does not gunzip"
cmp -s "$W/deb-notices" "$NOTICES" || fail "$DEB: /usr/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt.gz does not gunzip to the bytes of $NOTICES"
cmp -s "$W/deb/usr/share/doc/yaar-review/NOTICE" "$REPO/NOTICE" || fail "$DEB: /usr/share/doc/yaar-review/NOTICE differs from $REPO/NOTICE"

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
sed 's|^usr/|yaar-review/|' "$W/programs" > "$W/programs-tar" || fail "$W/programs-tar: cannot write the list of the archive's programs"
cp "$W/programs-tar" "$W/files.tar" || fail "$W/files.tar: cannot write the list of the files the archive should hold"
cat >> "$W/files.tar" <<LIST
yaar-review/share/applications/$ID.desktop
yaar-review/share/doc/yaar-review/LICENSE
yaar-review/share/doc/yaar-review/NOTICE
yaar-review/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt
yaar-review/share/doc/yaar-review/copyright
yaar-review/share/icons/hicolor/256x256/apps/$ID.png
LIST
sort "$W/files.tar" > "$W/expected" || fail "$TAR: cannot sort the list of the files it should hold"
( cd "$W/tar" && find yaar-review ! -type d > "$W/found.unsorted" ) || fail "$TAR: find cannot list its unpacked tree"
sort "$W/found.unsorted" > "$W/found" || fail "$TAR: cannot sort the list of its files"
if ! cmp -s "$W/expected" "$W/found"; then
  comm -13 "$W/expected" "$W/found" > "$W/extra" || fail "$TAR: cannot compare its files with the expected ones"
  comm -23 "$W/expected" "$W/found" > "$W/lacking" || fail "$TAR: cannot compare its files with the expected ones"
  fail "$TAR: holds files it should not: $(tr '\n' ' ' < "$W/extra") and lacks: $(tr '\n' ' ' < "$W/lacking")"
fi
modes_ok "$W/tar.list" "$TAR" "" "$W/programs-tar" ""
( cd "$W/tar" && find . \( -name standin-lsp -o -name standin-agent -o -name yaar-script \) > "$W/stray" ) || fail "$TAR: find cannot search its unpacked tree for test programs"
[ ! -s "$W/stray" ] || fail "$TAR: holds a test program, which no package ships: $(tr '\n' ' ' < "$W/stray")"
for f in "share/applications/$ID.desktop" "share/icons/hicolor/256x256/apps/$ID.png"; do
  [ -s "$W/tar/yaar-review/$f" ] || fail "$TAR: holds no yaar-review/$f, or it is empty"
  cmp -s "$W/tar/yaar-review/$f" "$W/deb/usr/$f" || fail "$TAR: yaar-review/$f differs from /usr/$f of $DEB"
done
for f in LICENSE NOTICE copyright THIRD-PARTY-NOTICES.txt; do
  [ -s "$W/tar/yaar-review/share/doc/yaar-review/$f" ] || fail "$TAR: holds no yaar-review/share/doc/yaar-review/$f, or it is empty"
done
cmp -s "$W/tar/yaar-review/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt" "$NOTICES" || fail "$TAR: yaar-review/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt is not the bytes of $NOTICES"
cmp -s "$W/tar/yaar-review/share/doc/yaar-review/copyright" "$W/deb/usr/share/doc/yaar-review/copyright" || fail "$TAR: yaar-review/share/doc/yaar-review/copyright differs from /usr/share/doc/yaar-review/copyright of $DEB"
cmp -s "$W/tar/yaar-review/share/doc/yaar-review/NOTICE" "$REPO/NOTICE" || fail "$TAR: yaar-review/share/doc/yaar-review/NOTICE differs from $REPO/NOTICE"
cmp -s "$W/tar/yaar-review/share/doc/yaar-review/LICENSE" "$REPO/LICENSE" || fail "$TAR: yaar-review/share/doc/yaar-review/LICENSE differs from $REPO/LICENSE"

[ "$(id -u)" -eq 0 ] || fail "the install needs root, and this runs as $(id -un): run it in the container of the job"
if ! DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "$DEB" > "$W/apt.log" 2>&1; then
  tail -n 20 "$W/apt.log" >&2
  fail "$DEB: apt-get install did not install it"
fi
command -v readelf > /dev/null || fail "readelf is not installed in this container (package binutils): the libraries each program links cannot be read"
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
  # Both the file and what it resolves to are asked: a merged /usr gives dpkg's list the one and the
  # loader the other. A diversion line names no package.
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
  # ldd cannot see a library missing from Depends when the container already holds it, so each
  # directly linked library is followed to its package.
  readelf -d "/$f" > "$W/dynamic.out" 2>&1 || { cat "$W/dynamic.out" >&2; fail "/$f: readelf cannot read its dynamic section"; }
  sed -n 's/^.*(NEEDED).*Shared library: \[\(.*\)\].*$/\1/p' "$W/dynamic.out" > "$W/needed" || fail "/$f: cannot read its NEEDED libraries"
  [ -s "$W/needed" ] || fail "/$f: readelf lists no NEEDED library, which a program of this app always has"
  while read -r so; do
    needs_package "$f" "$so" "$W/ldd.out"
  done < "$W/needed"
done < "$W/programs"
for d in /usr/libexec/yaar-review /usr/share/doc/yaar-review; do
  got=$(stat -c '%a %U:%G' "$d") || fail "$d: stat cannot read it"
  [ "$got" = "755 root:root" ] || fail "$d installed by $DEB has mode and owner '$got', not '755 root:root'"
done
got=$(/usr/bin/yaar-review --version 2>&1) || fail "/usr/bin/yaar-review --version failed: $got"
[ "$got" = "yaar-review $VER" ] || fail "/usr/bin/yaar-review --version prints '$got', not 'yaar-review $VER'"
echo "$ME: $DEB and $TAR hold what they should, and the .deb installs"

#!/bin/bash
# Builds the two Linux packages from release programs that are already built: a .deb and a
# plain archive. Run it on the system the programs were built on (Ubuntu 22.04 for a
# release), for the processor of that system.
#
#   NOTICES=<file> package-linux.sh <programs-dir> <version> <out-dir>
#
# <programs-dir> holds the five programs as cargo builds them (target/release): yaar-review,
# yaar-keeper, yaar-claude-hook, yaar-claude-tools, yaar-pi-report. Any other program there
# (standin-lsp, standin-agent, yaar-script) is never taken.
#
# Writes into <out-dir>, and nothing else:
#   yaar-review_<version>_<arm64|amd64>.deb
#   yaar-review-<version>-linux-<aarch64|x86_64>.tar.gz
# No checksum file: the release's one checksum file is packaging/release/assemble.sh's.
#
# Needs: dpkg-deb and dpkg-shlibdeps (packages dpkg, dpkg-dev), tar, gzip.
#
# Layout of the installed .deb:
#   /usr/bin/yaar-review                          the Window (on PATH, named in the desktop entry)
#   /usr/libexec/yaar-review/yaar-keeper          the Keeper (not on PATH; the Window starts it)
#   /usr/libexec/yaar-review/yaar-claude-hook     Claude Code's hook program  } beside the Keeper,
#   /usr/libexec/yaar-review/yaar-claude-tools    Claude Code's tool server   } which finds them
#   /usr/libexec/yaar-review/yaar-pi-report       pi's reporting program      } there
#   /usr/share/applications/io.github.tsadoq.yaar.desktop
#   /usr/share/icons/hicolor/256x256/apps/io.github.tsadoq.yaar.png
#   /usr/share/doc/yaar-review/copyright, NOTICE, THIRD-PARTY-NOTICES.txt.gz
# The archive holds the same tree under yaar-review/ in place of /usr, with LICENSE added
# and the notices not compressed.
#
# Depends: what dpkg-shlibdeps finds in the five programs, then the hand-written part below.
#
# Environment:
#   NOTICES            the third-party notices to put in both packages, from
#                      packaging/notices.sh. Required.
#   EXTRA_DEPENDS      replaces the hand-written part of Depends (see below)
#   MAINTAINER         replaces the Maintainer field
#   SOURCE_DATE_EPOCH  time stamp of every packed file (default 2026-01-01), so that two
#                      runs on the same programs give the same bytes
set -euo pipefail
umask 022
[ $# -eq 4 ] || { sed -n '2,7p' "$0" >&2; exit 2; }
if [ -z "${NOTICES:-}" ] || [ ! -s "$NOTICES" ]; then
  echo "package-linux.sh: set NOTICES to the file packaging/notices.sh wrote" >&2; exit 2
fi
NOTICES=$(realpath "$NOTICES")
WINDOW=$(realpath "$1"); KEEPER=$(realpath "$2"); VER=$3; OUT=$4
if [ ! -f "$WINDOW" ] || [ ! -f "$KEEPER" ]; then
  echo "package-linux.sh: a program is missing: $WINDOW, $KEEPER" >&2; exit 1
fi
case $VER in
  [0-9]*) ;;
  *) echo "package-linux.sh: a version starts with a digit, not: $VER" >&2; exit 1 ;;
esac
case $VER in
  *[!0-9A-Za-z.+~]*) echo "package-linux.sh: only letters, digits, '.', '+' and '~' in a version, not: $VER" >&2; exit 1 ;;
esac
HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
case $(uname -m) in
  aarch64) DEBARCH=arm64; ARCH=aarch64 ;;
  x86_64) DEBARCH=amd64; ARCH=x86_64 ;;
  *) echo "package-linux.sh: unsupported processor: $(uname -m)" >&2; exit 1 ;;
esac

PKG=yaar-review
ID=io.github.tsadoq.yaar
# The address is the no-reply one of the repository owner's GitHub account.
MAINTAINER=${MAINTAINER:-"The YAAR Authors <Tsadoq@users.noreply.github.com>"}
# dpkg-shlibdeps finds the libraries the programs link. The ones below are opened with
# dlopen at run time, or are data and programs, so no tool can read them off the binary.
# Each was found by adding it after a failure, in a container that held only the archive
# (see README.md):
#   libwayland-client0   "failed to connect to Wayland compositor: The wayland library
#                        could not be loaded"
#   libvulkan1 and a Vulkan driver (mesa-vulkan-drivers, or whatever provides vulkan-icd)
#                        "Failed to create surface for any enabled backend"
#   fonts-dejavu-core    "failed to resolve font '.SystemUIFont' or any of the fallbacks"
#   git                  the app runs it; a dependency, not a recommendation (ADR-020)
# Not listed: libegl1 and libwayland-egl1. With them and no Vulkan driver the window drew
# under Wayland and did not open under X11 in the test container, so OpenGL is not offered
# as an alternative to the Vulkan driver.
EXTRA_DEPENDS=${EXTRA_DEPENDS:-"libwayland-client0, libvulkan1, mesa-vulkan-drivers | vulkan-icd, fonts-dejavu-core, git"}
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-1767225600}

W=$(mktemp -d); trap 'rm -rf "$W"' EXIT

write_copyright() { # <file>
  cat > "$1" <<'COPYRIGHT'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: YAAR
Source: https://github.com/Tsadoq/yaar
Comment: The two programs are linked with third-party libraries. Their
 licences and notices are in the file THIRD-PARTY-NOTICES.txt beside this
 one (compressed in the Debian package).

Files: *
Copyright: 2026 The YAAR Authors
License: Apache-2.0

License: Apache-2.0
 Licensed under the Apache License, Version 2.0 (the "License");
 you may not use this file except in compliance with the License.
 You may obtain a copy of the License at
 .
     http://www.apache.org/licenses/LICENSE-2.0
 .
 Unless required by applicable law or agreed to in writing, software
 distributed under the License is distributed on an "AS IS" BASIS,
 WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 See the License for the specific language governing permissions and
 limitations under the License.
 .
 On Debian systems, the complete text of the Apache License, Version 2.0
 can be found in "/usr/share/common-licenses/Apache-2.0".
COPYRIGHT
}

stage() { # <root> <prefix inside root>
  local r=$1/$2
  install -D -m 755 "$WINDOW" "$r/bin/yaar-review"
  install -D -m 755 "$KEEPER" "$r/libexec/yaar-review/yaar-keeper"
  install -D -m 644 "$HERE/linux/$ID.desktop" "$r/share/applications/$ID.desktop"
  install -D -m 644 "$HERE/linux/icon-256.png" "$r/share/icons/hicolor/256x256/apps/$ID.png"
  mkdir -p "$r/share/doc/$PKG"
  write_copyright "$r/share/doc/$PKG/copyright"
  install -m 644 "$HERE/../NOTICE" "$r/share/doc/$PKG/NOTICE"
}

# --- .deb
D=$W/deb; stage "$D" usr
gzip -n -9 < "$NOTICES" > "$D/usr/share/doc/$PKG/THIRD-PARTY-NOTICES.txt.gz"
chmod 644 "$D/usr/share/doc/$PKG/THIRD-PARTY-NOTICES.txt.gz"
mkdir -p "$D/DEBIAN" "$W/debian"
: > "$W/debian/control"   # dpkg-shlibdeps refuses to run without one
( cd "$W" && dpkg-shlibdeps -O "$D/usr/bin/yaar-review" "$D/usr/libexec/yaar-review/yaar-keeper" 2> shlibdeps.log > shlibs.out ) \
  || { echo "package-linux.sh: dpkg-shlibdeps failed:" >&2; cat "$W/shlibdeps.log" >&2; exit 1; }
cat "$W/shlibdeps.log" >&2
AUTO=$(sed -n 's/^shlibs:Depends=//p' "$W/shlibs.out")
[ -n "$AUTO" ] || { echo "package-linux.sh: dpkg-shlibdeps named no library" >&2; exit 1; }
SIZE=$(du -sk "$D/usr" | cut -f1)
cat > "$D/DEBIAN/control" <<CONTROL
Package: $PKG
Version: $VER
Architecture: $DEBARCH
Maintainer: $MAINTAINER
Installed-Size: $SIZE
Depends: $AUTO, $EXTRA_DEPENDS
Section: devel
Priority: optional
Homepage: https://github.com/Tsadoq/yaar
Description: Reviews what a coding agent changes, while it changes it
 YAAR is a desktop app for reviewing what a coding agent changes, while
 it changes it. This package holds its two programs: the window,
 yaar-review, and the background program the window starts, yaar-keeper.
CONTROL
find "$D" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
dpkg-deb --root-owner-group -Zxz --build "$D" "$OUT/${PKG}_${VER}_${DEBARCH}.deb" > /dev/null

# --- archive: unpack anywhere, run yaar-review/bin/yaar-review
T=$W/tar; stage "$T" "$PKG"
install -m 644 "$HERE/../LICENSE" "$T/$PKG/share/doc/$PKG/LICENSE"
install -m 644 "$NOTICES" "$T/$PKG/share/doc/$PKG/THIRD-PARTY-NOTICES.txt"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$SOURCE_DATE_EPOCH" -C "$T" -cf - "$PKG" \
  | gzip -n -9 > "$OUT/${PKG}-${VER}-linux-${ARCH}.tar.gz"

( cd "$OUT" && sha256sum "$PKG"_*.deb "$PKG"-*-linux-*.tar.gz > SHA256SUMS && ls -l && cat SHA256SUMS )

#!/usr/bin/env bash
# Installs YAAR (the window program "yaar-review" and the background program "yaar-keeper").
#
#   curl -fsSL https://tsadoq.github.io/yaar/install.sh | bash
#   curl -fsSL https://tsadoq.github.io/yaar/install.sh | bash -s -- --uninstall
#   curl -fsSL https://tsadoq.github.io/yaar/install.sh | bash -s -- --dry-run   # say what would be done, change nothing
#
# What it does, by system:
#   Debian, Ubuntu and systems built on them (ID or ID_LIKE in /etc/os-release names debian or
#   ubuntu, and apt-get exists): saves the repository key to /etc/apt/keyrings/yaar-review.asc,
#   writes /etc/apt/sources.list.d/yaar-review.sources, runs apt-get update and
#   apt-get install yaar-review. Uses sudo only when not already root. If apt refuses, the key
#   and the source file are put back as they were.
#   Other Linux: downloads the plain archive, checks its SHA-256 against SHA256SUMS, copies the
#   two programs, the desktop entry, the icon and the licence texts under ~/.local (bin/,
#   libexec/yaar-review/, share/). No root needed. The run-time libraries are then the user's business; the script
#   lists what the dynamic linker misses.
#   macOS: downloads the app as a .zip, checks its SHA-256, unpacks it with ditto, verifies the
#   code signature, copies YAAR.app to /Applications (or ~/Applications when that is not
#   writable). It never starts the app.
#
# The script never asks a question: it prints what it is about to do and does it. (sudo may ask
# for its password on the terminal; that is sudo, not this script.)
#
# Environment:
#   YAAR_BASE_URL      the site with this script and the apt repository
#                      (default: https://tsadoq.github.io/yaar)
#   YAAR_APT_URL       the apt repository                 (default: $YAAR_BASE_URL/apt)
#   YAAR_RELEASES_URL  the releases page                  (default: https://github.com/Tsadoq/yaar/releases)
#   YAAR_DOWNLOAD_URL  folder with SHA256SUMS, the archives and the .zip
#                                                         (default: $YAAR_RELEASES_URL/latest/download)
#   YAAR_METHOD        auto | apt | archive               (default: auto; ignored on macOS)
#   YAAR_PREFIX        target of the archive route        (default: $HOME/.local)
#   YAAR_INSTALL_DIR   target folder of the macOS app     (default: /Applications, else $HOME/Applications)
#   YAAR_OS_RELEASE    file read instead of /etc/os-release   (for tests)
#   YAAR_ARCH          processor name instead of `uname -m`   (for tests)
#
# The whole body is in functions and the only top-level command is the last line, so a download
# that stops half way runs nothing.
set -euo pipefail

DEFAULT_BASE_URL="https://tsadoq.github.io/yaar"
DEFAULT_RELEASES_URL="https://github.com/Tsadoq/yaar/releases"
PKG="yaar-review"
KEEPER="yaar-keeper"
APP_NAME="YAAR.app"
KEYRING="/etc/apt/keyrings/yaar-review.asc"
SOURCE_FILE="/etc/apt/sources.list.d/yaar-review.sources"
APT_LISTS="/var/lib/apt/lists"
DESKTOP_ID="io.github.tsadoq.yaar"
TMP=""

say()  { printf '%s\n' "yaar-install: $*"; }
warn() { printf '%s\n' "yaar-install: warning: $*" >&2; }
die()  { printf '%s\n' "yaar-install: error: $*" >&2; exit 1; }
have() { command -v "$1" > /dev/null 2>&1; }
cleanup() { if [ -n "$TMP" ] && [ -d "$TMP" ]; then rm -rf "$TMP"; fi; }

usage() {
  cat <<'USAGE'
usage: install.sh [--uninstall] [--dry-run] [--help]
  (no option)   install, or move to the newest published version
  --uninstall   remove what this script installed
  --dry-run     print the detected system and the steps, change nothing
Environment: YAAR_BASE_URL, YAAR_APT_URL, YAAR_RELEASES_URL, YAAR_DOWNLOAD_URL, YAAR_METHOD,
             YAAR_PREFIX, YAAR_INSTALL_DIR
USAGE
}

fetch() { # <url> <file>
  if have curl; then
    curl -fsSL --retry 2 -o "$2" "$1" || die "could not download $1"
  elif have wget; then
    wget -q -O "$2" "$1" || die "could not download $1"
  else
    die "neither curl nor wget is installed"
  fi
  [ -s "$2" ] || die "downloaded file is empty: $1"
}

sha256_of() { # <file>
  if have sha256sum; then sha256sum "$1" | awk '{print $1}'
  elif have shasum; then shasum -a 256 "$1" | awk '{print $1}'
  elif have openssl; then openssl dgst -sha256 "$1" | awk '{print $NF}'
  else die "no SHA-256 tool found (sha256sum, shasum or openssl)"
  fi
}

# Picks from SHA256SUMS the one file whose name ends with <suffix>, downloads it and checks it.
# Sets ARCHIVE to the local path.
fetch_checked() { # <suffix>
  local suffix="$1" name want got
  fetch "$DOWNLOAD_URL/SHA256SUMS" "$TMP/SHA256SUMS"
  name=$(awk -v s="$suffix" '{ f = $2; sub(/^\*/, "", f); n = length(f) - length(s) + 1;
           if (n > 0 && substr(f, n) == s && f ~ /^yaar-review-[0-9A-Za-z.+~-]+$/) print f }' "$TMP/SHA256SUMS" | sort | tail -n 1)
  [ -n "$name" ] || die "SHA256SUMS at $DOWNLOAD_URL names no file ending in $suffix (no build for this system?)"
  want=$(awk -v n="$name" '{ f = $2; sub(/^\*/, "", f); if (f == n) print $1 }' "$TMP/SHA256SUMS" | head -n 1)
  say "downloading $DOWNLOAD_URL/$name"
  fetch "$DOWNLOAD_URL/$name" "$TMP/$name"
  got=$(sha256_of "$TMP/$name")
  if [ "$got" != "$want" ]; then
    die "SHA-256 of $name is $got, SHA256SUMS says $want. Nothing was installed."
  fi
  say "SHA-256 matches SHA256SUMS ($want)"
  ARCHIVE="$TMP/$name"
}

os_release_value() { # <KEY>: the value from os-release, read as text and never executed
  local f="${YAAR_OS_RELEASE:-/etc/os-release}"
  [ -r "$f" ] || return 0
  sed -n "s/^$1=//p" "$f" | head -n 1 | tr -d "\"'"
}

is_debian_like() {
  local id like w
  id=$(os_release_value ID); like=$(os_release_value ID_LIKE)
  for w in $id $like; do
    case "$w" in debian|ubuntu) have apt-get && have dpkg && return 0 ;; esac
  done
  return 1
}

detect() {
  OS=$(uname -s)
  ARCH="${YAAR_ARCH:-$(uname -m)}"
  case "$ARCH" in
    x86_64|amd64) ARCH=x86_64 ;;
    aarch64|arm64) ARCH=aarch64 ;;
    *) die "unsupported processor: $ARCH" ;;
  esac
  case "$OS" in
    Darwin) METHOD=macos ;;
    Linux)
      METHOD="${YAAR_METHOD:-auto}"
      case "$METHOD" in
        auto) if is_debian_like; then METHOD=apt; else METHOD=archive; fi ;;
        apt|archive) ;;
        *) die "YAAR_METHOD must be auto, apt or archive, not '$METHOD'" ;;
      esac ;;
    *) die "unsupported system: $OS" ;;
  esac
  DISTRO="$(os_release_value ID)${YAAR_OS_RELEASE:+ (from $YAAR_OS_RELEASE)}"
  [ "$OS" = Linux ] || DISTRO="-"
}

need_root_tool() { # sets SUDO
  SUDO=""
  if [ "$(id -u)" -ne 0 ]; then
    have sudo || die "this route needs root and sudo is not installed. Run the script as root, or set YAAR_METHOD=archive to install under \$HOME without root."
    SUDO="sudo"
  fi
}

# ---------------------------------------------------------------- apt route
apt_refresh_and_install() {
  $SUDO apt-get update < /dev/null \
    && $SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y "$PKG" < /dev/null
}

# Puts the key and the source file back as they were before this run.
apt_put_back() { # <had-key: 0|1> <had-source: 0|1>
  if [ "$1" = 1 ]; then $SUDO install -m 0644 "$TMP/old.key" "$KEYRING"; else $SUDO rm -f "$KEYRING"; fi
  if [ "$2" = 1 ]; then $SUDO install -m 0644 "$TMP/old.sources" "$SOURCE_FILE"; else $SUDO rm -f "$SOURCE_FILE"; fi
}

install_apt() {
  local had_key=0 had_source=0
  need_root_tool
  say "route: apt repository at $APT_URL"
  say "about to do, ${SUDO:+with sudo, }as root:"
  say "  1. save the repository key to $KEYRING"
  say "  2. write $SOURCE_FILE"
  say "  3. apt-get update, then apt-get install $PKG (apt checks the signature and pulls the libraries it needs)"
  [ "$DRY_RUN" = 0 ] || return 0
  fetch "$APT_URL/yaar-review.asc" "$TMP/key.asc"
  grep -q -e '-----BEGIN PGP PUBLIC KEY BLOCK-----' "$TMP/key.asc" \
    || die "$APT_URL/yaar-review.asc is not an armoured public key. Nothing was changed."
  printf 'Types: deb\nURIs: %s\nSuites: stable\nComponents: main\nSigned-By: %s\n' "$APT_URL" "$KEYRING" > "$TMP/new.sources"
  if [ -f "$KEYRING" ]; then cp "$KEYRING" "$TMP/old.key"; had_key=1; fi
  if [ -f "$SOURCE_FILE" ]; then cp "$SOURCE_FILE" "$TMP/old.sources"; had_source=1; fi
  $SUDO install -d -m 0755 /etc/apt/keyrings
  $SUDO install -m 0644 "$TMP/key.asc" "$KEYRING"
  $SUDO install -m 0644 "$TMP/new.sources" "$SOURCE_FILE"
  if ! apt_refresh_and_install; then
    apt_put_back "$had_key" "$had_source"
    die "apt did not install $PKG (its messages are above). The key and the source file are as they were before."
  fi
  say "installed: $(dpkg-query -W -f='${Package} ${Version}' "$PKG")"
  say "start it with: $PKG    Later versions arrive with: sudo apt-get update && sudo apt-get upgrade"
}

# The names apt gives the index files of a repository: the address without its scheme, "/" as "_".
apt_list_prefix() { # <uri>
  local p="${1#*://}"
  p="${p%/}"
  case "$p" in ""|*[!0-9A-Za-z.:/-]*) return 0 ;; esac
  printf '%s' "$p" | tr '/' '_'
}

uninstall_apt() {
  local uri prefix
  need_root_tool
  say "about to do, ${SUDO:+with sudo, }as root: apt-get remove $PKG; delete $SOURCE_FILE, $KEYRING and apt's copy of the repository's index"
  [ "$DRY_RUN" = 0 ] || return 0
  if dpkg-query -W -f='${Status}' "$PKG" 2> /dev/null | grep -q "install ok installed"; then
    $SUDO env DEBIAN_FRONTEND=noninteractive apt-get remove -y "$PKG" < /dev/null
  else
    say "$PKG is not installed"
  fi
  if [ -f "$SOURCE_FILE" ]; then
    uri=$(sed -n 's/^URIs: *//p' "$SOURCE_FILE" | head -n 1)
    prefix=$(apt_list_prefix "$uri")
    if [ -n "$prefix" ]; then
      $SUDO find "$APT_LISTS" -maxdepth 2 -type f -name "${prefix}_dists_stable_*" -exec rm -f {} +
    fi
  fi
  $SUDO rm -f "$SOURCE_FILE" "$KEYRING"
  say "removed. Libraries that came with it stay until: ${SUDO:+sudo }apt-get autoremove"
}

# ---------------------------------------------------------------- archive route (other Linux)
put() { # <from> <to> <mode>: copy beside the target, then rename over it (works while the old file is running)
  cp "$1" "$2.new.$$"
  chmod "$3" "$2.new.$$"
  mv -f "$2.new.$$" "$2"
}

install_archive() {
  local prefix="${YAAR_PREFIX:-$HOME/.local}" src missing f icons="share/icons/hicolor/256x256/apps" doc="share/doc/$PKG"
  say "route: plain archive from $DOWNLOAD_URL into $prefix (no root)"
  say "about to do: download the archive for $ARCH, check its SHA-256, copy"
  say "  $prefix/bin/$PKG, $prefix/libexec/$PKG/$KEEPER,"
  say "  $prefix/share/applications/$DESKTOP_ID.desktop, $prefix/$icons/$DESKTOP_ID.png,"
  say "  $prefix/$doc/ (the licence texts)"
  [ "$DRY_RUN" = 0 ] || return 0
  have tar || die "tar is not installed"
  have gzip || die "gzip is not installed"
  fetch_checked "-linux-$ARCH.tar.gz"
  mkdir "$TMP/x"
  tar -xzf "$ARCHIVE" -C "$TMP/x"
  src="$TMP/x/$PKG"
  if [ ! -f "$src/bin/$PKG" ] || [ ! -f "$src/libexec/$PKG/$KEEPER" ]; then
    die "the archive does not have the expected layout ($PKG/bin, $PKG/libexec). Nothing was installed."
  fi
  mkdir -p "$prefix/bin" "$prefix/libexec/$PKG" "$prefix/share/applications" "$prefix/$icons"
  put "$src/libexec/$PKG/$KEEPER" "$prefix/libexec/$PKG/$KEEPER" 755
  put "$src/bin/$PKG" "$prefix/bin/$PKG" 755
  if [ -f "$src/share/applications/$DESKTOP_ID.desktop" ]; then
    put "$src/share/applications/$DESKTOP_ID.desktop" "$prefix/share/applications/$DESKTOP_ID.desktop" 644
  fi
  if [ -f "$src/$icons/$DESKTOP_ID.png" ]; then
    put "$src/$icons/$DESKTOP_ID.png" "$prefix/$icons/$DESKTOP_ID.png" 644
  fi
  if [ -d "$src/$doc" ]; then
    mkdir -p "$prefix/$doc"
    for f in "$src/$doc"/*; do
      if [ -f "$f" ]; then put "$f" "$prefix/$doc/$(basename "$f")" 644; fi
    done
  fi
  say "installed under $prefix"
  case ":$PATH:" in *":$prefix/bin:"*) ;; *) say "note: $prefix/bin is not on your PATH; start it as $prefix/bin/$PKG" ;; esac
  if have ldd; then
    missing=$(ldd "$prefix/bin/$PKG" 2> /dev/null | awk '/not found/ {print $1}' | sort -u | tr '\n' ' ')
    if [ -n "$missing" ]; then
      warn "the system lacks libraries the program links: $missing"
    fi
  fi
  say "this route installs no libraries. The window needs: libxcb, libxkbcommon, libxkbcommon-x11, the Wayland"
  say "client library, a Vulkan loader with a driver, a font such as DejaVu Sans, and git."
}

uninstall_archive() {
  local prefix="${YAAR_PREFIX:-$HOME/.local}" icons="share/icons/hicolor/256x256/apps" doc="share/doc/$PKG" d
  say "about to do: delete the files this script put under $prefix, and the folders that are then empty"
  [ "$DRY_RUN" = 0 ] || return 0
  rm -f "$prefix/bin/$PKG" "$prefix/libexec/$PKG/$KEEPER" \
        "$prefix/share/applications/$DESKTOP_ID.desktop" "$prefix/$icons/$DESKTOP_ID.png"
  rm -rf "${prefix:?}/$doc"
  for d in "libexec/$PKG" libexec bin share/applications "$icons" share/icons/hicolor/256x256 \
           share/icons/hicolor share/icons share/doc share; do
    rmdir "$prefix/$d" 2> /dev/null || true
  done
  say "removed"
}

# ---------------------------------------------------------------- macOS
mac_dest() {
  if [ -n "${YAAR_INSTALL_DIR:-}" ]; then printf '%s' "$YAAR_INSTALL_DIR"
  elif [ -w /Applications ]; then printf '%s' /Applications
  else printf '%s' "$HOME/Applications"
  fi
}

install_macos() {
  local dest old
  dest=$(mac_dest)
  say "route: macOS app from $DOWNLOAD_URL into $dest"
  say "about to do: download the .zip, check its SHA-256, unpack, verify the code signature, copy $APP_NAME to $dest"
  [ "$DRY_RUN" = 0 ] || return 0
  fetch_checked "-macos.zip"
  mkdir "$TMP/x"
  /usr/bin/ditto -x -k "$ARCHIVE" "$TMP/x"
  [ -d "$TMP/x/$APP_NAME" ] || die "the archive does not contain $APP_NAME. Nothing was installed."
  /usr/bin/codesign --verify --deep --strict "$TMP/x/$APP_NAME" || die "the code signature of the unpacked app does not verify. Nothing was installed."
  say "code signature verifies"
  mkdir -p "$dest"
  old=""
  if [ -e "$dest/$APP_NAME" ]; then
    old="$dest/.yaar-review-old.$$"
    say "replacing the copy already in $dest"
    mv "$dest/$APP_NAME" "$old"
  fi
  if ! /usr/bin/ditto "$TMP/x/$APP_NAME" "$dest/$APP_NAME"; then
    rm -rf "${dest:?}/$APP_NAME"
    [ -z "$old" ] || mv "$old" "$dest/$APP_NAME"
    die "could not copy the app to $dest; the previous state is restored"
  fi
  [ -z "$old" ] || rm -rf "$old"
  say "installed: $dest/$APP_NAME"
  say "this script does not start it. Open it from Finder or Launchpad."
}

uninstall_macos() {
  local dest
  dest=$(mac_dest)
  say "about to do: delete $dest/$APP_NAME"
  [ "$DRY_RUN" = 0 ] || return 0
  if [ -d "$dest/$APP_NAME" ]; then rm -rf "${dest:?}/$APP_NAME"; say "removed"; else say "not found in $dest"; fi
}

# ---------------------------------------------------------------- entry
main() {
  local action=install arg
  DRY_RUN=0
  for arg in "$@"; do
    case "$arg" in
      --uninstall) action=uninstall ;;
      --dry-run) DRY_RUN=1 ;;
      -h|--help) usage; exit 0 ;;
      *) usage >&2; die "unknown option: $arg" ;;
    esac
  done
  BASE_URL="${YAAR_BASE_URL:-$DEFAULT_BASE_URL}"; BASE_URL="${BASE_URL%/}"
  APT_URL="${YAAR_APT_URL:-$BASE_URL/apt}"; APT_URL="${APT_URL%/}"
  RELEASES_URL="${YAAR_RELEASES_URL:-$DEFAULT_RELEASES_URL}"; RELEASES_URL="${RELEASES_URL%/}"
  DOWNLOAD_URL="${YAAR_DOWNLOAD_URL:-$RELEASES_URL/latest/download}"; DOWNLOAD_URL="${DOWNLOAD_URL%/}"
  detect
  say "system: $OS $ARCH, distribution: $DISTRO, route: $METHOD, action: $action$( [ "$DRY_RUN" = 0 ] || printf ' (dry run)')"
  TMP=$(mktemp -d "${TMPDIR:-/tmp}/yaar-install.XXXXXX")
  trap cleanup EXIT
  "${action}_${METHOD}"
}

main "$@"

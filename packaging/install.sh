#!/usr/bin/env bash
# Installs YAAR (the window program "yaar-review", the background program "yaar-keeper" and
# the three programs beside it that the agent CLIs run: "yaar-claude-hook",
# "yaar-claude-tools" and "yaar-pi-report").
#
#   curl -fsSL <base>/install.sh | bash
#   curl -fsSL <base>/install.sh | bash -s -- --uninstall [--purge] [--stop-agents]
#   curl -fsSL <base>/install.sh | bash -s -- --dry-run   # say what would be done, change nothing
#
# <base> is the one value of packaging/site/base-url. The copy in the repository holds, as the
# values of DEFAULT_BASE_URL and TEAM_ID, two placeholders (a name between two at signs: the
# two lines below show them), which packaging/release/assemble.sh replaces when it puts the
# script on the site (the team id from packaging/macos/team-id). No other line of the script
# writes either placeholder, so the replaced copy holds neither. Run from the repository with
# the base address unreplaced, the script needs YAAR_BASE_URL and says so (for every action
# but --help).
#
# Before anything else: HOME must name a folder that exists (refused otherwise, with a
# sentence). Run as root (uid 0), the user's folders are root's: the script says so in one
# line ("running as root: the files of <HOME> are removed, not another user's") before an
# uninstall, and acts on them.
#
# What it does, by system:
#   Debian, Ubuntu and systems built on them (ID or ID_LIKE in /etc/os-release names debian or
#   ubuntu, and apt-get exists): saves the repository key to /etc/apt/keyrings/yaar-review.asc,
#   writes /etc/apt/sources.list.d/yaar-review.sources (Signed-By that file; never apt-key),
#   runs apt-get update and apt-get install yaar-review. Uses sudo only when not already root.
#   Each of the two files is written only when its bytes differ from what is there, so a
#   second run leaves both as they were, modification time included. If apt refuses, the key
#   and the source file are put back as they were.
#   Other Linux: downloads the plain archive, checks its SHA-256 against SHA256SUMS, copies the
#   programs, the desktop entry, the icon and the licence texts under ~/.local (bin/,
#   libexec/yaar-review/, share/). No root needed. The run-time libraries are then the user's
#   business; the script lists what the dynamic linker misses.
#   macOS: downloads yaar-review-<version>-macos-universal.zip, checks its SHA-256 against
#   SHA256SUMS, unpacks it with ditto into its temporary folder, and before it copies anything
#   checks the app: its code signature and its team (`codesign --verify --deep --strict
#   -R='anchor apple generic and certificate leaf[subject.OU] = "<TEAM_ID>"'`), Gatekeeper's
#   verdict (spctl --assess --type execute: accepted, from a notarised Developer ID) and the
#   stapled ticket (stapler validate). The team check pins the signer: any notarised app of
#   another team, put in place of the release's .zip, is refused although Gatekeeper would
#   accept it. The destination is /Applications, or ~/Applications when that is not
#   writable. At the start, leftovers of an interrupted run, `.YAAR.app.new.*` and
#   `.yaar-review-old.*` in either folder, are removed. The app is copied with ditto into
#   <dest>/.YAAR.app.new.<pid> (the same file system), then an older YAAR.app is moved to
#   <dest>/.yaar-review-old.<pid>, the new one is moved to <dest>/YAAR.app, and the old one is
#   deleted: no run leaves a half-copied YAAR.app. The app came by curl and carries no
#   quarantine mark; the script adds none, as its own Gatekeeper check stands for the one
#   macOS makes at a first open. "Already installed", which changes nothing and says so,
#   means both: <dest>/YAAR.app's CFBundleShortVersionString is the version downloaded, and
#   `codesign --verify --deep --strict` passes on it; otherwise the app is replaced. It never
#   starts the app.
#
# YAAR_ALLOW_UNNOTARIZED=1, for the rehearsal's app signed ad hoc: taken only when
# YAAR_DOWNLOAD_URL starts with http://127.0.0.1: or http://localhost: (a server on the
# machine itself); then the checks of Gatekeeper's verdict, of the ticket and of the team are
# skipped and the signature is still verified. With any other download address the script
# refuses before it downloads anything: "YAAR_ALLOW_UNNOTARIZED is taken only for a download
# from 127.0.0.1 or localhost". A TEAM_ID that is still its placeholder is likewise taken only
# under that rule; otherwise the macOS route refuses: "this copy of the install script names
# no team".
#
# The uninstall, run as the user (it uses sudo only for apt). It removes what this script
# installed by any route, and on macOS looks in both /Applications and ~/Applications (and
# YAAR_INSTALL_DIR when set). With nothing installed it still does 3 and 4 and succeeds.
#   1. The app must have stopped. Its processes are the user's processes whose program is one
#      this script is about to remove, matched by its full path (the bundle's
#      Contents/MacOS/yaar-review and Contents/Helpers/yaar-keeper; /usr/bin/yaar-review and
#      /usr/libexec/yaar-review/yaar-keeper; the same under the archive's prefix): on macOS
#      `ps -axo pid=,comm=`, on Linux /proc/<pid>/exe; never `pgrep -f`, never a name alone. A
#      Keeper of another copy, a build folder's, is left alone. When only a Keeper runs, the
#      script waits up to 10 s for it to leave by itself (it lingers 5 s after its last
#      Window when no agent runs in its terminals). If one still runs after that, the
#      uninstall refuses, changing nothing: "YAAR is running, and agents may be running in
#      its terminals: quit YAAR (with the menu entry that also stops the agents, if you want
#      them stopped) and run the uninstall again, or add --stop-agents". With --stop-agents it
#      says that the agents running in YAAR's terminals end with it, sends SIGTERM to those
#      processes, waits up to 10 s, and sends SIGKILL to what of them remains. It never
#      signals any other process; an agent's own process is not matched.
#   2. removes the app: the package (apt-get remove), the files of the archive route, or the
#      bundle, wherever it is.
#   3. removes the app's own files from the user's folders: the cache folder
#      (~/Library/Caches/yaar-review; ${XDG_CACHE_HOME:-~/.cache}/yaar-review); the runtime
#      folder ($XDG_RUNTIME_DIR/yaar-review, on Linux and only when XDG_RUNTIME_DIR is set);
#      in the data folder (~/Library/Application Support/yaar-review;
#      ${XDG_DATA_HOME:-~/.local/share}/yaar-review) everything but the user's reviews and
#      pi's extension: the socket keeper.sock, its lock keeper.sock.lock, the pieces the
#      Keeper wrote for Claude Code under agents/ (all of agents/ but agents/pi) and any other
#      file; on macOS also what the system keeps under the app's identifier: `defaults delete
#      io.github.tsadoq.yaar` (the documented way, which cfprefsd does not write back), then
#      ~/Library/Preferences/io.github.tsadoq.yaar.plist if still there, ~/Library/Saved
#      Application State/io.github.tsadoq.yaar.savedState, and the folder
#      io.github.tsadoq.yaar in `getconf DARWIN_USER_CACHE_DIR`. And the short socket folder
#      of the user's own Keeper, if there is one: the Link puts the socket at
#      /tmp/yaar-review-<hash>/keeper.sock when <data or runtime folder>/keeper.sock is longer
#      than the system allows (103 bytes on macOS, 107 on Linux), <hash> being the 64-bit
#      FNV-1a hash of that path's bytes as 16 lowercase hexadecimal digits (offset
#      0xcbf29ce484222325, prime 0x100000001b3, computed in bash's 64-bit arithmetic, which
#      wraps as the Link's does, one byte at a time under LC_ALL=C (it holds under macOS's
#      bash 3.2 and under bash 5); the Link's test vector is the check: the path its test
#      `the_address_of_a_path_too_long_keeps_its_value` names gives ad4c554483ec5ddb). Only
#      that one folder, under /private/tmp on macOS (/tmp is a link there) and /tmp on Linux,
#      and only when it is a folder of the user's own and not a link (looked at as the one path
#      <tmp>/<it>, never by walking <tmp>: a walk of a folder that other programs change
#      can fail half way): the short folders of other Keepers of the same user, a build's or a
#      test's, are not the app's.
#   4. keeps the user's reviews, the data folder's store.db (with -wal and -shm) and states/,
#      and pi's extension, agents/pi/ (pi's own settings may name its file; removing it
#      would leave pi pointing at a file that is gone), and says where they are and that
#      --purge removes them. With --purge the data folder goes whole.
#   It touches nothing of the user's repositories and nothing of pi's or Claude Code's own
#   settings. If the user let the app add its extension to pi's settings, the line naming it
#   stays there: the script prints which line to remove by hand (the entry ending
#   "agents/pi/extension/yaar.ts" in the list "extensions" of ~/.pi/agent/settings.json),
#   without reading the file, and, with --purge, that pi will not find the file any more
#   until that line is removed.
#
# The script never asks a question: it prints what it is about to do and does it. (sudo may ask
# for its password on the terminal; that is sudo, not this script.)
#
# Environment:
#   YAAR_BASE_URL      the site with this script and the apt repository
#                      (default: the value of packaging/site/base-url, put in by assemble.sh)
#   YAAR_APT_URL       the apt repository                 (default: $YAAR_BASE_URL/apt)
#   YAAR_RELEASES_URL  the releases page                  (default: https://github.com/Tsadoq/yaar/releases)
#   YAAR_DOWNLOAD_URL  folder with SHA256SUMS, the archives and the .zip
#                                                         (default: $YAAR_RELEASES_URL/latest/download)
#   YAAR_METHOD        auto | apt | archive               (default: auto; ignored on macOS)
#   YAAR_PREFIX        target of the archive route        (default: $HOME/.local)
#   YAAR_INSTALL_DIR   target folder of the macOS app     (default: /Applications, else $HOME/Applications)
#   YAAR_OS_RELEASE    file read instead of /etc/os-release   (for tests)
#   YAAR_ARCH          processor name instead of `uname -m`   (for tests)
#   YAAR_ALLOW_UNNOTARIZED   1: see above; from a loopback download address only   (for tests)
#
# Exit codes: 0 done (or nothing to do); 1 refused or failed, with nothing changed that was
# not put back (a sentence says why); 2 a usage error.
#
# The whole body is in functions and the only top-level command is the last line, so a download
# that stops half way runs nothing. That line ends with a word the script then requires as its
# last argument, because a download cut inside the line itself, just after "main", would be a
# whole command that runs an install whatever the user asked for.
#
# Where the text above leaves a choice:
#   - --purge and --stop-agents belong to --uninstall; without it they are a usage error (2).
#   - With YAAR_METHOD unset or auto, the uninstall on a Debian-like system removes the package
#     (when apt holds one) and the files of the archive route, both; it uses sudo only when apt
#     holds something to remove. An explicit YAAR_METHOD limits it to that route.
#   - The reviews that stay include store.db-journal, which belongs with its database.
#   - The "http://127.0.0.1:" and "http://localhost:" of YAAR_ALLOW_UNNOTARIZED must be followed
#     by a port and then "/" or the end: "http://127.0.0.1:80@host" is another host's address.
#   - Where the developer tools are not installed, `stapler validate` cannot run: the script
#     says so and relies on Gatekeeper's verdict, which it has just taken.
set -euo pipefail

DEFAULT_BASE_URL="@BASE_URL@"
TEAM_ID="@TEAM_ID@"
DEFAULT_RELEASES_URL="https://github.com/Tsadoq/yaar/releases"
PKG="yaar-review"
KEEPER="yaar-keeper"
APP_NAME="YAAR.app"
KEYRING="/etc/apt/keyrings/yaar-review.asc"
SOURCE_FILE="/etc/apt/sources.list.d/yaar-review.sources"
APT_LISTS="/var/lib/apt/lists"
DESKTOP_ID="io.github.tsadoq.yaar"
# The last argument of the one call on the last line. A script cut off inside that line
# lacks it, and main then refuses: a cut at "main" would otherwise run an install with no
# arguments, whatever the user asked for.
END_MARK="end-of-install-script"
TMP=""

say()  { printf '%s\n' "yaar-install: $*"; }
warn() { printf '%s\n' "yaar-install: warning: $*" >&2; }
die()  { printf '%s\n' "yaar-install: error: $*" >&2; exit 1; }
have() { command -v "$1" > /dev/null 2>&1; }
cleanup() { if [ -n "$TMP" ] && [ -d "$TMP" ]; then rm -rf "$TMP"; fi; }

usage() {
  cat <<'USAGE'
usage: install.sh [--uninstall [--purge] [--stop-agents]] [--dry-run] [--help]
  (no option)     install, or move to the newest published version
  --uninstall     remove what this script installed; your reviews stay
  --purge         with --uninstall: remove your reviews and pi's extension file as well
  --stop-agents   with --uninstall: stop YAAR if it runs (the agents in its terminals end)
  --dry-run       print the detected system and the steps, change nothing
Environment: YAAR_BASE_URL, YAAR_APT_URL, YAAR_RELEASES_URL, YAAR_DOWNLOAD_URL, YAAR_METHOD,
             YAAR_PREFIX, YAAR_INSTALL_DIR
USAGE
}

# A path under a base folder, as the Link joins them: one trailing slash of the base is
# not doubled.
join_path() { # <base> <relative>
  printf '%s/%s' "${1%/}" "$2"
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
# Sets ARCHIVE to the local path. A name is letters, digits and . _ + ~ - only (the _ is the
# one of x86_64).
fetch_checked() { # <suffix>
  local suffix="$1" name want got
  fetch "$DOWNLOAD_URL/SHA256SUMS" "$TMP/SHA256SUMS"
  name=$(awk -v s="$suffix" '{ f = $2; sub(/^\*/, "", f); n = length(f) - length(s) + 1;
           if (n > 0 && substr(f, n) == s && f ~ /^yaar-review-[0-9A-Za-z._+~-]+$/) print f }' "$TMP/SHA256SUMS" | sort | tail -n 1)
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

as_root() { # <command> [args...]: the command as root, through sudo when this is not root
  if [ -n "$SUDO" ]; then sudo "$@"; else "$@"; fi
}

# ---------------------------------------------------------------- apt route
apt_refresh_and_install() {
  as_root apt-get update < /dev/null \
    && as_root env DEBIAN_FRONTEND=noninteractive apt-get install -y "$PKG" < /dev/null
}

# Puts back what this run wrote: the key and the source file as they were before it, and
# nothing else (a file this run left alone is not touched).
apt_put_back() { # <wrote-key> <had-key> <wrote-source> <had-source>
  if [ "$1" = 1 ]; then
    if [ "$2" = 1 ]; then as_root install -m 0644 "$TMP/old.key" "$KEYRING"; else as_root rm -f "$KEYRING"; fi
  fi
  if [ "$3" = 1 ]; then
    if [ "$4" = 1 ]; then as_root install -m 0644 "$TMP/old.sources" "$SOURCE_FILE"; else as_root rm -f "$SOURCE_FILE"; fi
  fi
}

install_apt() {
  local had_key=0 had_source=0 wrote_key=0 wrote_source=0
  need_root_tool
  say "route: apt repository at $APT_URL"
  say "about to do, ${SUDO:+with sudo, }as root:"
  say "  1. save the repository key to $KEYRING (only when it differs from the file there)"
  say "  2. write $SOURCE_FILE (only when it differs from the file there)"
  say "  3. apt-get update, then apt-get install $PKG (apt checks the signature and pulls the libraries it needs)"
  [ "$DRY_RUN" = 0 ] || return 0
  fetch "$APT_URL/yaar-review.asc" "$TMP/key.asc"
  grep -q -e '-----BEGIN PGP PUBLIC KEY BLOCK-----' "$TMP/key.asc" \
    || die "$APT_URL/yaar-review.asc is not an armoured public key. Nothing was changed."
  printf 'Types: deb\nURIs: %s\nSuites: stable\nComponents: main\nSigned-By: %s\n' "$APT_URL" "$KEYRING" > "$TMP/new.sources"
  if [ -f "$KEYRING" ]; then cp "$KEYRING" "$TMP/old.key"; had_key=1; fi
  if [ -f "$SOURCE_FILE" ]; then cp "$SOURCE_FILE" "$TMP/old.sources"; had_source=1; fi
  # A file that is already as it should be is left alone, its modification time and its
  # folder's included: a second run changes nothing.
  if [ "$had_key" = 0 ] || ! cmp -s "$TMP/key.asc" "$KEYRING"; then
    [ -d /etc/apt/keyrings ] || as_root install -d -m 0755 /etc/apt/keyrings
    as_root install -m 0644 "$TMP/key.asc" "$KEYRING"
    wrote_key=1
  fi
  if [ "$had_source" = 0 ] || ! cmp -s "$TMP/new.sources" "$SOURCE_FILE"; then
    as_root install -m 0644 "$TMP/new.sources" "$SOURCE_FILE"
    wrote_source=1
  fi
  if ! apt_refresh_and_install; then
    apt_put_back "$wrote_key" "$had_key" "$wrote_source" "$had_source"
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

# Whether apt holds anything of this app: the package, the key or the source file.
apt_has_traces() {
  have dpkg-query || return 1
  if dpkg-query -W -f='${Status}' "$PKG" 2> /dev/null | grep -q "install ok installed"; then return 0; fi
  [ -f "$SOURCE_FILE" ] || [ -f "$KEYRING" ]
}

uninstall_apt() {
  local uri prefix
  say "step 2, the package: apt-get remove $PKG; delete $SOURCE_FILE, $KEYRING and apt's copy of the repository's index"
  if ! apt_has_traces; then
    say "apt holds nothing of $PKG (no package, no key, no source file)"
    return 0
  fi
  need_root_tool
  say "about to do, ${SUDO:+with sudo, }as root: the removal above"
  [ "$DRY_RUN" = 0 ] || return 0
  if dpkg-query -W -f='${Status}' "$PKG" 2> /dev/null | grep -q "install ok installed"; then
    as_root env DEBIAN_FRONTEND=noninteractive apt-get remove -y "$PKG" < /dev/null
  else
    say "$PKG is not installed"
  fi
  if [ -f "$SOURCE_FILE" ]; then
    uri=$(sed -n 's/^URIs: *//p' "$SOURCE_FILE" | head -n 1)
    prefix=$(apt_list_prefix "$uri")
    if [ -n "$prefix" ]; then
      as_root find "$APT_LISTS" -maxdepth 2 -type f -name "${prefix}_dists_stable_*" -exec rm -f {} +
    fi
  fi
  as_root rm -f "$SOURCE_FILE" "$KEYRING"
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
  say "step 2, the files of the archive route: delete what this script put under $prefix, and the folders that are then empty"
  [ "$DRY_RUN" = 0 ] || return 0
  rm -f "$prefix/bin/$PKG" "$prefix/libexec/$PKG/$KEEPER" \
        "$prefix/share/applications/$DESKTOP_ID.desktop" "$prefix/$icons/$DESKTOP_ID.png"
  rm -rf "${prefix:?}/$doc"
  for d in "libexec/$PKG" libexec bin share/applications "$icons" share/icons/hicolor/256x256 \
           share/icons/hicolor share/icons share/doc share; do
    # A folder that still holds something is not this script's to remove.
    if [ -d "$prefix/$d" ] && [ -z "$(ls -A "$prefix/$d")" ]; then rmdir "$prefix/$d"; fi
  done
  say "removed"
}

# ---------------------------------------------------------------- macOS
# The folder the app is installed into.
mac_dest() {
  if [ -n "${YAAR_INSTALL_DIR:-}" ]; then printf '%s' "$YAAR_INSTALL_DIR"
  elif [ -w /Applications ]; then printf '%s' /Applications
  else printf '%s' "$HOME/Applications"
  fi
}

# Every folder the app may be in, one per line: the system's, the user's, and
# YAAR_INSTALL_DIR when set.
mac_folders() {
  printf '%s\n' /Applications "$HOME/Applications"
  if [ -n "${YAAR_INSTALL_DIR:-}" ] && [ "${YAAR_INSTALL_DIR%/}" != "$HOME/Applications" ]; then
    printf '%s\n' "$YAAR_INSTALL_DIR"
  fi
}

# Removes what an interrupted run left in the folders above.
mac_remove_leftovers() {
  local d f
  while IFS= read -r d; do
    [ -d "$d" ] || continue
    for f in "$d"/.YAAR.app.new.* "$d"/.yaar-review-old.*; do
      if [ -e "$f" ] || [ -L "$f" ]; then
        say "removing what an interrupted run left: $f"
        rm -rf "$f" || die "could not remove $f, which an interrupted run left"
      fi
    done
  done <<EOF
$(mac_folders)
EOF
}

# Whether the download address is a server on this machine itself: the address starts with
# the scheme, the host 127.0.0.1 or localhost and a port, and the port is followed by a
# slash or the end. A "user:password@" before another host ("http://127.0.0.1:80@host") is
# no server on this machine and is not taken.
is_loopback_url() { # <url>
  local re='^http://(127\.0\.0\.1|localhost):[0-9]+(/|$)'
  [[ "$1" =~ $re ]]
}

# Decides, before anything is downloaded, whether the checks of the notarisation and of the
# team are made. Sets MAC_STRICT to 1 (made) or 0 (skipped, for the rehearsal's app).
mac_gate() {
  MAC_STRICT=1
  if [ "${YAAR_ALLOW_UNNOTARIZED:-}" = 1 ]; then
    is_loopback_url "$DOWNLOAD_URL" \
      || die "YAAR_ALLOW_UNNOTARIZED is taken only for a download from 127.0.0.1 or localhost, and YAAR_DOWNLOAD_URL is $DOWNLOAD_URL. Nothing was downloaded."
    MAC_STRICT=0
    say "YAAR_ALLOW_UNNOTARIZED: Gatekeeper's verdict, the ticket and the team are not checked (download from this machine)"
    return 0
  fi
  case "$TEAM_ID" in
    @*@) die "this copy of the install script names no team, so it cannot tell the app's signer: use the install.sh of the site. Nothing was downloaded." ;;
  esac
  case "$TEAM_ID" in
    *[!A-Z0-9]*|"") die "the Team ID in this copy of the install script is not ten letters and digits. Nothing was downloaded." ;;
  esac
  [ "${#TEAM_ID}" -eq 10 ] || die "the Team ID in this copy of the install script is not ten letters and digits. Nothing was downloaded."
}

# The checks of an unpacked app, before anything is copied.
mac_verify() { # <app>
  local app="$1" out
  if [ "$MAC_STRICT" = 1 ]; then
    /usr/bin/codesign --verify --deep --strict \
      -R="anchor apple generic and certificate leaf[subject.OU] = \"$TEAM_ID\"" "$app" \
      || die "the code signature of the unpacked app does not verify, or the app was not signed by team $TEAM_ID. Nothing was installed."
    say "code signature verifies, signed by team $TEAM_ID"
    if ! out=$(/usr/sbin/spctl --assess --type execute --verbose "$app" 2>&1); then
      die "Gatekeeper (spctl --assess) does not accept the app: $out. Nothing was installed."
    fi
    say "Gatekeeper accepts the app"
    # stapler comes with the developer tools; where they are not installed, Gatekeeper's
    # verdict above stands alone (xcrun would open a dialog asking for them).
    if /usr/bin/xcode-select -p > /dev/null 2>&1 && /usr/bin/xcrun --find stapler > /dev/null 2>&1; then
      /usr/bin/xcrun stapler validate "$app" || die "the notarisation ticket is not stapled to the app (stapler validate failed). Nothing was installed."
      say "the notarisation ticket is stapled"
    else
      warn "stapler is not installed on this Mac (no developer tools): the stapled ticket was not checked"
    fi
  else
    /usr/bin/codesign --verify --deep --strict "$app" \
      || die "the code signature of the unpacked app does not verify. Nothing was installed."
    say "code signature verifies"
  fi
}

mac_version() { # <app>: its CFBundleShortVersionString
  /usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$1/Contents/Info.plist" 2> /dev/null
}

install_macos() {
  local dest app ver current new old
  mac_gate
  dest=$(mac_dest)
  say "route: macOS app from $DOWNLOAD_URL into $dest"
  say "about to do: remove what an interrupted run left, download the .zip, check its SHA-256, unpack, verify the app, copy $APP_NAME to $dest"
  [ "$DRY_RUN" = 0 ] || return 0
  mac_remove_leftovers
  fetch_checked "-macos-universal.zip"
  mkdir "$TMP/x"
  /usr/bin/ditto -x -k "$ARCHIVE" "$TMP/x"
  app="$TMP/x/$APP_NAME"
  [ -d "$app" ] || die "the archive does not contain $APP_NAME. Nothing was installed."
  mac_verify "$app"
  ver=$(mac_version "$app") || die "the app in the archive has no CFBundleShortVersionString. Nothing was installed."
  if [ -d "$dest/$APP_NAME" ] && current=$(mac_version "$dest/$APP_NAME") && [ "$current" = "$ver" ] \
     && /usr/bin/codesign --verify --deep --strict "$dest/$APP_NAME" > /dev/null 2>&1; then
    say "already installed: version $ver in $dest/$APP_NAME, its signature verifies. Nothing was changed."
    return 0
  fi
  mkdir -p "$dest" || die "could not make $dest"
  new="$dest/.YAAR.app.new.$$"
  old="$dest/.yaar-review-old.$$"
  /usr/bin/ditto "$app" "$new" || { rm -rf "${new:?}"; die "could not copy the app to $dest. Nothing was changed."; }
  if [ -e "$dest/$APP_NAME" ] || [ -L "$dest/$APP_NAME" ]; then
    say "replacing the copy already in $dest"
    mv "$dest/$APP_NAME" "$old" || { rm -rf "${new:?}"; die "could not move the copy in $dest aside. Nothing was changed."; }
  fi
  if ! mv "$new" "$dest/$APP_NAME"; then
    if [ -e "$old" ]; then mv "$old" "$dest/$APP_NAME" || warn "the previous app is at $old"; fi
    rm -rf "${new:?}"
    die "could not move the new app into place in $dest. The previous state is restored."
  fi
  rm -rf "${old:?}"
  say "installed: version $ver in $dest/$APP_NAME"
  say "this script does not start it. Open it from Finder or Launchpad."
}

uninstall_macos() {
  local d found=0
  say "step 2, the app: delete $APP_NAME in /Applications, in $HOME/Applications${YAAR_INSTALL_DIR:+ and in $YAAR_INSTALL_DIR}, and what an interrupted install left there"
  [ "$DRY_RUN" = 0 ] || return 0
  mac_remove_leftovers
  while IFS= read -r d; do
    if [ -e "$d/$APP_NAME" ] || [ -L "$d/$APP_NAME" ]; then
      rm -rf "${d:?}/$APP_NAME" || die "could not remove $d/$APP_NAME (is it another user's copy?). Nothing else was removed."
      say "removed $d/$APP_NAME"
      found=1
    fi
  done <<EOF
$(mac_folders)
EOF
  [ "$found" = 1 ] || say "no $APP_NAME found in those folders"
}

# ---------------------------------------------------------------- the uninstall
data_folder() {
  if [ "$OS" = Darwin ]; then join_path "$(join_path "$HOME" "Library/Application Support")" "$PKG"
  elif [ -n "${XDG_DATA_HOME:-}" ]; then join_path "$XDG_DATA_HOME" "$PKG"
  else join_path "$(join_path "$HOME" ".local/share")" "$PKG"
  fi
}

cache_folder() {
  if [ "$OS" = Darwin ]; then join_path "$(join_path "$HOME" "Library/Caches")" "$PKG"
  elif [ -n "${XDG_CACHE_HOME:-}" ]; then join_path "$XDG_CACHE_HOME" "$PKG"
  else join_path "$(join_path "$HOME" ".cache")" "$PKG"
  fi
}

# Where the Link puts the Keeper's socket before it shortens the path: in the runtime folder
# on Linux when XDG_RUNTIME_DIR is set, else in the data folder.
socket_wanted() {
  if [ "$OS" != Darwin ] && [ -n "${XDG_RUNTIME_DIR:-}" ]; then
    join_path "$(join_path "$XDG_RUNTIME_DIR" "$PKG")" keeper.sock
  else
    join_path "$(data_folder)" keeper.sock
  fi
}

# The 64-bit FNV-1a hash of the bytes of <string>, as 16 lowercase hexadecimal digits: the
# hash the Link puts in the name of a short socket folder. bash's arithmetic is 64 bits wide
# and wraps, one byte at a time. The bytes come from od, so the result does not depend on
# the locale.
fnv1a_hex() { # <string>
  printf '%s' "$1" | od -An -v -tu1 | tr -s ' ' '\n' | {
    local hash=$((0xcbf29ce484222325)) prime=$((0x100000001b3)) byte
    while read -r byte; do
      [ -n "$byte" ] || continue
      hash=$(( (hash ^ byte) * prime ))
    done
    printf '%016x\n' "$hash"
  }
}

# The name of the short socket folder of the user's own Keeper, or nothing when its socket
# path is short enough to be used as it is (103 bytes on macOS, 107 on Linux).
short_socket_name() {
  local wanted max length
  wanted=$(socket_wanted)
  if [ "$OS" = Darwin ]; then max=103; else max=107; fi
  length=$(( $(printf '%s' "$wanted" | wc -c) ))
  [ "$length" -gt "$max" ] || return 0
  printf '%s-%s' "$PKG" "$(fnv1a_hex "$wanted")"
}

# The full paths of the programs of the app this script is about to remove, one per line:
# the form it was started under and the form the system names it by.
app_programs() {
  local d prefix
  if [ "$OS" = Darwin ]; then
    while IFS= read -r d; do
      printf '%s\n' "$d/$APP_NAME/Contents/MacOS/$PKG" "$d/$APP_NAME/Contents/Helpers/$KEEPER"
    done <<EOF
$(mac_folders)
EOF
  else
    prefix="${YAAR_PREFIX:-$HOME/.local}"
    printf '%s\n' "/usr/bin/$PKG" "/usr/libexec/$PKG/$KEEPER" "$prefix/bin/$PKG" "$prefix/libexec/$PKG/$KEEPER"
  fi
}

# The same paths with their folders resolved: a link in a folder's path (a /Applications
# on another volume, a home that is a link) changes what the system calls the program.
resolved_programs() {
  local p dir base
  while IFS= read -r p; do
    printf '%s\n' "$p"
    dir=$(dirname "$p"); base=$(basename "$p")
    if [ -d "$dir" ]; then printf '%s/%s\n' "$(cd -P "$dir" && pwd)" "$base"; fi
  done <<EOF
$(app_programs)
EOF
}

# The user's own processes whose program is one of the app's, matched by its full path and
# never by a name alone: "<pid> keeper <path>" or "<pid> window <path>" per line. A Keeper of
# another copy (a build folder's) has another path and is not listed.
app_processes() {
  local programs pid exe kind
  programs=$(resolved_programs)
  if [ "$OS" = Darwin ]; then
    ps -axo pid=,uid=,comm= > "$TMP/ps.out" || die "ps failed, so the app's processes cannot be told"
    PROGRAMS="$programs" awk -v me="$(id -u)" -v keeper="/$KEEPER" '
      BEGIN { n = split(ENVIRON["PROGRAMS"], a, "\n"); for (i = 1; i <= n; i++) want[a[i]] = 1 }
      $2 == me {
        path = $0; sub(/^[ \t]*[0-9]+[ \t]+[0-9]+[ \t]+/, "", path)
        if (path in want) {
          k = (length(path) >= length(keeper) && substr(path, length(path) - length(keeper) + 1) == keeper) ? "keeper" : "window"
          print $1, k, path
        }
      }' "$TMP/ps.out"
  else
    for pid in /proc/[0-9]*; do
      [ -O "$pid" ] || continue
      if exe=$(readlink "$pid/exe" 2> /dev/null); then
        exe="${exe% (deleted)}"
        case "
$programs
" in
          *"
$exe
"*)
            case "$exe" in */"$KEEPER") kind=keeper ;; *) kind=window ;; esac
            printf '%s %s %s\n' "${pid#/proc/}" "$kind" "$exe"
            ;;
        esac
      fi
    done
  fi
}

# Says which processes the list (the lines of app_processes) holds, on one line.
say_running() { # <list>
  local pid kind path out=""
  while read -r pid kind path; do
    [ -n "$pid" ] || continue
    out="$out${out:+; }$pid $kind $path"
  done <<EOF
$1
EOF
  say "still running: $out"
}

# Waits up to <seconds> for the app to have no process, polling every 0.2 s. When the
# deadline passes it says which processes it last saw and returns 1.
# The list is read into a variable first: a failure of app_processes inside a test or a
# here-document would end only its own subshell and read as "no process".
wait_until_stopped() { # <seconds>
  local i=0 limit=$(( $1 * 5 )) list
  while [ "$i" -lt "$limit" ]; do
    list=$(app_processes) || exit 1
    [ -n "$list" ] || return 0
    sleep 0.2
    i=$((i + 1))
  done
  list=$(app_processes) || exit 1
  [ -n "$list" ] || return 0
  say_running "$list"
  return 1
}

signal_app() { # <TERM|KILL>
  local pid kind list
  list=$(app_processes) || exit 1
  while read -r pid kind; do
    [ -n "$pid" ] || continue
    if ! kill "-$1" "$pid" 2> /dev/null; then say "process $pid had already left"; fi
  done <<EOF
$list
EOF
}

# Whether the list (the lines of app_processes) holds a Window.
has_window() { # <list>
  local pid kind path
  while read -r pid kind path; do
    if [ "$kind" = window ]; then return 0; fi
  done <<EOF
$1
EOF
  return 1
}

# Part 1 of the uninstall: the app must have stopped.
ensure_app_stopped() {
  local running
  running=$(app_processes) || exit 1
  say "step 1, the app must have stopped: its programs are matched by their full path"
  if [ -z "$running" ]; then say "the app is not running"; return 0; fi
  if [ "$DRY_RUN" != 0 ]; then
    say "the app is running now: the uninstall would refuse, or with --stop-agents stop it"
    return 0
  fi
  if [ "$STOP_AGENTS" = 1 ]; then
    say "stopping the app with --stop-agents: the agents running in YAAR's terminals end with it"
    signal_app TERM
    if ! wait_until_stopped 10; then
      say "still running after 10 s: sending SIGKILL to what remains of it"
      signal_app KILL
      wait_until_stopped 5 || die "the app's processes did not end. Nothing was removed."
    fi
    say "the app has stopped"
    return 0
  fi
  if has_window "$running"; then
    say_running "$running"
  else
    # Only a Keeper: it leaves by itself 5 s after its last Window when no agent runs.
    say "only the Keeper runs: waiting up to 10 s for it to leave by itself"
    if wait_until_stopped 10; then say "the app has stopped"; return 0; fi
  fi
  die "YAAR is running, and agents may be running in its terminals: quit YAAR (with the menu entry that also stops the agents, if you want them stopped) and run the uninstall again, or add --stop-agents. Nothing was changed."
}

# What of the data folder stays without --purge: the user's reviews and pi's extension.
clean_data_folder() { # <folder>
  local d="$1" e name f
  for e in "$d"/* "$d"/.[!.]* "$d"/..?*; do
    [ -e "$e" ] || [ -L "$e" ] || continue
    name="${e##*/}"
    case "$name" in
      store.db|store.db-wal|store.db-shm|store.db-journal|states) ;;
      agents)
        # The pieces written for Claude Code go; pi's extension stays.
        for f in "$e"/* "$e"/.[!.]* "$e"/..?*; do
          [ -e "$f" ] || [ -L "$f" ] || continue
          [ "${f##*/}" = pi ] || rm -rf "${f:?}"
        done
        if [ -d "$e" ] && [ -z "$(ls -A "$e")" ]; then rmdir "$e"; fi
        ;;
      *) rm -rf "${e:?}" ;;
    esac
  done
}

# The part of the uninstall that is the system's: what macOS keeps under the app's identifier.
remove_mac_files() {
  local out cache
  if ! out=$(defaults delete "$DESKTOP_ID" 2>&1); then
    case "$out" in
      *"does not exist"*) ;;
      *) die "could not delete the preferences of $DESKTOP_ID (defaults said: $out)" ;;
    esac
  fi
  rm -f "$HOME/Library/Preferences/$DESKTOP_ID.plist"
  rm -rf "${HOME:?}/Library/Saved Application State/$DESKTOP_ID.savedState"
  cache=$(getconf DARWIN_USER_CACHE_DIR) || die "getconf DARWIN_USER_CACHE_DIR failed"
  case "$cache" in
    /?*) cache="${cache%/}"; rm -rf "${cache:?}/$DESKTOP_ID" ;;
    *) die "getconf DARWIN_USER_CACHE_DIR printed '$cache', not a folder" ;;
  esac
}

# Part 3 of the uninstall: the app's own files in the user's folders.
remove_user_files() {
  local data cache runtime name tmp found
  data=$(data_folder); cache=$(cache_folder)
  say "step 3, the app's own files in your folders:"
  say "  the cache folder $cache"
  if [ "$OS" != Darwin ] && [ -n "${XDG_RUNTIME_DIR:-}" ]; then
    runtime=$(join_path "$XDG_RUNTIME_DIR" "$PKG")
    say "  the runtime folder $runtime"
  fi
  if [ "$PURGE" = 1 ]; then
    say "  the whole data folder $data (--purge)"
  else
    say "  the data folder $data, but your reviews and pi's extension (step 4)"
  fi
  if [ "$OS" = Darwin ]; then
    say "  what the system keeps under $DESKTOP_ID: its preferences (defaults delete), saved window state, its cache"
  fi
  name=$(short_socket_name)
  [ -z "$name" ] || say "  the short socket folder $name of your own Keeper, in /tmp"
  [ "$DRY_RUN" = 0 ] || return 0
  rm -rf "${cache:?}"
  if [ -n "${runtime:-}" ]; then rm -rf "${runtime:?}"; fi
  if [ "$PURGE" = 1 ]; then rm -rf "${data:?}"; elif [ -d "$data" ]; then clean_data_folder "$data"; fi
  if [ "$OS" = Darwin ]; then remove_mac_files; fi
  if [ -n "$name" ]; then
    # Only a folder of that very name, of the user's own, and not a link: the short folders
    # of other Keepers (a build's, a test's) are not the app's.
    if [ "$OS" = Darwin ]; then tmp=/private/tmp; else tmp=/tmp; fi
    found="$tmp/$name"
    if [ -d "$found" ] && [ ! -L "$found" ] && [ -O "$found" ]; then rm -rf "${found:?}"; say "removed $found"; fi
  fi
}

# Part 4 of the uninstall, and what to do by hand about pi.
report_kept() {
  local data kept="" would=""
  data=$(data_folder)
  if [ "$DRY_RUN" != 0 ]; then would="(dry run) "; fi
  say "step 4, your reviews and pi's extension:"
  if [ "$PURGE" = 1 ]; then
    say "  ${would}removed with the rest of $data (--purge)"
    say "  if you let YAAR add its extension to pi's settings, pi will not find the file any more until that line is removed"
  else
    if [ -e "$data/store.db" ]; then kept="$kept $data/store.db (your reviews)"; fi
    if [ -d "$data/states" ]; then kept="$kept $data/states/ (what they refer to)"; fi
    if [ -d "$data/agents/pi" ]; then kept="$kept $data/agents/pi/ (pi's extension: pi's own settings may name its file)"; fi
    if [ -n "$kept" ]; then
      say "  ${would}kept:$kept"
      say "  --uninstall --purge removes them too"
    else
      say "  none were found in $data; --uninstall --purge would remove them"
    fi
  fi
  say "  nothing of your repositories, of pi's or of Claude Code's own settings was touched"
  say "  if you let YAAR add its extension to pi's settings, the line naming it stays there: remove the entry ending \"agents/pi/extension/yaar.ts\" from the list \"extensions\" in ~/.pi/agent/settings.json by hand (this script does not read that file)"
}

uninstall() {
  if [ "$(id -u)" -eq 0 ]; then
    say "running as root: the files of $HOME are removed, not another user's"
  fi
  ensure_app_stopped
  case "$METHOD" in
    macos) uninstall_macos ;;
    *)
      # Any route this script installed by: with YAAR_METHOD left to itself, the package
      # (when apt holds one) and the archive's files both go.
      if [ "${YAAR_METHOD:-auto}" = auto ]; then
        if is_debian_like; then uninstall_apt; fi
        uninstall_archive
      elif [ "$METHOD" = apt ]; then
        uninstall_apt
      else
        uninstall_archive
      fi
      ;;
  esac
  remove_user_files
  report_kept
}

# ---------------------------------------------------------------- entry
main() {
  local action=install arg last="" n=$# i=0
  DRY_RUN=0; PURGE=0; STOP_AGENTS=0
  for arg in "$@"; do
    i=$((i + 1))
    last="$arg"
    [ "$i" -lt "$n" ] || continue
    case "$arg" in
      --uninstall) action=uninstall ;;
      --purge) PURGE=1 ;;
      --stop-agents) STOP_AGENTS=1 ;;
      --dry-run) DRY_RUN=1 ;;
      -h|--help) usage; exit 0 ;;
      *) usage >&2; printf '%s\n' "yaar-install: error: unknown option: $arg" >&2; exit 2 ;;
    esac
  done
  [ "$last" = "$END_MARK" ] || die "this script ends before its last line (the download was cut short), so nothing was done: run the install line again"
  if [ "$action" = install ] && { [ "$PURGE" = 1 ] || [ "$STOP_AGENTS" = 1 ]; }; then
    usage >&2; printf '%s\n' "yaar-install: error: --purge and --stop-agents belong with --uninstall" >&2; exit 2
  fi
  # Nothing may read what is left of the script on the standard input, which a download
  # piped into bash is still feeding.
  exec < /dev/null
  [ -n "${HOME:-}" ] || die "HOME is not set: the script acts on the folders of the user's home and has none to name"
  case "$HOME" in
    /*) ;;
    *) die "HOME is '$HOME', which is not an absolute path: the script acts on the folders of the user's home" ;;
  esac
  [ -d "$HOME" ] || die "HOME is $HOME, which is not a folder that exists: the script acts on the folders of the user's home"
  BASE_URL="${YAAR_BASE_URL:-$DEFAULT_BASE_URL}"; BASE_URL="${BASE_URL%/}"
  case "$BASE_URL" in
    @*@) die "this copy of the install script holds no base address (it is the copy in the repository): set YAAR_BASE_URL to the site's address, or use the install.sh of the site" ;;
  esac
  APT_URL="${YAAR_APT_URL:-$BASE_URL/apt}"; APT_URL="${APT_URL%/}"
  RELEASES_URL="${YAAR_RELEASES_URL:-$DEFAULT_RELEASES_URL}"; RELEASES_URL="${RELEASES_URL%/}"
  DOWNLOAD_URL="${YAAR_DOWNLOAD_URL:-$RELEASES_URL/latest/download}"; DOWNLOAD_URL="${DOWNLOAD_URL%/}"
  detect
  say "system: $OS $ARCH, distribution: $DISTRO, route: $METHOD, action: $action$( [ "$DRY_RUN" = 0 ] || printf ' (dry run)')"
  TMP=$(mktemp -d "${TMPDIR:-/tmp}/yaar-install.XXXXXX") || die "could not make a temporary folder in ${TMPDIR:-/tmp}"
  trap cleanup EXIT
  trap 'exit 1' HUP INT TERM
  case "$action" in
    install) "install_${METHOD}" ;;
    uninstall) uninstall ;;
  esac
}

main "$@" "$END_MARK"

#!/usr/bin/env bash
# Installs YAAR, or removes it.
#
# curl -fsSL <base>/install.sh | bash
# curl -fsSL <base>/install.sh | bash -s -- --uninstall [--purge] [--stop-agents]
# curl -fsSL <base>/install.sh | bash -s -- --dry-run   # say what would be done, change nothing
#
# DEFAULT_BASE_URL and TEAM_ID hold placeholders in the repository copy, replaced by
# packaging/release/assemble.sh (no other line may contain either); unreplaced, the script
# needs YAAR_BASE_URL. YAAR_ALLOW_UNNOTARIZED=1 (ad hoc signed rehearsal app) skips the Gatekeeper,
# ticket and team checks and is taken only when YAAR_DOWNLOAD_URL is a loopback server.
# Exit: 0 done or nothing to do, 1 refused or failed (a sentence says why), 2 usage.
# The body is all functions and the last line the only top-level command: a cut download runs
# nothing.
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
# The last argument of the call on the last line: a script cut inside that line, at "main",
# would otherwise run an install with no arguments.
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
             YAAR_PREFIX, YAAR_INSTALL_DIR, YAAR_SKIP_SERVERS
USAGE
}

# Joins as the Link does: the bytes of the result are hashed.
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

apt_refresh_and_install() {
  as_root apt-get update < /dev/null \
    && as_root env DEBIAN_FRONTEND=noninteractive apt-get install -y "$PKG" < /dev/null
}

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
  # A file already as it should be is left alone, its folder's mtime included:
  # a second run changes nothing.
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
  install_language_servers "/usr/bin/$PKG"
  say "start it with: $PKG    Later versions arrive with: sudo apt-get update && sudo apt-get upgrade"
}

# apt names a repository's index files by its address without the scheme, '/' as '_'.
apt_list_prefix() { # <uri>
  local p="${1#*://}"
  p="${p%/}"
  case "$p" in ""|*[!0-9A-Za-z.:/-]*) return 0 ;; esac
  printf '%s' "$p" | tr '/' '_'
}

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
  install_language_servers "$prefix/bin/$PKG"
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
    if [ -d "$prefix/$d" ] && [ -z "$(ls -A "$prefix/$d")" ]; then rmdir "$prefix/$d"; fi
  done
  say "removed"
}

mac_dest() {
  if [ -n "${YAAR_INSTALL_DIR:-}" ]; then printf '%s' "$YAAR_INSTALL_DIR"
  elif [ -w /Applications ]; then printf '%s' /Applications
  else printf '%s' "$HOME/Applications"
  fi
}

mac_folders() {
  printf '%s\n' /Applications "$HOME/Applications"
  if [ -n "${YAAR_INSTALL_DIR:-}" ] && [ "${YAAR_INSTALL_DIR%/}" != "$HOME/Applications" ]; then
    printf '%s\n' "$YAAR_INSTALL_DIR"
  fi
}

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

# True for http://127.0.0.1:<port> or http://localhost:<port> followed by '/' or the end;
# "http://127.0.0.1:80@host" is another host and is not taken.
is_loopback_url() { # <url>
  local re='^http://(127\.0\.0\.1|localhost):[0-9]+(/|$)'
  [[ "$1" =~ $re ]]
}

is_local_url() { # <url>
  local re='^https?://(127\.0\.0\.1|localhost)(:[0-9]+)?(/|$)'
  case "$1" in file://*) return 0 ;; esac
  [[ "$1" =~ $re ]]
}

is_local_install() {
  is_local_url "$BASE_URL" || is_local_url "$DOWNLOAD_URL" || is_local_url "$APT_URL"
}

install_language_servers() { # <binary>
  local bin="$1"
  if [ "${YAAR_SKIP_SERVERS:-}" = 1 ] || is_local_install; then
    return 0
  fi
  [ -x "$bin" ] || return 0
  say "installing built-in language servers (skip with YAAR_SKIP_SERVERS=1)"
  "$bin" --install-servers || warn "could not install all language servers (they will download on demand)"
}

# Sets MAC_STRICT (1 checked, 0 skipped for the rehearsal app) before anything is downloaded.
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
    # xcrun would open a dialog asking for the developer tools:
    # without them Gatekeeper's verdict stands alone.
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
  install_language_servers "$dest/$APP_NAME/Contents/MacOS/$PKG"
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

socket_wanted() {
  if [ "$OS" != Darwin ] && [ -n "${XDG_RUNTIME_DIR:-}" ]; then
    join_path "$(join_path "$XDG_RUNTIME_DIR" "$PKG")" keeper.sock
  else
    join_path "$(data_folder)" keeper.sock
  fi
}

# The hash in the name of the Link's short socket folder: 64-bit FNV-1a (offset 0xcbf29ce484222325,
# prime 0x100000001b3) of the bytes from od, as 16 hex digits; bash wraps as the Link does. The
# Link's test the_address_of_a_path_too_long_keeps_its_value gives ad4c554483ec5ddb.
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

# Nothing when the socket path fits as it is (103 bytes on macOS, 107 on Linux).
short_socket_name() {
  local wanted max length
  wanted=$(socket_wanted)
  if [ "$OS" = Darwin ]; then max=103; else max=107; fi
  length=$(( $(printf '%s' "$wanted" | wc -c) ))
  [ "$length" -gt "$max" ] || return 0
  printf '%s-%s' "$PKG" "$(fnv1a_hex "$wanted")"
}

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

# A link in the path (/Applications on another volume, a linked home) changes
# what the system calls the program.
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

# The list is read into a variable first: a failure of app_processes inside a test or
# here-document would end only its subshell and read as "no process".
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

has_window() { # <list>
  local pid kind path
  while read -r pid kind path; do
    if [ "$kind" = window ]; then return 0; fi
  done <<EOF
$1
EOF
  return 1
}

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
    # A Keeper leaves by itself 5 s after its last Window when no agent runs.
    say "only the Keeper runs: waiting up to 10 s for it to leave by itself"
    if wait_until_stopped 10; then say "the app has stopped"; return 0; fi
  fi
  die "YAAR is running, and agents may be running in its terminals: quit YAAR (with the menu entry that also stops the agents, if you want them stopped) and run the uninstall again, or add --stop-agents. Nothing was changed."
}

clean_data_folder() { # <folder>
  local d="$1" e name f
  for e in "$d"/* "$d"/.[!.]* "$d"/..?*; do
    [ -e "$e" ] || [ -L "$e" ] || continue
    name="${e##*/}"
    case "$name" in
      store.db|store.db-wal|store.db-shm|store.db-journal|states) ;;
      agents)
        for f in "$e"/* "$e"/.[!.]* "$e"/..?*; do
          [ -e "$f" ] || [ -L "$f" ] || continue
          # agents/pi stays: pi's own settings may still name its extension file.
          [ "${f##*/}" = pi ] || rm -rf "${f:?}"
        done
        if [ -d "$e" ] && [ -z "$(ls -A "$e")" ]; then rmdir "$e"; fi
        ;;
      *) rm -rf "${e:?}" ;;
    esac
  done
}

remove_mac_files() {
  local out cache
  # Asked first: the words for a missing domain differ between macOS releases, and an
  # app that never ran has none, which is no error; a delete that fails on a present domain is.
  if defaults read "$DESKTOP_ID" > /dev/null 2>&1; then
    out=$(defaults delete "$DESKTOP_ID" 2>&1) ||
      die "could not delete the preferences of $DESKTOP_ID (defaults said: $out)"
  fi
  rm -f "$HOME/Library/Preferences/$DESKTOP_ID.plist"
  rm -rf "${HOME:?}/Library/Saved Application State/$DESKTOP_ID.savedState"
  cache=$(getconf DARWIN_USER_CACHE_DIR) || die "getconf DARWIN_USER_CACHE_DIR failed"
  case "$cache" in
    /?*) cache="${cache%/}"; rm -rf "${cache:?}/$DESKTOP_ID" ;;
    *) die "getconf DARWIN_USER_CACHE_DIR printed '$cache', not a folder" ;;
  esac
}

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
    # Only a folder of that very name, the user's own and not a link:
    # other Keepers' short folders are not the app's.
    if [ "$OS" = Darwin ]; then tmp=/private/tmp; else tmp=/tmp; fi
    found="$tmp/$name"
    if [ -d "$found" ] && [ ! -L "$found" ] && [ -O "$found" ]; then rm -rf "${found:?}"; say "removed $found"; fi
  fi
}

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
  # Nothing may read what is left of the script on stdin, which a
  # download piped into bash is still feeding.
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

#!/bin/bash
# Row PKG-05M: the install line installs the app on macOS and the uninstall removes it.
#
#   .github/scripts/macos-install.sh install
#   .github/scripts/macos-install.sh uninstall
#
# The site and the release files are served on port 8000 (serve-site.sh). The rehearsal's app
# is signed ad hoc and not notarised, so both runs set YAAR_ALLOW_UNNOTARIZED=1, which the
# install script takes, and only from a loopback address, to skip its checks of the
# notarisation and of the team; it still verifies the signature. No other variable changes
# the routes: YAAR_BASE_URL=http://127.0.0.1:8000,
# YAAR_DOWNLOAD_URL=http://127.0.0.1:8000/download; the app goes to its default place,
# /Applications, which every assertion below means by /Applications: YAAR_INSTALL_DIR, when
# set (a builder on the owner's Mac sets it to a scratch folder, and HOME and TMPDIR too),
# replaces it. Port: YAAR_TEST_PORT when set (a builder picks a free one), else 8000.
# install:
#   1. The door stays shut away from a loopback address: the install line with
#      YAAR_ALLOW_UNNOTARIZED=1 and YAAR_DOWNLOAD_URL=https://example.invalid/download exits
#      1, says that the variable is taken only from 127.0.0.1 or localhost, downloads
#      nothing (it says so before any download) and leaves no YAAR.app in /Applications.
#   2. `curl -fsSL $YAAR_BASE_URL/install.sh | bash`, exit 0, and its output says the route
#      is macos and that the SHA-256 matched SHA256SUMS;
#   3. /Applications/YAAR.app exists, `codesign --verify --deep --strict` passes on it, its
#      CFBundleShortVersionString is $VERSION, no file of it carries com.apple.quarantine
#      (`xattr -r`), and /Applications holds no `.YAAR.app.new.*` and no `.yaar-review-old.*`;
#   4. the same line run a second time exits 0, says the version is already installed, and
#      leaves the bundle's files as they were (a listing with sizes and modification times of
#      every file, before and after, equal).
# uninstall:
#   1. `curl -fsSL $YAAR_BASE_URL/install.sh | bash -s -- --uninstall`, exit 0;
#   2. /Applications/YAAR.app is gone; no process runs whose program is
#      /Applications/YAAR.app/Contents/MacOS/yaar-review or
#      /Applications/YAAR.app/Contents/Helpers/yaar-keeper (matched by the full path, as the
#      install script's own uninstall matches them: `ps -axo pid=,comm=`; a Keeper of a build
#      folder, another agent's on the owner's Mac among them, is not the app's); the user's
#      lock file of the Keeper is gone
#      (~/Library/Application Support/yaar-review/keeper.sock.lock). What stays (the
#      reviews) is user-files.sh's to judge.
#
# Exit codes: 0 holds; 1 does not; 2 usage error.
#
# What the header leaves, decided here: (1) install 1 also needs $dest to hold no YAAR.app
# when it starts, or "leaves no YAAR.app" could not fail; (2) VERSION is read from the
# environment (the job's) and the step fails when it is unset; (3) "no file carries
# com.apple.quarantine" is read off `xattr -rl`; (4) the listing of install 4 holds, for
# every path of the bundle, its kind, size, modification time, inode and mode: a bundle that
# was taken apart and put back whole could show the same sizes and times, not the same inodes; (5) the
# uninstall's step 2 also asserts that $dest holds no `.YAAR.app.new.*` and no
# `.yaar-review-old.*`; (6) the install line is run through `bash -s --`, which is `bash`
# with no arguments; (7) a command that stops the script without a sentence of its own is
# named in one, and the exit code is 1.
set -euo pipefail
# -E: the ERR trap below also runs inside functions and command substitutions.
set -E

NAME=macos-install.sh

# One sentence on standard error, and exit 1. The marker file tells the trap that a sentence
# was said, also when fail ran in a subshell and could set no variable of the step's.
scratch=""
fail() {
    echo "$NAME: $*" >&2
    if [ -n "$scratch" ]; then
        : > "$scratch/said"
    fi
    exit 1
}

usage() {
    echo "$NAME: $*" >&2
    echo "usage: .github/scripts/macos-install.sh install | uninstall" >&2
    exit 2
}

# What the step was doing when a command that nobody guarded failed: set -e ends the step with
# that command's own status and words, and the exit trap puts one sentence of the step's own
# in front of them and exits 1, the code the header names.
last_command=""
trap 'last_command=$BASH_COMMAND' ERR

cleanup() {
    local status=$?
    trap - EXIT ERR
    set +e
    if [ "$status" -ne 0 ] && [ "$status" -ne 2 ] && [ -n "$scratch" ] && [ ! -e "$scratch/said" ]; then
        echo "$NAME: ${last_command:-a command} failed with status $status" >&2
        status=1
    fi
    if [ -n "$scratch" ]; then
        rm -rf "$scratch"
    fi
    exit "$status"
}
trap cleanup EXIT

[ "$#" -eq 1 ] || usage "one argument is needed, install or uninstall"
case "$1" in
    install | uninstall) action=$1 ;;
    *) usage "the argument is install or uninstall, not: $1" ;;
esac

[ "$(uname -s)" = Darwin ] || fail "it runs on macOS, and this is $(uname -s)"
for tool in curl bash codesign xattr stat find sort cmp ps awk grep; do
    command -v "$tool" > /dev/null || fail "$tool is not there"
done
[ -x /usr/libexec/PlistBuddy ] || fail "/usr/libexec/PlistBuddy is not there"
[ -n "${VERSION:-}" ] || fail "VERSION is not set: it is the version the app must have (the job's env)"
[ -d "${HOME:-}" ] || fail "HOME is not a folder"

port=${YAAR_TEST_PORT:-8000}
case "$port" in
    '' | *[!0-9]*) fail "YAAR_TEST_PORT is a port number, not: $port" ;;
esac
base="http://127.0.0.1:$port"
download="$base/download"

# Where the app goes. The install script puts it in YAAR_INSTALL_DIR when that is set and in
# /Applications otherwise; every path below means that place.
dest=${YAAR_INSTALL_DIR:-/Applications}
dest=${dest%/}
app="$dest/YAAR.app"
window="$app/Contents/MacOS/yaar-review"
keeper="$app/Contents/Helpers/yaar-keeper"
lock="$HOME/Library/Application Support/yaar-review/keeper.sock.lock"

scratch=$(mktemp -d) || fail "no scratch folder could be made"

# The install line, `curl -fsSL <base>/install.sh | bash -s -- <arguments>`, with the two
# addresses of the served site and the test variable, as the header says. Its whole output is
# written to the log named by $1 and shown; the exit code is left in $line_status.
# $1: the log, $2: the download address, then the install script's arguments.
line_status=0
install_line() {
    local log=$1 address=$2
    shift 2
    if curl -fsSL "$base/install.sh" \
        | env YAAR_BASE_URL="$base" YAAR_DOWNLOAD_URL="$address" YAAR_ALLOW_UNNOTARIZED=1 \
            bash -s -- "$@" > "$log" 2>&1; then
        line_status=0
    else
        line_status=$?
    fi
    sed 's/^/    | /' "$log"
}

# Every file of the bundle with its kind, size, modification time, inode and mode, sorted.
# The inode is there because a bundle that was taken apart and put back whole, with the times
# of its files kept by ditto, would show the same sizes and the same times.
listing_of() {
    find -P "$1" -exec stat -f '%N %HT %z %m %i %Lp' {} + | LC_ALL=C sort
}

# What stands in $dest of an interrupted install, one per line.
leftovers() {
    find "$dest" -maxdepth 1 \( -name '.YAAR.app.new.*' -o -name '.yaar-review-old.*' \)
}

# The processes whose program is the file $1, as "<pid>" lines. macOS's ps gives the full
# path of the program in comm, so a Keeper of a build folder, another agent's, is not matched.
processes_of() {
    ps -axo pid=,comm= | awk -v want="$1" '
        {
            program = $2
            for (i = 3; i <= NF; i++) program = program " " $i
            if (program == want) print $1
        }'
}

if [ "$action" = install ]; then
    # --- 1. The door stays shut away from a loopback address.
    [ ! -e "$app" ] || fail "$app is there before the install line ran: a step before this one left it"
    echo "install 1: the test variable with a download from a remote address"
    install_line "$scratch/door.log" https://example.invalid/download
    [ "$line_status" -eq 1 ] || fail "the install line with YAAR_ALLOW_UNNOTARIZED=1 and a download from https://example.invalid exited $line_status and not 1"
    grep -q '127\.0\.0\.1 or localhost' "$scratch/door.log" \
        || fail "the install line with a download from https://example.invalid does not say that YAAR_ALLOW_UNNOTARIZED is taken only from 127.0.0.1 or localhost: the door is open"
    grep -q 'Nothing was downloaded' "$scratch/door.log" \
        || fail "the install line with a download from https://example.invalid does not say that it downloaded nothing"
    if grep -q 'downloading' "$scratch/door.log"; then
        fail "the install line with a download from https://example.invalid began a download"
    fi
    [ ! -e "$app" ] || fail "the install line with a download from https://example.invalid left $app"
    echo "install 1: refused, exit 1, nothing downloaded, no $app"

    # --- 2. The install line, from the served site.
    echo "install 2: the install line from $base/install.sh"
    install_line "$scratch/install.log" "$download"
    [ "$line_status" -eq 0 ] || fail "the install line exited $line_status and not 0"
    grep -q 'route: macOS' "$scratch/install.log" || fail "the install line's output does not say the route is macOS"
    grep -q 'SHA-256 matches SHA256SUMS' "$scratch/install.log" || fail "the install line's output does not say that the SHA-256 matched SHA256SUMS"
    echo "install 2: exit 0, route macOS, the SHA-256 matched SHA256SUMS"

    # --- 3. The app is there, whole, and not quarantined.
    [ -d "$app" ] || fail "$app is not there after the install line"
    codesign --verify --deep --strict "$app" || fail "codesign --verify --deep --strict does not pass on $app"
    found=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist") \
        || fail "$app/Contents/Info.plist has no CFBundleShortVersionString"
    [ "$found" = "$VERSION" ] || fail "$app is version $found and the release is $VERSION"
    xattr -rl "$app" > "$scratch/xattr" 2> "$scratch/xattr.err" || fail "xattr could not list the attributes under $app: $(cat "$scratch/xattr.err")"
    if grep -q 'com\.apple\.quarantine' "$scratch/xattr"; then
        fail "a file of $app carries com.apple.quarantine, which the install line adds to nothing: $(grep -B1 'com\.apple\.quarantine' "$scratch/xattr" | head -n 3 | tr '\n' ' ')"
    fi
    left=$(leftovers) || fail "the folder $dest could not be listed"
    [ -z "$left" ] || fail "$dest holds what an interrupted install leaves: $(printf '%s' "$left" | tr '\n' ' ')"
    echo "install 3: $app is version $found, its signature verifies, no file is quarantined, $dest holds no leftover"

    # --- 4. A second run changes nothing.
    listing_of "$app" > "$scratch/before"
    [ -s "$scratch/before" ] || fail "the listing of $app is empty"
    echo "install 4: the install line a second time"
    install_line "$scratch/again.log" "$download"
    [ "$line_status" -eq 0 ] || fail "the install line run a second time exited $line_status and not 0"
    grep -qi 'already installed' "$scratch/again.log" || fail "the install line run a second time does not say that the version is already installed"
    listing_of "$app" > "$scratch/after"
    if ! cmp -s "$scratch/before" "$scratch/after"; then
        fail "the second run changed the files of $app (a listing with sizes, modification times and inodes, before and after): $(diff "$scratch/before" "$scratch/after" | head -n 6 | tr '\n' ' ')"
    fi
    left=$(leftovers) || fail "the folder $dest could not be listed"
    [ -z "$left" ] || fail "the second run left $(printf '%s' "$left" | tr '\n' ' ') in $dest"
    echo "install 4: exit 0, already installed, $(wc -l < "$scratch/before" | tr -d ' ') entries of the bundle as they were"
else
    # --- 1. The uninstall, from the served site.
    echo "uninstall 1: the install line with --uninstall from $base/install.sh"
    install_line "$scratch/uninstall.log" "$download" --uninstall
    [ "$line_status" -eq 0 ] || fail "the install line with --uninstall exited $line_status and not 0"
    echo "uninstall 1: exit 0"

    # --- 2. The app and the Keeper's lock are gone, and nothing of the app runs.
    if [ -e "$app" ] || [ -L "$app" ]; then
        fail "$app is still there after the uninstall"
    fi
    running=""
    for program in "$window" "$keeper"; do
        pids=$(processes_of "$program") || fail "the processes could not be listed"
        if [ -n "$pids" ]; then
            running="$running $program (pid $(printf '%s' "$pids" | tr '\n' ' '))"
        fi
    done
    [ -z "$running" ] || fail "the app still runs after the uninstall:$running"
    if [ -e "$lock" ] || [ -L "$lock" ]; then
        fail "the Keeper's lock file $lock is still there after the uninstall"
    fi
    left=$(leftovers) || fail "the folder $dest could not be listed"
    [ -z "$left" ] || fail "the uninstall left $(printf '%s' "$left" | tr '\n' ' ') in $dest"
    echo "uninstall 2: $app is gone, no process runs a program of it, $lock is gone"
fi

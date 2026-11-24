#!/bin/bash
# The install line installs the app on macOS and the uninstall removes it, from the site on 8000.
# .github/scripts/macos-install.sh install | uninstall   (VERSION must be set)
# Not notarised: YAAR_ALLOW_UNNOTARIZED=1. YAAR_INSTALL_DIR overrides the default.
set -euo pipefail
set -E

NAME=macos-install.sh

scratch=""
fail() {
    echo "$NAME: $*" >&2
    if [ -n "$scratch" ]; then
        : > "$scratch/said"
    fi
    exit 1
}

# Only usage leaves with status 2, so a grep, cmp or diff that fails with 2 is still named.
usage_said=no
usage() {
    usage_said=yes
    echo "$NAME: $*" >&2
    echo "usage: .github/scripts/macos-install.sh install | uninstall" >&2
    exit 2
}

last_command=""
trap 'last_command=$BASH_COMMAND' ERR

cleanup() {
    local status=$?
    trap - EXIT ERR
    set +e
    if [ "$status" -ne 0 ] && [ "$usage_said" = no ] && [ -n "$scratch" ] && [ ! -e "$scratch/said" ]; then
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

port=8000
base="http://127.0.0.1:$port"
download="$base/download"

dest=${YAAR_INSTALL_DIR:-/Applications}
dest=${dest%/}
app="$dest/YAAR.app"
window="$app/Contents/MacOS/yaar-review"
keeper="$app/Contents/Helpers/yaar-keeper"
lock="$HOME/Library/Application Support/yaar-review/keeper.sock.lock"

scratch=$(mktemp -d) || fail "no scratch folder could be made"

# The install line, `curl -fsSL <base>/install.sh | bash -s -- <arguments>`: $1 the log, $2 the
# download address. $line_path goes first on the install script's search path; $line_proxy, when
# set, is every proxy of that run with no_proxy unset, so any fetch goes to it.
line_status=0
line_path=""
line_proxy=""
install_line() {
    local log=$1 address=$2
    local proxies=()
    shift 2
    if [ -n "$line_proxy" ]; then
        proxies=(-u no_proxy -u NO_PROXY "http_proxy=$line_proxy" "https_proxy=$line_proxy" "HTTPS_PROXY=$line_proxy" "ALL_PROXY=$line_proxy" "all_proxy=$line_proxy")
    fi
    # ${proxies[@]+...}: bash 3.2, macOS's, takes an empty array for an unset variable.
    if curl -fsSL "$base/install.sh" \
        | env ${proxies[@]+"${proxies[@]}"} PATH="${line_path:+$line_path:}$PATH" YAAR_BASE_URL="$base" YAAR_DOWNLOAD_URL="$address" YAAR_ALLOW_UNNOTARIZED=1 \
            bash -s -- "$@" > "$log" 2>&1; then
        line_status=0
    else
        line_status=$?
    fi
    sed 's/^/    | /' "$log"
}

# The inode is listed because a bundle taken apart and put back with times kept by ditto shows
# the same sizes and times.
listing_of() {
    find -P "$1" -exec stat -f '%N %HT %z %m %i %Lp' {} + | LC_ALL=C sort
}

leftovers() {
    find "$dest" -maxdepth 1 \( -name '.YAAR.app.new.*' -o -name '.yaar-review-old.*' \)
}

# macOS's ps gives the full program path in comm, so another build folder's Keeper is not matched.
processes_of() {
    ps -axo pid=,comm= | awk -v want="$1" '
        {
            program = $2
            for (i = 3; i <= NF; i++) program = program " " $i
            if (program == want) print $1
        }'
}

if [ "$action" = install ]; then
    [ ! -e "$app" ] || fail "$app is there before the install line ran: a step before this one left it"
    echo "install 1: the test variable with a download from a remote address"
    # A curl and a wget that record each call and fail like a machine with no network, first on the
    # door's search path.
    mkdir "$scratch/door-bin" || fail "the folder $scratch/door-bin could not be made"
    for fetcher in curl wget; do
        printf '%s\n' '#!/bin/sh' "echo \"$fetcher \$*\" >> \"$scratch/door-fetches\"" 'exit 6' > "$scratch/door-bin/$fetcher" \
            || fail "the stand-in $fetcher could not be written in $scratch/door-bin"
        chmod 755 "$scratch/door-bin/$fetcher" || fail "the stand-in $fetcher could not be made executable"
    done
    # A fetch through the proxy leaves a CONNECT, or a request line with a whole address, in the
    # site's log (the step's own `GET /install.sh` is neither); the count before the run is the
    # baseline.
    served_log="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/served-$port/log"
    [ -f "$served_log" ] || fail "$served_log, the log of the site served on port $port, is not there: the step before this one did not serve it with serve-site.sh"
    proxied_before=$(awk '/"CONNECT |"[A-Z]+ [A-Za-z][A-Za-z0-9+.-]*:\/\// { n++ } END { print n + 0 }' "$served_log") || fail "$served_log could not be read"
    line_path="$scratch/door-bin"
    line_proxy="$base"
    install_line "$scratch/door.log" https://example.invalid/download
    line_path=""
    line_proxy=""
    proxied_after=$(awk '/"CONNECT |"[A-Z]+ [A-Za-z][A-Za-z0-9+.-]*:\/\// { n++ } END { print n + 0 }' "$served_log") || fail "$served_log could not be read"
    if [ "$proxied_after" -ne "$proxied_before" ]; then
        fail "the install line with a download from https://example.invalid tried to reach an address through a proxy before it refused: the log of the served site gained $(grep -E '"CONNECT |"[A-Z]+ [A-Za-z][A-Za-z0-9+.-]*://' "$served_log" | tail -n 2 | tr '\n' ';')"
    fi
    if [ -e "$scratch/door-fetches" ]; then
        fail "the install line with a download from https://example.invalid tried to download before it refused: $(head -n 2 "$scratch/door-fetches" | tr '\n' ';')"
    fi
    [ "$line_status" -eq 1 ] || fail "the install line with YAAR_ALLOW_UNNOTARIZED=1 and a download from https://example.invalid exited $line_status and not 1"
    grep -q '127\.0\.0\.1 or localhost' "$scratch/door.log" \
        || fail "the install line with a download from https://example.invalid does not say that YAAR_ALLOW_UNNOTARIZED is taken only from 127.0.0.1 or localhost: the door is open"
    grep -q 'Nothing was downloaded' "$scratch/door.log" \
        || fail "the install line with a download from https://example.invalid does not say that it downloaded nothing"
    if grep -q 'downloading' "$scratch/door.log"; then
        fail "the install line with a download from https://example.invalid began a download"
    fi
    # A door that said its sentence and then fetched SHA256SUMS (announced with no "downloading"
    # line) would end on that fetch's error, not on the refusal.
    last=$(grep -v '^[[:space:]]*$' "$scratch/door.log" | tail -n 1) || fail "the output of the install line with a download from https://example.invalid could not be read"
    case "$last" in
        *'Nothing was downloaded'*) ;;
        *) fail "the install line with a download from https://example.invalid does not end on its refusal, so it went on after it: the last line is: $last" ;;
    esac
    named=$(awk '/example\.invalid/ { n++ } END { print n + 0 }' "$scratch/door.log")
    [ "$named" -eq 1 ] || fail "the install line with a download from https://example.invalid names the address on $named lines and not on the one of its refusal: it tried to reach it"
    if grep -qE 'curl:|could not download' "$scratch/door.log"; then
        fail "the install line with a download from https://example.invalid ran curl against it: $(grep -E 'curl:|could not download' "$scratch/door.log" | head -n 2 | tr '\n' ' ')"
    fi
    [ ! -e "$app" ] || fail "the install line with a download from https://example.invalid left $app"
    echo "install 1: refused, exit 1, nothing downloaded, no $app"

    echo "install 2: the install line from $base/install.sh"
    install_line "$scratch/install.log" "$download"
    [ "$line_status" -eq 0 ] || fail "the install line exited $line_status and not 0"
    grep -q 'route: macOS' "$scratch/install.log" || fail "the install line's output does not say the route is macOS"
    grep -q 'SHA-256 matches SHA256SUMS' "$scratch/install.log" || fail "the install line's output does not say that the SHA-256 matched SHA256SUMS"
    echo "install 2: exit 0, route macOS, the SHA-256 matched SHA256SUMS"

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
    echo "uninstall 1: the install line with --uninstall from $base/install.sh"
    install_line "$scratch/uninstall.log" "$download" --uninstall
    [ "$line_status" -eq 0 ] || fail "the install line with --uninstall exited $line_status and not 0"
    echo "uninstall 1: exit 0"

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

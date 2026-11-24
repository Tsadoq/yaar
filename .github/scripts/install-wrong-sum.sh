#!/bin/bash
# Row PKG-10, its clause "and the install line checks what it downloaded against it": the
# install line, run against a site whose SHA256SUMS holds one wrong sum, refuses with its
# sentence and installs nothing.
#
#   .github/scripts/install-wrong-sum.sh <rehearsal-dir> [<user>]
#
# <rehearsal-dir> is the artifact `rehearsal`: site/ (install.sh, apt/) and release/ (the
# packages and SHA256SUMS). Linux: run as root, <user> is required, and the install line runs
# as <user> on the archive route (YAAR_METHOD=archive, which downloads yaar-review-<version>-
# linux-<x86_64|aarch64>.tar.gz of this processor and needs no root). macOS: no <user>, run
# by GitHub's Mac only (GITHUB_ACTIONS must be "true": the install line is run for real), and
# the install line runs with YAAR_ALLOW_UNNOTARIZED=1, as the install step does for the
# rehearsal's app, which gets it to the download; the package is yaar-review-<version>-
# macos-universal.zip.
#   1. Makes a scratch folder with a copy of release/ made of links to every file but
#      SHA256SUMS and SHA256SUMS.asc, and a SHA256SUMS that is release/'s with the first
#      digit of the sum of the package above changed (a 0 becomes a 1, any other digit a 0);
#      nothing else of it differs. The package itself is the real file.
#   2. Serves site/ and that copy with serve-site.sh on 127.0.0.1:8001 (YAAR_WRONG_SUM_PORT
#      names another), so that the sums of the job's own site on 8000 stay as they were.
#   3. Runs `curl -fsSL http://127.0.0.1:8001/install.sh | bash` with YAAR_BASE_URL and
#      YAAR_DOWNLOAD_URL=http://127.0.0.1:8001/download, and a folder of its own to install
#      into (Linux: YAAR_PREFIX, a folder made as <user> in <user>'s home; macOS:
#      YAAR_INSTALL_DIR, under $RUNNER_TEMP), and keeps its output.
#   4. Asserts: the install line exited 1; its output holds the sentence
#      "SHA-256 of <package> is <the sum of the real file>, SHA256SUMS says <the wrong sum>.
#      Nothing was installed." and does not hold "SHA-256 matches"; the folder it was to
#      install into is empty; on Linux no yaar-review is under <user>'s ~/.local either.
#   5. Stops the server by its pid and removes the scratch folder and the folder made in 3,
#      whatever happens.
# The refusal is shown to rest on the sums, and not on anything else the install line could
# say: the sentence names both the sum of the file as the script computed it, which must be
# the sum release/'s SHA256SUMS holds, and the sum served.
#
# Exit codes: 0 holds; 1 does not; 2 usage error.
#
# What the header leaves to the body: (1) the package is picked as install.sh picks it: of the
# lines of SHA256SUMS whose file name ends with the suffix of this system, the last by name;
# (2) a command that stops the script without a sentence of its own is named in one, and the
# exit code is 1; (3) the folder the install line installs into is made before it runs, so
# that "nothing was installed" is a folder that stayed empty and not one that was never made.
set -euo pipefail
# -E: the ERR trap below also runs inside functions and command substitutions.
set -E

NAME=install-wrong-sum.sh
HERE=$(cd "$(dirname "$0")" && pwd)

# Whether a sentence of this script's own was said (fail sets it), and whether the usage error,
# the one place that may leave with status 2, was said.
said=no
usage_said=no
fail() {
    said=yes
    echo "$NAME: $*" >&2
    exit 1
}
usage() {
    usage_said=yes
    said=yes
    echo "$NAME: $*" >&2
    echo "usage: .github/scripts/install-wrong-sum.sh <rehearsal-dir> [<user>]" >&2
    exit 2
}

# What the script was doing when a command that nobody guarded failed: set -e ends it with that
# command's own status and words, and the exit trap puts one sentence in front of them and exits
# 1, the code the header names.
last_command=""
trap 'last_command=$BASH_COMMAND' ERR

scratch=""
into=""
port=${YAAR_WRONG_SUM_PORT:-8001}
served=""
cleanup() {
    local status=$?
    trap - EXIT ERR
    set +e
    if [ "$status" -ne 0 ] && [ "$usage_said" = no ] && [ "$said" = no ]; then
        echo "$NAME: ${last_command:-a command} failed with status $status" >&2
        status=1
    fi
    if [ -n "$served" ]; then
        if ! "$HERE/serve-site.sh" stop "$port" > /dev/null 2>&1; then
            echo "$NAME: the server of port $port could not be stopped" >&2
            if [ "$status" -eq 0 ]; then status=1; fi
        fi
        rm -rf "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/served-$port"
    fi
    if [ -n "$into" ]; then rm -rf "$into"; fi
    if [ -n "$scratch" ]; then rm -rf "$scratch"; fi
    exit "$status"
}
trap cleanup EXIT

case "$#" in
    1 | 2) ;;
    *) usage "a folder of the rehearsal is needed, and on Linux a user" ;;
esac
rehearsal=$1
user=${2:-}
[ -d "$rehearsal/site" ] || usage "$rehearsal/site is not a folder"
[ -d "$rehearsal/release" ] || usage "$rehearsal/release is not a folder"
[ -f "$rehearsal/release/SHA256SUMS" ] || usage "$rehearsal/release/SHA256SUMS is not a file"
case "$port" in
    '' | *[!0-9]*) usage "YAAR_WRONG_SUM_PORT is a port number, not: $port" ;;
esac
rehearsal=$(cd -P "$rehearsal" && pwd) || fail "$rehearsal could not be entered"

system=$(uname -s)
case "$system" in
    Linux)
        [ -n "$user" ] || usage "on Linux the user the install line runs as is needed"
        [ "$(id -u)" -eq 0 ] || usage "on Linux it runs as root, and this is user $(id -un)"
        for tool in sudo getent curl python3; do
            command -v "$tool" > /dev/null || fail "$tool is not installed"
        done
        machine=$(uname -m)
        case "$machine" in
            x86_64 | amd64) machine=x86_64 ;;
            aarch64 | arm64) machine=aarch64 ;;
            *) fail "the processor $machine is not one the packages are built for" ;;
        esac
        suffix="-linux-$machine.tar.gz"
        entry=$(getent passwd "$user") || fail "there is no user $user"
        home=$(printf '%s\n' "$entry" | cut -d: -f6)
        [ -d "$home" ] || fail "the home of $user, $home, is not a folder"
        ;;
    Darwin)
        [ -z "$user" ] || usage "on macOS no user is given: the install line runs as the runner's own user"
        [ "${GITHUB_ACTIONS:-}" = true ] || usage "on macOS it runs on GitHub's Mac only: it runs the install line, and GITHUB_ACTIONS is not true"
        for tool in curl python3; do
            command -v "$tool" > /dev/null || fail "$tool is not installed"
        done
        suffix="-macos-universal.zip"
        home=""
        ;;
    *) usage "it runs on Linux and macOS, and this is $system" ;;
esac

scratch=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/wrong-sum.XXXXXX") || fail "no scratch folder could be made"

# --- 1. The package the install line downloads, and a SHA256SUMS with a wrong sum for it.
package=$(awk -v s="$suffix" '{ f = $2; sub(/^\*/, "", f); n = length(f) - length(s) + 1;
    if (n > 0 && substr(f, n) == s && f ~ /^yaar-review-[0-9A-Za-z.+~-]+$/) print f }' "$rehearsal/release/SHA256SUMS" | sort | tail -n 1) \
    || fail "$rehearsal/release/SHA256SUMS could not be read"
[ -n "$package" ] || fail "$rehearsal/release/SHA256SUMS names no file ending in $suffix"
[ -f "$rehearsal/release/$package" ] || fail "$rehearsal/release/$package, named in SHA256SUMS, is not there"
real=$(awk -v n="$package" '{ f = $2; sub(/^\*/, "", f); if (f == n) print $1 }' "$rehearsal/release/SHA256SUMS" | head -n 1) \
    || fail "$rehearsal/release/SHA256SUMS could not be read"
[ -n "$real" ] || fail "$rehearsal/release/SHA256SUMS holds no sum for $package"
case "$real" in
    0*) wrong="1${real#0}" ;;
    *) wrong="0${real#?}" ;;
esac
mkdir "$scratch/release" || fail "the folder $scratch/release could not be made"
for f in "$rehearsal/release"/*; do
    case "${f##*/}" in
        SHA256SUMS | SHA256SUMS.asc) ;;
        *) ln -s "$f" "$scratch/release/${f##*/}" || fail "$f could not be linked into $scratch/release" ;;
    esac
done
awk -v n="$package" -v w="$wrong" '{ f = $2; sub(/^\*/, "", f); if (f == n) $0 = w substr($0, length($1) + 1); print }' \
    "$rehearsal/release/SHA256SUMS" > "$scratch/release/SHA256SUMS" || fail "the SHA256SUMS with a wrong sum could not be written"
# One line out and one line in: comm -3 prints the lines that only one of the two files holds.
changed=$(comm -3 <(LC_ALL=C sort "$rehearsal/release/SHA256SUMS") <(LC_ALL=C sort "$scratch/release/SHA256SUMS") | wc -l | tr -d ' ') \
    || fail "the two SHA256SUMS could not be compared"
[ "$changed" -eq 2 ] || fail "the SHA256SUMS made for this check differs from the real one on $((changed / 2)) lines and not one"
echo "$NAME: $package, whose sum is $real, is listed with $wrong"

# --- 2. The site served, with that SHA256SUMS.
"$HERE/serve-site.sh" start "$rehearsal/site" "$scratch/release" "$port" || fail "serve-site.sh could not serve the site with the wrong sum on port $port"
served=yes
base="http://127.0.0.1:$port"

# --- 3. The install line.
if [ "$system" = Linux ]; then
    into=$(sudo -u "$user" mktemp -d "$home/.yaar-wrong-sum.XXXXXX") || fail "no folder to install into could be made in $home as $user"
else
    into=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/wrong-sum-apps.XXXXXX") || fail "no folder to install into could be made"
fi
status=0
# The log is the root shell's file on purpose, which sudo does not need to touch; the shell that
# runs the install line expands its variables, not this one.
if [ "$system" = Linux ]; then
    # shellcheck disable=SC2016,SC2024
    sudo -u "$user" env YAAR_BASE_URL="$base" YAAR_DOWNLOAD_URL="$base/download" \
        YAAR_METHOD=archive YAAR_PREFIX="$into" \
        bash -c 'curl -fsSL "$YAAR_BASE_URL/install.sh" | bash' > "$scratch/install.log" 2>&1 || status=$?
else
    # shellcheck disable=SC2016
    env YAAR_BASE_URL="$base" YAAR_DOWNLOAD_URL="$base/download" \
        YAAR_ALLOW_UNNOTARIZED=1 YAAR_INSTALL_DIR="$into" \
        bash -c 'curl -fsSL "$YAAR_BASE_URL/install.sh" | bash' > "$scratch/install.log" 2>&1 || status=$?
fi
sed 's/^/    | /' "$scratch/install.log"

# --- 4. It refused, with its sentence, and installed nothing.
[ "$status" -eq 1 ] || fail "the install line given a wrong sum for $package exited $status and not 1"
sentence="SHA-256 of $package is $real, SHA256SUMS says $wrong. Nothing was installed."
grep -qF -- "$sentence" "$scratch/install.log" \
    || fail "the install line given a wrong sum for $package does not say \"$sentence\": it did not compare the file with the sum it was served"
if grep -qF 'SHA-256 matches' "$scratch/install.log"; then
    fail "the install line given a wrong sum for $package says the SHA-256 matches"
fi
left=$(ls -A "$into") || fail "$into could not be listed"
[ -z "$left" ] || fail "the install line given a wrong sum for $package left $left in $into"
if [ "$system" = Linux ]; then
    for p in "$home/.local/bin/yaar-review" "$home/.local/libexec/yaar-review"; do
        if [ -e "$p" ] || [ -L "$p" ]; then
            fail "the install line given a wrong sum for $package left $p"
        fi
    done
fi
echo "$NAME: the install line refused $package with the wrong sum $wrong, said so, and installed nothing"

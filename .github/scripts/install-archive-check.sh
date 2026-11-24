#!/bin/bash
# The archive route of install.sh for both processors: its own archive is found in SHA256SUMS,
# installed on the right sum, and a wrong sum is refused. Linux only (exit 2 elsewhere).
# .github/scripts/install-archive-check.sh
set -euo pipefail
set -E

NAME="install-archive-check.sh"
HERE=$(cd "$(dirname "$0")" && pwd)

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
    echo "usage: .github/scripts/install-archive-check.sh" >&2
    exit 2
}

last_command=""
trap 'last_command=$BASH_COMMAND' ERR

scratch=""
cleanup() {
    local status=$?
    trap - EXIT ERR
    set +e
    if [ "$status" -ne 0 ] && [ "$usage_said" = no ] && [ "$said" = no ]; then
        echo "$NAME: ${last_command:-a command} failed with status $status" >&2
        status=1
    fi
    if [ -n "$scratch" ]; then rm -rf "$scratch"; fi
    exit "$status"
}
trap cleanup EXIT

[ "$#" -eq 0 ] || usage "no argument is taken"
install=$HERE/../../packaging/install.sh
[ -f "$install" ] || usage "$install is not a file"
[ "$(uname -s)" = Linux ] || usage "it runs on Linux only: on $(uname -s) install.sh does not take the archive route"
for tool in curl tar gzip sha256sum awk; do
    command -v "$tool" > /dev/null || fail "$tool is not installed"
done

scratch=$(mktemp -d "${TMPDIR:-/tmp}/install-archive-check.XXXXXX") || fail "no scratch folder could be made"
mkdir "$scratch/home" "$scratch/tmp" "$scratch/release" || fail "the folders of the scratch could not be made"

version=0.1.0
for machine in x86_64 aarch64; do
    root="$scratch/build-$machine/yaar-review"
    mkdir -p "$root/bin" "$root/libexec/yaar-review" || fail "the layout of the $machine archive could not be made"
    # A real, dynamic program, which the install line's look at the libraries can read.
    cp "$(command -v sha256sum)" "$root/bin/yaar-review"
    printf 'window program for %s\n' "$machine" >> "$root/bin/yaar-review"
    cp "$(command -v sha256sum)" "$root/libexec/yaar-review/yaar-keeper"
    printf 'keeper for %s\n' "$machine" >> "$root/libexec/yaar-review/yaar-keeper"
    chmod 755 "$root/bin/yaar-review" "$root/libexec/yaar-review/yaar-keeper"
    tar -czf "$scratch/release/yaar-review-$version-linux-$machine.tar.gz" -C "$scratch/build-$machine" yaar-review \
        || fail "the $machine archive could not be made"
done
printf 'a zip\n' > "$scratch/release/yaar-review-$version-macos-universal.zip"
printf 'a deb\n' > "$scratch/release/yaar-review_${version}_amd64.deb"
printf 'a deb\n' > "$scratch/release/yaar-review_${version}_arm64.deb"
(cd "$scratch/release" && sha256sum ./* | sed 's|  \./|  |') > "$scratch/sums" || fail "SHA256SUMS could not be made"
[ "$(wc -l < "$scratch/sums" | tr -d ' ')" -eq 5 ] || fail "the SHA256SUMS made here does not list five files"

# Sets status, the log of the run ($scratch/log) and the prefix ($prefix).
run_install() {
    local machine=$1 folder=$2
    prefix="$scratch/prefix-$machine-$3"
    mkdir "$prefix" || fail "$prefix could not be made"
    status=0
    env -i PATH="$PATH" HOME="$scratch/home" TMPDIR="$scratch/tmp" \
        YAAR_BASE_URL=http://127.0.0.1:9 YAAR_DOWNLOAD_URL="file://$folder" \
        YAAR_METHOD=archive YAAR_ARCH="$machine" YAAR_PREFIX="$prefix" \
        bash "$install" > "$scratch/log" 2>&1 || status=$?
    sed 's/^/    | /' "$scratch/log"
}

for machine in x86_64 aarch64; do
    archive="yaar-review-$version-linux-$machine.tar.gz"
    real=$(awk -v n="$archive" '$2 == n { print $1 }' "$scratch/sums")
    [ -n "$real" ] || fail "the SHA256SUMS made here holds no sum for $archive"

    mkdir "$scratch/right-$machine" || fail "the folder of the right sums could not be made"
    for f in "$scratch/release"/*; do ln -s "$f" "$scratch/right-$machine/${f##*/}" || fail "$f could not be linked"; done
    cp "$scratch/sums" "$scratch/right-$machine/SHA256SUMS" || fail "SHA256SUMS could not be copied"
    run_install "$machine" "$scratch/right-$machine" right
    [ "$status" -eq 0 ] || fail "the install line for $machine, given the right sums, exited $status and not 0: it did not find or accept $archive"
    grep -qF -- "SHA-256 matches SHA256SUMS ($real)" "$scratch/log" \
        || fail "the install line for $machine, given the right sums, does not say \"SHA-256 matches SHA256SUMS ($real)\""
    [ -f "$prefix/bin/yaar-review" ] || fail "the install line for $machine left no $prefix/bin/yaar-review"
    grep -aqF -- "window program for $machine" "$prefix/bin/yaar-review" \
        || fail "the program the install line for $machine put at $prefix/bin/yaar-review is not the one of the $machine archive"

    case "$real" in
        0*) wrong="1${real#0}" ;;
        *) wrong="0${real#?}" ;;
    esac
    mkdir "$scratch/wrong-$machine" || fail "the folder of the wrong sums could not be made"
    for f in "$scratch/release"/*; do ln -s "$f" "$scratch/wrong-$machine/${f##*/}" || fail "$f could not be linked"; done
    awk -v n="$archive" -v w="$wrong" '$2 == n { $1 = w } { print $1 "  " $2 }' "$scratch/sums" > "$scratch/wrong-$machine/SHA256SUMS" \
        || fail "the SHA256SUMS with a wrong sum could not be written"
    [ "$(comm -3 <(LC_ALL=C sort "$scratch/sums") <(LC_ALL=C sort "$scratch/wrong-$machine/SHA256SUMS") | wc -l | tr -d ' ')" -eq 2 ] \
        || fail "the SHA256SUMS made for $machine differs from the right one on more than one line"
    run_install "$machine" "$scratch/wrong-$machine" wrong
    [ "$status" -eq 1 ] || fail "the install line for $machine, given a wrong sum for $archive, exited $status and not 1"
    sentence="SHA-256 of $archive is $real, SHA256SUMS says $wrong. Nothing was installed."
    grep -qF -- "$sentence" "$scratch/log" \
        || fail "the install line for $machine, given a wrong sum for $archive, does not say \"$sentence\""
    if grep -qF 'SHA-256 matches' "$scratch/log"; then
        fail "the install line for $machine, given a wrong sum for $archive, says the SHA-256 matches"
    fi
    left=$(ls -A "$prefix") || fail "$prefix could not be listed"
    [ -z "$left" ] || fail "the install line for $machine, given a wrong sum for $archive, left $left in $prefix"
    echo "$NAME: $machine: the right sum installs its program, a wrong sum is refused and nothing is installed"
done
echo "$NAME: the archive route finds, checks and refuses as it should on both processors"

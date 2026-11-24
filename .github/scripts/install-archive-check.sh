#!/bin/bash
# Row PKG-10, its clause "and the install line checks what it downloaded against it", on the
# route no machine of the rehearsal can show for both processors: the archive route of
# install.sh finds the archive of its own processor in SHA256SUMS, downloads it, checks it
# against the sum, and refuses a wrong sum with its sentence.
#
#   .github/scripts/install-archive-check.sh [<install.sh>]
#
# <install.sh> defaults to packaging/install.sh of this tree. Linux only: on another system
# the script says so and exits 2 (install.sh takes its macOS route there, which is not the
# one this checks). It needs bash, curl, tar, gzip, sha256sum and awk, writes only into a
# scratch folder it makes (HOME, TMPDIR and the prefix of the install are in it), and reads
# nothing of the user's.
#   1. Makes a folder of release files laid out as a release is: for each processor,
#      x86_64 and aarch64, an archive yaar-review-0.1.0-linux-<processor>.tar.gz holding the
#      layout the install line expects (the programs in each say which processor it is for), a
#      .zip and a .deb of the same version whose names the search must pass over, and a
#      SHA256SUMS that lists all five with their right sums.
#   2. For each processor, as YAAR_ARCH names it: runs install.sh with YAAR_METHOD=archive, a
#      download address that is that folder (file://), and YAAR_PREFIX in the scratch folder.
#      It must exit 0, say "SHA-256 matches", and leave under the prefix the program of that
#      processor and not the other's.
#   3. For each processor, again with a SHA256SUMS in which the sum of that archive has its
#      first digit changed (a 0 becomes a 1, any other digit a 0) and nothing else differs:
#      it must exit 1, say "SHA-256 of <archive> is <the real sum>, SHA256SUMS says <the
#      wrong sum>. Nothing was installed.", not say "SHA-256 matches", and leave the prefix
#      empty.
# The two cases of one processor make the refusal a refusal of the sum: the same run with
# the right sum installs, so a search that finds no archive, or an archive that cannot be
# read, fails the first case before it can pass the second.
#
# Exit codes: 0 holds; 1 does not; 2 usage error.
#
# What the header leaves to the body: a command that stops the script without a sentence of
# its own is named in one, and the exit code is 1; the archives are made here, small, so
# what is checked is the install line's choice and checks and not a real build.
set -euo pipefail
# -E: the ERR trap below also runs inside functions and command substitutions.
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
    echo "usage: .github/scripts/install-archive-check.sh [<install.sh>]" >&2
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

[ "$#" -le 1 ] || usage "at most one argument, the install script"
install=${1:-$HERE/../../packaging/install.sh}
[ -f "$install" ] || usage "$install is not a file"
[ "$(uname -s)" = Linux ] || usage "it runs on Linux only: on $(uname -s) install.sh does not take the archive route"
for tool in curl tar gzip sha256sum awk; do
    command -v "$tool" > /dev/null || fail "$tool is not installed"
done

scratch=$(mktemp -d "${TMPDIR:-/tmp}/install-archive-check.XXXXXX") || fail "no scratch folder could be made"
mkdir "$scratch/home" "$scratch/tmp" "$scratch/release" || fail "the folders of the scratch could not be made"

# --- 1. The release files and their SHA256SUMS.
version=0.1.0
for machine in x86_64 aarch64; do
    root="$scratch/build-$machine/yaar-review"
    mkdir -p "$root/bin" "$root/libexec/yaar-review" || fail "the layout of the $machine archive could not be made"
    # A real, dynamic program, which the install line's look at the libraries can read, with
    # the name of the processor it is for appended.
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

# One run of the install line: <machine> <folder holding SHA256SUMS and the files>; sets
# status and the log of the run in $scratch/log, and the prefix in $prefix.
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

    # --- 2. The right sum: it installs the program of this processor.
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

    # --- 3. A wrong sum: it refuses with its sentence and installs nothing.
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

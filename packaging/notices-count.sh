#!/bin/sh
# Row LIC-02: each package holds a notices file with one entry for every linked dependency of
# the lock file, and a dependency with no licence text to reproduce fails.
#
#   packaging/notices-count.sh <packages-dir>
#
# <packages-dir> holds some or all of the six release files as the jobs make them (names in
# packaging/release/assemble.sh): yaar-review_<V>_amd64.deb, yaar-review_<V>_arm64.deb,
# yaar-review-<V>-linux-x86_64.tar.gz, yaar-review-<V>-linux-aarch64.tar.gz,
# yaar-review-<V>-macos-universal.zip and .dmg. Each of the first five that is there is
# counted (the .dmg holds the same app as the .zip: macos-bundle-check.sh compares the two
# byte for byte); a folder with none of them fails. That every file is there is
# release-files-check.sh's to say. For each: take the notices file out of the package (.deb:
# usr/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt.gz, gunzipped; archive:
# yaar-review/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt; .zip:
# YAAR.app/Contents/Resources/THIRD-PARTY-NOTICES.txt), and its targets (amd64/x86_64:
# x86_64-unknown-linux-gnu; arm64/aarch64: aarch64-unknown-linux-gnu; macos-universal: both
# aarch64-apple-darwin and x86_64-apple-darwin, the union).
#
# For each of the three parts of the notices (packaging/notices.sh writes them: PART 1 the
# Window yaar-review; PART 2 the Keeper yaar-keeper and pi's reporting program
# yaar-pi-report; PART 3 Claude Code's hook program yaar-claude-hook and tool server
# yaar-claude-tools):
#
# THE WANTED SET is every crate that
#   cargo tree --locked --offline -e normal --target <triple> -p <package>... --prefix none --format '{p}'
# prints, for the part's packages (PART 1: yaar-window; PART 2: yaar-keeper and
# yaar-adapter-pi; PART 3: yaar-companion-claude), over each target of the package,
# deduplicated, each line read as follows:
#   - a trailing " (*)" (a repeat) and a trailing " (proc-macro)" are stripped, in either
#     order; a proc-macro crate is counted like any other;
#   - a trailing " (<path>)" that remains is stripped too: a crate from a path prints its
#     folder there, and that alone never decides whether it is counted;
#   - "name vX.Y.Z" becomes "name X.Y.Z" (the "v" removed).
# THE WORKSPACE'S OWN CRATES are left out, and only they: the names of
# `cargo metadata --locked --offline --no-deps --format-version 1` (`.packages[].name`, read
# with jq). gpui-pre-apple, the toolkit's Apple crate that Cargo.toml patches in from
# vendor/gpui-pre-apple-0.3.12, is not a member (the workspace excludes vendor/): it prints a
# path and is counted, as the third-party code it is.
#
# THE FOUND SET is every "name version" of the part's "Used by:" lines, minus the names of
# the workspace's own crates (cargo-about lists them as users of the licence they carry).
#
# It fails, naming the package, the part and each crate, when:
#   - a wanted crate is not found (a linked dependency with no entry);
#   - a found crate is not wanted (an entry for something not linked: the count is wrong);
#   - a licence block of the part that names a crate has no text after its "Used by:" lines
#     (a dependency with no licence text to reproduce);
#   - a package holds no notices file, or two packages of one system and processor hold
#     different ones.
# Prints one line per package: "<file>: <n> crates in <m> licences, each with its text".
#
# Needs cargo (the fetched crates of the lock file; nothing is compiled), jq, gzip, tar,
# unzip.
# Exit codes: 0 every package counts; 1 a package fails (sentences on standard error);
# 2 a usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "notices-count.sh: not built yet" >&2
exit 99

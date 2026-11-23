#!/bin/sh
# Row LIC-02: each package holds a notices file with one entry for every linked dependency of
# the lock file, and a dependency with no licence text to reproduce fails.
#
#   packaging/notices-count.sh <packages-dir>
#
# <packages-dir> holds the six release files as the jobs make them (names in
# packaging/release/assemble.sh): yaar-review_<V>_amd64.deb, yaar-review_<V>_arm64.deb,
# yaar-review-<V>-linux-x86_64.tar.gz, yaar-review-<V>-linux-aarch64.tar.gz,
# yaar-review-<V>-macos-universal.zip and .dmg. For each of the first five (the .dmg holds
# the same app as the .zip, whose bytes `What the Mac bundle holds` compared): take the
# notices file out of the package (.deb: usr/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt.gz,
# gunzipped; archive: yaar-review/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt; .zip:
# YAAR.app/Contents/Resources/THIRD-PARTY-NOTICES.txt), and its targets (amd64/x86_64:
# x86_64-unknown-linux-gnu; arm64/aarch64: aarch64-unknown-linux-gnu; macos-universal: both
# aarch64-apple-darwin and x86_64-apple-darwin, the union).
#
# For each of the three parts of the notices (packaging/notices.sh writes them: PART 1 the
# Window yaar-review; PART 2 the Keeper yaar-keeper and pi's reporting program
# yaar-pi-report; PART 3 Claude Code's hook program yaar-claude-hook and tool server
# yaar-claude-tools), the wanted set is every "name version" that
#   cargo tree --locked --offline -e normal --target <triple> -p <package>... --prefix none --format '{p}'
# prints, for the part's packages (PART 1: yaar-window; PART 2: yaar-keeper and
# yaar-adapter-pi; PART 3: yaar-companion-claude), over each target of the package, with the
# crates of the workspace left out (their line holds a path in parentheses), deduplicated.
# The found set is every "name version" of the part's "Used by:" lines. It fails, naming the
# package, the part and each crate, when:
#   - a wanted crate is not found (a linked dependency with no entry);
#   - a found crate is not wanted (an entry for something not linked: the count is wrong);
#   - a licence block of the part that names a crate has no text after its "Used by:" lines
#     (a dependency with no licence text to reproduce);
#   - a package holds no notices file, or two packages of one system and processor hold
#     different ones.
# Prints one line per package: "<file>: <n> crates in <m> licences, each with its text".
#
# Needs cargo (the fetched crates of the lock file; nothing is compiled), gzip, tar, unzip.
# Exit codes: 0 every package counts; 1 a package fails (sentences on standard error);
# 2 a usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "notices-count.sh: not built yet" >&2
exit 99

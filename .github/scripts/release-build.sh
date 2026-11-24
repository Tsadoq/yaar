#!/bin/sh
# Builds the programs the packages ship for release, the same way on every machine: the
# Window yaar-review, the Keeper yaar-keeper, Claude Code's hook program yaar-claude-hook and
# tool server yaar-claude-tools, and pi's reporting program yaar-pi-report (packages
# yaar-window, yaar-keeper, yaar-companion-claude, yaar-adapter-pi). Never standin-lsp, which
# is a program of the Navigator's tests and of no package.
#
#   release-build.sh [cargo arguments, such as --target aarch64-apple-darwin]
#
# Each part of the notices is built in a cargo invocation of its own (the Window; the Keeper
# with pi's reporting program; Claude Code's two programs). One invocation for all four
# would let Cargo merge the features that each package asks of a shared dependency, so a
# program would link crates that the notices of its part, which are made and counted per
# part, do not hold: the Window's tree turns on serde_json's preserve_order, and the Companion
# would then link indexmap, hashbrown, foldhash and equivalent. packaging/notices-count.sh
# reads the invocations below and fails when they are not these three, or when any other
# line of this file runs cargo (an env assignment, command, exec, a second command on a
# line: the count cannot read what such a line builds).
#
# Two builds of one commit give the same programs: the paths of the build machine are
# mapped away, and the one random seed in the dependency tree is fixed.
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CARGO_DIR=${CARGO_HOME:-$HOME/.cargo}
RUSTUP_DIR=${RUSTUP_HOME:-$HOME/.rustup}
RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$ROOT=/build"
RUSTFLAGS="$RUSTFLAGS --remap-path-prefix=$CARGO_DIR=/cargo --remap-path-prefix=$RUSTUP_DIR=/rustup"
export RUSTFLAGS
export CONST_RANDOM_SEED="${CONST_RANDOM_SEED:-1}"
if [ "$(uname -s)" = Darwin ]; then
  # The declared floor on macOS.
  export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-12.0}"
fi
cd "$ROOT"
cargo build --release --locked -p yaar-window "$@"
cargo build --release --locked -p yaar-keeper -p yaar-adapter-pi "$@"
cargo build --release --locked -p yaar-companion-claude "$@"

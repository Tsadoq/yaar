#!/bin/sh
# Builds the programs the packages ship for release, the same way on every machine: the
# Window yaar-review, the Keeper yaar-keeper, Claude Code's hook program yaar-claude-hook and
# tool server yaar-claude-tools, and pi's reporting program yaar-pi-report (packages
# yaar-window, yaar-keeper, yaar-companion-claude, yaar-adapter-pi). Never standin-lsp, which
# is a program of the Navigator's tests and of no package.
#
#   release-build.sh [cargo arguments, such as --target aarch64-apple-darwin]
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
exec cargo build --release --locked -p yaar-window -p yaar-keeper "$@"

#!/bin/sh
# Builds the shipped programs (never standin-lsp): release-build.sh [cargo args, e.g. --target X].
# One cargo run per notices part: a single run merges features, linking crates the notices lack.
# packaging/notices-count.sh reads these three lines and fails on any other cargo invocation.
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CARGO_DIR=${CARGO_HOME:-$HOME/.cargo}
RUSTUP_DIR=${RUSTUP_HOME:-$HOME/.rustup}
# Build paths are mapped away and the dependency tree's one random seed is fixed
# (CONST_RANDOM_SEED), so two builds of one commit give the same programs.
RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$ROOT=/build"
RUSTFLAGS="$RUSTFLAGS --remap-path-prefix=$CARGO_DIR=/cargo --remap-path-prefix=$RUSTUP_DIR=/rustup"
export RUSTFLAGS
export CONST_RANDOM_SEED="${CONST_RANDOM_SEED:-1}"
if [ "$(uname -s)" = Darwin ]; then
  export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-12.0}"
fi
cd "$ROOT"
cargo build --release --locked -p yaar-window "$@"
cargo build --release --locked -p yaar-keeper -p yaar-adapter-pi "$@"
cargo build --release --locked -p yaar-companion-claude "$@"

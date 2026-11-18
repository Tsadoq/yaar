#!/bin/sh
# Plants one dependency the build must refuse, and passes only if the check that guards
# against it refuses it, by name.
#
#   plant.sh licence        a crate under GPL-3.0-only         cargo deny check licenses
#   plant.sh ban <crate>    a crate with a banned name         cargo deny check bans
#   plant.sh git            a crate from a git repository      cargo deny check sources
#
# The last one stands for a crate of the Zed editor, which would arrive from Zed's git
# repository: the build takes crates from the registry alone. That run also holds
# deny.toml to it: no git source may be let in at all.
#
# Everything happens in a copy of the tree under a temporary folder: the plant never
# reaches the repository or its lock file. The plant goes under each program the packages
# ship and under the Companion, one at a time.
#
# "cargo deny failed" would prove nothing: it also fails for a stale lock, a broken
# manifest or a missing database. So each run is judged twice. Without the plant the check
# must pass, which also shows that the interface toolkit, which is in the tree, is let
# in. With the plant it must fail, and among its errors must be the diagnostic of that
# check for the planted crate.
#
# Needs cargo-deny, jq and, for the last one, git. Asks nothing of the network: the crates
# are those of the lock file, already fetched, and the git repository is made here.
set -eu

usage() {
  echo "usage: plant.sh licence | ban <crate> | git" >&2
  exit 2
}

KIND=${1:-}
case "$KIND" in
  licence) [ $# -eq 1 ] || usage; CHECK=licenses; CODE=rejected; CRATE=planted-gpl ;;
  ban)     [ $# -eq 2 ] || usage; CHECK=bans; CODE=banned; CRATE=$2 ;;
  git)     [ $# -eq 1 ] || usage; CHECK=sources; CODE=source-not-allowed; CRATE=planted-from-git ;;
  *)       usage ;;
esac

# The programs the packages ship, and the Companion.
MANIFESTS="crates/window/window/Cargo.toml crates/keeper/keeper/Cargo.toml companions/claude/Cargo.toml"

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
# The resolved path: the plant is named by it in a manifest and in an address.
TMP=$(cd "$TMP" && pwd -P)
TREE=$TMP/tree
LOG=$TMP/diagnostics.jsonl

fail() {
  echo "plant.sh: $*" >&2
  exit 1
}

# Runs the check in the copy, with the environment given as arguments added. Its
# diagnostics go to $LOG, one JSON object on a line. $OFFLINE is word-split on purpose.
deny() {
  # shellcheck disable=SC2086
  (cd "$TREE" && env "$@" cargo deny $OFFLINE --format json check "$CHECK") > /dev/null 2> "$LOG"
}

# The same run as a person reads it, for the log of a failed step.
show() {
  # shellcheck disable=SC2086
  (cd "$TREE" && env "$@" cargo deny $OFFLINE --color never check "$CHECK") >&2 || true
}

# Prints how many errors of the last run are the diagnostic $CODE for the crate $CRATE.
# A line that is no JSON counts for nothing.
refusals() {
  jq -R -s -r --arg code "$CODE" --arg crate "$CRATE" '
    [split("\n")[] | fromjson?
      | select(.type == "diagnostic" and .fields.severity == "error" and .fields.code == $code)
      | select(any(.fields.graphs[]?; .Krate.name == $crate))]
    | length' "$LOG"
}

# The copy, without what a build leaves. It belongs to whoever runs this: in a container
# that is root, and the checkout there was made by another user.
mkdir "$TREE" "$TMP/plant"
tar -C "$ROOT" --exclude=./target --exclude=./.git --exclude=./dist -cf "$TMP/tree.tar" .
tar -C "$TREE" --no-same-owner -xf "$TMP/tree.tar"
rm "$TMP/tree.tar"
for manifest in $MANIFESTS; do
  [ -f "$TREE/$manifest" ] || fail "$manifest is not in the tree"
done

# The planted crate: an empty library that carries the one thing the check looks at.
PLANT=$TMP/plant/$CRATE
mkdir -p "$PLANT/src"
: > "$PLANT/src/lib.rs"
case "$KIND" in
  licence) LICENCE=GPL-3.0-only ;;
  *)       LICENCE=Apache-2.0 ;;
esac
cat > "$PLANT/Cargo.toml" <<EOF
[package]
name = "$CRATE"
version = "0.1.0"
edition = "2021"
license = "$LICENCE"
EOF

# How the manifests name the plant, and what the planted run needs around it. A crate
# planted by its path needs no fetch, so cargo is kept offline, as in the control.
OFFLINE=--offline
PLANTED_OFFLINE=--offline
set --
case "$KIND" in
  licence | ban)
    WHERE="path = \"$PLANT\""
    ;;
  git)
    git_plain() {
      env GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 \
        GIT_AUTHOR_NAME=plant GIT_AUTHOR_EMAIL=plant@example.invalid \
        GIT_COMMITTER_NAME=plant GIT_COMMITTER_EMAIL=plant@example.invalid \
        git -C "$PLANT" "$@"
    }
    git_plain init --quiet -b main
    git_plain add .
    git_plain commit --quiet -m "A crate that arrives from a git repository"
    WHERE="git = \"file://$PLANT\""
    # Offline, cargo would refuse to read even this repository, and the check would fail
    # before it had judged anything. Reading it asks nothing of the network. Cargo keeps
    # what it reads from git under its home, so this run gets a home of its own, with the
    # fetched crates of the registry linked in, and leaves nothing behind in the real one.
    PLANTED_OFFLINE=""
    REAL_HOME=${CARGO_HOME:-$HOME/.cargo}
    mkdir "$TMP/cargo-home"
    for kept in registry config.toml config; do
      [ ! -e "$REAL_HOME/$kept" ] || ln -s "$REAL_HOME/$kept" "$TMP/cargo-home/$kept"
    done
    set -- "CARGO_HOME=$TMP/cargo-home"
    ;;
esac

# Control: the tree as it is passes the check.
if ! deny; then
  show
  fail "cargo deny check $CHECK fails before anything is planted"
fi
echo "control: cargo deny check $CHECK passes on the tree as it is"

# Kept under names cargo does not read: a Cargo.toml above the plant would be taken for
# the root of its workspace.
cp "$TREE/Cargo.lock" "$TMP/lock.kept"
OFFLINE=$PLANTED_OFFLINE
for manifest in $MANIFESTS; do
  cp "$TREE/$manifest" "$TMP/manifest.kept"
  printf '\n[dependencies.%s]\n%s\n' "$CRATE" "$WHERE" >> "$TREE/$manifest"

  if deny "$@"; then
    fail "$CRATE planted in $manifest, and cargo deny check $CHECK passes"
  fi
  if [ "$(refusals)" -lt 1 ]; then
    show "$@"
    fail "$CRATE planted in $manifest: the check fails, but not with '$CODE' for $CRATE"
  fi
  echo "$manifest: $CRATE planted, refused ($CODE)"

  cp "$TMP/manifest.kept" "$TREE/$manifest"
  cp "$TMP/lock.kept" "$TREE/Cargo.lock"
done

if [ "$KIND" = git ]; then
  # The plant shows that a git repository nobody allowed is refused, not that Zed's is.
  # So no git source may be allowed at all, by its address or as an organisation's.
  grep -v '^[[:space:]]*#' "$TREE/deny.toml" > "$TMP/settings"
  if [ "$(grep -c 'allow-git' "$TMP/settings")" != 1 ] || ! grep -q '^allow-git = \[\]$' "$TMP/settings"; then
    fail "deny.toml must say 'allow-git = []', once"
  fi
  if grep -q 'allow-org' "$TMP/settings"; then
    fail "deny.toml allows the repositories of an organisation"
  fi
  echo "deny.toml allows no git source"
fi

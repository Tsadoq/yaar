#!/bin/sh
# Plants one dependency the build must refuse, and passes only if the check that guards
# against it refuses it, by name.
#
#   plant.sh licence        a crate under GPL-3.0-only         cargo deny check licenses
#   plant.sh ban <crate>    a crate with a banned name         cargo deny check bans
#   plant.sh git            a crate from a git repository      cargo deny check sources
#
# The last one stands for a crate of the Zed editor that arrives from Zed's git
# repository: the build takes crates from the registry alone. That run also holds
# deny.toml to it: the registry is the one source it may name. A crate Zed publishes on
# the registry is a ban like any other, by its name.
#
# Everything happens in a copy of the tree under a temporary folder: the plant never
# reaches the repository or its lock file. The plant goes under each program the packages
# ship and under the Companion, one at a time.
#
# "cargo deny failed" would prove nothing: it also fails for a stale lock, a broken
# manifest or a missing database. So each run is judged twice. Without the plant the check
# must pass, with the interface toolkit among the crates it judged: that shows the
# toolkit is let in. With the plant it must fail, and among its errors must be the
# diagnostic of that check for the planted crate. A run whose diagnostics cannot be read
# fails too.
#
# Each part of the check is asked alone here, while a real dependency meets the whole
# check in a step of the workflow. So every run also holds the workflow to that step, and
# to a job that fails the run when the step fails: what is refused here fails the run
# only through them.
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
# The interface toolkit, by the crate the app names in its manifests.
TOOLKIT=gpui-kit
# The step of the run that meets a real dependency, and the whole of what it may be.
WORKFLOW=.github/workflows/ci.yml
RUN_STEP="Licences, advisories, bans, sources"
RUN_COMMAND="cargo deny check"
STEP_LINE="      - name: $RUN_STEP"

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

# A plant is refused here by one part of the check, asked alone. That says something of
# the run only while the run asks the whole check of the real tree, and fails when it
# fails. So the workflow must hold that step once; the step must be its command and
# nothing else, with no word that would let the run go on after it; and the job whose
# step it is must say nothing that could let the run pass when the job fails, or without
# the job.
#
# YAML reads no comment and no empty line, wherever one stands: a key behind a comment
# still belongs to what stands before the comment. So both are left out first.
awk '!/^[ \t]*#/ && !/^[ \t]*$/' "$TREE/$WORKFLOW" > "$TMP/workflow"
steps=$(grep -c -x -F -e "$STEP_LINE" "$TMP/workflow") || true
[ "$steps" = 1 ] || fail "$WORKFLOW must hold the step '$RUN_STEP' once"
echo "        run: $RUN_COMMAND" > "$TMP/step.wanted"
# The lines of the step: those after its name that are indented deeper than a step is.
awk -v name="$STEP_LINE" '
  $0 == name { inside = 1; next }
  inside && !/^        / { inside = 0 }
  inside { print }' "$TMP/workflow" > "$TMP/step.found"
if ! diff "$TMP/step.wanted" "$TMP/step.found" >&2; then
  fail "the step '$RUN_STEP' of $WORKFLOW must be 'run: $RUN_COMMAND' and nothing else"
fi
# The keys of the job whose step it is: the lines indented as the keys of a job are,
# between two lines indented less. They are printed only if the step stands under `jobs:`
# and straight under the `steps:` of that job. The same words elsewhere are text, such as
# a name written over several lines, and no step that runs.
awk -v name="$STEP_LINE" '
  function job_ends() { if (held) printf "%s", keys; held = 0; keys = ""; last = "" }
  !/^    / { job_ends(); if (!/^ /) top = $0; next }
  /^    [^ ]/ { keys = keys $0 "\n"; last = $0 }
  $0 == name && top == "jobs:" && last == "    steps:" { held = 1 }
  END { job_ends() }' "$TMP/workflow" > "$TMP/job.found"
[ -s "$TMP/job.found" ] \
  || fail "the step '$RUN_STEP' of $WORKFLOW must be one of the steps of a job"
# A job is let say what the job says today, none of which keeps a failed step from
# failing the run. Any other key is refused, not only `continue-on-error` and `if`: a
# list of the keys that do harm would miss the same key written in quotes.
while IFS= read -r line; do
  case "${line%%:*}" in
    "    name" | "    strategy" | "    runs-on" | "    container") ;;
    "    timeout-minutes" | "    steps") ;;
    *)
      fail "the job of the step '$RUN_STEP' in $WORKFLOW says '${line#    }':" \
        "only its name, strategy, runner, container, time limit and steps are known" \
        "to leave the run failing when the step fails"
      ;;
  esac
done < "$TMP/job.found"

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
# And the toolkit is in what was judged, or the control would say nothing of it. The
# list is cargo deny's own, one line with each crate as "name version source".
# shellcheck disable=SC2086
(cd "$TREE" && cargo deny $OFFLINE list --format json --layout crate) > "$TMP/judged.json" 2> /dev/null \
  || fail "cargo deny cannot list the crates it judges"
grep -q -F -e "\"$TOOLKIT " "$TMP/judged.json" \
  || fail "$TOOLKIT is not among the crates the check judges: the control shows nothing of the toolkit"
echo "control: cargo deny check $CHECK passes on the tree as it is, $TOOLKIT in it"

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
  # Anything but a count of one or more fails: a run that could not be read is no refusal.
  count=$(refusals) || fail "$CRATE planted in $manifest: jq could not read the diagnostics of the check"
  if ! [ "$count" -ge 1 ]; then
    show "$@"
    fail "$CRATE planted in $manifest: the check fails, but not with '$CODE' for $CRATE"
  fi
  echo "$manifest: $CRATE planted, refused ($CODE)"

  cp "$TMP/manifest.kept" "$TREE/$manifest"
  cp "$TMP/lock.kept" "$TREE/Cargo.lock"
done

if [ "$KIND" = git ]; then
  # The plant shows that a git repository nobody allowed is refused, not that Zed's is:
  # a repository on this machine is refused whatever the lists of allowed ones say. So
  # the settings themselves are held: the registry is the one source, and no key lets a
  # git repository in, by its address, as an organisation's or as a private one. With
  # comments and empty lines left out, the section must be these lines.
  cat > "$TMP/sources.wanted" <<'EOF'
[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
allow-git = []
EOF
  # Every table whose name starts with sources, the tables under it included, however
  # the name is spaced or quoted.
  awk '
    /^[ \t]*(#|$)/ { next }
    /^[ \t]*\[/ { name = $0; gsub(/[^a-z.\[]/, "", name); inside = (name ~ /^\[+sources/) }
    inside { print }' "$TREE/deny.toml" > "$TMP/sources.found"
  if ! diff "$TMP/sources.wanted" "$TMP/sources.found" >&2; then
    fail "deny.toml: [sources] must let in the registry and nothing else"
  fi
  echo "deny.toml lets in the registry and no git source"
fi

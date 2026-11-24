#!/bin/sh
# Plants one dependency the build must refuse; passes only if the check refuses it by name.
# plant.sh licence        a crate under GPL-3.0-only         cargo deny check licenses
# plant.sh ban <crate>    a crate with a banned name         cargo deny check bans
# plant.sh git            a crate from a git repository      cargo deny check sources
# Needs cargo-deny, jq and, for git, git. Works in a copy of the tree, offline.
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

# The licence check leaves dev-dependencies out unless told not to, so a licence is planted under
# all three kinds; the other checks judge a crate whatever its kind.
case "$KIND" in
  licence) SECTIONS="dependencies dev-dependencies build-dependencies" ;;
  *)       SECTIONS="dependencies" ;;
esac
MANIFESTS="crates/window/window/Cargo.toml crates/keeper/keeper/Cargo.toml crates/keeper/adapter-pi/Cargo.toml companions/claude/Cargo.toml"
TOOLKIT=gpui-kit
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

deny() {
  # shellcheck disable=SC2086
  (cd "$TREE" && env "$@" cargo deny $OFFLINE --format json check "$CHECK") > /dev/null 2> "$LOG"
}

show() {
  # shellcheck disable=SC2086
  (cd "$TREE" && env "$@" cargo deny $OFFLINE --color never check "$CHECK") >&2 || true
}

refusals() {
  jq -R -s -r --arg code "$CODE" --arg crate "$CRATE" '
    [split("\n")[] | fromjson?
      | select(.type == "diagnostic" and .fields.severity == "error" and .fields.code == $code)
      | select(any(.fields.graphs[]?; .Krate.name == $crate))]
    | length' "$LOG"
}

# The copy belongs to whoever runs this: in a container root, the checkout was made by another user.
mkdir "$TREE" "$TMP/plant"
tar -C "$ROOT" --exclude=./target --exclude=./.git --exclude=./dist -cf "$TMP/tree.tar" .
tar -C "$TREE" --no-same-owner -xf "$TMP/tree.tar"
rm "$TMP/tree.tar"
for manifest in $MANIFESTS; do
  [ -f "$TREE/$manifest" ] || fail "$manifest is not in the tree"
done

# A plant is refused by one part of the check, asked alone: that says something only while the
# workflow runs the whole check in one step that fails the run. YAML reads no comment or empty
# line, so those are dropped first.
awk '!/^[ \t]*#/ && !/^[ \t]*$/' "$TREE/$WORKFLOW" > "$TMP/workflow"
steps=$(grep -c -x -F -e "$STEP_LINE" "$TMP/workflow") || true
[ "$steps" = 1 ] || fail "$WORKFLOW must hold the step '$RUN_STEP' once"
echo "        run: $RUN_COMMAND" > "$TMP/step.wanted"
awk -v name="$STEP_LINE" '
  $0 == name { inside = 1; next }
  inside && !/^        / { inside = 0 }
  inside { print }' "$TMP/workflow" > "$TMP/step.found"
if ! diff "$TMP/step.wanted" "$TMP/step.found" >&2; then
  fail "the step '$RUN_STEP' of $WORKFLOW must be 'run: $RUN_COMMAND' and nothing else"
fi
awk -v name="$STEP_LINE" '
  function job_ends() { if (held) printf "%s", keys; held = 0; keys = ""; last = "" }
  !/^    / { job_ends(); if (!/^ /) top = $0; next }
  /^    [^ ]/ { keys = keys $0 "\n"; last = $0 }
  $0 == name && top == "jobs:" && last == "    steps:" { held = 1 }
  END { job_ends() }' "$TMP/workflow" > "$TMP/job.found"
[ -s "$TMP/job.found" ] \
  || fail "the step '$RUN_STEP' of $WORKFLOW must be one of the steps of a job"
# Any other key is refused: a list of the harmful keys would miss one written in quotes.
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
    # Offline, cargo would refuse to read even this repository. Cargo keeps what it reads from git
    # under its home, so this run gets a home of its own with the registry linked in.
    PLANTED_OFFLINE=""
    REAL_HOME=${CARGO_HOME:-$HOME/.cargo}
    mkdir "$TMP/cargo-home"
    for kept in registry config.toml config; do
      [ ! -e "$REAL_HOME/$kept" ] || ln -s "$REAL_HOME/$kept" "$TMP/cargo-home/$kept"
    done
    set -- "CARGO_HOME=$TMP/cargo-home"
    ;;
esac

if ! deny; then
  show
  fail "cargo deny check $CHECK fails before anything is planted"
fi
# A failure of cargo deny alone proves nothing (a stale lock fails it too): the check must pass
# without the plant, with the toolkit among the crates it judged.
# shellcheck disable=SC2086
(cd "$TREE" && cargo deny $OFFLINE list --format json --layout crate) > "$TMP/judged.json" 2> /dev/null \
  || fail "cargo deny cannot list the crates it judges"
grep -q -F -e "\"$TOOLKIT " "$TMP/judged.json" \
  || fail "$TOOLKIT is not among the crates the check judges: the control shows nothing of the toolkit"
echo "control: cargo deny check $CHECK passes on the tree as it is, $TOOLKIT in it"

# Kept under names cargo does not read: a Cargo.toml here would become the workspace root.
cp "$TREE/Cargo.lock" "$TMP/lock.kept"
OFFLINE=$PLANTED_OFFLINE
for manifest in $MANIFESTS; do
  for section in $SECTIONS; do
    cp "$TREE/$manifest" "$TMP/manifest.kept"
    printf '\n[%s.%s]\n%s\n' "$section" "$CRATE" "$WHERE" >> "$TREE/$manifest"

    if deny "$@"; then
      fail "$CRATE planted under [$section] of $manifest, and cargo deny check $CHECK passes"
    fi
    count=$(refusals) || fail "$CRATE planted under [$section] of $manifest: jq could not read the diagnostics of the check"
    if ! [ "$count" -ge 1 ]; then
      show "$@"
      fail "$CRATE planted under [$section] of $manifest: the check fails, but not with '$CODE' for $CRATE"
    fi
    echo "$manifest: $CRATE planted under [$section], refused ($CODE)"

    cp "$TMP/manifest.kept" "$TREE/$manifest"
    cp "$TMP/lock.kept" "$TREE/Cargo.lock"
  done
done

if [ "$KIND" = git ]; then
  # The plant shows that an unallowed git repository is refused, not that Zed's is, so these
  # settings are held too: the section must be these lines, comments and empty lines left out.
  cat > "$TMP/sources.wanted" <<'EOF'
[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
allow-git = []
EOF
  awk '
    /^[ \t]*(#|$)/ { next }
    /^[ \t]*\[/ { name = $0; gsub(/[^a-z.\[]/, "", name); inside = (name ~ /^\[+sources/) }
    inside { print }' "$TREE/deny.toml" > "$TMP/sources.found"
  if ! diff "$TMP/sources.wanted" "$TMP/sources.found" >&2; then
    fail "deny.toml: [sources] must let in the registry and nothing else"
  fi
  echo "deny.toml lets in the registry and no git source"
fi

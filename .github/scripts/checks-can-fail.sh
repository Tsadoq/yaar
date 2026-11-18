#!/bin/sh
# Shows that plant.sh and own-licence.sh can fail. A check that cannot is no check.
#
#   checks-can-fail.sh
#
# Each case takes a fresh copy of the tree, switches off the one thing a check guards,
# runs that check from the copy, and passes only if the check fails and says why in the
# words expected here. A case whose change did not take leaves the check passing, so the
# case fails: none can pass idly. Three cases break the run in another way instead, to
# show that a planted check does not take any failure for a refusal.
#
# Needs what the two scripts need: cargo-deny, jq, git.
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
COPY=$TMP/tree
tar -C "$ROOT" --exclude=./target --exclude=./.git --exclude=./dist -cf "$TMP/tree.tar" .

cases=0

# A fresh copy of the tree at $COPY.
copy() {
  rm -rf "$COPY"
  mkdir "$COPY"
  tar -C "$COPY" --no-same-owner -xf "$TMP/tree.tar"
}

# Rewrites one file of the copy through a sed program.
change() {
  sed -e "$2" "$COPY/$1" > "$TMP/changed"
  cat "$TMP/changed" > "$COPY/$1"
}

# Runs a script of the copy, and passes only if it fails with the words given.
fails_with() {
  words=$1
  script=$2
  shift 2
  cases=$((cases + 1))
  if "$COPY/.github/scripts/$script" "$@" > "$TMP/output" 2>&1; then
    cat "$TMP/output" >&2
    echo "checks-can-fail.sh: case $cases: $script $* passes, with what it guards switched off" >&2
    exit 1
  fi
  if ! grep -q -F -e "$words" "$TMP/output"; then
    cat "$TMP/output" >&2
    echo "checks-can-fail.sh: case $cases: $script $* fails, but not with: $words" >&2
    exit 1
  fi
  echo "case $cases: $script${1:+ $*} fails: $(grep -F -e "$words" "$TMP/output" | head -n 1 | sed 's/^ *//')"
}

# The licence of the plant is put on the list of allowed licences.
copy
change deny.toml 's/^    "MIT",$/    "MIT", "GPL-3.0-only",/'
fails_with "and cargo deny check licenses passes" plant.sh licence

# The check is let leave out the dependencies that only tests and examples need: a crate
# under a licence outside the list then passes as a dev-dependency.
copy
change deny.toml '/^include-dev = true$/d'
fails_with "under [dev-dependencies] of" plant.sh licence

# The ban list is emptied.
copy
change deny.toml '/^    { crate = /d'
fails_with "and cargo deny check bans passes" plant.sh ban wry
fails_with "and cargo deny check bans passes" plant.sh ban deno_core
fails_with "and cargo deny check bans passes" plant.sh ban gpui
fails_with "and cargo deny check bans passes" plant.sh ban zed-font-kit

# Nothing says any more who may link a library Zed publishes. The toolkit links it, so
# the tree as it is fails: the library is in the tree, and only the toolkit lets it in.
copy
change deny.toml 's/^    { crate = "zed-font-kit", wrappers = [^]]*\], /    { crate = "zed-font-kit", /'
fails_with "cargo deny check bans fails before anything is planted" plant.sh ban zed-font-kit

# The toolkit is taken out of what the check judges: a control that passes then says
# nothing of it.
copy
change deny.toml '/^\[graph\]$/a\
exclude = ["gpui-kit"]'
fails_with "gpui-kit is not among the crates the check judges" plant.sh ban wry

# The check fails with nothing planted: a failure alone is not taken for a refusal.
copy
change deny.toml '/^    "MIT",$/d'
fails_with "cargo deny check licenses fails before anything is planted" plant.sh licence

# The planted run fails for a manifest cargo cannot read: neither is that.
copy
fails_with "the check fails, but not with 'banned'" plant.sh ban "not a crate name"

# The diagnostics cannot be read: nor is a run nobody could judge. This jq fails as one
# does that is not installed, or that meets diagnostics of another shape.
copy
mkdir "$TMP/broken"
printf '#!/bin/sh\nexit 5\n' > "$TMP/broken/jq"
chmod +x "$TMP/broken/jq"
kept_path=$PATH
PATH=$TMP/broken:$PATH
fails_with "jq could not read the diagnostics" plant.sh ban wry
PATH=$kept_path

# The step of the run no longer checks licences. A dependency under a licence outside the
# list would then pass the run, whatever a plant shows.
copy
change .github/workflows/ci.yml 's/^        run: cargo deny check$/        run: cargo deny check advisories bans sources/'
fails_with ">         run: cargo deny check advisories bans sources" plant.sh licence

# The step of the run may fail without failing the run.
copy
change .github/workflows/ci.yml '/^        run: cargo deny check$/a\
        continue-on-error: true'
fails_with ">         continue-on-error: true" plant.sh ban wry

# The same, behind a comment. A comment ends nothing: the word still belongs to the step.
copy
change .github/workflows/ci.yml '/^        run: cargo deny check$/a\
      # May fail for now.\
        continue-on-error: true'
fails_with ">         continue-on-error: true" plant.sh licence

# The step of the run is gone.
copy
change .github/workflows/ci.yml 's/^      - name: Licences, advisories, bans, sources$/      - name: Advisories/'
fails_with "must hold the step 'Licences, advisories, bans, sources' once" plant.sh git

# The step is there in words only: they are text in the name of the job, and the step
# that runs is another.
copy
change .github/workflows/ci.yml '/^    name: checks linux /c\
    name: |\
      - name: Licences, advisories, bans, sources\
        run: cargo deny check
s/^      - name: Licences, advisories, bans, sources$/      - name: Advisories/
s/^        run: cargo deny check$/        run: cargo deny check advisories/'
fails_with "must be one of the steps of a job" plant.sh ban deno_core

# The job of the step may fail without failing the run.
copy
change .github/workflows/ci.yml '/^  checks-linux:$/a\
    continue-on-error: true'
fails_with "says 'continue-on-error: true'" plant.sh licence

# The job of the step is left out of the run.
copy
change .github/workflows/ci.yml '/^  checks-linux:$/a\
    if: false'
fails_with "says 'if: false'" plant.sh ban gpui

# Git sources are no longer refused.
copy
change deny.toml 's/^unknown-git = "deny"$/unknown-git = "allow"/'
fails_with "and cargo deny check sources passes" plant.sh git

# Zed's repository is let in by its address. The planted repository is still refused.
copy
change deny.toml 's|^allow-git = \[\]$|allow-git = ["https://github.com/zed-industries/zed"]|'
fails_with '> allow-git = ["https://github.com/zed-industries/zed"]' plant.sh git

# Zed's repositories are let in as an organisation.
copy
printf '\n[sources.allow-org]\ngithub = ["zed-industries"]\n' >> "$COPY/deny.toml"
fails_with '> github = ["zed-industries"]' plant.sh git

# Zed's repositories are let in as private ones, by the start of their address.
copy
change deny.toml '/^allow-git = \[\]$/a\
private = ["https://github.com/zed-industries"]'
fails_with '> private = ["https://github.com/zed-industries"]' plant.sh git

# One crate of the workspace is put under another licence.
copy
change crates/shared/link/Cargo.toml 's/^license.workspace = true$/license = "MIT"/'
fails_with "yaar-link: MIT" own-licence.sh

# The whole workspace is.
copy
change Cargo.toml 's/^license = "Apache-2.0"$/license = "MIT"/'
fails_with "yaar-keeper: MIT" own-licence.sh

# One crate declares no licence.
copy
change companions/claude/Cargo.toml '/^license.workspace = true$/d'
fails_with "yaar-companion-claude: no licence declared" own-licence.sh

# The licence file is another text.
copy
echo "One more condition." >> "$COPY/LICENSE"
fails_with "LICENSE is not the text of the Apache License 2.0" own-licence.sh

echo "$cases cases, each failed as it should"

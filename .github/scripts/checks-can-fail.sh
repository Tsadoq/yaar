#!/bin/sh
# Shows that plant.sh and own-licence.sh can fail: each case breaks what one check guards in a
# fresh copy of the tree and passes only if the check then fails with the expected words.
# Needs cargo-deny, jq, git.
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
COPY=$TMP/tree
tar -C "$ROOT" --exclude=./target --exclude=./.git --exclude=./dist -cf "$TMP/tree.tar" .

cases=0

copy() {
  rm -rf "$COPY"
  mkdir "$COPY"
  tar -C "$COPY" --no-same-owner -xf "$TMP/tree.tar"
}

change() {
  sed -e "$2" "$COPY/$1" > "$TMP/changed"
  cat "$TMP/changed" > "$COPY/$1"
}

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

copy
change deny.toml 's/^    "MIT",$/    "MIT", "GPL-3.0-only",/'
fails_with "and cargo deny check licenses passes" plant.sh licence

copy
change deny.toml '/^include-dev = true$/d'
fails_with "under [dev-dependencies] of" plant.sh licence

copy
change deny.toml '/^    { crate = /d'
fails_with "and cargo deny check bans passes" plant.sh ban wry
fails_with "and cargo deny check bans passes" plant.sh ban deno_core
fails_with "and cargo deny check bans passes" plant.sh ban gpui
fails_with "and cargo deny check bans passes" plant.sh ban zed-font-kit

copy
# Without its wrappers the toolkit's own link to zed-font-kit is banned: it fails before any plant.
change deny.toml 's/^    { crate = "zed-font-kit", wrappers = [^]]*\], /    { crate = "zed-font-kit", /'
fails_with "cargo deny check bans fails before anything is planted" plant.sh ban zed-font-kit

copy
change deny.toml '/^\[graph\]$/a\
exclude = ["gpui-kit"]'
fails_with "gpui-kit is not among the crates the check judges" plant.sh ban wry

copy
change deny.toml '/^    "MIT",$/d'
fails_with "cargo deny check licenses fails before anything is planted" plant.sh licence

copy
fails_with "the check fails, but not with 'banned'" plant.sh ban "not a crate name"

copy
mkdir "$TMP/broken"
printf '#!/bin/sh\nexit 5\n' > "$TMP/broken/jq"
chmod +x "$TMP/broken/jq"
kept_path=$PATH
PATH=$TMP/broken:$PATH
fails_with "jq could not read the diagnostics" plant.sh ban wry
PATH=$kept_path

copy
change .github/workflows/ci.yml 's/^        run: cargo deny check$/        run: cargo deny check advisories bans sources/'
fails_with ">         run: cargo deny check advisories bans sources" plant.sh licence

copy
change .github/workflows/ci.yml '/^        run: cargo deny check$/a\
        continue-on-error: true'
fails_with ">         continue-on-error: true" plant.sh ban wry

# A comment ends nothing: the word still belongs to the step.
copy
change .github/workflows/ci.yml '/^        run: cargo deny check$/a\
      # May fail for now.\
        continue-on-error: true'
fails_with ">         continue-on-error: true" plant.sh licence

copy
change .github/workflows/ci.yml 's/^      - name: Licences, advisories, bans, sources$/      - name: Advisories/'
fails_with "must hold the step 'Licences, advisories, bans, sources' once" plant.sh git

# The step's lines appear only as text inside the job's name; the step that really runs is another.
copy
change .github/workflows/ci.yml '/^    name: checks linux /c\
    name: |\
      - name: Licences, advisories, bans, sources\
        run: cargo deny check
s/^      - name: Licences, advisories, bans, sources$/      - name: Advisories/
s/^        run: cargo deny check$/        run: cargo deny check advisories/'
fails_with "must be one of the steps of a job" plant.sh ban deno_core

copy
change .github/workflows/ci.yml '/^  checks-linux:$/a\
    continue-on-error: true'
fails_with "says 'continue-on-error: true'" plant.sh licence

copy
change .github/workflows/ci.yml '/^  checks-linux:$/a\
    if: false'
fails_with "says 'if: false'" plant.sh ban gpui

copy
change deny.toml 's/^unknown-git = "deny"$/unknown-git = "allow"/'
fails_with "and cargo deny check sources passes" plant.sh git

copy
change deny.toml 's|^allow-git = \[\]$|allow-git = ["https://github.com/zed-industries/zed"]|'
fails_with '> allow-git = ["https://github.com/zed-industries/zed"]' plant.sh git

copy
printf '\n[sources.allow-org]\ngithub = ["zed-industries"]\n' >> "$COPY/deny.toml"
fails_with '> github = ["zed-industries"]' plant.sh git

copy
change deny.toml '/^allow-git = \[\]$/a\
private = ["https://github.com/zed-industries"]'
fails_with '> private = ["https://github.com/zed-industries"]' plant.sh git

copy
change crates/shared/link/Cargo.toml 's/^license.workspace = true$/license = "MIT"/'
fails_with "yaar-link: MIT" own-licence.sh

copy
change Cargo.toml 's/^license = "Apache-2.0"$/license = "MIT"/'
fails_with "yaar-keeper: MIT" own-licence.sh

copy
change companions/claude/Cargo.toml '/^license.workspace = true$/d'
fails_with "yaar-companion-claude: no licence declared" own-licence.sh

copy
echo "One more condition." >> "$COPY/LICENSE"
fails_with "LICENSE is not the text of the Apache License 2.0" own-licence.sh

echo "$cases cases, each failed as it should"

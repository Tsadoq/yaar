#!/bin/sh
# Shows on a copy of this repository that bump.sh refuses what it should, and that after a bump only
# the version lines differ, `cargo metadata --locked --offline` holds and version.sh prints the new
# version. packaging/release/bump-check.sh (needs git, cargo, the fetched crates; changes nothing here)
set -eu

NAME=bump-check.sh
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
[ $# -eq 0 ] || { echo "usage: packaging/release/bump-check.sh" >&2; exit 2; }
[ -x "$HERE/bump.sh" ] || fail "$HERE/bump.sh is not an executable file"
for tool in git cargo tar awk; do
  command -v "$tool" > /dev/null 2>&1 || fail "$tool is not installed"
done

TMP=$(mktemp -d "${TMPDIR:-/tmp}/bump-check.XXXXXX") || fail "no scratch folder could be made"
trap 'rm -rf "$TMP"' EXIT
# The files git knows or would add, as they are now: the uncommitted bump.sh is the one tried. The
# workspace is read with the configuration of whoever runs this, and safe.directory names it for a
# checkout that another user made, as the job's container has; only the scratch copy is read with no
# configuration of the user's.
git -c safe.directory="$ROOT" -C "$ROOT" ls-files -co --exclude-standard > "$TMP/listed" 2> "$TMP/err" \
  || fail "git cannot list the files of $ROOT: $(head -n 3 "$TMP/err")"
while IFS= read -r f; do
  if [ -f "$ROOT/$f" ]; then printf '%s\n' "$f"; fi
done < "$TMP/listed" > "$TMP/files"
[ -s "$TMP/files" ] || fail "git lists no file of $ROOT: there is nothing to copy"
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=check GIT_AUTHOR_EMAIL=check@example.invalid
export GIT_COMMITTER_NAME=check GIT_COMMITTER_EMAIL=check@example.invalid

# --no-same-owner: run as root, the copy would keep the owner the checkout has, and git would refuse it.
copy_to() {
  mkdir "$1"
  tar -C "$ROOT" -cf - -T "$TMP/files" | tar -C "$1" --no-same-owner -xf -
  (cd "$1" && git init -q . && git symbolic-ref HEAD refs/heads/main && git add -A && git commit -q -m 'chore: the copy')
}
COPY=$TMP/copy
copy_to "$COPY"
cd "$COPY"
START=$(packaging/version.sh) || fail "version.sh cannot read the version of the copy"

BUMP=$COPY/packaging/release/bump.sh
# bump <version>...: leaves the status in $STATUS and what it said in $SAID.
bump() {
  STATUS=0
  "$BUMP" "$@" > "$TMP/out" 2> "$TMP/err" || STATUS=$?
  SAID=$(cat "$TMP/out" "$TMP/err")
}
untouched() { # <what it shows>
  [ -z "$(git status --porcelain)" ] || fail "$1: the tree changed: $(git status --porcelain | tr '\n' ' ')"
}
refused() { # <what it shows> <status> <words in the sentence> <argument>...
  what=$1
  want=$2
  words=$3
  shift 3
  bump "$@"
  [ "$STATUS" -eq "$want" ] || fail "$what: bump.sh $* exited $STATUS and not $want: $SAID"
  case $SAID in
    *"$words"*) ;;
    *) fail "$what: the sentence does not say '$words': $SAID" ;;
  esac
  untouched "$what"
}

refused "a call with no version" 2 usage
refused "a call with two versions" 2 usage 0.9.0 0.9.1
for bad in abc 1.2 1.2.3.4 01.2.3 1.02.3 1.2.03 v0.9.0 0.9.0-rc1 0.9.x ''; do
  refused "the version '$bad' is not three numbers" 2 "three numbers" "$bad"
done
refused "the present version" 1 "not greater than the present version $START" "$START"

# A refusal for a tree that is not clean; neither a changed file nor a new one is passed over.
printf 'a change\n' >> README.md
bump 9.9.9
[ "$STATUS" -eq 1 ] || fail "a changed file did not stop the bump (status $STATUS): $SAID"
case $SAID in *"uncommitted changes"*) ;; *) fail "a changed file is refused without saying why: $SAID" ;; esac
git checkout -q README.md
printf 'new\n' > a-file-that-is-new
bump 9.9.9
[ "$STATUS" -eq 1 ] || fail "a new file did not stop the bump (status $STATUS): $SAID"
case $SAID in *"uncommitted changes"*) ;; *) fail "a new file is refused without saying why: $SAID" ;; esac
rm a-file-that-is-new
untouched "after the two refusals for a tree with changes"

# The control: the lock file of a workspace whose version moved alone is not one that holds.
cp -R "$COPY" "$TMP/control"
(
  cd "$TMP/control"
  sed 's/^version = "[0-9.]*"/version = "9.9.9"/' Cargo.toml > Cargo.toml.new
  mv Cargo.toml.new Cargo.toml
  if cargo metadata --locked --offline --format-version 1 > /dev/null 2>&1; then
    echo "$NAME: cargo metadata --locked --offline holds for a Cargo.toml raised without Cargo.lock: the check below could not fail" >&2
    exit 1
  fi
) || exit 1
rm -rf "$TMP/control"

# The bump.
cargo metadata --no-deps --locked --offline --format-version 1 > "$TMP/metadata" 2> "$TMP/err" \
  || fail "cargo metadata --locked --offline fails on the copy before any bump: $(head -n 3 "$TMP/err")"
MEMBERS=$(grep -o '"manifest_path"' "$TMP/metadata" | wc -l | tr -d ' ')
[ "$MEMBERS" -gt 1 ] || fail "cargo metadata names $MEMBERS workspace members: the check cannot count what bump.sh must change"

NEW=0.9.0
bump "$NEW"
[ "$STATUS" -eq 0 ] || fail "bump.sh $NEW exited $STATUS: $SAID"
[ "$(packaging/version.sh)" = "$NEW" ] || fail "version.sh prints '$(packaging/version.sh)' after the bump to $NEW"
cargo metadata --locked --offline --format-version 1 > /dev/null 2> "$TMP/err" \
  || fail "cargo metadata --locked --offline fails after the bump: $(head -n 3 "$TMP/err")"
[ "$(git diff --name-only | LC_ALL=C sort | tr '\n' ' ')" = 'Cargo.lock Cargo.toml ' ] \
  || fail "the bump changed $(git diff --name-only | tr '\n' ' ') and not just Cargo.lock and Cargo.toml"
git diff -U0 Cargo.toml | grep '^[-+][^-+]' > "$TMP/toml.diff" || true
[ "$(cat "$TMP/toml.diff")" = "-version = \"$START\"
+version = \"$NEW\"" ] || fail "the lines of Cargo.toml that changed are not the one version line: $(tr '\n' ' ' < "$TMP/toml.diff")"
git diff -U0 Cargo.lock | grep '^[-+][^-+]' > "$TMP/lock.diff" || true
if grep -qv '^[-+]version = "' "$TMP/lock.diff"; then
  fail "a line of Cargo.lock that is not a version line changed: $(grep -v '^[-+]version = "' "$TMP/lock.diff" | head -n 1)"
fi
[ "$(grep -c "^-version = \"$START\"" "$TMP/lock.diff")" -eq "$MEMBERS" ] || fail "Cargo.lock changed $(grep -c "^-version" "$TMP/lock.diff") version lines from $START, and the workspace has $MEMBERS members"
[ "$(grep -c "^+version = \"$NEW\"" "$TMP/lock.diff")" -eq "$MEMBERS" ] || fail "Cargo.lock holds $(grep -c "^+version = \"$NEW\"" "$TMP/lock.diff") new version lines, and the workspace has $MEMBERS members"
[ "$(grep -c '^+version' "$TMP/lock.diff")" -eq "$MEMBERS" ] || fail "Cargo.lock changed a version line that is not a member's"

# A tree with the bump in it is not clean, and one that is committed is again.
bump 0.9.1
[ "$STATUS" -eq 1 ] || fail "a second bump over an uncommitted bump exited $STATUS and not 1: $SAID"
git add -A
git commit -q -m 'chore: bump'
bump 0.10.0
[ "$STATUS" -eq 0 ] || fail "bump.sh 0.10.0 over 0.9.0 exited $STATUS (10 is more than 9): $SAID"
[ "$(packaging/version.sh)" = 0.10.0 ] || fail "version.sh prints '$(packaging/version.sh)' after the bump to 0.10.0"
git add -A
git commit -q -m 'chore: bump'
refused "a version below the present one" 1 "not greater than the present version 0.10.0" 0.9.9
cargo metadata --locked --offline --format-version 1 > /dev/null 2> "$TMP/err" \
  || fail "cargo metadata --locked --offline fails after the second bump: $(head -n 3 "$TMP/err")"

# A crate under vendor/ has no source in the lock file, as a member has, and can stand at the version
# of the app. Only the members change, by name.
new_workspace() { # <folder>: three members (a glob and a folder named by itself) and a vendored crate, all at 0.3.12
  mkdir -p "$1/crates/one/src" "$1/crates/two/src" "$1/tools/solo/src" "$1/vendor/vend/src"
  cat > "$1/Cargo.toml" << 'TOML'
[workspace]
resolver = "3"
members = [
    "crates/*", # every folder with a manifest
    "tools/solo",
]
exclude = ["vendor"]

[workspace.package]
version = "0.3.12"
edition = "2024"
publish = false
TOML
  for member in crates/one crates/two tools/solo; do
    printf '[package]\nname = "%s"\nversion.workspace = true\nedition.workspace = true\npublish.workspace = true\n\n[dependencies]\nvend = { path = "%s" }\n' \
      "$(basename "$member")" "../../vendor/vend" > "$1/$member/Cargo.toml"
    : > "$1/$member/src/lib.rs"
  done
  printf '[package]\nname = "vend"\nversion = "0.3.12"\nedition = "2024"\npublish = false\n' > "$1/vendor/vend/Cargo.toml"
  : > "$1/vendor/vend/src/lib.rs"
  mkdir -p "$1/packaging"
  cp "$COPY/packaging/version.sh" "$1/packaging/version.sh"
  mkdir -p "$1/packaging/release"
  cp "$BUMP" "$1/packaging/release/bump.sh"
  # Outside the repository rustup has no toolchain to choose unless the pin comes along.
  cp "$COPY/rust-toolchain.toml" "$1/rust-toolchain.toml"
  (cd "$1" && cargo generate-lockfile --offline > /dev/null 2> "$TMP/err" \
    && git init -q . && git symbolic-ref HEAD refs/heads/main && git add -A && git commit -q -m 'chore: the workspace') \
    || fail "the scratch workspace could not be made: $(head -n 3 "$TMP/err")"
}
lock_of() { # <lock file> <crate>: the version the lock gives the crate
  awk -v n="$2" '$0 == "name = \"" n "\"" { getline; print; exit }' "$1"
}
W=$TMP/workspace
new_workspace "$W"
cd "$W"
BUMP=$W/packaging/release/bump.sh
for crate in one two solo vend; do
  [ "$(lock_of Cargo.lock "$crate")" = 'version = "0.3.12"' ] || fail "the scratch lock file does not hold $crate at 0.3.12: $(lock_of Cargo.lock "$crate")"
done
bump 0.3.13
[ "$STATUS" -eq 0 ] || fail "bump.sh 0.3.13 over a workspace whose vendored crate stands at 0.3.12 exited $STATUS: $SAID"
for crate in one two solo; do
  [ "$(lock_of Cargo.lock "$crate")" = 'version = "0.3.13"' ] || fail "the member $crate stands at '$(lock_of Cargo.lock "$crate")' after the bump to 0.3.13"
done
[ "$(lock_of Cargo.lock vend)" = 'version = "0.3.12"' ] || fail "the vendored crate vend, which is no member, went to '$(lock_of Cargo.lock vend)'"
case $SAID in
  *"3 workspace members"*) ;;
  *) fail "the bump does not say it changed 3 members: $SAID" ;;
esac
cargo metadata --locked --offline --format-version 1 > /dev/null 2> "$TMP/err" \
  || fail "cargo metadata --locked --offline fails after a bump over a vendored crate at the old version: $(head -n 3 "$TMP/err")"
git checkout -q .

# A member whose lock entry does not stand at the present version is a fault of the lock file, not a
# bump of fewer entries.
awk '$0 == "name = \"two\"" { print; getline; print "version = \"0.3.0\""; next } { print }' Cargo.lock > Cargo.lock.new
mv Cargo.lock.new Cargo.lock
git commit -q -a -m 'chore: a lock file out of step'
bump 0.3.13
[ "$STATUS" -eq 1 ] || fail "a member out of step in Cargo.lock: bump.sh exited $STATUS and not 1: $SAID"
case $SAID in
  *"2 entries"*"3 members"*"two"*) ;;
  *) fail "a member out of step in Cargo.lock: the sentence does not say 2 entries, 3 members and the member two: $SAID" ;;
esac
untouched "a member out of step in Cargo.lock"
git reset -q --hard HEAD~1

# A members pattern the script cannot read, and a member with no name.
sed 's|"crates/\*", # every folder with a manifest|"crates/*/src",|' Cargo.toml > Cargo.toml.new
mv Cargo.toml.new Cargo.toml
git commit -q -a -m 'chore: a pattern of no known form'
bump 0.3.13
[ "$STATUS" -eq 1 ] || fail "a glob in the middle of a member path: bump.sh exited $STATUS and not 1: $SAID"
case $SAID in *"a glob this script does not read"*) ;; *) fail "a glob in the middle of a member path is refused without saying why: $SAID" ;; esac
untouched "a glob in the middle of a member path"
git reset -q --hard HEAD~1
printf '[package]\nversion.workspace = true\n' > crates/one/Cargo.toml
git commit -q -a -m 'chore: a member with no name'
bump 0.3.13
[ "$STATUS" -eq 1 ] || fail "a member with no name: bump.sh exited $STATUS and not 1: $SAID"
case $SAID in *"crates/one/Cargo.toml holds no name"*) ;; *) fail "a member with no name is refused without saying which: $SAID" ;; esac
untouched "a member with no name"
git reset -q --hard HEAD~1

# A root package beside [workspace] is a member the script does not count, and a member path in single
# quotes would be left out: each is refused, and nothing is written.
printf '\n[package]\nname = "root"\nversion.workspace = true\n' >> Cargo.toml
git commit -q -a -m 'chore: a root package beside the workspace'
bump 0.3.13
[ "$STATUS" -eq 1 ] || fail "a root [package] beside [workspace]: bump.sh exited $STATUS and not 1: $SAID"
case $SAID in *"[package] beside [workspace]"*) ;; *) fail "a root [package] beside [workspace] is refused without saying why: $SAID" ;; esac
untouched "a root [package] beside [workspace]"
git reset -q --hard HEAD~1
sed "s|\"tools/solo\",|'tools/solo',|" Cargo.toml > Cargo.toml.new
mv Cargo.toml.new Cargo.toml
git commit -q -a -m 'chore: a member path in single quotes'
bump 0.3.13
[ "$STATUS" -eq 1 ] || fail "a member path in single quotes: bump.sh exited $STATUS and not 1: $SAID"
case $SAID in *"single quotes"*) ;; *) fail "a member path in single quotes is refused without saying why: $SAID" ;; esac
untouched "a member path in single quotes"
git reset -q --hard HEAD~1
cd "$COPY"
BUMP=$COPY/packaging/release/bump.sh

echo "$NAME: bump.sh refuses what it should, and a bump moves $MEMBERS lock entries and one line of Cargo.toml, with cargo metadata --locked --offline holding"

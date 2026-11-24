#!/bin/bash
# Row PKG-09: after the uninstall no file of the app remains in the user's folders, the
# Keeper's lock file included, but the user's reviews (and pi's extension file, which pi's
# own settings may still name) until the uninstall is asked to remove them too; and nothing
# of the user's is removed.
#
#   .github/scripts/user-files.sh plant <user>
#   .github/scripts/user-files.sh snapshot <user> <file>
#   .github/scripts/user-files.sh present <user>
#   .github/scripts/user-files.sh check <user> <file>
#
# Run as root on Linux (the job's steps are root; what is done as <user> goes through
# `sudo -u <user>`), as the user on macOS (the runner's user is the user).
#
# THE ROOTS. The folders walked are the colon-separated list YAAR_USER_FILES_ROOTS when it
# is set (a builder on the owner's Mac sets it to its scratch HOME, sets HOME to the same
# folder, and walks nothing else). Unset, they are: the user's home; on Linux
# /run/user/<uid> when it exists, and /tmp; on macOS /private/tmp (never /tmp, which is a
# link there and which a walk that follows no link would not enter) and the user's own
# temporary and cache folders (`getconf DARWIN_USER_TEMP_DIR`, `getconf
# DARWIN_USER_CACHE_DIR`, run as that user). Each root is resolved once with `cd -P`.
# PRUNED, never entered: on GitHub's Mac, which runs every job in its runner user's home,
# the folder above $RUNNER_TEMP (that is $HOME/work, which holds the checkout and the job's
# scratch), $RUNNER_TOOL_CACHE, $HOME/Library/Android, $HOME/.android, $HOME/.rustup,
# $HOME/.cargo, $HOME/.npm, $HOME/.nvm and $HOME/Library/Developer, each only when it exists
# and lies under a root; on Linux nothing is pruned (the job's checkout and scratch lie
# under /__w, outside the roots). YAAR_USER_FILES_PRUNE (colon-separated) adds to the list.
#
# JUDGED PATHS. On Linux every path under the roots that belongs to <user>. On macOS, where
# the runner's own services write into the home while the job runs, a path is judged when
# its FULL path, case ignored, holds "yaar-review", "io.github.tsadoq" or "yaar.app" (never
# the bare "yaar": the runner's checkout path holds it); other new paths are printed, not
# judged.
#
# THE USER'S OWN FILES, which no uninstall may touch (`plant` makes them before the install
# and before the job's first snapshot, owned by <user>):
#   Linux:  ~/.local/share/yaar-review-notes/notes.txt (a file beside the data folder, whose
#           name begins as the app's does) and ~/projects/mine, a git repository with one
#           commit (an identity in its own .git/config; GIT_CONFIG_GLOBAL=/dev/null,
#           GIT_CONFIG_NOSYSTEM=1);
#   macOS:  ~/Library/Application Support/yaar-review-notes/notes.txt and
#           ~/yaar-review-user-repo, the same repository.
#   Their paths are judged paths on both systems.
#
# THE DATA FOLDER is ~/.local/share/yaar-review on Linux (the job sets no XDG_DATA_HOME and,
# through sudo and env -i, no XDG_RUNTIME_DIR, so the Keeper's socket and its lock lie there
# too) and ~/Library/Application Support/yaar-review on macOS.
#
# snapshot: writes to <file> the sorted list of every path under the roots that belongs to
#   the user, without following links (`find -P <root> ... -user <uid>`, pruned as above),
#   each as "<kind> <path>" (kind: d, f, l, s for a socket, o other).
# present, run after the steps that ran the app and before the uninstall: asserts that the
#   data folder holds keeper.sock.lock (the Keeper ran as the user and took its lock) and
#   store.db (the Keeper opened its Store: the worktree step made a workspace). Fails naming
#   each missing file. This is what makes "the Keeper's lock file included" and "the reviews
#   stay" clauses that can fail.
# check, after the job's uninstall step (`--uninstall`, which keeps the reviews), with <file>
#   the snapshot taken before the install:
#   1. A snapshot. (a) Among the judged paths that <file> does not hold, only these may be
#      there: the data folder's store.db (with -wal and -shm), its folder states and what is
#      under it, its folder agents/pi and what is under it, and the folders above them up to
#      the home. Anything else fails, the lock file keeper.sock.lock first of all. (b) store.db
#      must be there (the reviews stayed). (c) Every path of <file> must still be there, on
#      Linux every one, on macOS every judged one: an uninstall that removed a file of the
#      user's (the planted ones among them) fails, naming it.
#   2. Runs the install line once more, as the user, with `--uninstall --purge`
#      (YAAR_BASE_URL=http://127.0.0.1:8000, as the job's other install steps).
#   3. A snapshot: no judged path that <file> does not hold may be there, but on Linux these,
#      which Mesa and sudo write for any program and the app does not name:
#      ~/.cache/mesa_shader_cache* (Mesa's shader cache, whose name changes between Mesa
#      releases: mesa_shader_cache, mesa_shader_cache_db) with what is under them, ~/.cache
#      (when only those are under it) and ~/.sudo_as_admin_successful. Every path of <file>
#      must still be there, as in 1 (c).
#   Another path that the system writes for any program may join that list only by the
#   orchestrator's ruling, with the program that writes it named.
#   The job's own scratch files lie under $RUNNER_TEMP, never in the user's folders; the
#   steps that make files in the home (macos-login-path.sh, linux-login-path.sh) remove them
#   before this one, and put back byte for byte the start-up file they changed.
#   Every path that fails is printed, with its kind and size.
#
# Exit codes: 0 holds; 1 a leftover, a missing file or a removed file (each named); 2 usage
# error.
#
# What the header leaves, decided here: (1) the allowed list of Mesa's cache and sudo's file
# (point 3) applies to point 1 as well: neither is removed by an uninstall or a purge, so a
# strict reading of point 1 would fail on every machine where the app drew a window;
# (2) `plant` refuses when the planted folder or repository is already there (a stale state
# would hide a removal); (3) a path that vanishes while the walk runs, or a folder the user
# cannot enter, is noted and left out, any other message of `find` fails the walk; (4) on
# macOS a user other than the one running the script is refused (the header says the runner's
# user is the user), and the home is $HOME; (5) the walk of point 1 and 3 goes on after a
# failed point, so that one run names everything, and the purge of point 2 runs whatever
# point 1 found.
set -euo pipefail

NAME=user-files.sh
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
usage() {
  echo "usage: $NAME plant <user> | snapshot <user> <file> | present <user> | check <user> <file>" >&2
  exit 2
}

[ $# -ge 1 ] || usage
CMD=$1
shift
case $CMD in
  plant | present) [ $# -eq 1 ] || usage ;;
  snapshot | check) [ $# -eq 2 ] || usage ;;
  *) usage ;;
esac
USER_NAME=$1
FILE=${2:-}
case $USER_NAME in
  '' | *[!A-Za-z0-9._-]*)
    echo "$NAME: not a user name: $USER_NAME" >&2
    exit 2
    ;;
esac

case $(uname -s) in
  Linux) OS=linux ;;
  Darwin) OS=macos ;;
  *) fail "this system is neither Linux nor macOS: $(uname -s)" ;;
esac

if [ "$OS" = linux ]; then
  USER_UID=$(id -u "$USER_NAME" 2> /dev/null) || {
    echo "$NAME: no user $USER_NAME on this system" >&2
    exit 2
  }
  HOME_DIR=$(getent passwd "$USER_NAME" | cut -d: -f6)
  DATA_DIR=$HOME_DIR/.local/share/yaar-review
  NOTES_DIR=$HOME_DIR/.local/share/yaar-review-notes
  REPO_DIR=$HOME_DIR/projects/mine
else
  [ "$USER_NAME" = "$(id -un)" ] || {
    echo "$NAME: on macOS the script runs as the user, $USER_NAME is not $(id -un)" >&2
    exit 2
  }
  USER_UID=$(id -u)
  HOME_DIR=$HOME
  DATA_DIR="$HOME_DIR/Library/Application Support/yaar-review"
  NOTES_DIR="$HOME_DIR/Library/Application Support/yaar-review-notes"
  REPO_DIR=$HOME_DIR/yaar-review-user-repo
fi
[ -n "$HOME_DIR" ] && [ -d "$HOME_DIR" ] || fail "the home of $USER_NAME, '$HOME_DIR', is not a folder"
HOME_DIR=$(cd -P "$HOME_DIR" && pwd -P)

# Runs a command as the user, with the one variable it needs; the rest of the environment is
# sudo's own, which is what a user's shell is given.
run_as() {
  if [ "$(id -u)" -eq "$USER_UID" ]; then
    env HOME="$HOME_DIR" "$@"
  else
    sudo -n -u "$USER_NAME" env HOME="$HOME_DIR" "$@"
  fi
}

SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/yaar-user-files.XXXXXX")
trap 'rm -rf "$SCRATCH"' EXIT

# ---------------------------------------------------------------------------------- plant

plant() {
  [ ! -e "$NOTES_DIR" ] || fail "$NOTES_DIR is there already: plant runs once, on a fresh home"
  [ ! -e "$REPO_DIR" ] || fail "$REPO_DIR is there already: plant runs once, on a fresh home"
  run_as mkdir -p "$NOTES_DIR"
  run_as sh -c 'printf "%s\n" "My own notes. No uninstall of the app may touch this file." > "$1"' sh "$NOTES_DIR/notes.txt"
  run_as mkdir -p "$REPO_DIR"
  git_as() {
    run_as env GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 git -C "$REPO_DIR" "$@"
  }
  git_as init -q -b main
  git_as config user.name "A User"
  git_as config user.email "user@example.invalid"
  run_as sh -c 'printf "%s\n" "a file of the user" > "$1"' sh "$REPO_DIR/README"
  git_as add README
  git_as commit -q -m "The user's own commit"
  echo "$NAME: planted $NOTES_DIR/notes.txt and the repository $REPO_DIR"
}

# ------------------------------------------------------------------------------- the walk

# One path to a line "<kind> <path>": d, f, l, s, o.
CLASSIFY='for p; do
  if [ -L "$p" ]; then k=l
  elif [ -d "$p" ]; then k=d
  elif [ -f "$p" ]; then k=f
  elif [ -S "$p" ]; then k=s
  elif [ -e "$p" ]; then k=o
  else continue
  fi
  printf "%s %s\n" "$k" "$p"
done'

resolve() {
  (cd -P "$1" 2> /dev/null && pwd -P)
}

# The roots, one per line, each resolved once.
roots() {
  local list r items
  if [ -n "${YAAR_USER_FILES_ROOTS:-}" ]; then
    list=$YAAR_USER_FILES_ROOTS
  elif [ "$OS" = linux ]; then
    list=$HOME_DIR
    if [ -d "/run/user/$USER_UID" ]; then list=$list:/run/user/$USER_UID; fi
    list=$list:/tmp
  else
    local t c
    t=$(getconf DARWIN_USER_TEMP_DIR) || fail "getconf DARWIN_USER_TEMP_DIR failed"
    c=$(getconf DARWIN_USER_CACHE_DIR) || fail "getconf DARWIN_USER_CACHE_DIR failed"
    list=$HOME_DIR:/private/tmp:$t:$c
  fi
  IFS=: read -r -a items <<< "$list"
  for r in ${items[@]+"${items[@]}"}; do
    [ -n "$r" ] || continue
    resolve "$r" || fail "the root $r is not a folder"
  done
}

# The folders never entered, one per line.
prunes() {
  local list="" c items
  if [ "$OS" = macos ]; then
    for c in "${RUNNER_TEMP:+$(dirname "$RUNNER_TEMP")}" "${RUNNER_TOOL_CACHE:-}" \
      "$HOME_DIR/Library/Android" "$HOME_DIR/.android" "$HOME_DIR/.rustup" "$HOME_DIR/.cargo" \
      "$HOME_DIR/.npm" "$HOME_DIR/.nvm" "$HOME_DIR/Library/Developer"; do
      [ -n "$c" ] || continue
      list=$list:$c
    done
  fi
  list=$list:${YAAR_USER_FILES_PRUNE:-}
  IFS=: read -r -a items <<< "$list"
  for c in ${items[@]+"${items[@]}"}; do
    if [ -n "$c" ] && [ -d "$c" ]; then resolve "$c"; fi
  done
}

# Writes the sorted snapshot to $1.
take_snapshot() {
  local out=$1 root p first
  local rootlist prunelist
  rootlist=$(roots)
  prunelist=$(prunes)
  : > "$SCRATCH/walk.raw"
  while IFS= read -r root; do
    [ -n "$root" ] || continue
    local args=()
    if [ -n "$prunelist" ]; then
      first=1
      args+=('(')
      while IFS= read -r p; do
        [ -n "$p" ] || continue
        if [ "$first" -eq 0 ]; then args+=(-o); fi
        args+=(-path "$p")
        first=0
      done <<< "$prunelist"
      args+=(')' -prune -o)
    fi
    local status=0
    find -P "$root" ${args[@]+"${args[@]}"} -user "$USER_UID" -exec sh -c "$CLASSIFY" sh {} + \
      >> "$SCRATCH/walk.raw" 2> "$SCRATCH/walk.err" || status=$?
    # A path that vanished while the walk ran, or a folder the user cannot enter, holds
    # nothing to judge; every other message fails the walk.
    if grep -v -e 'No such file or directory' -e 'Permission denied' "$SCRATCH/walk.err" > "$SCRATCH/walk.bad"; [ -s "$SCRATCH/walk.bad" ]; then
      echo "$NAME: walking $root failed:" >&2
      sed 's/^/  /' "$SCRATCH/walk.bad" >&2
      exit 1
    fi
    if [ "$status" -ne 0 ] && [ ! -s "$SCRATCH/walk.err" ]; then
      fail "walking $root failed with status $status and no message"
    fi
    if [ -s "$SCRATCH/walk.err" ]; then
      sed "s|^|$NAME: note, left out: |" "$SCRATCH/walk.err" >&2
    fi
  done <<< "$rootlist"
  LC_ALL=C sort -u "$SCRATCH/walk.raw" > "$out"
}

snapshot() {
  take_snapshot "$FILE.part"
  mv "$FILE.part" "$FILE"
  echo "$NAME: $(wc -l < "$FILE" | tr -d ' ') paths of $USER_NAME in $FILE"
}

# ---------------------------------------------------------------------------------- present

present() {
  local bad=0 f
  for f in keeper.sock.lock store.db; do
    if [ ! -e "$DATA_DIR/$f" ]; then
      echo "$NAME: $DATA_DIR/$f is missing: the app did not run as $USER_NAME, or not far enough" >&2
      bad=1
    else
      echo "$NAME: there: $DATA_DIR/$f"
    fi
  done
  [ "$bad" -eq 0 ] || exit 1
}

# ----------------------------------------------------------------------------------- check

paths_of() {
  cut -c3- "$1" | LC_ALL=C sort -u
}

# On macOS only a path whose full path holds one of three names is judged.
judge() {
  local p lower
  if [ "$OS" = linux ]; then
    cat
    return 0
  fi
  while IFS= read -r p; do
    lower=$(printf '%s' "$p" | tr '[:upper:]' '[:lower:]')
    case $lower in
      *yaar-review* | *io.github.tsadoq* | *yaar.app*) printf '%s\n' "$p" ;;
    esac
  done
}

# A path written for any program by Mesa or by sudo (Linux only).
system_written() {
  [ "$OS" = linux ] || return 1
  case $1 in
    "$HOME_DIR"/.cache/mesa_shader_cache | "$HOME_DIR"/.cache/mesa_shader_cache/* \
      | "$HOME_DIR"/.cache/mesa_shader_cache_db | "$HOME_DIR"/.cache/mesa_shader_cache_db/* \
      | "$HOME_DIR/.sudo_as_admin_successful")
      return 0
      ;;
  esac
  return 1
}

# What may be there after --uninstall: the reviews and pi's extension, and the folders above.
kept_by_uninstall() {
  case $1 in
    "$DATA_DIR/store.db" | "$DATA_DIR/store.db-wal" | "$DATA_DIR/store.db-shm" \
      | "$DATA_DIR/states" | "$DATA_DIR/states"/* \
      | "$DATA_DIR/agents/pi" | "$DATA_DIR/agents/pi"/*)
      return 0
      ;;
  esac
  # A folder above one of them, below the home.
  case $DATA_DIR/ in
    "$1"/*)
      [ "$1" != "$HOME_DIR" ] && case $1 in "$HOME_DIR"/*) return 0 ;; esac
      ;;
  esac
  case $DATA_DIR/agents/pi/ in
    "$1"/*)
      [ "$1" != "$HOME_DIR" ] && case $1 in "$HOME_DIR"/*) return 0 ;; esac
      ;;
  esac
  return 1
}

describe() {
  local kind size
  if [ -L "$1" ]; then
    kind=link
  elif [ -d "$1" ]; then
    kind=folder
  elif [ -f "$1" ]; then
    kind=file
  elif [ -S "$1" ]; then
    kind=socket
  else
    kind=other
  fi
  if [ "$kind" = file ]; then
    size="$(wc -c < "$1" | tr -d ' ') bytes"
  else
    size="no size"
  fi
  printf '%s, %s' "$kind" "$size"
}

FAILED=0

# The judged paths that $2 holds and $1 does not, less what $3 allows ("kept" or "none").
leftovers() {
  local before=$1 now=$2 allow=$3 phase=$4 p
  paths_of "$before" > "$SCRATCH/before.paths"
  paths_of "$now" > "$SCRATCH/now.paths"
  LC_ALL=C comm -13 "$SCRATCH/before.paths" "$SCRATCH/now.paths" > "$SCRATCH/new.paths"
  if [ "$OS" = macos ]; then
    while IFS= read -r p; do
      case $(printf '%s' "$p" | tr '[:upper:]' '[:lower:]') in
        *yaar-review* | *io.github.tsadoq* | *yaar.app*) ;;
        *) echo "$NAME: new, not judged: $p" ;;
      esac
    done < "$SCRATCH/new.paths"
  fi
  judge < "$SCRATCH/new.paths" > "$SCRATCH/judged.paths"
  # ~/.cache is the system's to leave only when nothing but Mesa's cache is under it.
  local cache_other=0
  while IFS= read -r p; do
    case $p in
      "$HOME_DIR"/.cache/*) system_written "$p" || cache_other=1 ;;
    esac
  done < "$SCRATCH/judged.paths"
  : > "$SCRATCH/bad.paths"
  # The lock file first of all, then the rest.
  while IFS= read -r p; do
    case $p in */keeper.sock.lock) printf '%s\n' "$p" >> "$SCRATCH/bad.paths" ;; esac
  done < "$SCRATCH/judged.paths"
  while IFS= read -r p; do
    case $p in */keeper.sock.lock) continue ;; esac
    if system_written "$p"; then continue; fi
    if [ "$OS" = linux ] && [ "$p" = "$HOME_DIR/.cache" ] && [ "$cache_other" -eq 0 ]; then continue; fi
    if [ "$allow" = kept ] && kept_by_uninstall "$p"; then continue; fi
    printf '%s\n' "$p" >> "$SCRATCH/bad.paths"
  done < "$SCRATCH/judged.paths"
  while IFS= read -r p; do
    [ -n "$p" ] || continue
    echo "$NAME: $p ($(describe "$p")) is left $phase and belongs to the app" >&2
    FAILED=1
  done < "$SCRATCH/bad.paths"
}

# Every path of the snapshot $1 (the judged ones on macOS) is still there.
still_there() {
  local before=$1 phase=$2 p
  paths_of "$before" | judge > "$SCRATCH/mine.paths"
  while IFS= read -r p; do
    [ -n "$p" ] || continue
    if [ ! -e "$p" ] && [ ! -L "$p" ]; then
      echo "$NAME: $p was there before the install and is gone $phase: nothing of the user's may be removed" >&2
      FAILED=1
    fi
  done < "$SCRATCH/mine.paths"
}

check() {
  [ -f "$FILE" ] || fail "$FILE, the snapshot taken before the install, is missing"
  # 1. After --uninstall.
  take_snapshot "$SCRATCH/after-uninstall"
  leftovers "$FILE" "$SCRATCH/after-uninstall" kept "after --uninstall"
  if [ ! -e "$DATA_DIR/store.db" ]; then
    echo "$NAME: $DATA_DIR/store.db is gone after --uninstall: the user's reviews must stay until --purge" >&2
    FAILED=1
  fi
  still_there "$FILE" "after --uninstall"
  # 2. The purge.
  echo "$NAME: --uninstall --purge"
  if ! run_as env YAAR_BASE_URL=http://127.0.0.1:8000 bash -c \
    'set -o pipefail; curl -fsSL "$YAAR_BASE_URL/install.sh" | bash -s -- --uninstall --purge'; then
    fail "the install line with --uninstall --purge failed"
  fi
  # 3. After --purge.
  take_snapshot "$SCRATCH/after-purge"
  leftovers "$FILE" "$SCRATCH/after-purge" none "after --purge"
  still_there "$FILE" "after --purge"
  if [ "$FAILED" -ne 0 ]; then
    echo "$NAME: files of the app remain, or files of the user's are gone: see above" >&2
    exit 1
  fi
  echo "$NAME: no file of the app is left, and none of the user's is gone"
}

case $CMD in
  plant) plant ;;
  snapshot) snapshot ;;
  present) present ;;
  check) check ;;
esac

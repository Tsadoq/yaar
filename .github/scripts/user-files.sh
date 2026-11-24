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
#   data folder holds keeper.sock.lock (the Keeper ran as the user and took its lock),
#   store.db (the Keeper opened its Store: the worktree step made a workspace),
#   agents/pi/extension/yaar.ts (the Keeper writes pi's extension when it starts) and the
#   folder states (the Keeper records the states a review refers to: the step of PKG-12 makes
#   a worktree with an agent through the installed app, which leaves it, holding at least
#   one path: an empty states fails too). Fails naming each missing file. This is what makes
#   "the Keeper's lock file included", "the reviews stay" (their states too) and "pi's
#   extension file stays" clauses that can fail. It also writes
#   the record of what the uninstall must leave (see check 1 (d)) to the file
#   YAAR_USER_FILES_RECORD names, else $RUNNER_TEMP/user-files-present, which holds pi's
#   extension file, the folder states and every path under it; with neither set it fails.
# check, after the job's uninstall step (`--uninstall`, which keeps the reviews), with <file>
#   the snapshot taken before the install:
#   1. A snapshot. (a) Among the judged paths that <file> does not hold, only these may be
#      there: the data folder's store.db (with -wal and -shm), its folder states and what is
#      under it, its folder agents/pi and what is under it, and the folders above them up to
#      the home. Anything else fails, the lock file keeper.sock.lock first of all. (b) store.db
#      must be there (the reviews stayed). (c) Every path of <file> must still be there, on
#      Linux every one, on macOS every judged one: an uninstall that removed a file of the
#      user's (the planted ones among them) fails, naming it. (d) Every path in the record
#      that `present` wrote must still be there: pi's extension file, the folder states and
#      every path under it.
#      A missing record fails (present did not run).
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
# point 1 found; (6) the home is resolved with `cd -P` before the data folder and the planted
# places are derived from it, so that a home behind a link (a scratch HOME under macOS's
# $TMPDIR) is compared with the resolved paths of the walk; (7) a command that stops the
# script without a sentence of its own is named in one, and the exit code is 1; (8) pi's
# extension, agents/pi/extension/yaar.ts, is written by the Keeper at every start, and the
# folder states by the worktree with an agent that the step of PKG-12 makes through the
# installed app, so present demands both, and the folder holding at least one path, and
# check (d) asserts the extension, the folder and every path under it stay.
set -euo pipefail

NAME=user-files.sh
SAID=0
fail() {
  SAID=1
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

# The home is resolved first, with `cd -P`, because every path the walk prints is resolved
# and the folders derived from the home are compared with those paths: a home that lies behind
# a link (a scratch HOME under macOS's $TMPDIR, which is under /var, a link to /private/var)
# would otherwise never match.
if [ "$OS" = linux ]; then
  USER_UID=$(id -u "$USER_NAME" 2> /dev/null) || {
    echo "$NAME: no user $USER_NAME on this system" >&2
    exit 2
  }
  HOME_DIR=$(getent passwd "$USER_NAME" | cut -d: -f6) || fail "getent cannot read the home of $USER_NAME"
else
  [ "$USER_NAME" = "$(id -un)" ] || {
    echo "$NAME: on macOS the script runs as the user, $USER_NAME is not $(id -un)" >&2
    exit 2
  }
  USER_UID=$(id -u)
  HOME_DIR=$HOME
fi
if [ -z "$HOME_DIR" ] || [ ! -d "$HOME_DIR" ]; then fail "the home of $USER_NAME, '$HOME_DIR', is not a folder"; fi
HOME_DIR=$(cd -P "$HOME_DIR" && pwd -P) || fail "the home of $USER_NAME, '$HOME_DIR', cannot be entered"
if [ "$OS" = linux ]; then
  DATA_DIR=$HOME_DIR/.local/share/yaar-review
  NOTES_DIR=$HOME_DIR/.local/share/yaar-review-notes
  REPO_DIR=$HOME_DIR/projects/mine
else
  DATA_DIR="$HOME_DIR/Library/Application Support/yaar-review"
  NOTES_DIR="$HOME_DIR/Library/Application Support/yaar-review-notes"
  REPO_DIR=$HOME_DIR/yaar-review-user-repo
fi

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
# A command that set -e stopped on says so in a sentence, with the code this script's header
# names, instead of leaving the failing tool's own status.
cleanup() {
  local status=$1 cmd=$2
  if [ "$status" -ne 0 ] && [ "$SAID" -eq 0 ]; then
    echo "$NAME: '$cmd' failed with status $status" >&2
    status=1
  fi
  rm -rf "$SCRATCH"
  exit "$status"
}
trap 'cleanup "$?" "$BASH_COMMAND"' EXIT

# ---------------------------------------------------------------------------------- plant

plant() {
  [ ! -e "$NOTES_DIR" ] || fail "$NOTES_DIR is there already: plant runs once, on a fresh home"
  [ ! -e "$REPO_DIR" ] || fail "$REPO_DIR is there already: plant runs once, on a fresh home"
  run_as mkdir -p "$NOTES_DIR" || fail "cannot make $NOTES_DIR as $USER_NAME"
  # shellcheck disable=SC2016 # the shell that runs it expands $1, not this one
  run_as sh -c 'printf "%s\n" "My own notes. No uninstall of the app may touch this file." > "$1"' sh "$NOTES_DIR/notes.txt" || fail "cannot write $NOTES_DIR/notes.txt as $USER_NAME"
  run_as mkdir -p "$REPO_DIR" || fail "cannot make $REPO_DIR as $USER_NAME"
  git_as() {
    run_as env GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 git -C "$REPO_DIR" "$@"
  }
  git_as init -q -b main || fail "git init of $REPO_DIR failed"
  git_as config user.name "A User" || fail "git config in $REPO_DIR failed"
  git_as config user.email "user@example.invalid" || fail "git config in $REPO_DIR failed"
  # shellcheck disable=SC2016 # the shell that runs it expands $1, not this one
  run_as sh -c 'printf "%s\n" "a file of the user" > "$1"' sh "$REPO_DIR/README" || fail "cannot write $REPO_DIR/README as $USER_NAME"
  git_as add README || fail "git add in $REPO_DIR failed"
  git_as commit -q -m "The user's own commit" || fail "git commit in $REPO_DIR failed"
  echo "$NAME: planted $NOTES_DIR/notes.txt and the repository $REPO_DIR"
}

# ------------------------------------------------------------------------------- the walk

# One path to a line "<kind> <path>": d, f, l, s, o.
# shellcheck disable=SC2016 # a program for the sh that find starts
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
  # fail inside the substitution says its sentence in the subshell: the parent only ends.
  rootlist=$(roots) || {
    SAID=1
    exit 1
  }
  prunelist=$(prunes) || {
    SAID=1
    exit 1
  }
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
      fail "walking $root failed: $(tr '\n' ' ' < "$SCRATCH/walk.bad")"
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

# Where present leaves what it saw for check: a file of the job's scratch, never of the user's.
record_file() {
  if [ -n "${YAAR_USER_FILES_RECORD:-}" ]; then
    printf '%s\n' "$YAAR_USER_FILES_RECORD"
  elif [ -n "${RUNNER_TEMP:-}" ]; then
    printf '%s\n' "$RUNNER_TEMP/user-files-present"
  else
    fail "neither RUNNER_TEMP nor YAAR_USER_FILES_RECORD is set: there is no place for the record of what the app wrote"
  fi
}

present() {
  local bad=0 f record under
  record=$(record_file) || {
    SAID=1
    exit 1
  }
  # The extension is what pi's own settings may name, so it is a file the app must have
  # written for "pi's extension file stays" to be a clause that can fail.
  for f in keeper.sock.lock store.db agents/pi/extension/yaar.ts states; do
    if [ ! -e "$DATA_DIR/$f" ]; then
      echo "$NAME: $DATA_DIR/$f is missing: the app did not run as $USER_NAME, or not far enough" >&2
      bad=1
    else
      echo "$NAME: there: $DATA_DIR/$f"
    fi
  done
  [ "$bad" -eq 0 ] || fail "the files of the app named above are missing"
  # The folder states must hold a state store, or an uninstall that empties it and keeps the
  # folder would pass: what the folder holds is what the reviews refer to.
  under=$(find "$DATA_DIR/states" -mindepth 1 -print) || fail "the folder $DATA_DIR/states could not be read"
  [ -n "$under" ] || fail "the folder $DATA_DIR/states holds no state store: the step of PKG-12 recorded no state"
  # What no --uninstall may remove besides store.db, which check looks at by name: pi's
  # extension, states/ and every path under it.
  {
    printf '%s\n' "$DATA_DIR/agents/pi/extension/yaar.ts"
    printf '%s\n' "$DATA_DIR/states"
    printf '%s\n' "$under"
  } > "$record" || fail "cannot write $record"
}

# ----------------------------------------------------------------------------------- check

paths_of() {
  cut -c3- "$1" | LC_ALL=C sort -u
}

# On macOS only a path whose full path holds one of three names is judged. The rule is held
# once, as an awk function, and each list is read by one awk process: the snapshot of a
# runner's home has over a hundred thousand paths, and a process for each of them (a `tr`)
# takes longer than the job may. LC_ALL=C: the case that is ignored is ASCII's, and a path
# with bytes that are no text is read as it is.
JUDGED_FN='function judged(path,   lower) {
  lower = tolower(path)
  return index(lower, "yaar-review") || index(lower, "io.github.tsadoq") || index(lower, "yaar.app")
}'

judge() {
  if [ "$OS" = linux ]; then
    cat
    return 0
  fi
  LC_ALL=C awk "$JUDGED_FN"' judged($0) { print }'
}

# A path written for any program by Mesa or by sudo (Linux only).
system_written() {
  [ "$OS" = linux ] || return 1
  case $1 in
    "$HOME_DIR"/.cache/mesa_shader_cache* | "$HOME_DIR/.sudo_as_admin_successful")
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
    kind="link"
  elif [ -d "$1" ]; then
    kind=folder
  elif [ -f "$1" ]; then
    kind="file"
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
    # The prefix goes in by the environment: an awk variable would read its backslashes.
    PREFIX="$NAME: new, not judged: " LC_ALL=C awk "$JUDGED_FN"' !judged($0) { print ENVIRON["PREFIX"] $0 }' "$SCRATCH/new.paths"
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
  local p record
  [ -f "$FILE" ] || fail "$FILE, the snapshot taken before the install, is missing"
  record=$(record_file) || {
    SAID=1
    exit 1
  }
  [ -f "$record" ] || fail "$record, which present writes before the uninstall, is missing: present did not run"
  # 1. After --uninstall.
  take_snapshot "$SCRATCH/after-uninstall"
  leftovers "$FILE" "$SCRATCH/after-uninstall" kept "after --uninstall"
  if [ ! -e "$DATA_DIR/store.db" ]; then
    echo "$NAME: $DATA_DIR/store.db is gone after --uninstall: the user's reviews must stay until --purge" >&2
    FAILED=1
  fi
  still_there "$FILE" "after --uninstall"
  while IFS= read -r p; do
    [ -n "$p" ] || continue
    if [ ! -e "$p" ]; then
      echo "$NAME: $p was there before the uninstall and is gone after --uninstall: it stays until --purge" >&2
      FAILED=1
    fi
  done < "$record"
  # 2. The purge.
  echo "$NAME: --uninstall --purge"
  # shellcheck disable=SC2016 # the shell that runs it expands $YAAR_BASE_URL
  if ! run_as env YAAR_BASE_URL=http://127.0.0.1:8000 bash -c \
    'set -o pipefail; curl -fsSL "$YAAR_BASE_URL/install.sh" | bash -s -- --uninstall --purge'; then
    fail "the install line with --uninstall --purge failed"
  fi
  # 3. After --purge.
  take_snapshot "$SCRATCH/after-purge"
  leftovers "$FILE" "$SCRATCH/after-purge" none "after --purge"
  still_there "$FILE" "after --purge"
  if [ "$FAILED" -ne 0 ]; then
    fail "files of the app remain, or files of the user's are gone: see above"
  fi
  echo "$NAME: no file of the app is left, and none of the user's is gone"
}

case $CMD in
  plant) plant ;;
  snapshot) snapshot ;;
  present) present ;;
  check) check ;;
esac

#!/bin/bash
# After --uninstall no file of the app is left in the user's folders but the reviews and pi's
# extension file; after --uninstall --purge none is; nothing of the user's is ever removed.
# `check` also runs the uninstall with --purge and asserts that nothing of the app is left.
# .github/scripts/user-files.sh plant <user> | snapshot <user> <file> | present <user> |
#   check <user> <file>; root on Linux.
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

# The home is resolved first, with `cd -P`: the walk prints resolved paths, and a home behind a
# link (a scratch HOME under macOS's $TMPDIR) would never match them.
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

# Runs a command as the user with the one variable it needs; the rest is sudo's own environment.
run_as() {
  if [ "$(id -u)" -eq "$USER_UID" ]; then
    env HOME="$HOME_DIR" "$@"
  else
    sudo -n -u "$USER_NAME" env HOME="$HOME_DIR" "$@"
  fi
}

SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/yaar-user-files.XXXXXX")
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

roots() {
  local list r items
  if [ -n "${YAAR_USER_FILES_ROOTS:-}" ]; then
    # A run on a machine with a user's real files sets the roots and HOME to its scratch folder.
    list=$YAAR_USER_FILES_ROOTS
  elif [ "$OS" = linux ]; then
    list=$HOME_DIR
    if [ -d "/run/user/$USER_UID" ]; then list=$list:/run/user/$USER_UID; fi
    list=$list:/tmp
  else
    local t c
    t=$(getconf DARWIN_USER_TEMP_DIR) || fail "getconf DARWIN_USER_TEMP_DIR failed"
    c=$(getconf DARWIN_USER_CACHE_DIR) || fail "getconf DARWIN_USER_CACHE_DIR failed"
    # /private/tmp, never /tmp: a link on macOS, which a walk that follows no link would not enter.
    list=$HOME_DIR:/private/tmp:$t:$c
  fi
  IFS=: read -r -a items <<< "$list"
  for r in ${items[@]+"${items[@]}"}; do
    [ -n "$r" ] || continue
    resolve "$r" || fail "the root $r is not a folder"
  done
}

prunes() {
  # On GitHub's Mac every job runs in the runner user's home: its checkout, scratch and tool caches
  # are pruned, as they are not the app's.
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
  # check demands the lock gone and the rest kept after --uninstall: each must be here first,
  # or that check could not fail.
  for f in keeper.sock.lock store.db agents/pi/extension/yaar.ts states; do
    if [ ! -e "$DATA_DIR/$f" ]; then
      echo "$NAME: $DATA_DIR/$f is missing: the app did not run as $USER_NAME, or not far enough" >&2
      bad=1
    else
      echo "$NAME: there: $DATA_DIR/$f"
    fi
  done
  [ "$bad" -eq 0 ] || fail "the files of the app named above are missing"
  # The folder states must hold a state store, or an uninstall that empties it would pass.
  under=$(find "$DATA_DIR/states" -mindepth 1 -print) || fail "the folder $DATA_DIR/states could not be read"
  [ -n "$under" ] || fail "the folder $DATA_DIR/states holds no state store: the app, as it ran here, recorded no state"
  {
    printf '%s\n' "$DATA_DIR/agents/pi/extension/yaar.ts"
    printf '%s\n' "$DATA_DIR/states"
    printf '%s\n' "$under"
  } > "$record" || fail "cannot write $record"
}

paths_of() {
  cut -c3- "$1" | LC_ALL=C sort -u
}

# On macOS only a path whose full path, case ignored, holds one of these names is judged (never
# the bare "yaar": the runner's checkout path holds it). One awk process reads each list: a
# runner's home has over 100,000 paths, too many for a process each. LC_ALL=C: ASCII case.
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

# True for a path that Mesa or sudo writes on Linux for any program; the app does not name it.
system_written() {
  [ "$OS" = linux ] || return 1
  case $1 in
    "$HOME_DIR"/.cache/mesa_shader_cache* | "$HOME_DIR/.sudo_as_admin_successful")
      return 0
      ;;
  esac
  return 1
}

kept_by_uninstall() {
  case $1 in
    "$DATA_DIR/store.db" | "$DATA_DIR/store.db-wal" | "$DATA_DIR/store.db-shm" \
      | "$DATA_DIR/states" | "$DATA_DIR/states"/* \
      | "$DATA_DIR/agents/pi" | "$DATA_DIR/agents/pi"/*)
      return 0
      ;;
  esac
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
  local cache_other=0
  while IFS= read -r p; do
    case $p in
      "$HOME_DIR"/.cache/*) system_written "$p" || cache_other=1 ;;
    esac
  done < "$SCRATCH/judged.paths"
  : > "$SCRATCH/bad.paths"
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
  echo "$NAME: --uninstall --purge"
  # shellcheck disable=SC2016 # the shell that runs it expands $YAAR_BASE_URL
  if ! run_as env YAAR_BASE_URL=http://127.0.0.1:8000 bash -c \
    'set -o pipefail; curl -fsSL "$YAAR_BASE_URL/install.sh" | bash -s -- --uninstall --purge'; then
    fail "the install line with --uninstall --purge failed"
  fi
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

#!/bin/bash
# What the owner's checklist scripts share. Sourced by the scripts beside it, never run:
#
#   . "$(dirname "$0")/lib.sh"
#
# The caller has `set -euo pipefail` on and sets CK_NAME to its own file name before the
# first call, so that every sentence on standard error names the file that said it.
# Written for bash 3.2, the one macOS ships: no associative arrays, no `mapfile`, no
# `${var,,}`, and no expansion of an empty array under `set -u`.
#
# What it holds: the results folder, the clock, the programs of the app found by their full
# path, a wait that polls, and the walker that prints a checklist one item at a time and
# writes the answers as a Markdown file (the sign-off file).
#
# The walker's answers come from standard input, a terminal or a file of answers, one per
# line, so that a walk can be replayed:
#   yes | y          the item holds
#   no | n           the item does not hold; the next line says what is wrong
#   note             the next line is a note on the item; the item is then asked again
#   skip             the item was not walked; the next line says why
# After `no` and after `skip` the next line is never empty. A walk that reaches the end of
# its input before its last item stops with a sentence and writes no file.
#
# Exit codes of what calls it: 0 done; 1 failed (a sentence on standard error says why);
# 2 usage error.

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  echo "lib.sh: it is sourced by the scripts beside it, not run" >&2
  exit 2
fi

CK_NAME=${CK_NAME:-lib.sh}

ck_die() {
  printf '%s: %s\n' "$CK_NAME" "$1" >&2
  exit 1
}

ck_usage_error() {
  printf '%s: %s (run it with --help)\n' "$CK_NAME" "$1" >&2
  exit 2
}

# Prints the header of the script given (its first block of comment lines).
ck_help() {
  awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$1"
}

ck_os() {
  uname -s
}

# The day's folder of results, made when asked for: ~/yaar-checklists/<YYYY-MM-DD>/
ck_results_dir() {
  [ -n "${HOME:-}" ] && [ -d "$HOME" ] || ck_die "HOME names no folder, so there is nowhere to write the results"
  local d
  d="$HOME/yaar-checklists/$(date +%Y-%m-%d)"
  mkdir -p "$d" || ck_die "cannot make the results folder $d"
  printf '%s\n' "$d"
}

# Milliseconds since 1970, the unit of the app's trace marks.
ck_now_ms() {
  if [ "$(ck_os)" = Darwin ]; then
    /usr/bin/perl -MTime::HiRes=time -e 'printf "%.0f\n", time*1000'
  else
    date +%s%3N
  fi
}

# ck_wait_until <seconds> <command...>: runs the command every 0.2 s until it succeeds or
# the deadline passes; 0 when it succeeded, 1 when the deadline passed. The command is a
# check that returns, never one that needs `set -e` to fail.
ck_wait_until() {
  local limit=$1
  shift
  local deadline=$((SECONDS + limit))
  while :; do
    if "$@"; then
      return 0
    fi
    if [ "$SECONDS" -ge "$deadline" ]; then
      return 1
    fi
    sleep 0.2
  done
}

# ck_pids_of <program path>: the pids of this user's processes whose program is that file,
# matched by its full path and never by a name alone.
ck_pids_of() {
  local want=$1
  if [ "$(ck_os)" = Darwin ]; then
    ps -axo uid=,pid=,comm= | awk -v uid="$(id -u)" -v want="$want" '
      $1 == uid { pid = $2; $1 = ""; $2 = ""; sub(/^ +/, ""); if ($0 == want) print pid }'
  else
    local d exe
    for d in /proc/[0-9]*; do
      [ -O "$d" ] || continue
      exe=$(readlink "$d/exe" 2>/dev/null) || continue
      if [ "$exe" = "$want" ]; then
        printf '%s\n' "${d#/proc/}"
      fi
    done
  fi
}

# The two programs of the app that make up "the app's own processes", by system.
#   ck_window_program   the Window; ck_keeper_program   the Keeper
# macOS: inside the bundle YAAR_APP (default /Applications/YAAR.app). Linux: the package's
# place under /usr, else the archive's under YAAR_PREFIX (default ~/.local); YAAR_REVIEW
# and YAAR_KEEPER name them outright.
ck_app_bundle() {
  printf '%s\n' "${YAAR_APP:-/Applications/YAAR.app}"
}

ck_window_program() {
  if [ -n "${YAAR_REVIEW:-}" ]; then
    printf '%s\n' "$YAAR_REVIEW"
  elif [ "$(ck_os)" = Darwin ]; then
    printf '%s\n' "$(ck_app_bundle)/Contents/MacOS/yaar-review"
  elif [ -x /usr/bin/yaar-review ]; then
    printf '%s\n' /usr/bin/yaar-review
  else
    printf '%s\n' "${YAAR_PREFIX:-$HOME/.local}/bin/yaar-review"
  fi
}

ck_keeper_program() {
  if [ -n "${YAAR_KEEPER:-}" ]; then
    printf '%s\n' "$YAAR_KEEPER"
  elif [ "$(ck_os)" = Darwin ]; then
    printf '%s\n' "$(ck_app_bundle)/Contents/Helpers/yaar-keeper"
  elif [ -x /usr/libexec/yaar-review/yaar-keeper ]; then
    printf '%s\n' /usr/libexec/yaar-review/yaar-keeper
  else
    printf '%s\n' "${YAAR_PREFIX:-$HOME/.local}/libexec/yaar-review/yaar-keeper"
  fi
}

# The resident memory of a process in kB, or nothing when it has gone.
ck_rss_kb() {
  local out
  if out=$(ps -o rss= -p "$1" 2>/dev/null); then
    printf '%s\n' "${out// /}"
  fi
}

# ck_median_p95: reads numbers, one a line, from standard input and prints
# "n=<count> median=<m> p95=<p>" (nothing but "n=0" when it reads none). The median of an
# even count is the mean of the two in the middle; the 95th percentile is the value at
# rank round(0.95 * (n - 1)) of the sorted list.
ck_median_p95() {
  sort -n | awk '
    NF { v[n++] = $1 }
    END {
      if (n == 0) { print "n=0"; exit }
      m = (n % 2) ? v[(n - 1) / 2] : (v[n / 2 - 1] + v[n / 2]) / 2
      r = int(0.95 * (n - 1) + 0.5)
      printf "n=%d median=%.2f p95=%.2f\n", n, m, v[r]
    }'
}

# ---------------------------------------------------------------------------------------
# The walker
# ---------------------------------------------------------------------------------------

CK_WORK=""
CK_MODE=walk
CK_TOTAL=0
CK_N=0
CK_ASK_VERDICT=""
CK_ASK_NOTE=""

# ck_begin: makes the walker's scratch folder, removed when the script ends.
ck_begin() {
  CK_WORK=$(mktemp -d "${TMPDIR:-/tmp}/yaar-checklist.XXXXXX") || ck_die "cannot make a scratch folder under ${TMPDIR:-/tmp}"
  # shellcheck disable=SC2064
  trap "rm -rf '$CK_WORK'" EXIT
  : > "$CK_WORK/rows.tsv"
  : > "$CK_WORK/context.txt"
  CK_N=0
}

# ck_context <label> <value>: one line of the file's header block.
ck_context() {
  printf -- '- %s: %s\n' "$1" "$2" >> "$CK_WORK/context.txt"
}

# ck_read_line <item id>: the next line of the answers, into CK_LINE.
CK_LINE=""
ck_read_line() {
  CK_LINE=""
  if ! IFS= read -r CK_LINE; then
    if [ -z "$CK_LINE" ]; then
      ck_die "the answers ended before $1 was answered"
    fi
  fi
}

# ck_read_text <item id> <prompt>: a line that is not empty, into CK_LINE.
ck_read_text() {
  while :; do
    printf '%s ' "$2"
    ck_read_line "$1"
    if [ -n "$CK_LINE" ]; then
      return 0
    fi
    echo "(it cannot be empty)" >&2
  done
}

# ck_item <id> <title> <what to do> <what is expected>: one item. In CK_MODE=count it only
# counts, so that the walk can say "item 3 of 17".
ck_item() {
  local id=$1 title=$2 todo=$3 expect=$4
  if [ "$CK_MODE" = count ]; then
    CK_TOTAL=$((CK_TOTAL + 1))
    return 0
  fi
  CK_N=$((CK_N + 1))
  if [ "$id" = "$CK_N" ]; then
    printf '\nItem %d of %d: %s\n' "$CK_N" "$CK_TOTAL" "$title"
  else
    printf '\nItem %d of %d: %s  %s\n' "$CK_N" "$CK_TOTAL" "$id" "$title"
  fi
  printf '  Do:       %s\n' "$todo"
  printf '  Expected: %s\n' "$expect"
  local verdict="" notes="" reply=""
  while [ -z "$verdict" ]; do
    printf 'yes / no / note / skip: '
    ck_read_line "item $CK_N of $CK_TOTAL ($id)"
    reply=$CK_LINE
    case $reply in
      yes | y) verdict=yes ;;
      no | n)
        verdict=no
        ck_read_text "item $CK_N of $CK_TOTAL ($id)" "What is wrong (a defect or a decision), one line:"
        notes="$notes${notes:+; }$CK_LINE"
        ;;
      skip)
        verdict=skipped
        ck_read_text "item $CK_N of $CK_TOTAL ($id)" "Why it was not walked, one line:"
        notes="$notes${notes:+; }$CK_LINE"
        ;;
      note)
        ck_read_text "item $CK_N of $CK_TOTAL ($id)" "Note, one line:"
        notes="$notes${notes:+; }$CK_LINE"
        ;;
      *) echo "Answer yes, no, note or skip." >&2 ;;
    esac
  done
  # A tab would split the row and a pipe would split a table cell.
  notes=$(printf '%s' "$notes" | tr '\t' ' ' | sed 's/|/\\|/g')
  printf '%s\t%s\t%s\t%s\n' "$id" "$title" "$verdict" "$notes" >> "$CK_WORK/rows.tsv"
}

# ck_walk <function that calls ck_item for each item>: counts, then walks.
ck_walk() {
  CK_MODE=count
  CK_TOTAL=0
  "$1"
  CK_MODE=walk
  CK_N=0
  : > "$CK_WORK/rows.tsv"
  "$1"
}

# ck_refused: the ids answered no or skip, comma separated; nothing when all are yes.
ck_refused() {
  awk -F'\t' '$3 != "yes" { printf "%s%s", (n++ ? ", " : ""), $1 } END { if (n) printf "\n" }' "$CK_WORK/rows.tsv"
}

# ck_signoff <statement>: after a walk with no "no" and no "skip", asks whether the
# maintainer signs <statement>; otherwise the sign-off is the list of items refused.
# The sign-off line is left in CK_SIGN.
CK_SIGN=""
ck_signoff() {
  local refused
  refused=$(ck_refused)
  if [ -n "$refused" ]; then
    CK_SIGN="Not signed. Items answered no or not walked: $refused."
    printf '\n%s\n' "$CK_SIGN"
    return 0
  fi
  printf '\nEvery item holds. Sign: "%s"? yes / no: ' "$1"
  ck_read_line "the sign-off"
  case $CK_LINE in
    yes | y) CK_SIGN="Signed: $1." ;;
    *)
      ck_read_text "the sign-off" "Why it is not signed, one line:"
      CK_SIGN="Not signed: $CK_LINE"
      ;;
  esac
}

# ck_write <file> <title> <sign-off line>: writes the Markdown file at once or not at all.
ck_write() {
  local out=$1 title=$2 sign=$3
  local tmp="$out.new.$$"
  {
    printf '# %s\n\n' "$title"
    cat "$CK_WORK/context.txt"
    printf '\n| Item | Title | Answer | Note |\n| --- | --- | --- | --- |\n'
    awk -F'\t' '{ printf "| %s | %s | %s | %s |\n", $1, $2, $3, $4 }' "$CK_WORK/rows.tsv"
    printf '\n## Sign-off\n\n%s\n' "$sign"
  } > "$tmp" || ck_die "cannot write $tmp"
  mv "$tmp" "$out" || ck_die "cannot move $tmp to $out"
}

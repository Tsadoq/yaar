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

# What the script leaves behind is removed by one trap on EXIT: the walker's scratch folder
# (CK_WORK), the folder of ck_tmpdir (CK_TMPDIR), and then the function a script names in
# CK_EXTRA_CLEANUP (the one place a script ends what it started). The exit status is the
# script's own.
CK_WORK=""
CK_TMPDIR=""
CK_EXTRA_CLEANUP=""

ck_cleanup() {
  if [ -n "$CK_WORK" ]; then
    rm -rf "$CK_WORK"
  fi
  if [ -n "$CK_TMPDIR" ]; then
    rm -rf "$CK_TMPDIR"
  fi
  if [ -n "$CK_EXTRA_CLEANUP" ]; then
    "$CK_EXTRA_CLEANUP"
  fi
}
trap ck_cleanup EXIT

# ck_tmpdir: this script's scratch folder, made on first use, under `mktemp -d`.
ck_tmpdir() {
  if [ -z "$CK_TMPDIR" ]; then
    CK_TMPDIR=$(mktemp -d "${TMPDIR:-/tmp}/yaar-checklist.XXXXXX") || ck_die "cannot make a scratch folder under ${TMPDIR:-/tmp}"
  fi
  printf '%s\n' "$CK_TMPDIR"
}

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
  if [ -z "${HOME:-}" ] || [ ! -d "$HOME" ]; then
    ck_die "HOME names no folder, so there is nowhere to write the results"
  fi
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
# matched by its full path and never by a name alone. On macOS `ps -o comm=` shows the path the
# program was started by (the one the system's `open` uses is absolute); on Linux
# /proc/<pid>/exe is the file with every link followed, so a path that goes through a link
# is not matched there.
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

# ck_mark_ms <file> <name>: the time of the first line `yaar-trace <name> epoch_ms=<n>` in a
# file the app wrote with YAAR_TRACE_START set; nothing when there is none.
ck_mark_ms() {
  sed -n "s/^yaar-trace $2 epoch_ms=\([0-9][0-9]*\)\$/\1/p" "$1" | head -n 1
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

CK_MODE=walk
CK_TOTAL=0
CK_N=0

# ck_begin: makes the walker's scratch folder, removed when the script ends (ck_cleanup).
ck_begin() {
  CK_WORK=$(mktemp -d "${TMPDIR:-/tmp}/yaar-checklist.XXXXXX") || ck_die "cannot make a scratch folder under ${TMPDIR:-/tmp}"
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

# ck_walker_context <dry-run 0|1> [macos|linux]: the date, the version and the system of this machine, as
# lines of the file's header. In a dry run nothing of the system or of the app is asked.
ck_walker_context() {
  ck_context "Date" "$(date +%Y-%m-%d)"
  if [ "$1" -eq 1 ]; then
    ck_context "Run" "dry run: nothing of the system or of the app was asked"
    ck_context "Version" "not asked: dry run"
    ck_context "System" "not asked: dry run"
    return 0
  fi
  local program version system product build pretty platform=${2:-}
  if [ -z "$platform" ]; then
    if [ "$(ck_os)" = Darwin ]; then platform=macos; else platform=linux; fi
  fi
  program=$(ck_window_program)
  [ -x "$program" ] || ck_die "no program at $program: install the app first, or name its Window program in YAAR_REVIEW"
  version=$("$program" --version) || ck_die "$program --version failed"
  ck_context "Version" "$version"
  if [ "$platform" = macos ]; then
    product=$(sw_vers -productVersion) || ck_die "sw_vers -productVersion failed"
    build=$(sw_vers -buildVersion) || ck_die "sw_vers -buildVersion failed"
    system="macOS $product ($build)"
  else
    [ -r /etc/os-release ] || ck_die "/etc/os-release cannot be read, so the system cannot be named"
    pretty=$(sed -n 's/^PRETTY_NAME="\{0,1\}\([^"]*\)"\{0,1\}$/\1/p' /etc/os-release | head -n 1)
    system="${pretty:-unknown} / session ${XDG_SESSION_TYPE:-unknown} / desktop ${XDG_CURRENT_DESKTOP:-unknown}"
  fi
  ck_context "System" "$system"
  ck_context "Program" "$program"
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

# ---------------------------------------------------------------------------------------
# The measuring kit on the Mac (typing.sh, scroll.sh, flood.sh)
# ---------------------------------------------------------------------------------------
# The kit is the one the phase 4 measures used, read and never rebuilt or edited:
#   fakeinput   posts real key and scroll events to the system, and logs the moment of each
#               post on the system's uptime clock in nanoseconds; it refuses to post unless
#               the target process is the frontmost one
#   adecap.app  logs the display time of every frame of one process's window whose pixels
#               changed, on the same clock, through the screen capture of the system
# Where they are: YAAR_KIT (default ~/ade-phase4-2026-11-12/gpui-measure), or YAAR_FAKEINPUT
# and YAAR_ADECAP outright.

ck_kit_dir() {
  printf '%s\n' "${YAAR_KIT:-$HOME/ade-phase4-2026-11-12/gpui-measure}"
}

ck_fakeinput() {
  printf '%s\n' "${YAAR_FAKEINPUT:-$(ck_kit_dir)/tools/fakeinput}"
}

ck_adecap() {
  printf '%s\n' "${YAAR_ADECAP:-$(ck_kit_dir)/tools/adecap.app}"
}

# ck_front_pid: the pid of the frontmost application.
ck_front_pid() {
  "$(ck_fakeinput)" info | sed -n 's/^front_pid=\([0-9]*\).*/\1/p'
}

# ck_lit_preflight <need capture: 0|1>: what a measure that posts input needs, each refusal
# one sentence: a Mac, the tools, mains power, an unlocked screen, the right to post
# events, and for a capture the right to record the screen.
ck_lit_preflight() {
  [ "$(ck_os)" = Darwin ] || ck_die "this measure runs on the Mac only"
  local fakein cap info
  fakein=$(ck_fakeinput)
  cap=$(ck_adecap)
  [ -x "$fakein" ] || ck_die "no fakeinput at $fakein: it is the phase 4 kit's tool, built once (YAAR_KIT or YAAR_FAKEINPUT names it)"
  if [ "$1" = 1 ]; then
    [ -d "$cap" ] || ck_die "no adecap.app at $cap: it is the phase 4 kit's tool (YAAR_KIT or YAAR_ADECAP names it)"
  fi
  # Each is read whole into a variable first: a pipe into `grep -q` ends it early, and with
  # pipefail the writer's broken pipe would turn a match into a failure.
  local power session
  power=$(pmset -g batt) || ck_die "pmset -g batt failed"
  case $power in
    *"AC Power"*) ;;
    *) ck_die "the Mac is not on mains power, which the measures need" ;;
  esac
  session=$(ioreg -n Root -d1 -a) || ck_die "ioreg could not read the session"
  session=$(printf '%s' "$session" | tr -d '\n\t ')
  case $session in
    *"CGSSessionScreenIsLocked</key><true/>"*) ck_die "the screen is locked, and a locked screen shows no frames and takes no input" ;;
  esac
  info=$("$fakein" info) || ck_die "$fakein info failed"
  printf '%s\n' "$info" | grep -q '^post_event_access=true$' \
    || ck_die "this terminal may not post events: turn on Accessibility for it in System Settings, Privacy and Security"
  printf '%s\n' "$info"
}

# ck_go <what will be posted>: nothing is posted before the owner types the word go.
ck_go() {
  printf '%s\nType go to start, anything else to stop: ' "$1"
  ck_read_line "the go"
  [ "$CK_LINE" = go ] || ck_die "you did not type go, so nothing was posted"
}

# ck_cap_start <log file> <pid> <stop file>: starts adecap on the process and waits until it
# says it runs.
ck_cap_start() {
  rm -f "$3"
  open -n -g "$(ck_adecap)" --args --pid "$2" --log "$1" --seconds 150 --stop "$3"
  capture_up() {
    grep -q '^start' "$1" 2> /dev/null || grep -q '^error\|^access=false' "$1" 2> /dev/null
  }
  ck_wait_until 10 capture_up "$1" || ck_die "adecap said nothing in 10 s ($1)"
  if grep -q '^error\|^access=false' "$1"; then
    ck_die "adecap cannot capture the window: $(grep '^error\|^access' "$1" | head -n 1) (Screen Recording is given to adecap.app, not to the terminal)"
  fi
}

# ck_cap_stop <log file> <stop file>: asks adecap to end and waits for its last line.
ck_cap_stop() {
  : > "$2"
  capture_ended() {
    grep -q '^end' "$1"
  }
  ck_wait_until 15 capture_ended "$1" || ck_die "adecap did not end in 15 s ($1)"
}

# ck_key_latencies <input log> <capture log>: one number a line, the milliseconds from each
# posted key to the first captured frame that changed after it. A key with no changed frame
# before the next key (plus 20 ms) or within 400 ms is counted as missed; a frame sooner than
# 12 ms after the post is not this key's (the system cannot show one so soon) and is counted
# as early. "missed=<n> early=<n>" goes to standard error.
ck_key_latencies() {
  local sorted
  sorted="$(ck_tmpdir)/caps.$$.$RANDOM"
  awk '$1 == "cap" { v = ($2 + 0 > 0) ? $2 : $3; print v }' "$2" | sort -n > "$sorted"
  awk -v caps="$sorted" '
    BEGIN {
      nc = 0
      while ((getline line < caps) > 0) { D[nc++] = line + 0 }
    }
    $1 == "post" { P[np++] = $2 + 0 }
    END {
      j = 0; missed = 0; early = 0
      for (i = 0; i < np; i++) {
        t = P[i]
        nxt = (i + 1 < np) ? P[i + 1] : t + 400000000
        while (j < nc && D[j] <= t) j++
        lim = nxt + 20000000
        if (t + 400000000 < lim) lim = t + 400000000
        if (j >= nc || D[j] >= lim) { missed++; continue }
        ms = (D[j] - t) / 1000000
        if (ms < 12) { early++; continue }
        printf "%.3f\n", ms
      }
      printf "missed=%d early=%d\n", missed, early > "/dev/stderr"
    }' "$1"
  rm -f "$sorted"
}

# ck_frame_stats <input log> <capture log> <refresh Hz>: for each scroll phase of the input
# log (a `sstart` line to the next `send` line), the changed frames the capture saw in it,
# and in total: "frames=<n> seconds=<s> fps=<f> long=<n> long_pct=<p>", where long counts
# the gaps between consecutive frames longer than two refresh intervals.
ck_frame_stats() {
  local sorted
  sorted="$(ck_tmpdir)/caps.$$.$RANDOM"
  awk '$1 == "cap" { v = ($2 + 0 > 0) ? $2 : $3; print v }' "$2" | sort -n > "$sorted"
  awk -v caps="$sorted" -v hz="$3" '
    BEGIN {
      nc = 0
      while ((getline line < caps) > 0) { D[nc++] = line + 0 }
      limit = 2 * 1000000000 / hz
    }
    $1 == "sstart" { start = $2 + 0; open = 1; next }
    $1 == "send" && open {
      end = $2 + 0; open = 0
      frames = 0; long = 0; prev = 0
      for (k = 0; k < nc; k++) {
        if (D[k] < start || D[k] > end) continue
        frames++
        if (prev > 0 && D[k] - prev > limit) long++
        prev = D[k]
      }
      secs = (end - start) / 1000000000
      printf "phase %d: frames=%d seconds=%.2f fps=%.1f long=%d long_pct=%.2f\n", ++ph, frames, secs, (secs > 0 ? frames / secs : 0), long, (frames > 1 ? 100 * long / (frames - 1) : 0)
      tf += frames; ts += secs; tl += long
    }
    END {
      printf "total: frames=%d seconds=%.2f fps=%.1f long=%d long_pct=%.2f\n", tf, ts, (ts > 0 ? tf / ts : 0), tl, (tf > ph ? 100 * tl / (tf - ph) : 0)
    }' "$1"
  rm -f "$sorted"
}

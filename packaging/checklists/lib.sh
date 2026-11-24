#!/bin/bash
# Sourced by the checklist scripts, never run; the caller sets `set -euo pipefail` and CK_NAME.
# Bash 3.2 (macOS): no associative arrays, mapfile, ${var,,}, empty-array expansion under `set -u`.
# Walker answers on stdin, one per line: yes|y; no|n or skip, then a non-empty line (what is wrong,
# why); note, then a note line (the item is asked again). Input ending early writes no file.

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  echo "lib.sh: it is sourced by the scripts beside it, not run" >&2
  exit 2
fi

CK_NAME=${CK_NAME:-lib.sh}

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

# Never call it inside $(...) or a pipeline: the subshell would skip the trap and leave the folder.
ck_tmpdir() {
  if [ -z "$CK_TMPDIR" ]; then
    CK_TMPDIR=$(mktemp -d "${TMPDIR:-/tmp}/yaar-checklist.XXXXXX") || ck_die "cannot make a scratch folder under ${TMPDIR:-/tmp}"
  fi
}

ck_die() {
  printf '%s: %s\n' "$CK_NAME" "$1" >&2
  exit 1
}

ck_usage_error() {
  printf '%s: %s (run it with --help)\n' "$CK_NAME" "$1" >&2
  exit 2
}

# Prints the script's first comment block: a checklist script's header is its --help text.
ck_help() {
  awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$1"
}

ck_os() {
  uname -s
}

# Reads all of lspci's output so pipefail never sees a closed pipe.
ck_graphics_line() {
  if ! command -v lspci > /dev/null; then
    printf '%s\n' "lspci is not installed"
    return 0
  fi
  local line
  line=$(lspci | awk 'tolower($0) ~ /vga|3d|display/ && !seen { print; seen = 1 }') || ck_die "lspci failed, so the graphics line is not known"
  printf '%s\n' "${line:-lspci shows no display controller}"
}

ck_results_dir() {
  if [ -z "${HOME:-}" ] || [ ! -d "$HOME" ]; then
    ck_die "HOME names no folder, so there is nowhere to write the results"
  fi
  local d
  d="$HOME/yaar-checklists/$(date +%Y-%m-%d)"
  mkdir -p "$d" || ck_die "cannot make the results folder $d"
  printf '%s\n' "$d"
}

ck_now_ms() {
  if [ "$(ck_os)" = Darwin ]; then
    /usr/bin/perl -MTime::HiRes=time -e 'printf "%.0f\n", time*1000'
  else
    date +%s%3N
  fi
}

# The command must return a status, not rely on `set -e`.
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

# macOS: `ps -o comm=` shows the path used to start it. Linux: /proc/<pid>/exe has links resolved,
# so a path through a link does not match.
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

ck_new_pids_of() {
  local known pid
  known=" $(printf '%s' "$2" | tr '\n' ' ') "
  for pid in $(ck_pids_of "$1"); do
    case $known in
      *" $pid "*) ;;
      *) printf '%s\n' "$pid" ;;
    esac
  done
}

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

ck_rss_kb() {
  local out
  if out=$(ps -o rss= -p "$1" 2>/dev/null); then
    printf '%s\n' "${out// /}"
  fi
}

# Marks are in a file written with YAAR_TRACE_START set.
ck_mark_ms() {
  sed -n "s/^yaar-trace $2 epoch_ms=\([0-9][0-9]*\)\$/\1/p" "$1" | head -n 1
}

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

CK_MODE=walk
CK_TOTAL=0
CK_N=0

ck_begin() {
  CK_WORK=$(mktemp -d "${TMPDIR:-/tmp}/yaar-checklist.XXXXXX") || ck_die "cannot make a scratch folder under ${TMPDIR:-/tmp}"
  : > "$CK_WORK/rows.tsv"
  : > "$CK_WORK/context.txt"
  CK_N=0
}

ck_context() {
  printf -- '- %s: %s\n' "$1" "$2" >> "$CK_WORK/context.txt"
}

CK_LINE=""
ck_read_line() {
  CK_LINE=""
  if ! IFS= read -r CK_LINE; then
    if [ -z "$CK_LINE" ]; then
      ck_die "the answers ended before $1 was answered"
    fi
  fi
}

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

ck_walk() {
  CK_MODE=count
  CK_TOTAL=0
  "$1"
  CK_MODE=walk
  CK_N=0
  : > "$CK_WORK/rows.tsv"
  "$1"
}

ck_refused() {
  awk -F'\t' '$3 != "yes" { printf "%s%s", (n++ ? ", " : ""), $1 } END { if (n) printf "\n" }' "$CK_WORK/rows.tsv"
}

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

ck_write() {
  local out=$1 title=$2 sign=$3
  local tmp="$out.new.$$"
  {
    printf '# %s\n\n' "$title"
    cat "$CK_WORK/context.txt"
    printf '\n| Item | Title | Answer | Note |\n| --- | --- | --- | --- |\n'
    awk -F'\t' '{ printf "| %s | %s | %s | %s |\n", $1, $2, $3, $4 }' "$CK_WORK/rows.tsv"
    printf '\n## Sign-off\n\n%s\n' "$sign"
  } > "$tmp" || {
    rm -f "$tmp"
    ck_die "cannot write $tmp"
  }
  mv "$tmp" "$out" || {
    rm -f "$tmp"
    ck_die "cannot move $tmp to $out"
  }
}

# fakeinput posts real key and scroll events and logs each post in ns on the system uptime clock;
# it refuses unless the target process is frontmost. adecap.app logs the display time of every
# changed frame of one process's window on the same clock.

# No kit tool (fakeinput, adecap.app, stream.pl) has a default place: each is named by its own
# variable or found as tools/<name> under YAAR_KIT; none found stops with the variable to set.
# Called inside $(...), the stop exits a subshell and the caller's set -e ends the script.
ck_kit_tool() {
  local given=$1 variable=$2 name=$3 what=$4
  if [ -n "$given" ]; then
    printf '%s\n' "$given"
  elif [ -n "${YAAR_KIT:-}" ]; then
    printf '%s\n' "$YAAR_KIT/tools/$name"
  else
    ck_die "no $name is named: set $variable to $what, or set YAAR_KIT to a folder that holds it as tools/$name"
  fi
}

ck_fakeinput() {
  ck_kit_tool "${YAAR_FAKEINPUT:-}" YAAR_FAKEINPUT fakeinput "the fakeinput program, which posts key and scroll events and logs each post"
}

ck_adecap() {
  ck_kit_tool "${YAAR_ADECAP:-}" YAAR_ADECAP adecap.app "the adecap.app screen capture, which logs the display time of every changed frame of one window"
}

ck_stream_script() {
  ck_kit_tool "${YAAR_STREAM:-}" YAAR_STREAM stream.pl "a perl script that writes a steady stream of output lines"
}

ck_front_pid() {
  "$(ck_fakeinput)" info | sed -n 's/^front_pid=\([0-9]*\).*/\1/p'
}

ck_lit_preflight() {
  [ "$(ck_os)" = Darwin ] || ck_die "this measure runs on the Mac only"
  local fakein cap info
  fakein=$(ck_fakeinput)
  [ -x "$fakein" ] || ck_die "no fakeinput program at $fakein: build it, and name it in YAAR_FAKEINPUT (or its folder in YAAR_KIT)"
  if [ "$1" = 1 ]; then
    cap=$(ck_adecap)
    [ -d "$cap" ] || ck_die "no adecap.app at $cap: name it in YAAR_ADECAP (or its folder in YAAR_KIT)"
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
  case "
$info
" in
    *"
post_event_access=true
"*) ;;
    *) ck_die "this terminal may not post events: turn on Accessibility for it in System Settings, Privacy and Security" ;;
  esac
  printf '%s\n' "$info"
}

ck_go() {
  printf '%s\nType go to start, anything else to stop: ' "$1"
  ck_read_line "the go"
  [ "$CK_LINE" = go ] || ck_die "you did not type go, so nothing was posted"
}

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

ck_cap_stop() {
  : > "$2"
  capture_ended() {
    grep -q '^end' "$1"
  }
  ck_wait_until 15 capture_ended "$1" || ck_die "adecap did not end in 15 s ($1)"
}

# A missing display time (0) is replaced by the time the capture saw the frame.
ck_display_times() {
  awk '$1 == "cap" { v = ($2 + 0 > 0) ? $2 : $3; print v }' "$1" | sort -n
}

# Missed: no changed frame before the next key plus 20 ms, or within 400 ms. Early: a frame under
# 12 ms after the post, which the system cannot show that soon. missed and early go to stderr.
ck_key_latencies() {
  awk '
    FILENAME == ARGV[1] { D[nc++] = $1 + 0; next }
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
    }' <(ck_display_times "$2") "$1"
}

# long: a gap between consecutive frames over two refresh intervals.
ck_frame_stats() {
  awk -v hz="$3" '
    BEGIN { limit = 2 * 1000000000 / hz }
    FILENAME == ARGV[1] { D[nc++] = $1 + 0; next }
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
    }' <(ck_display_times "$2") "$1"
}

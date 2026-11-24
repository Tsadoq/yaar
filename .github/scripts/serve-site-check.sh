#!/bin/sh
# Shows what serve-site.sh must do when the machine it runs on is not as quick or as
# tidy as the one it was written on. The install jobs rest on it: a server that does not
# answer in time, or answers "not modified" to a site that changed, fails a job for a reason
# that has nothing to do with the app.
#
#   .github/scripts/serve-site-check.sh
#
# Needs what serve-site.sh needs (python3, curl, ps), on Linux or macOS. It serves fake
# files from 127.0.0.1 on a port the system hands it, makes its folders in a scratch folder
# (RUNNER_TEMP and TMPDIR of the runs are in it), and reads nothing of the user's. Three
# cases, each on its own port; the first two stand in for a python3 that is slow, by a
# sitecustomize.py on PYTHONPATH of the server's own process:
#   1. The reverse lookup of 127.0.0.1 takes 15 s (what python3's own server asks the
#      resolver at its bind, and what GitHub's Mac answered slowly once): `start` must still
#      serve within its 10 s, and a GET of /install.sh and of /download/ must answer 200
#      with the files' bytes.
#   2. The server's process hangs before it binds: `start` must fail, in its 10 s, with the
#      sentence "did not answer 200 within 10 s" and the server's log, and the process it
#      started must be gone when it returns, so that nothing holds the port for the steps
#      after.
#   3. The site is switched to one whose files are older than the ones served before (a new
#      deployment need not be newer to the second): a GET with the Last-Modified of the first
#      answer as If-Modified-Since must answer 200 with the new bytes, not 304, since apt
#      sends exactly that header and a 304 keeps the old index of the repository.
# Controls: the sitecustomize really slows the lookup and really hangs (read in a python3
# of its own, before the cases), and case 3 has a first answer whose Last-Modified is a
# real date.
#
# Exit codes: 0 holds; 1 does not.
#
# What the header leaves: a process of the cases is found by the process id that the
# sitecustomize writes to a file before it does anything else, never by name; the processes
# of a failed case are ended at the end, whatever happens, and the scratch folder removed.
set -eu

NAME=serve-site-check.sh
HERE=$(cd "$(dirname "$0")" && pwd)
SERVE=$HERE/serve-site.sh
SAID=0
SCRATCH=
PORTS=
fail() {
  SAID=1
  echo "$NAME: $*" >&2
  exit 1
}
cleanup() {
  status=$?
  trap - EXIT
  set +e
  if [ "$status" -ne 0 ] && [ "$SAID" -eq 0 ]; then
    echo "$NAME: a command failed with status $status" >&2
    status=1
  fi
  if [ -n "$SCRATCH" ]; then
    for port in $PORTS; do "$SERVE" stop "$port" > /dev/null 2>&1; done
    if [ -f "$SCRATCH/pids" ]; then
      while read -r pid; do kill "$pid" 2> /dev/null; done < "$SCRATCH/pids"
    fi
    rm -rf "$SCRATCH"
  fi
  exit "$status"
}
trap cleanup EXIT

for tool in python3 curl ps; do
  command -v "$tool" > /dev/null 2>&1 || fail "$tool is not installed"
done
SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/serve-site-check.XXXXXX") || fail "no scratch folder could be made"
mkdir "$SCRATCH/tmp" "$SCRATCH/slow" || fail "the folders of the scratch could not be made"
export TMPDIR="$SCRATCH/tmp" RUNNER_TEMP="$SCRATCH/tmp"

cat > "$SCRATCH/slow/sitecustomize.py" << 'PY'
import os
import socket
import time

# The first thing: say whose process this is, so that the check can tell it is gone.
pids = os.environ.get("YAAR_CHECK_PIDS")
if pids:
    with open(pids, "a") as f:
        f.write("%d\n" % os.getpid())
hang = float(os.environ.get("YAAR_CHECK_HANG", "0"))
if hang:
    time.sleep(hang)
delay = float(os.environ.get("YAAR_CHECK_LOOKUP_DELAY", "0"))
if delay:
    def gethostbyaddr(*args):
        time.sleep(delay)
        raise socket.herror(1, "no name, after a long wait")

    socket.gethostbyaddr = gethostbyaddr
PY

free_port() {
  python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
}

# True while the process is there and is not a zombie.
alive() {
  kill -0 "$1" 2> /dev/null || return 1
  state=$(ps -o stat= -p "$1" 2> /dev/null) || return 1
  case $state in
    '' | Z*) return 1 ;;
  esac
  return 0
}

fresh_site() { # <name> <mtime as touch -t> <text>: a site folder with the files the cases ask for
  mkdir -p "$SCRATCH/$1/apt/dists/stable"
  printf '%s\n' "$3" > "$SCRATCH/$1/install.sh"
  printf '%s\n' "$3" > "$SCRATCH/$1/apt/dists/stable/InRelease"
  touch -t "$2" "$SCRATCH/$1/install.sh" "$SCRATCH/$1/apt/dists/stable/InRelease"
}

fresh_site site-a 202601020000 "the first site"
fresh_site site-b 202601010000 "the second site, older by a day"
mkdir "$SCRATCH/release"
printf 'a package\n' > "$SCRATCH/release/package.txt"
touch -t 202601010000 "$SCRATCH/release/package.txt"
printf 'a sum\n' > "$SCRATCH/release/SHA256SUMS"

# --- The controls: the stand-in is what the cases say it is.
[ "$(YAAR_CHECK_LOOKUP_DELAY=15 PYTHONPATH="$SCRATCH/slow" python3 -c 'import socket; print(socket.gethostbyaddr.__module__)')" = sitecustomize ] \
  || fail "control: the sitecustomize did not reach the python3 it was put in front of"
started=$(date +%s)
YAAR_CHECK_LOOKUP_DELAY=2 PYTHONPATH="$SCRATCH/slow" python3 -c 'import socket
try:
    socket.gethostbyaddr("127.0.0.1")
except OSError:
    pass'
[ "$(($(date +%s) - started))" -ge 2 ] || fail "control: a reverse lookup of 127.0.0.1 with the delay set did not wait"
started=$(date +%s)
YAAR_CHECK_HANG=2 PYTHONPATH="$SCRATCH/slow" python3 -c 'pass'
[ "$(($(date +%s) - started))" -ge 2 ] || fail "control: a python3 with the hang set did not wait"

# --- 1. A slow reverse lookup does not keep the server from starting.
port=$(free_port)
PORTS="$PORTS $port"
: > "$SCRATCH/pids"
began=$(date +%s)
if ! YAAR_CHECK_PIDS="$SCRATCH/pids" YAAR_CHECK_LOOKUP_DELAY=15 PYTHONPATH="$SCRATCH/slow" \
  "$SERVE" start "$SCRATCH/site-a" "$SCRATCH/release" "$port" > "$SCRATCH/out" 2>&1; then
  sed 's/^/    | /' "$SCRATCH/out" >&2
  fail "case 1: start did not serve when the reverse lookup of 127.0.0.1 takes 15 s (it gave up after $(($(date +%s) - began)) s)"
fi
[ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$port/install.sh")" = 200 ] || fail "case 1: GET /install.sh did not answer 200"
[ "$(curl -s "http://127.0.0.1:$port/download/package.txt")" = "a package" ] || fail "case 1: GET /download/package.txt did not answer with the file"
"$SERVE" stop "$port" > /dev/null || fail "case 1: the server could not be stopped"
echo "$NAME: case 1: the server serves within 10 s though the reverse lookup of 127.0.0.1 takes 15 s"

# --- 2. A server that cannot start is ended by start, whose sentence says so.
port=$(free_port)
PORTS="$PORTS $port"
: > "$SCRATCH/pids"
if YAAR_CHECK_PIDS="$SCRATCH/pids" YAAR_CHECK_HANG=60 PYTHONPATH="$SCRATCH/slow" \
  "$SERVE" start "$SCRATCH/site-a" "$SCRATCH/release" "$port" > "$SCRATCH/out" 2>&1; then
  fail "case 2: start succeeded with a server that hangs before it binds"
fi
sed 's/^/    | /' "$SCRATCH/out"
grep -qF "did not answer 200 within 10 s" "$SCRATCH/out" || fail "case 2: start did not say \"did not answer 200 within 10 s\""
[ -s "$SCRATCH/pids" ] || fail "case 2: the stand-in wrote no process id: the server was never started"
deadline=$(($(date +%s) + 5))
while read -r pid; do
  while alive "$pid"; do
    [ "$(date +%s)" -lt "$deadline" ] || fail "case 2: start failed and left the server it started running, pid $pid"
    sleep 0.2
  done
done < "$SCRATCH/pids"
echo "$NAME: case 2: a server that cannot start is ended by start, which says why"

# --- 3. A site older than the one before it is served whole to a client that has the first.
port=$(free_port)
PORTS="$PORTS $port"
"$SERVE" start "$SCRATCH/site-a" "$SCRATCH/release" "$port" > /dev/null || fail "case 3: start did not serve"
url="http://127.0.0.1:$port/apt/dists/stable/InRelease"
curl -s -D "$SCRATCH/headers" -o "$SCRATCH/first" "$url" || fail "case 3: the first GET failed"
modified=$(sed -n 's/^[Ll]ast-[Mm]odified: *//p' "$SCRATCH/headers" | tr -d '\r')
[ -n "$modified" ] || fail "case 3: the first answer has no Last-Modified"
"$SERVE" switch "$SCRATCH/site-b" "$port" > /dev/null || fail "case 3: switch did not point the server at the second site"
code=$(curl -s -o "$SCRATCH/second" -w '%{http_code}' -H "If-Modified-Since: $modified" "$url") || fail "case 3: the second GET failed"
[ "$code" = 200 ] || fail "case 3: a GET after the site changed, with If-Modified-Since: $modified, answered $code and not 200"
[ "$(cat "$SCRATCH/second")" = "the second site, older by a day" ] || fail "case 3: the answer after the switch is not the second site's file: $(cat "$SCRATCH/second")"
code=$(curl -s -o /dev/null -w '%{http_code}' -H "If-Modified-Since: $modified" "http://127.0.0.1:$port/download/package.txt") || fail "case 3: the GET of a release file failed"
[ "$code" = 200 ] || fail "case 3: a GET of a release file with If-Modified-Since: $modified answered $code and not 200"
"$SERVE" stop "$port" > /dev/null || fail "case 3: the server could not be stopped"
echo "$NAME: case 3: a changed site is served whole to a client that holds the one before"

echo "$NAME: serve-site.sh holds in all three cases"

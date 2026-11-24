#!/bin/sh
# Serves a release as GitHub Pages would, on 127.0.0.1: / is the site, /download/ the release files.
# serve-site.sh start <site-dir> <release-dir> <port> | switch <site-dir> <port> | stop <port>
# The server is fully detached (setsid or nohup, stdin from /dev/null) or a container's step hangs.
set -eu

NAME=serve-site.sh
SAID=0
on_exit() {
  if [ "$1" -ne 0 ] && [ "$SAID" -eq 0 ]; then
    echo "$NAME: a command failed with status $1" >&2
    exit 1
  fi
}
trap 'on_exit "$?"' EXIT
fail() {
  SAID=1
  echo "$NAME: $*" >&2
  exit 1
}
usage() {
  SAID=1
  echo "usage: $NAME start <site-dir> <release-dir> <port> | switch <site-dir> <port> | stop <port>" >&2
  exit 2
}

check_port() {
  case $1 in
    '' | *[!0-9]*)
      SAID=1
      echo "$NAME: the port is a number, not: $1" >&2
      exit 2
      ;;
  esac
  if [ "$1" -lt 1 ] || [ "$1" -gt 65535 ]; then
    SAID=1
    echo "$NAME: the port is between 1 and 65535, not: $1" >&2
    exit 2
  fi
}

check_dir() {
  [ -d "$1" ] || fail "$1 is not a folder"
}

served_dir() {
  echo "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/served-$1"
}

# True while the process is there and is not a zombie.
alive() {
  [ -n "$1" ] || return 1
  kill -0 "$1" 2> /dev/null || return 1
  state=$(ps -o stat= -p "$1" 2> /dev/null) || return 1
  case $state in
    '' | Z*) return 1 ;;
  esac
  return 0
}

print_log() {
  if [ -f "$1/log" ]; then
    echo "$NAME: the server's log, $1/log:" >&2
    sed 's/^/  /' "$1/log" >&2
  fi
}

write_handler() {
  cat > "$1" << 'PY'
import http.server
import os
import posixpath
import socketserver
import sys
import urllib.parse

port = int(sys.argv[1])
served = sys.argv[2]
site = os.path.join(served, "root")
download = os.path.join(served, "root-download")

class Handler(http.server.SimpleHTTPRequestHandler):
    def send_head(self):
        # Every request is answered whole. The parent answers 304 to a client whose
        # If-Modified-Since is not older than the file, to the second, and a site that is
        # switched to files no newer than the last one's would then keep the old index in
        # apt's lists.
        del self.headers["If-Modified-Since"]
        return super().send_head()

    def translate_path(self, path):
        path = urllib.parse.unquote(urllib.parse.urlsplit(path).path)
        parts = [p for p in posixpath.normpath(path).split("/") if p not in ("", ".", "..")]
        if parts and parts[0] == "download":
            return os.path.join(download, *parts[1:])
        return os.path.join(site, *parts)

class Server(http.server.ThreadingHTTPServer):
    def server_bind(self):
        # HTTPServer's own bind asks the resolver for the name of the address it bound
        # (socket.getfqdn), which nothing here uses and which can take longer than the
        # wait of `start`.
        socketserver.TCPServer.server_bind(self)
        host, port = self.server_address[:2]
        self.server_name = host
        self.server_port = port

try:
    os.setsid()
except OSError:
    pass
server = Server(("127.0.0.1", port), Handler)
tmp = os.path.join(served, "pid.new")
with open(tmp, "w") as f:
    f.write("%d\n" % os.getpid())
os.rename(tmp, os.path.join(served, "pid"))
server.serve_forever()
PY
}

end_started() {
  kill "$started" 2> /dev/null || return 0
  gone=$(($(date +%s) + 5))
  while alive "$started"; do
    if [ "$(date +%s)" -ge "$gone" ]; then
      kill -9 "$started" 2> /dev/null || true
      break
    fi
    sleep 0.2
  done
  wait "$started" 2> /dev/null || true
}

start() {
  [ $# -eq 3 ] || usage
  check_port "$3"
  check_dir "$1"
  check_dir "$2"
  command -v python3 > /dev/null 2>&1 || fail "python3 is not installed"
  command -v curl > /dev/null 2>&1 || fail "curl is not installed"
  site=$(cd -P "$1" && pwd) || fail "cannot enter $1"
  release=$(cd -P "$2" && pwd) || fail "cannot enter $2"
  port=$3
  dir=$(served_dir "$port")
  if [ -f "$dir/pid" ]; then
    old=$(cat "$dir/pid") || fail "cannot read $dir/pid"
    if alive "$old"; then
      fail "a server of this script already runs on port $port, pid $old: stop it first"
    fi
  fi
  rm -rf "$dir" || fail "cannot remove $dir"
  mkdir -p "$dir/root-download" || fail "cannot make $dir/root-download"
  ln -s "$site" "$dir/root" || fail "cannot link $site as $dir/root"
  for f in "$release"/*; do
    [ -e "$f" ] || continue
    ln -s "$f" "$dir/root-download/$(basename "$f")" || fail "cannot link $f into $dir/root-download"
  done
  write_handler "$dir/serve.py" || fail "cannot write $dir/serve.py"
  if command -v setsid > /dev/null 2>&1; then
    setsid python3 "$dir/serve.py" "$port" "$dir" < /dev/null > "$dir/log" 2>&1 &
  else
    nohup python3 "$dir/serve.py" "$port" "$dir" < /dev/null > "$dir/log" 2>&1 &
  fi
  started=$!
  deadline=$(($(date +%s) + 10))
  code=000
  while :; do
    if [ -s "$dir/pid" ]; then
      code=$(curl -s --max-time 3 -o /dev/null -w '%{http_code}' "http://127.0.0.1:$port/install.sh") || code=000
      [ "$code" = 200 ] && break
    elif ! kill -0 "$started" 2> /dev/null; then
      print_log "$dir"
      fail "the server on port $port ended before it served"
    fi
    if [ "$(date +%s)" -ge "$deadline" ]; then
      print_log "$dir"
      if [ -s "$dir/pid" ]; then written="the server wrote its process id"; else written="the server wrote no process id"; fi
      end_started
      fail "GET http://127.0.0.1:$port/install.sh did not answer 200 within 10 s, the last answer was $code ($written; the process was ended)"
    fi
    sleep 0.2
  done
  echo "$NAME: serving $site and $release on http://127.0.0.1:$port, pid $(cat "$dir/pid" || echo unknown)"
}

switch() {
  [ $# -eq 2 ] || usage
  check_port "$2"
  check_dir "$1"
  site=$(cd -P "$1" && pwd) || fail "cannot enter $1"
  dir=$(served_dir "$2")
  [ -L "$dir/root" ] || fail "no server of this script on port $2: $dir/root is missing"
  ln -sfn "$site" "$dir/root" || fail "cannot point $dir/root at $site"
  echo "$NAME: port $2 now serves $site"
}

stop() {
  [ $# -eq 1 ] || usage
  check_port "$1"
  dir=$(served_dir "$1")
  [ -f "$dir/pid" ] || fail "no server of this script on port $1: $dir/pid is missing"
  pid=$(cat "$dir/pid") || fail "cannot read $dir/pid"
  if alive "$pid"; then
    kill "$pid" 2> /dev/null || fail "cannot signal the server of port $1, pid $pid"
    deadline=$(($(date +%s) + 10))
    while alive "$pid"; do
      if [ "$(date +%s)" -ge "$deadline" ]; then
        fail "the server of port $1, pid $pid, still runs 10 s after SIGTERM"
      fi
      sleep 0.2
    done
  fi
  echo "$NAME: the server of port $1 has ended"
}

[ $# -ge 1 ] || usage
action=$1
shift
case $action in
  start) start "$@" ;;
  switch) switch "$@" ;;
  stop) stop "$@" ;;
  *) usage ;;
esac

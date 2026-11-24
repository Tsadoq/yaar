#!/bin/bash
# apt refuses a repository whose index or package changed after signing; run on a fresh client.
# .github/scripts/linux-tampered.sh <rehearsal-dir>: altered copies of its site on 8001, 8002, 8003.
# apt 3 words a changed signed text as "Message has been manipulated".
set -euo pipefail

NAME=linux-tampered.sh
SAID=0
fail() {
  SAID=1
  echo "$NAME: $*" >&2
  exit 1
}

if [ $# -ne 1 ]; then
  echo "usage: $NAME <rehearsal-dir>" >&2
  exit 2
fi
REHEARSAL=$1
if [ ! -d "$REHEARSAL/site" ] || [ ! -d "$REHEARSAL/release" ]; then
  echo "$NAME: $REHEARSAL holds no site/ and release/" >&2
  exit 2
fi
REHEARSAL=$(cd "$REHEARSAL" && pwd)
HERE=$(cd "$(dirname "$0")" && pwd)
TESTKEY=/etc/apt/keyrings/yaar-test.asc
TESTSOURCE=/etc/apt/sources.list.d/yaar-test.sources
KEY=/etc/apt/keyrings/yaar-review.asc
SOURCE=/etc/apt/sources.list.d/yaar-review.sources
export DEBIAN_FRONTEND=noninteractive

SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/yaar-tampered.XXXXXX")
SERVING=()
cleanup() {
  local status=$1 cmd=$2 port
  if [ "$status" -ne 0 ] && [ "$SAID" -eq 0 ]; then
    echo "$NAME: '$cmd' failed with status $status" >&2
    status=1
  fi
  rm -f "$TESTKEY" "$TESTSOURCE"
  for port in 8000 8001 8002 8003; do
    rm -rf /var/lib/apt/lists/127.0.0.1:"$port"_apt_*
  done
  for port in ${SERVING[@]+"${SERVING[@]}"}; do
    "$HERE/serve-site.sh" stop "$port" || echo "$NAME: the server of port $port could not be stopped" >&2
  done
  rm -rf "$SCRATCH"
  exit "$status"
}
trap 'cleanup "$?" "$BASH_COMMAND"' EXIT

if dpkg-query -W yaar-review > /dev/null 2>&1; then
  fail "yaar-review is installed already: this step runs on a client that has nothing of the app"
fi
if [ -e "$KEY" ] || [ -e "$SOURCE" ]; then
  fail "$KEY or $SOURCE is there already: this step runs on a client that has nothing of the app"
fi

cp -a "$REHEARSAL/site" "$SCRATCH/index"
FOUND=0
for packages in "$SCRATCH"/index/apt/dists/stable/main/binary-*/Packages; do
  [ -f "$packages" ] || fail "$packages is missing from the site"
  grep -q '^Section: devel$' "$packages" || fail "$packages holds no line 'Section: devel' to change"
  sed -i 's/^Section: devel$/Section: devil/' "$packages"
  gzip -n -9 -c "$packages" > "$packages.gz"
  FOUND=$((FOUND + 1))
done
[ "$FOUND" -ge 1 ] || fail "$REHEARSAL/site holds no Packages file"

cp -a "$REHEARSAL/site" "$SCRATCH/signed"
for f in InRelease Release; do
  path=$SCRATCH/signed/apt/dists/stable/$f
  [ -f "$path" ] || fail "$path is missing from the site"
  grep -q '^Suite: stable$' "$path" || fail "$path holds no line 'Suite: stable' to change"
  sed -i '0,/^Suite: stable$/s//Suite: stabla/' "$path"
done

cp -a "$REHEARSAL/site" "$SCRATCH/package"
DEBS=$(find "$SCRATCH/package/apt/pool" -type f -name '*.deb' | LC_ALL=C sort)
[ -n "$DEBS" ] || fail "$REHEARSAL/site holds no .deb in apt/pool"
while IFS= read -r deb; do
  size=$(wc -c < "$deb" | tr -d ' ')
  last=$(tail -c 1 "$deb" | od -An -tu1 | tr -d ' ')
  new=$((last ^ 1))
  printf '%b' "\\0$(printf '%o' "$new")" | dd of="$deb" bs=1 seek=$((size - 1)) conv=notrunc 2> /dev/null
  [ "$(wc -c < "$deb" | tr -d ' ')" -eq "$size" ] || fail "$deb changed its length"
  [ "$(tail -c 1 "$deb" | od -An -tu1 | tr -d ' ')" -eq "$new" ] || fail "the last byte of $deb was not changed"
done <<< "$DEBS"

for pair in index:8001 signed:8002 package:8003; do
  copy=${pair%%:*}
  port=${pair##*:}
  "$HERE/serve-site.sh" start "$SCRATCH/$copy" "$REHEARSAL/release" "$port"
  SERVING+=("$port")
done

install_line() {
  sudo -n -u "$1" env YAAR_BASE_URL="http://127.0.0.1:$2" \
    YAAR_DOWNLOAD_URL=http://127.0.0.1:8000/download \
    bash -c 'set -o pipefail; curl -fsSL "$YAAR_BASE_URL/install.sh" | bash'
}

refused_by_install_line() {
  local copy=$1 port=$2 status=0 saw="" want
  shift 2
  install_line tester "$port" > "$SCRATCH/$copy.out" 2>&1 || status=$?
  sed "s|^|$NAME: [$copy] |" "$SCRATCH/$copy.out"
  [ "$status" -ne 0 ] || fail "the install line succeeded against the $copy copy: apt accepted a repository that was changed after signing"
  for want in "$@"; do
    if grep -q -F -e "$want" "$SCRATCH/$copy.out"; then saw=$want; fi
  done
  [ -n "$saw" ] || fail "the install line failed against the $copy copy (status $status) but its output holds none of: $*"
  if dpkg-query -W yaar-review > /dev/null 2>&1; then
    fail "yaar-review is installed after the install line was refused on the $copy copy"
  fi
  [ ! -e "$KEY" ] || fail "$KEY is left after the install line was refused on the $copy copy"
  [ ! -e "$SOURCE" ] || fail "$SOURCE is left after the install line was refused on the $copy copy"
  echo "$NAME: the $copy copy was refused (status $status, '$saw'), and nothing was left"
}

refused_by_install_line index 8001 "File has unexpected size" "Hash Sum mismatch"
refused_by_install_line signed 8002 "NOSPLIT" "BADSIG" "signatures were invalid" "Message has been manipulated" "OpenPGP signature verification failed"
refused_by_install_line package 8003 "Hash Sum mismatch"

# Root writes a key and a source of the test's own and updates that source only.
test_source() {
  local port=$1
  mkdir -p /etc/apt/keyrings
  cp "$REHEARSAL/site/apt/yaar-review.asc" "$TESTKEY"
  chmod 644 "$TESTKEY"
  cat > "$TESTSOURCE" << SRC
Types: deb
URIs: http://127.0.0.1:$port/apt
Suites: stable
Components: main
Signed-By: $TESTKEY
SRC
}
update_test_source() {
  apt-get update -o Dir::Etc::sourcelist="$TESTSOURCE" -o Dir::Etc::sourceparts=- 2>&1
}
clean_test_source() {
  rm -f "$TESTKEY" "$TESTSOURCE"
  rm -rf /var/lib/apt/lists/127.0.0.1:"$1"_apt_*
}

for pair in index:8001 signed:8002; do
  copy=${pair%%:*}
  port=${pair##*:}
  test_source "$port"
  status=0
  update_test_source > "$SCRATCH/$copy.update" || status=$?
  sed "s|^|$NAME: [$copy apt-get update] |" "$SCRATCH/$copy.update"
  clean_test_source "$port"
  [ "$status" -eq 100 ] || fail "apt-get update on the $copy copy alone exited with $status, not 100: apt did not refuse it"
  grep -q -e '^E:' -e '^W:' "$SCRATCH/$copy.update" || fail "apt-get update on the $copy copy alone exited with 100 but said nothing"
  echo "$NAME: apt itself refused the $copy copy (apt-get update exit 100)"
done

test_source 8000
status=0
update_test_source > "$SCRATCH/control.update" || status=$?
sed "s|^|$NAME: [control apt-get update] |" "$SCRATCH/control.update"
clean_test_source 8000
[ "$status" -eq 0 ] || fail "apt-get update on the unchanged site exited with $status: the control does not pass"
if grep -E '^(W|E):' "$SCRATCH/control.update" > /dev/null; then
  fail "apt-get update on the unchanged site printed a W: or E: line: the control does not pass"
fi
grep -q 'yaar-test\|127.0.0.1:8000' "$SCRATCH/control.update" || fail "apt-get update on the unchanged site never named the site: the control did not reach it"
echo "$NAME: the unchanged site updated with no W: and no E:"

for port in 8001 8002 8003; do
  "$HERE/serve-site.sh" stop "$port"
done
SERVING=()
echo "$NAME: every copy was refused, the control passed"

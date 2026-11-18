#!/bin/sh
# Fails unless the app's own source is under the Apache License 2.0, and under nothing
# else: every crate of the workspace declares exactly "Apache-2.0", and the file LICENSE
# at the root is the text of that licence.
#
#   own-licence.sh
#
# `cargo deny check licenses` does not hold this: it would pass a crate of the app under
# any licence of its list. The crates are those cargo finds in the workspace, so a new
# crate is checked without being named anywhere.
#
# Needs jq.
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"

# The text as the Apache Software Foundation publishes it, 11,358 bytes:
# https://www.apache.org/licenses/LICENSE-2.0.txt
APACHE_2_0_SHA256=cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30

failed=0

metadata=$(cargo metadata --format-version 1 --no-deps --locked --offline)
crates=$(printf '%s\n' "$metadata" | jq -r '.packages | length')
others=$(printf '%s\n' "$metadata" \
  | jq -r '.packages[] | select(.license != "Apache-2.0") | "  \(.name): \(.license // "no licence declared")"')
if [ "$crates" -lt 1 ]; then
  echo "own-licence.sh: cargo finds no crate in the workspace" >&2
  failed=1
elif [ -n "$others" ]; then
  echo "own-licence.sh: these crates of the workspace do not declare Apache-2.0:" >&2
  printf '%s\n' "$others" >&2
  failed=1
else
  echo "the $crates crates of the workspace declare Apache-2.0"
fi

if [ ! -f LICENSE ]; then
  echo "own-licence.sh: there is no file LICENSE at the root" >&2
  failed=1
else
  if command -v sha256sum > /dev/null; then
    found=$(sha256sum LICENSE | cut -d' ' -f1)
  else
    found=$(shasum -a 256 LICENSE | cut -d' ' -f1)
  fi
  if [ "$found" != "$APACHE_2_0_SHA256" ]; then
    echo "own-licence.sh: LICENSE is not the text of the Apache License 2.0, byte for byte" >&2
    failed=1
  else
    echo "LICENSE is the text of the Apache License 2.0"
  fi
fi

exit "$failed"

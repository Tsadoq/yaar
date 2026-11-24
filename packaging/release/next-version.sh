#!/bin/sh
# Prints the version to release from the commit messages since the newest tag v<x.y.z> that HEAD
# reaches, or nothing; with no such tag, the version of Cargo.toml. With --notes <tag>, prints the
# changes of that tag since the tag before it as markdown. Runs in the repository of the
# current folder.
# packaging/release/next-version.sh | packaging/release/next-version.sh --notes <tag>
set -eu

NAME=next-version.sh
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
usage() {
  echo "usage: packaging/release/next-version.sh | packaging/release/next-version.sh --notes <tag>" >&2
  exit 2
}
export LC_ALL=C

MODE=next
case ${1-} in
  '') ;;
  --notes)
    [ $# -eq 2 ] || usage
    MODE=notes
    ;;
  *) usage ;;
esac
[ "$MODE" = notes ] || [ $# -eq 0 ] || usage

HERE=$(cd "$(dirname "$0")" && pwd)
TOP=$(git rev-parse --show-toplevel 2>&1) || fail "this folder is not inside a git repository that git will read: $TOP"
cd "$TOP"

TAG_FORM='^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'

# newest_tag <commit> [<tag>]: the greatest version tag that <commit> reaches, below <tag> when given.
newest_tag() {
  limit=${2-}
  tags=$(git tag --merged "$1" --list 'v*') || fail "git cannot list the tags that $1 reaches"
  printf '%s\n' "$tags" | grep -E "$TAG_FORM" | awk -v below="${limit#v}" '
    function newer(a, b,   x, y, i) {
      split(a, x, ".")
      split(b, y, ".")
      for (i = 1; i <= 3; i++) {
        if (x[i] + 0 > y[i] + 0) return 1
        if (x[i] + 0 < y[i] + 0) return 0
      }
      return 0
    }
    { v = substr($0, 2)
      if (below != "" && !newer(below, v)) next
      if (best == "" || newer(v, best)) best = v }
    END { if (best != "") print "v" best }' || true
}

# The form of a subject is the one commit-lint.sh holds a pull request to: one line in commit-subject,
# whose groups are the type (1), the scope (3), the bang (4) and the description (5).
SUBJECT_FILE=$HERE/commit-subject
[ -f "$SUBJECT_FILE" ] || fail "$SUBJECT_FILE is missing: it holds the form of a subject"
SUBJECT_FORM=$(head -n 1 "$SUBJECT_FILE") || fail "$SUBJECT_FILE cannot be read"
[ -n "$SUBJECT_FORM" ] || fail "$SUBJECT_FILE is empty"

# The non-merge commits of a range, one line each: B or -, the type, the scope, the description. Each
# commit is read alone, so that nothing a body holds can be taken for another commit. A commit that
# is not of the form, and one of the form chore(release):, is left out.
classified() {
  commits=$(git rev-list --no-merges "$@") || fail "git cannot list the commits of $*"
  for sha in $commits; do
    message=$(git show -s --format=%s%n%b "$sha") || fail "git cannot read the commit $sha"
    subject=$(printf '%s\n' "$message" | head -n 1)
    fields=$(printf '%s\n' "$subject" | sed -nE "s/$SUBJECT_FORM/\\1\\
\\3\\
\\4\\
\\5/p")
    [ -n "$fields" ] || continue
    {
      IFS= read -r type
      IFS= read -r scope
      IFS= read -r bang
      IFS= read -r desc
    } << FIELDS
$fields
FIELDS
    if [ "$type" = chore ] && [ "$scope" = release ]; then continue; fi
    desc=$(printf '%s' "$desc" | tr '\t' ' ')
    desc=${desc%"${desc##*[! ]}"}
    breaking=-
    if [ -n "$bang" ] || printf '%s\n' "$message" | tail -n +2 | grep -Eq '^BREAKING[ -]CHANGE: '; then breaking=B; fi
    printf '%s\t%s\t%s\t%s\n' "$breaking" "$type" "$scope" "$desc"
  done
}

if [ "$MODE" = next ]; then
  LAST=$(newest_tag HEAD)
  if [ -z "$LAST" ]; then
    [ -f packaging/version.sh ] || fail "$TOP/packaging/version.sh is missing: it reads the version of Cargo.toml"
    packaging/version.sh
    exit
  fi
  LEVEL=$(classified "$LAST..HEAD" | awk -F'\t' '
    $1 == "B" { level = 3; next }
    $2 == "feat" && level < 2 { level = 2; next }
    ($2 == "fix" || $2 == "perf" || $2 == "revert") && level < 1 { level = 1 }
    END { print level + 0 }')
  [ "$LEVEL" -gt 0 ] || exit 0
  IFS=. read -r MAJOR MINOR PATCH << NUMBERS
${LAST#v}
NUMBERS
  if [ "$LEVEL" -eq 3 ] && [ "$MAJOR" -gt 0 ]; then
    echo "$((MAJOR + 1)).0.0"
  elif [ "$LEVEL" -ge 2 ]; then
    echo "$MAJOR.$((MINOR + 1)).0"
  else
    echo "$MAJOR.$MINOR.$((PATCH + 1))"
  fi
  exit
fi

TAG=$2
printf '%s\n' "$TAG" | grep -Eq "$TAG_FORM" || fail "$TAG is not a tag of the form v<x.y.z>"
git rev-parse --verify --quiet "refs/tags/$TAG^{commit}" > /dev/null || fail "there is no tag $TAG in this repository"
BEFORE=$(newest_tag "$TAG" "$TAG")
if [ -n "$BEFORE" ]; then RANGE="$BEFORE..$TAG"; else RANGE=$TAG; fi
classified --reverse "$RANGE" | awk -F'\t' '
  function add(section, line) { items[section] = items[section] line "\n" }
  {
    line = "- " ($3 != "" ? "**" $3 "**: " : "") $4
    if ($1 == "B") add(1, line)
    else if ($2 == "feat") add(2, line)
    else if ($2 == "fix") add(3, line)
    else if ($2 == "perf") add(4, line)
    else if ($2 == "revert") add(5, line)
  }
  END {
    split("Breaking changes|New|Fixed|Faster|Reverted", heading, "|")
    for (s = 1; s <= 5; s++) {
      if (items[s] == "") continue
      if (shown) printf "\n"
      printf "## %s\n\n%s", heading[s], items[s]
      shown = 1
    }
  }'

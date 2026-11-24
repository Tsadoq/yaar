#!/bin/sh
# Row LIC-02: each package holds a notices file with one entry for every linked dependency of
# the lock file, and a dependency with no licence text to reproduce fails.
#
#   packaging/notices-count.sh <packages-dir>
#
# <packages-dir> holds some or all of the six release files as the jobs make them (names in
# packaging/release/assemble.sh): yaar-review_<V>_amd64.deb, yaar-review_<V>_arm64.deb,
# yaar-review-<V>-linux-x86_64.tar.gz, yaar-review-<V>-linux-aarch64.tar.gz,
# yaar-review-<V>-macos-universal.zip and .dmg. Each of the first five that is there is
# counted (the .dmg holds the same app as the .zip: macos-bundle-check.sh compares the two
# byte for byte); a folder with none of them fails. That every file is there is
# release-files-check.sh's to say. For each: take the notices file out of the package (.deb:
# usr/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt.gz, gunzipped; archive:
# yaar-review/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt; .zip:
# YAAR.app/Contents/Resources/THIRD-PARTY-NOTICES.txt), and its targets (amd64/x86_64:
# x86_64-unknown-linux-gnu; arm64/aarch64: aarch64-unknown-linux-gnu; macos-universal: both
# aarch64-apple-darwin and x86_64-apple-darwin, the union).
#
# For each of the three parts of the notices (packaging/notices.sh writes them: PART 1 the
# Window yaar-review; PART 2 the Keeper yaar-keeper and pi's reporting program
# yaar-pi-report; PART 3 Claude Code's hook program yaar-claude-hook and tool server
# yaar-claude-tools):
#
# THE WANTED SET is every crate that
#   cargo tree --locked --offline -e normal --target <triple> -p <package>... --prefix none --format '{p}'
# prints, for the part's packages (PART 1: yaar-window; PART 2: yaar-keeper and
# yaar-adapter-pi; PART 3: yaar-companion-claude), over each target of the package,
# deduplicated, each line read as follows:
#   - a trailing " (*)" (a repeat) and a trailing " (proc-macro)" are stripped, in either
#     order; a proc-macro crate is counted like any other;
#   - a trailing " (<path>)" that remains is stripped too: a crate from a path prints its
#     folder there, and that alone never decides whether it is counted;
#   - "name vX.Y.Z" becomes "name X.Y.Z" (the "v" removed).
# Cargo works out the dependencies of a proc-macro crate (and of a build script) for the
# machine it runs on, not for --target, so a plain `cargo tree --target aarch64-unknown-linux-gnu`
# on an x86-64 machine also prints a crate that only an x86-64 machine would compile into a
# proc-macro (core_detect 1.0.0, behind rust-i18n's proc-macro): a crate that no aarch64
# program links and that the notices rightly lack. So the count asks cargo as if it ran on
# the triple it counts: RUSTC names a stand-in that reports <triple> as the host and answers
# nothing but cargo's questions (its version, its cfg, its file names; it refuses all else,
# so it compiles nothing), and CARGO_TARGET_DIR is a folder of the count's own, one for each
# triple, so that the repository's build folder is not touched. Every edge, a proc-macro's
# own dependencies included, is then worked out for the package's target whatever machine
# runs the count, as cargo-about does for the notices. This is exact for the Linux packages,
# which the jobs build on their own target; for the .zip it is the union over both Mac targets.
# THE WORKSPACE'S OWN CRATES are left out, and only they: the names of
# `cargo metadata --locked --offline --no-deps --format-version 1` (`.packages[].name`, read
# with jq). gpui-pre-apple, the toolkit's Apple crate that Cargo.toml patches in from
# vendor/gpui-pre-apple-0.3.12, is not a member (the workspace excludes vendor/): it prints a
# path and is counted, as the third-party code it is.
#
# THE FOUND SET is every "name version" of the part's "Used by:" lines, minus the names of
# the workspace's own crates (cargo-about lists them as users of the licence they carry).
#
# THE BUILD: the wanted set of a part is what Cargo links when that part's packages are built
# together and alone, so .github/scripts/release-build.sh (or the file RELEASE_BUILD_SH
# names, which the control uses) must build the programs the same way. The file is read once,
# before any package, and read closed: every line that is not a comment and holds `cargo` as
# a word (a word of the shell: `X=1 cargo`, `command cargo`, `cd d; cargo` and `exec cargo`
# hold it; so does a path or a variable that ends in cargo: `/usr/bin/cargo`,
# `$HOME/.cargo/bin/cargo`, `$CARGO`, `${CARGO:-cargo}`; `$HOME/.cargo` and `--prefix=/cargo`
# do not) must be exactly `cargo build` followed by --release,
# --locked, -p <package> and "$@", and nothing else; any other such line is unreadable and
# fails, named whole, since it may build what the notices do not hold. `exec` is refused with
# the rest. A line that changes the arguments the cargo lines pass on, `set --`, `set` with
# an operand, or `shift`, fails too, named whole, since "$@" is then not the caller's and the
# cargo lines build something else than they read as; `set -eu` and `set -o pipefail` do not.
# Quotes are taken out of a line before it is read, so `ca""rgo` is cargo. The packages of
# the readable lines must be exactly the three sets above, one line for each. One line for
# several parts would let Cargo merge the features that their packages ask of a shared
# dependency, and a program would link crates that its part does not list. This reader
# catches a slip in that script, in the forms listed here; it does not stop an author who
# sets out to get around it (a sourced file, an eval of a variable), so any change
# to release-build.sh is read in review.
#
# It fails, naming the package, the part and each crate, when:
#   - a wanted crate is not found (a linked dependency with no entry);
#   - a found crate is not wanted (an entry for something not linked: the count is wrong);
#   - a licence block of the part that names a crate has no text after its "Used by:" lines
#     (a dependency with no licence text to reproduce);
#   - a package holds no notices file, or two packages of one system and processor hold
#     different ones;
#   - release-build.sh does not build the three parts one by one, as THE BUILD says, or holds a
#     line that runs cargo and cannot be read, or that changes the arguments of the cargo
#     lines (said once, whatever the folder holds).
# Prints one line per package: "<file>: <n> crates in <m> licences, each with its text".
#
# Needs cargo (the fetched crates of the lock file; nothing is compiled, and the rustc on the
# path or RUSTC names answers cargo's questions as it would on each counted triple), jq, gzip, tar,
# unzip, and dpkg-deb to read a .deb (it fails when a .deb is there and dpkg-deb is not). The
# variable RELEASE_BUILD_SH, when set, names the build script to read instead of
# .github/scripts/release-build.sh.
# Exit codes: 0 every package counts; 1 a package fails (sentences on standard error);
# 2 a usage error.
set -eu

NAME=notices-count.sh
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
usage() {
  echo "usage: packaging/notices-count.sh <packages-dir>" >&2
  exit 2
}

[ $# -eq 1 ] || usage
DIR=$1
[ -d "$DIR" ] || { echo "$NAME: $DIR is not a folder" >&2; exit 2; }

ROOT=$(cd "$(dirname "$0")/.." && pwd)
for tool in cargo jq gzip tar unzip awk sort comm cmp; do
  command -v "$tool" > /dev/null 2>&1 || fail "$tool is needed and is not on the path"
done

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# Where the notices lie in each kind of package (the header says so).
DEB_MEMBER=usr/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt.gz
ARCHIVE_MEMBER=yaar-review/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt
ZIP_MEMBER=YAAR.app/Contents/Resources/THIRD-PARTY-NOTICES.txt

# The packages of each part of the notices, by the number of the part.
part_packages() {
  case "$1" in
    1) echo "yaar-window" ;;
    2) echo "yaar-keeper yaar-adapter-pi" ;;
    3) echo "yaar-companion-claude" ;;
  esac
}
part_title() {
  case "$1" in
    1) echo "the Window" ;;
    2) echo "the Keeper and pi's reporting program" ;;
    3) echo "Claude Code's two programs" ;;
  esac
}

# The workspace's own crates: the names of the members, which are no third-party code.
(cd "$ROOT" && cargo metadata --locked --offline --no-deps --format-version 1) \
  > "$TMP/metadata.json" 2> "$TMP/metadata.err" \
  || { cat "$TMP/metadata.err" >&2; fail "cargo metadata of $ROOT failed"; }
jq -r '.packages[].name' "$TMP/metadata.json" > "$TMP/workspace.unsorted" \
  || fail "jq cannot read the packages of cargo metadata"
LC_ALL=C sort -u "$TMP/workspace.unsorted" > "$TMP/workspace"
[ -s "$TMP/workspace" ] || fail "cargo metadata of $ROOT names no workspace crate"

# The rustc that cargo tree is given (THE WANTED SET says why): it answers the questions
# cargo asks of a compiler as the machine $NOTICES_HOST would, through the real one
# ($NOTICES_REAL_RUSTC, the caller's RUSTC or the rustc on the path), and refuses everything
# else, so that it can never compile. Its version question gets the real answer with the
# host line changed; a question about cfg, file names or the sysroot gets the target put on
# it when it has none.
REAL_RUSTC=${RUSTC:-rustc}
RUSTC_AS=$TMP/rustc-as-the-triple
cat > "$RUSTC_AS" << 'STAND_IN'
#!/bin/sh
case " $* " in
  *" -vV "*) "$NOTICES_REAL_RUSTC" "$@" | sed "s/^host: .*/host: $NOTICES_HOST/"; exit ;;
  *" --target "* | *" --target="*) exec "$NOTICES_REAL_RUSTC" "$@" ;;
  *--print*) exec "$NOTICES_REAL_RUSTC" "$@" --target "$NOTICES_HOST" ;;
esac
echo "notices-count.sh: this rustc only answers cargo's questions and compiles nothing: $*" >&2
exit 3
STAND_IN
chmod +x "$RUSTC_AS"

# The crates cargo links into a part for one target, as lines "name version", sorted and
# deduplicated, in $TMP/wanted.<part>.<triple>; made once, then kept.
wanted() {
  part=$1 triple=$2
  out=$TMP/wanted.$part.$triple
  [ -s "$out" ] && return 0
  args=""
  for pkg in $(part_packages "$part"); do args="$args -p $pkg"; done
  # shellcheck disable=SC2086
  (cd "$ROOT" && NOTICES_REAL_RUSTC=$REAL_RUSTC NOTICES_HOST=$triple RUSTC=$RUSTC_AS \
    CARGO_TARGET_DIR=$TMP/target.$triple \
    cargo tree --locked --offline -e normal --target "$triple" $args \
    --prefix none --format '{p}') > "$TMP/tree" 2> "$TMP/tree.err" \
    || { cat "$TMP/tree.err" >&2; fail "cargo tree for $triple failed"; }
  # A line is "name vX.Y.Z", then maybe " (proc-macro)", " (*)" and " (<path>)" in some
  # order; only the first two words say which crate it is.
  awk '
    NF == 0 { next }
    $2 !~ /^v[0-9]/ { print "BAD " $0; next }
    { print $1, substr($2, 2) }' "$TMP/tree" > "$TMP/tree.read"
  if grep -q '^BAD ' "$TMP/tree.read"; then
    fail "cargo tree printed a line that is not \"name vX.Y.Z\": $(grep -m 1 '^BAD ' "$TMP/tree.read")"
  fi
  LC_ALL=C sort -u "$TMP/tree.read" > "$out"
  [ -s "$out" ] || fail "cargo tree for $triple printed no crate for the part of $(part_title "$part")"
}

# What the notices file at $1 says, as tagged lines in $2:
#   P <n>                        a part opens
#   C <n> <name> <version>       a "Used by:" line of part n
#   B<TAB><n><TAB><lines of text><TAB><licence><TAB><first crate>
#                                a licence block of part n that names crates; the last block
#                                of the file is closed by the end of the file, as no rule follows it
parse_notices() {
  awk '
    function finish() {
      if (inb) { printf "B\t%s\t%d\t%s\t%s\n", part, text, licence, first }
      inb = 0
    }
    $0 ~ /^PART [0-9]+ OF 3: / { finish(); part = $2; print "P " part; next }
    (length($0) == 80) && ($0 ~ /^-+$/ || $0 ~ /^=+$/) { finish(); lastnb = ""; next }
    $0 == "Used by:" && part != "" {
      finish(); inb = 1; reading = 1; first = ""; text = 0
      licence = lastnb
      next
    }
    inb && reading {
      if ($0 ~ /^  [^ ]/) {
        print "C " part " " $1 " " $2
        if (first == "") first = $1 " " $2
        next
      }
      reading = 0
      if ($0 ~ /^[ \t]*$/) next
    }
    inb { if ($0 !~ /^[ \t]*$/) text++; next }
    $0 !~ /^[ \t]*$/ { lastnb = $0 }
    END { finish() }' "$1" > "$2"
}

# The member of an archive listing $1 (one name a line) that is the path $2, with or
# without a leading "./"; prints it exactly as the listing has it.
find_member() {
  awk -v want="$2" '{ n = $0; sub(/^\.\//, "", n); if (n == want) { print $0; exit } }' "$1"
}

# Puts the notices of the package $1 into $2. Prints nothing; says what is wrong on
# standard error and returns 1.
take_notices() {
  pkg=$1 out=$2
  base=$(basename "$pkg")
  case "$base" in
    *.deb)
      command -v dpkg-deb > /dev/null 2>&1 || fail "dpkg-deb is needed to read $base and is not on the path"
      dpkg-deb --fsys-tarfile "$pkg" > "$TMP/fs.tar" 2> "$TMP/take.err" \
        || { cat "$TMP/take.err" >&2; echo "$NAME: $base: dpkg-deb cannot read it" >&2; return 1; }
      tar -tf "$TMP/fs.tar" > "$TMP/list" || { echo "$NAME: $base: its files cannot be listed" >&2; return 1; }
      member=$(find_member "$TMP/list" "$DEB_MEMBER")
      [ -n "$member" ] || { echo "$NAME: $base: it holds no notices file ($DEB_MEMBER)" >&2; return 1; }
      tar -xOf "$TMP/fs.tar" "$member" > "$TMP/notices.gz" || { echo "$NAME: $base: $member cannot be read" >&2; return 1; }
      gzip -dc < "$TMP/notices.gz" > "$out" || { echo "$NAME: $base: $member is not gzip" >&2; return 1; }
      ;;
    *.tar.gz)
      tar -tzf "$pkg" > "$TMP/list" || { echo "$NAME: $base: its files cannot be listed" >&2; return 1; }
      member=$(find_member "$TMP/list" "$ARCHIVE_MEMBER")
      [ -n "$member" ] || { echo "$NAME: $base: it holds no notices file ($ARCHIVE_MEMBER)" >&2; return 1; }
      tar -xzOf "$pkg" "$member" > "$out" || { echo "$NAME: $base: $member cannot be read" >&2; return 1; }
      ;;
    *.zip)
      unzip -Z1 "$pkg" > "$TMP/list" || { echo "$NAME: $base: its files cannot be listed" >&2; return 1; }
      member=$(find_member "$TMP/list" "$ZIP_MEMBER")
      [ -n "$member" ] || { echo "$NAME: $base: it holds no notices file ($ZIP_MEMBER)" >&2; return 1; }
      unzip -p "$pkg" "$member" > "$out" || { echo "$NAME: $base: $member cannot be read" >&2; return 1; }
      ;;
  esac
  [ -s "$out" ] || { echo "$NAME: $base: its notices file is empty" >&2; return 1; }
}

# The build script the count reads, and the check that it builds the programs part by part.
RELEASE_BUILD=${RELEASE_BUILD_SH:-$ROOT/.github/scripts/release-build.sh}
check_release_build() {
  [ -f "$RELEASE_BUILD" ] || { echo "$NAME: $RELEASE_BUILD is not a file, so how the programs are built cannot be read" >&2; return 1; }
  awk -v q="'" -v parts="$(part_packages 1)|$(part_packages 2)|$(part_packages 3)" '
    function sorted(s,   a, n, i, j, t, out) {
      n = split(s, a, " ")
      for (i = 2; i <= n; i++) { t = a[i]; for (j = i - 1; j >= 1 && a[j] > t; j--) a[j + 1] = a[j]; a[j + 1] = t }
      out = a[1]
      for (i = 2; i <= n; i++) out = out " " a[i]
      return out
    }
    BEGIN { np = split(parts, p, "|"); for (i = 1; i <= np; i++) want[sorted(p[i])] = 1 }
    /^[ \t]*#/ { next }
    {
      line = $0
      sub(/^[ \t]+/, "", line)
      # Quotes are taken out of the line, as the shell takes them out of a word, so that
      # `ca""rgo` and `"cargo"` are the word cargo; then the separators and assignments of the
      # shell are blanks, and the line is split into words.
      t = line
      gsub(/"/, "", t)
      gsub(q, "", t)
      gsub(/[;&|(){}`=!\\<>]/, " ", t)
      nt = split(t, tok, /[ \t]+/)
      # Does the line change the arguments that "$@" stands for on the cargo lines? `shift`,
      # and `set` followed by anything but options (`set -- "$@" --workspace`, `set x`), or by
      # a `--` later (`set -eu -- x`), do, and so does `eval set -- ...`, which holds the same
      # words. `set -eu` and `set -o pipefail` do not. A line that does is named whole: the
      # cargo lines then build something else than they read as.
      for (i = 1; i <= nt; i++) {
        if (tok[i] == "shift") changes = 1
        if (tok[i] != "set") continue
        if (i < nt && tok[i + 1] !~ /^[-+][A-Za-z]+$/) changes = 1
        for (j = i + 1; j <= nt; j++) if (tok[j] == "--") changes = 1
      }
      if (changes) { print "args\t" line; changes = 0; next }
      # Does the line hold cargo as a word? `X=1 cargo`, `command cargo`, `cd d; cargo`,
      # `(cargo` and `if a; then cargo` all do. So does a word that ends in cargo after a
      # slash, a `$`, a `:` or a `-` (`/usr/bin/cargo`, `$HOME/.cargo/bin/cargo`, `$CARGO`,
      # `${CARGO:-cargo}`), in any case, while `$HOME/.cargo` and `--prefix=/cargo` do not.
      holds = 0
      for (i = 1; i <= nt; i++) if (tolower(tok[i]) ~ /(^|[^.]\/|[$:-])cargo$/) holds = 1
      if (!holds) next
      # Only a line that is exactly `cargo build` and allowed words is read; any other line
      # that runs cargo (an env assignment, `command`, `exec`, a second command, a flag
      # before `build`) is unreadable, since it may build what the notices do not hold.
      if (line !~ /^cargo[ \t]+build([ \t]|$)/) { print "line\t" line; next }
      n = split(line, w, /[ \t]+/)
      pkgs = ""; bad = 0
      for (i = 3; i <= n; i++) {
        if (w[i] == "--release" || w[i] == "--locked" || w[i] == "\"$@\"") continue
        if (w[i] == "-p" && i < n) { i++; pkgs = pkgs " " w[i]; continue }
        print "bad\t" w[i] "\t" line; bad = 1
      }
      if (pkgs == "") { print "bad\tno -p <package>\t" line; bad = 1 }
      if (bad) next
      invocations++
      got[sorted(substr(pkgs, 2))]++
    }
    END {
      if (invocations == 0) print "none"
      for (k in got) {
        if (!(k in want)) print "extra\t" k
        else if (got[k] > 1) print "twice\t" k
      }
      for (k in want) if (!(k in got)) print "missing\t" k
    }' "$RELEASE_BUILD" > "$TMP/build.read" || fail "$RELEASE_BUILD cannot be read"
  [ -s "$TMP/build.read" ] || return 0
  while IFS=$TAB read -r what rest line; do
    case "$what" in
      none) echo "$NAME: $RELEASE_BUILD holds no cargo build line, so what the notices are for is not built there" >&2 ;;
      args) echo "$NAME: $RELEASE_BUILD: the line '$rest' changes the arguments that \"\$@\" stands for on the cargo lines (\"shift\", or \"set\" with an operand such as \"--\"), so those lines no longer build what they read as" >&2 ;;
      line) echo "$NAME: $RELEASE_BUILD: the line '$rest' runs cargo and is not exactly \"cargo build\" and the words the count reads (--release, --locked, -p <package> and \"\$@\"), so what it builds cannot be read (\"exec\", an assignment, \"command\", a second command or a flag before build are refused)" >&2 ;;
      bad) echo "$NAME: $RELEASE_BUILD: the cargo build line '$line' holds '$rest', which the count cannot read (it reads --release, --locked, -p <package> and \"\$@\")" >&2 ;;
      extra) echo "$NAME: $RELEASE_BUILD: one cargo invocation builds $rest, which is not one part of the notices: Cargo merges the features of the packages of one invocation, so a program would link crates that the notices of its part do not hold" >&2 ;;
      twice) echo "$NAME: $RELEASE_BUILD: $rest is built by more than one cargo invocation" >&2 ;;
      missing) echo "$NAME: $RELEASE_BUILD: no cargo invocation of its own builds $rest, a part of the notices" >&2 ;;
    esac
  done < "$TMP/build.read"
  return 1
}

TAB=$(printf '\t')
STATUS=0
check_release_build || STATUS=1
COUNTED=0
IDX=0
for pkg in "$DIR"/yaar-review_*_amd64.deb "$DIR"/yaar-review_*_arm64.deb \
           "$DIR"/yaar-review-*-linux-x86_64.tar.gz "$DIR"/yaar-review-*-linux-aarch64.tar.gz \
           "$DIR"/yaar-review-*-macos-universal.zip; do
  [ -f "$pkg" ] || continue
  COUNTED=$((COUNTED + 1))
  IDX=$((IDX + 1))
  base=$(basename "$pkg")
  case "$base" in
    *_amd64.deb | *-linux-x86_64.tar.gz) TARGETS="x86_64-unknown-linux-gnu" ;;
    *_arm64.deb | *-linux-aarch64.tar.gz) TARGETS="aarch64-unknown-linux-gnu" ;;
    *-macos-universal.zip) TARGETS="aarch64-apple-darwin x86_64-apple-darwin" ;;
  esac

  NOTICES=$TMP/notices.$IDX
  if ! take_notices "$pkg" "$NOTICES"; then
    STATUS=1
    continue
  fi

  # Two packages of one system and processor hold one notices file.
  KEY=${TARGETS%% *}
  case "$TARGETS" in *" "*) KEY=universal ;; esac
  if [ -f "$TMP/first.$KEY" ]; then
    same=0
    cmp -s "$TMP/first.$KEY" "$NOTICES" || same=$?
    case "$same" in
      0) ;;
      1)
        echo "$NAME: $base: its notices differ from those of $(cat "$TMP/first.$KEY.name"), which are for the same system and processor" >&2
        STATUS=1
        ;;
      *) fail "$base: cmp cannot compare its notices with those of $(cat "$TMP/first.$KEY.name")" ;;
    esac
  else
    cp "$NOTICES" "$TMP/first.$KEY"
    echo "$base" > "$TMP/first.$KEY.name"
  fi

  parse_notices "$NOTICES" "$TMP/parsed.$IDX" || fail "$base: its notices cannot be read"
  OK=1
  CRATES=0
  LICENCES=0
  for part in 1 2 3; do
    title=$(part_title "$part")
    if ! grep -q -x "P $part" "$TMP/parsed.$IDX"; then
      echo "$NAME: $base: its notices hold no PART $part OF 3 ($title)" >&2
      OK=0
      continue
    fi
    # What the part says it holds, without the workspace's own crates.
    awk -v part="$part" '$1 == "C" && $2 == part { print $3, $4 }' "$TMP/parsed.$IDX" > "$TMP/found.raw" \
      || fail "$base: its notices cannot be read for PART $part"
    LC_ALL=C sort -u "$TMP/found.raw" > "$TMP/found.all"
    awk 'NR == FNR { ws[$1] = 1; next } !($1 in ws)' "$TMP/workspace" "$TMP/found.all" > "$TMP/found" \
      || fail "$base: the crates of PART $part cannot be read"
    # What cargo links into it, for every target of the package.
    : > "$TMP/want.all"
    for triple in $TARGETS; do
      wanted "$part" "$triple"
      cat "$TMP/wanted.$part.$triple" >> "$TMP/want.all"
    done
    LC_ALL=C sort -u "$TMP/want.all" > "$TMP/want.sorted"
    awk 'NR == FNR { ws[$1] = 1; next } !($1 in ws)' "$TMP/workspace" "$TMP/want.sorted" > "$TMP/want" \
      || fail "$base: the crates cargo links into PART $part cannot be read"

    LC_ALL=C comm -13 "$TMP/found" "$TMP/want" > "$TMP/missing"
    LC_ALL=C comm -23 "$TMP/found" "$TMP/want" > "$TMP/extra"
    if [ -s "$TMP/missing" ]; then
      while IFS= read -r crate; do
        echo "$NAME: $base: PART $part ($title): no entry for $crate, which is linked" >&2
      done < "$TMP/missing"
      OK=0
    fi
    if [ -s "$TMP/extra" ]; then
      while IFS= read -r crate; do
        echo "$NAME: $base: PART $part ($title): an entry for $crate, which is not linked" >&2
      done < "$TMP/extra"
      OK=0
    fi
    # Every licence block that names a crate carries its text.
    awk -F '\t' -v part="$part" '$1 == "B" && $2 == part && $3 == 0 { print $5 "\t" $4 }' "$TMP/parsed.$IDX" > "$TMP/notext" \
      || fail "$base: the licence blocks of PART $part cannot be read"
    if [ -s "$TMP/notext" ]; then
      while IFS=$TAB read -r crate licence; do
        echo "$NAME: $base: PART $part ($title): no licence text for the licence block of $crate ($licence)" >&2
      done < "$TMP/notext"
      OK=0
    fi
    awk -F '\t' -v part="$part" '$1 == "B" && $2 == part { n++ } END { print n + 0 }' "$TMP/parsed.$IDX" > "$TMP/blocks" \
      || fail "$base: the licence blocks of PART $part cannot be counted"
    read -r blocks < "$TMP/blocks"
    awk 'END { print NR }' "$TMP/found" > "$TMP/n" || fail "$base: the crates of PART $part cannot be counted"
    read -r n < "$TMP/n"
    CRATES=$((CRATES + n))
    LICENCES=$((LICENCES + blocks))
  done
  if [ "$OK" = 1 ]; then
    echo "$base: $CRATES crates in $LICENCES licences, each with its text"
  else
    STATUS=1
  fi
done

[ "$COUNTED" -gt 0 ] || fail "$DIR holds none of the five packages that carry notices"
exit "$STATUS"

#!/bin/sh
# Writes hunks.tsv again: the hunks of git's histogram diff for every pair of
# pairs.tar.zst, in the form the tests of the crate read.
#
# Run it by hand, when the hunks should be those of another git. It needs git, tar and
# zstd. No test runs it or git: the tests read hunks.tsv as it is committed, so a
# recording that changes a line is a change somebody reads before it goes in.
#
# One line for each pair: its name, a tab, then its hunks with a space between them. A
# hunk is written -a,n+b,m with the numbers of git's own "@@ -a,n +b,m @@" line: lines
# are counted from 1, and where n or m is 0 the number before it is the line the change
# comes after. git leaves out a count of 1; here it is always written.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

mkdir "$work/pairs"
zstd -d -q -c "$here/pairs.tar.zst" | tar -x -f - -C "$work/pairs"

# Neither the user's configuration nor that of a repository around this script may reach
# the recording: either can turn off the step that moves a hunk to where a reader
# expects it. So git is told to read no configuration, and runs outside any repository.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
cd "$work/pairs"

{
    printf '# %s\n' "$(git --version)"
    for pair in p*; do
        # git answers 1 when the two files differ, as the two of every pair do.
        git diff --no-index --no-color --no-ext-diff --no-textconv -U0 \
            --diff-algorithm=histogram "$pair"/a.* "$pair"/b.* > "$work/patch" || [ $? -eq 1 ]
        awk -v pair="$pair" '
            /^@@ / {
                old = substr($2, 2)
                new = substr($3, 2)
                if (old !~ /,/) old = old ",1"
                if (new !~ /,/) new = new ",1"
                hunks = hunks " -" old "+" new
            }
            END { printf "%s\t%s\n", pair, substr(hunks, 2) }
        ' "$work/patch"
    done
} > "$work/hunks.tsv"

mv "$work/hunks.tsv" "$here/hunks.tsv"

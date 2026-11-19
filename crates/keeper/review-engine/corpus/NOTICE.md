# Where each anchor of the corpus goes

`anchors.tsv` says, for ten anchors of each of the 569 file changes of the diff's corpus
(`crates/shared/diff/corpus/`, whose notice names the four projects, their pinned heads and
their licences), where the lines an anchor sits on in the file before the change are in the
file after it. It holds line numbers and words, and no text of the projects.

| Column | Meaning |
| --- | --- |
| `pair` | The change, by its name in the diff's corpus: `p0000` |
| `anchor` | The anchor, `a0` to `a9`, of that change |
| `first`, `last` | The lines the anchor sits on in the file before, counted from 1 |
| `truth` | What became of them: `INTACT`, every line there, in one block; `PARTIAL`, some of them there; `GONE`, none; `SPLIT`, every line there with lines put between them; `EXCLUDE`, no verdict could be given |
| `first_after`, `last_after` | For `INTACT`, the lines of the block in the file after; for `PARTIAL`, the lines where what is left of them lies; `0` for `GONE` |

The verdicts were made in November 2026 by the earlier experiments of the
project: `git blame --reverse -M` between the two versions, with two repairs, and then a
reading by hand of every anchor on which methods disagreed, whose verdict replaced blame's
where the two differed. Four anchors are `EXCLUDE`: their reading by hand found no single
answer. `SPLIT` is no requirement's answer: a line put inside a commented range is a change
the thread keeps whole with a notice.

5,690 anchors: 3,486 `INTACT`, 1,031 `PARTIAL`, 1,019 `GONE`, 150 `SPLIT`, 4 `EXCLUDE`.

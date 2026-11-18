# Recorded changes of four other projects

`pairs.tar.zst` holds 569 recorded changes of single files: for each change the file
before it and the file after it, 1,138 files in all. They are test data. The tests of
this crate diff every pair and compare the hunks with the ones git gave, which are in
`hunks.tsv`. The files are compiled into the test program and into nothing else: no
program built from this repository for a user holds them.

Every one of the 1,138 files is an unchanged copy of a file of one of the four projects
below, as that project has it at one commit. `pins.tsv` says for each change which
project it is from, the two commits, and the path of the file at each. The files stay
under the licence of their project. Where a file names another origin or another holder
in its own comments, those lines are in the copy as the project has them.

| Project | Address | Read up to commit | Licence | Changes | Licence text |
| --- | --- | --- | --- | --- | --- |
| ripgrep | <https://github.com/BurntSushi/ripgrep> | `178cc027e283fd965cb72d0a7395b219eada170c` | The Unlicense or the MIT licence, as the user chooses | 140 | `licences/ripgrep/COPYING`, `LICENSE-MIT`, `UNLICENSE` |
| Flask | <https://github.com/pallets/flask> | `49943b5a94413a530761350f04e896f76e8d620e` | The BSD licence in three clauses | 144 | `licences/flask/LICENSE.txt` |
| Zod | <https://github.com/colinhacks/zod> | `19c40d2072288b4cafec851e9d056a72a88e5973` | The MIT licence | 140 | `licences/zod/LICENSE` |
| Codex | <https://github.com/openai/codex> | `ea1b6ad1cf2c39dacf10bf1f2667d9c6f7e7eb3f` | The Apache License, version 2.0 | 145 | `LICENSE` at the root of this repository |

The licence texts under `licences/` are copied word for word from each project at the
commit of the table. Codex comes with a notice file, which is copied as
`licences/codex/NOTICE`; the text of its licence is the one this repository is under.

Four of the changes from Flask, `p0145`, `p0171`, `p0193` and `p0261`, are changes of
`tests/test_cli.py`. Its first lines say that the file was part of Flask-CLI and was
modified under the terms of its Revised BSD License, with the copyright of 2015 held by
CERN.

## The files of this folder

| File | What it is |
| --- | --- |
| `pairs.tar.zst` | The copies: `pNNNN/a.<ext>` before the change and `pNNNN/b.<ext>` after it. One tar file with the names in order and with owner 0, time 0 and mode 644 on every file, packed with `zstd -19` |
| `pins.tsv` | Where each change is from: its name, how it was picked (`adjacent`: one commit of the file; `far8`: eight commits of the file apart; `rename`: the file changed its path; `agent`: a commit an agent co-wrote), the project, the commit before and the commit after, the path at each, and what git calls the change (`M` for a changed file, `R` with a number for a renamed one) |
| `hunks.tsv` | The hunks of git's histogram diff for each change, under a first line that names the git that was run |
| `record.sh` | Writes `hunks.tsv` again from the archive |
| `licences/` | The licence texts and the notice file named above |

The archive is written once. A new version of it would stay in the history of the
repository beside the old one, at 1.5 MB each.

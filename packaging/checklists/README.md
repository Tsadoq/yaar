# The owner's checklists

The checks of a release that no test on GitHub's machines can make: the platform conventions,
the speed limits, the real Claude Code and pi, Linux on real hardware. While a publishing run
waits for approval, they are walked on the files it made. Each has one command and a look.

| Abbreviation | Meaning |
| --- | --- |
| CONV, PERF, HW, ... | Ids of the rows of `test-plan/tests.toml` |
| release-final | The artifact of the publishing run holding `site/` and `release/` |
| results folder | `~/yaar-checklists/<YYYY-MM-DD>/`, where every script writes |
| the kit | The measuring tools of `~/ade-phase4-2026-11-12/gpui-measure`: `fakeinput`, `adecap.app`, `tools/stream.pl` |

Every script prints its usage with `--help`. Those that post input or record the screen
(`typing.sh`, `scroll.sh`, `flood.sh`) ask for the word `go` before they post anything. The
scripts that walk a list read their answers from the terminal or from a file of answers
(`conventions.sh macos < answers.txt`; the format is at the top of `lib.sh`), and take
`--dry-run`, which asks nothing of the system and writes a file of its own name.

## The files of the release

Download `release-final` and serve it on this machine:

```sh
gh run download <run id> --repo Tsadoq/yaar -n release-final -D ~/yaar-release-<version>
packaging/checklists/serve-release.sh ~/yaar-release-<version>
```

It checks the files against `SHA256SUMS`, serves on 127.0.0.1 only and prints the install and
the uninstall lines. Run the lines in another terminal. On a Linux machine, run the script
there, on a copy of the same folder.

## The items

| Item | Command | What to look at |
| --- | --- | --- |
| All items, on the files of the release | `serve-release.sh <release-final folder>` | The install line installs the app; the uninstall line removes it. A `.dmg` or `.zip` downloaded in a browser from the run's page carries the quarantine mark: that is CONV-01 item 13's first open |
| CONV-01, macOS, items 1 to 17 | `conventions.sh macos` | One answer an item; `conventions-macos.md` holds date, version, system, the answers and the sign-off. Items 11 and 16 print the light theme steps |
| CONV-02, Linux, items 1 to 11 | `conventions.sh linux` | The same, in `conventions-linux.md` |
| PERF-01, cold start to the diff, 0.3 s | `start-time.sh` | After a restart, a login and two minutes: `first_diff` of the first start against 300 ms. Posts no input |
| PERF-02, the same in the virtual machine | `start-time.sh --linux` | From the Mac, in the project's machine after its own restart with the `.deb` installed: `first_diff` against 300 ms |
| PERF-03, memory, 200 MB | `memory.sh` | The sum of the Window and the Keeper after 60 s idle, against 200 MB. Posts no input |
| PERF-04, typing, 10 ms behind Ghostty | `typing.sh` | Median and 95th percentile of key to screen, in the terminal pane idle, while output streams, and in a comment box, against Ghostty: the difference at most 10 ms. Needs the word `go`, Accessibility and Screen Recording |
| PERF-05, scrolling, a diff of 2,200 lines | `scroll.sh` | Frames a second against the refresh rate and the share of gaps over two refresh intervals, unified and split: at most 1%. Needs the word `go` |
| PERF-06, a flood of 200,000 lines | `flood.sh` | The generator's time in the terminal pane over its time alone: at most 1.5. Types a command into the pane; needs the word `go` |
| STACK-02, TERM-06, SCREEN-01, SCREEN-02, SYS-04 | `screen-walk.sh` | Runs the test suite first (SYS-04) and records it; then the four looks, with the test file of text written for TERM-06 |
| LOAD-01 to LOAD-05, LOAD-07, LOAD-08, CLI-01 to CLI-05, DELIVER-02, REPLY-01, TERM-05 | `real-clis.sh` | Prints each check in order and records the answers. Starts nothing of the CLIs |
| HW-01 to HW-03, SYS-02, SYS-03 | `linux-hardware.sh --server http://127.0.0.1:<port>` | On the Linux machine: installs from `serve-release.sh`'s server run there, measures memory, and records the answers. Typing against `foot`, scrolling and the flood are by hand |

STACK-02 stays by hand: "draws" needs eyes. SYS-02's other releases are the `install-linux`
jobs. HW-01 on Ubuntu 22.04 and Debian 12 with a software driver is a known empty window.

## Checking the scripts

```sh
packaging/checklists/self-test.sh
```

runs `conventions.sh` with files of answers and `--dry-run`, and `serve-release.sh` on a
folder it lays out as `release-final` in its own scratch, on a free port, then stops it by
its pid. It also reads made-up logs through the analysis of `typing.sh`, `scroll.sh` and
`start-time.sh`. It opens no window, posts no input, and writes nothing outside its scratch.

# The fixed projects of the Navigator's tests

Three small projects, one for each built-in row: `typescript` (with JavaScript files),
`python` and `rust`. A test makes a git repository of one with
`support::Scratch::fixture`, in the test's own scratch folder:

1. the files of `base/` are written and committed, `dot-gitignore` written as `.gitignore`;
   that commit is the base, the old side of the change;
2. the files of `ignored/`, when there is one, are written: the dependencies, which the
   project's `.gitignore` leaves out of git (`node_modules/`, `.venv/`), written by hand
   here and never installed by a package manager;
3. each path listed in `gone` is removed, and the files of `change/` written over the
   rest, and left uncommitted: the change on screen.

| Project | Deleted by the change | Moved by the change | Dependency on a removed line |
| --- | --- | --- | --- |
| `typescript` | `legacyScale`, `src/geometry.ts` | `titleCase`, `src/strings.ts` to `src/text/case.ts` | `Shape` of `node_modules/tinyshape/index.d.ts` |
| `python` | `legacy_scale`, `app/geometry.py` | `title_case`, `app/strings.py` to `app/text/case.py` | `Shape` of `.venv/lib/python3.12/site-packages/tinyshape` |
| `rust` | `legacy_scale`, `src/geometry.rs` | `title_case`, `src/strings.rs` to `src/text.rs` | none: the project has no dependency |

`typescript/base/tools/build.js` lies outside the project's `include`: a server that
acquires types for a file outside every project would do it for this one. The JavaScript
files are `src/helpers.js` and `src/report.js`.

Every name whose answer a test checks is a function or a type of the project or of its
hand-written dependency, called outside any macro. The Rust server expands a macro of the
standard library, `format!` among them, only when the source of the standard library is
installed, and the toolchain the repository pins installs none (its profile is
`minimal`): a name inside such a macro would answer nothing there. The Rust probes were
tried with the toolchain's library source hidden from rust-analyzer (its `cargo.sysroot`
set to a copy of the toolchain's folder without `lib/rustlib/src`), and each answered.

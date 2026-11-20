# pi companion (`X-PI-EXT`)

Owns: the `pi` wrapper that sits first on the search path of the app's shells, and the
extension it makes pi load. The extension is TypeScript, as pi requires; it is the one
piece of the app outside Rust. No package is installed for it, and nothing builds it: pi
loads the source as it is.

| File | What it is |
| --- | --- |
| `pi.sh` | The wrapper, a POSIX `sh` script. The pi Adapter (`crates/keeper/adapter-pi`) writes it into its own folder of the app as `bin/pi`, with the paths of the reporting program and of the extension put in, and puts that folder first on the search path of every shell of the app. It runs the next `pi` on the search path with `-e` and the extension, and every argument it was given; a subcommand of pi's goes through untouched |
| `yaar.ts` | The extension. It reports pi's events to the app through the reporting program `yaar-pi-report` (a program of the crate `yaar-adapter-pi`), hands an attached review to the user's message, and registers the reply tool `reply_to_thread`, which has no way to resolve a thread. It answers no dialog of pi's, the trust of a folder included |
| `test/harness.ts` | For the tests only: a stand-in for pi's extension interface, which loads the extension with Node and plays pi's events to it, as the test of the extension (`crates/keeper/adapter-pi/tests/extension.rs`) tells it |
| `test/package.json` | For the tests only: says that the files beside it are ECMAScript modules, so that Node loads the harness the same way whatever `package.json` lies in a folder above the repository; the test writes the same file beside the extension it installs |
| `test/pi-ai.mjs` | For the tests only: a stand-in for the one module of pi's the extension imports at run time, `@earendil-works/pi-ai`, which pi gives its extensions |

The payloads the extension reports, and what it is answered, are written down once, in the
crate doc of `yaar-adapter-pi`.

The app itself runs no JavaScript: only the user's pi runs the extension, and the test of
the extension runs it under Node, which that one test needs on the machine (Node 22.18 or
later, whose type stripping runs TypeScript as it is).

A pi started outside the app's shells finds no wrapper and loads nothing of the app; nothing
is written into `~/.pi` or into a worktree.

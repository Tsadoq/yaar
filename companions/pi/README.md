# pi companion

The extension the app asks pi to load. It is TypeScript, as pi requires, and the one piece
of the app outside Rust. Nothing builds it and no package is installed for it: pi loads
the source as it is.

| File | What it is |
| --- | --- |
| `yaar.ts` | The extension. The pi Adapter (`crates/keeper/adapter-pi`) writes it into the app's data folder as `agents/pi/extension/yaar.ts`, at the same path after every update of the app. Only after the user's yes does the Adapter add that path to the list `extensions` of `~/.pi/agent/settings.json`; taking the yes back takes it out. Inside the app the extension reports pi's events through the program that `YAAR_PI_REPORT` names, `yaar-pi-report` (a program of the crate `yaar-adapter-pi`), adds an attached review to the user's message, and registers the reply tool `reply_to_thread`, which cannot resolve a thread. It answers no dialog of pi's |
| `test/harness.ts` | For the tests only: a stand-in for pi's extension interface, which loads the extension with Node and plays pi's events to it, as `crates/keeper/adapter-pi/tests/extension.rs` tells it |
| `test/package.json` | For the tests only: marks the files beside it as ECMAScript modules, so that Node loads the harness the same way whatever `package.json` lies in a folder above the repository |
| `test/pi-ai.mjs` | For the tests only: a stand-in for `@earendil-works/pi-ai`, the one module of pi's that the extension imports at run time |

The payloads the extension reports, what it is answered, and the one entry the Adapter
writes into pi's settings are described in the crate documentation of `yaar-adapter-pi`.

The app runs no JavaScript. Only the user's pi runs the extension, and the test of the
extension runs it under Node 22.18 or later, whose type stripping runs TypeScript as it
is.

Outside the app `YAAR_PI_REPORT` is not set, and the extension registers nothing, reports
nothing and prints nothing. It writes nothing into a worktree.

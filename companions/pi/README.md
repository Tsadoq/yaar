# pi companion (`X-PI-EXT`)

Owns: the extension the app asks pi to load. The extension is TypeScript, as pi requires;
it is the one piece of the app outside Rust. No package is installed for it, and nothing
builds it: pi loads the source as it is.

| File | What it is |
| --- | --- |
| `yaar.ts` | The extension. The pi Adapter (`crates/keeper/adapter-pi`) writes it into its own folder of the app as `extension/yaar.ts`, at the same path after every update of the app. Only after the user's yes, which the app asks once, does the Adapter add that path to the list `extensions` of pi's own settings, `~/.pi/agent/settings.json`; taking the yes back takes it out. Inside the app it reports pi's events through the reporting program `yaar-pi-report` (a program of the crate `yaar-adapter-pi`), which every shell of the app names in `YAAR_PI_REPORT`, hands an attached review to the user's message, and registers the reply tool `reply_to_thread`, which has no way to resolve a thread. It answers no dialog of pi's, the trust of a folder included |
| `test/harness.ts` | For the tests only: a stand-in for pi's extension interface, which loads the extension with Node and plays pi's events to it, as the test of the extension (`crates/keeper/adapter-pi/tests/extension.rs`) tells it |
| `test/package.json` | For the tests only: says that the files beside it are ECMAScript modules, so that Node loads the harness the same way whatever `package.json` lies in a folder above the repository; the test writes the same file beside the extension it installs |
| `test/pi-ai.mjs` | For the tests only: a stand-in for the one module of pi's the extension imports at run time, `@earendil-works/pi-ai`, which pi gives its extensions |

The payloads the extension reports, what it is answered, and the one entry the Adapter
writes into pi's settings are written down once, in the crate doc of `yaar-adapter-pi`.

The app itself runs no JavaScript: only the user's pi runs the extension, and the test of
the extension runs it under Node, which that one test needs on the machine (Node 22.18 or
later, whose type stripping runs TypeScript as it is).

Once the user said yes, pi loads the extension outside the app too. There `YAAR_PI_REPORT`
is not set, and the extension registers nothing, reports nothing and prints nothing.
Nothing is written into a worktree.

// For the tests only: a stand-in for pi's extension interface, run by Node, which loads the
// app's extension as pi would and plays pi's events to it as the test tells it.
//
//   node harness.ts <the extension's file>
//
// It imports the extension and calls its default export with a stand-in `pi` that records
// each `pi.on(event, handler)` and `pi.registerTool(tool)`. It then writes one line of
// JSON, `{"registered": {"events": [<the events handled, in order>], "tools": [<each
// tool's name, description and parameters>]}}`, and reads its standard input a line at a
// time, each line one JSON object:
//
//   {"fire": "<event>", "event": {...}, "ctx": {"sessionId", "sessionFile", "cwd"}}
//     calls every handler of that event, in the order they were registered, with the
//     event and a stand-in context, awaits each, and writes `{"results": [...]}`, what
//     each returned, `null` for nothing;
//   {"call": "<tool>", "params": {...}, "ctx": {...}}
//     calls the tool's `execute` as pi does, with five arguments: an id of its own (a new
//     one for each call), the parameters, an AbortSignal that is never aborted, an
//     `onUpdate` that does nothing, and the stand-in context; awaits it, and writes
//     `{"result": ...}`, what it returned, or `{"error": "<message>"}`, what it threw;
//   {"env": {"<name>": "<value>" or null, ...}}
//     sets each variable in the harness's own environment (`process.env`), null taking
//     it away, and writes `{"env": "set"}`. The extension reads YAAR_PI_REPORT at each
//     report, so a test can fire one event with the variable and one without, in one
//     process and one chain of reports.
//
// Every line is handled to its end, and its answer written, before the next line is read.
// It is ECMAScript modules, as `package.json` beside it says, wherever Node is run from.
//
// The stand-in context gives `cwd`, `sessionManager.getSessionId()` and
// `sessionManager.getSessionFile()` from the line's `ctx`, and `isIdle()`, true unless
// the line's `ctx` says `"idle": false`. A line it cannot read ends it with 2. When its
// standard input ends, it ends with 0.
//
// Not built yet: the package of pi's extension writes it.

process.stderr.write("the stand-in for pi's extension interface is not built yet\n");
process.exit(70);

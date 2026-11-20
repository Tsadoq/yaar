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
//     each returned, `null` for nothing; a handler that throws gives `{"threw":
//     "<message>"}` instead, since pi would have to deal with it;
//   {"fireTogether": [{"fire", "event", "ctx"}, ...]}
//     calls the handlers of every event of the list one after the other without awaiting
//     any in between, as a host that does not wait for one event before it gives the
//     next would, awaits them all, and writes `{"results": [<the results of each event
//     as for "fire">]}`;
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

import { createInterface } from "node:readline";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

type Handler = (event: unknown, ctx: unknown) => unknown;

type Line = Record<string, any>;

const file = process.argv[2];
if (!file) {
  process.stderr.write("usage: node harness.ts <the extension's file>\n");
  process.exit(2);
}

/** Writes one line of JSON on the standard output. */
function say(value: unknown): void {
  process.stdout.write(JSON.stringify(value) + "\n");
}

const events: string[] = [];
const handlers = new Map<string, Handler[]>();
const tools: Array<{ name: string; description?: string; parameters?: unknown; execute: Function }> = [];

// What pi gives an extension: the two calls the app's extension makes.
const pi = {
  on(event: string, handler: Handler): void {
    events.push(event);
    handlers.set(event, [...(handlers.get(event) ?? []), handler]);
  },
  registerTool(tool: (typeof tools)[number]): void {
    tools.push(tool);
  },
};

const extension = await import(pathToFileURL(resolve(file)).href);
await extension.default(pi);

say({
  registered: {
    events,
    tools: tools.map(({ name, description, parameters }) => ({ name, description, parameters })),
  },
});

/** The context pi hands a handler or a tool, as the line's `ctx` describes it. */
function contextOf(ctx: Line | undefined) {
  const given = ctx ?? {};
  return {
    cwd: given.cwd,
    sessionManager: {
      getSessionId: () => given.sessionId,
      getSessionFile: () => given.sessionFile ?? null,
    },
    isIdle: () => given.idle !== false,
  };
}

/** Calls every handler of the event, one after the other, and gives what each returned. */
async function fire(line: Line): Promise<unknown> {
  const results: unknown[] = [];
  for (const handler of handlers.get(line.fire) ?? []) {
    results.push((await handler(line.event, contextOf(line.ctx))) ?? null);
  }
  return results;
}

let calls = 0;

for await (const text of createInterface({ input: process.stdin, crlfDelay: Infinity })) {
  if (text.trim() === "") {
    continue;
  }
  let line: Line;
  try {
    line = JSON.parse(text);
  } catch {
    process.stderr.write("the stand-in for pi's extension interface cannot read a line\n");
    process.exitCode = 2;
    break;
  }
  try {
    if (typeof line.fire === "string") {
      say({ results: await fire(line) });
    } else if (Array.isArray(line.fireTogether)) {
      const started = line.fireTogether.map((one: Line) => fire(one));
      say({ results: await Promise.all(started) });
    } else if (typeof line.call === "string") {
      const tool = tools.find((candidate) => candidate.name === line.call);
      if (!tool) {
        say({ error: `no tool named ${line.call}` });
        continue;
      }
      calls += 1;
      try {
        const result = await tool.execute(
          `call-${calls}`,
          line.params,
          new AbortController().signal,
          () => {},
          contextOf(line.ctx),
        );
        say({ result: result ?? null });
      } catch (thrown) {
        say({ error: thrown instanceof Error ? thrown.message : String(thrown) });
      }
    } else if (line.env && typeof line.env === "object") {
      for (const [name, value] of Object.entries(line.env)) {
        if (value === null) {
          delete process.env[name];
        } else {
          process.env[name] = String(value);
        }
      }
      say({ env: "set" });
    } else {
      process.stderr.write("the stand-in for pi's extension interface cannot read a line\n");
      process.exitCode = 2;
      break;
    }
  } catch (thrown) {
    say({ threw: thrown instanceof Error ? thrown.message : String(thrown) });
  }
}

// The extension of the app YAAR for pi: it tells the app what pi does, hands a review the
// user attached in the app to the user's message, and gives the agent the reply tool.
//
// pi loads it from the app's own folder because pi's own settings list it, which the app
// writes only after the user said yes. So pi loads it in every pi the user starts, outside
// the app too: there no YAAR_PI_REPORT is set, and the extension, looking for it when pi
// loads it, registers no handler and no tool, reports nothing and prints nothing. Inside
// the app every shell sets it. It is TypeScript, as pi requires, written with no syntax
// that type stripping cannot erase, so that Node runs it as it is in its test. It decides
// nothing: every report goes, as it is, to the reporting program YAAR_PI_REPORT names, and
// the app's answer comes back from it. What each report holds and what it is answered is
// written down in the crate doc of `yaar-adapter-pi`; this file follows that doc. A loaded
// extension reads YAAR_PI_REPORT again at each report, and every handler awaits its own
// report to its end.
//
// It registers no handler for the trust of a folder, nor for any dialog of pi's: the user
// answers them.

import { spawn } from "node:child_process";

import { Type } from "@earendil-works/pi-ai";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

/** The variable every shell of the app sets to the absolute path of the reporting program. */
export const REPORT_VARIABLE = "YAAR_PI_REPORT";

/** The reply tool, as pi lists it to the agent. */
export const REPLY_TOOL = "reply_to_thread";

/** The `event` of the report of a call of the reply tool. */
export const REPLY_EVENT = "reply_to_thread";

/** The events of pi the extension reports, each as it is, in the order pi gives them. */
export const REPORTED = [
  "session_start",
  "session_shutdown",
  "input",
  "agent_start",
  "tool_call",
  "tool_result",
  "agent_settled",
] as const;

/** What pi hands a handler or a tool besides the event: the parts of its context read here. */
interface Context {
  cwd: string;
  sessionManager: {
    getSessionId(): string;
    getSessionFile(): string | null | undefined;
  };
}

/** An event of pi, as it is: the extension reads only the keys the crate doc names. */
type PiEvent = Record<string, any>;

// Every report starts when the one before it ended, so that the app hears the events in
// the order pi gave them whatever the timing of the programs.
let chain: Promise<unknown> = Promise.resolve();

/**
 * Runs the reporting program YAAR_PI_REPORT names now, with `payload` on its standard
 * input, after every report started before it has ended, and gives what it printed once
 * the program has ended: nothing, an empty text, when the program cannot be run or no
 * YAAR_PI_REPORT is set. `tool` passes `--tool`.
 */
export function report(payload: Record<string, unknown>, tool: boolean): Promise<string> {
  const done = chain.then(() => run(payload, tool));
  // `run` never rejects; the chain goes on whatever a report became.
  chain = done;
  return done;
}

/** One run of the reporting program: its whole standard output, decoded once. */
function run(payload: Record<string, unknown>, tool: boolean): Promise<string> {
  return new Promise((finish) => {
    // Read again at each report: in pi the variable is the one it was loaded with.
    const program = process.env[REPORT_VARIABLE];
    if (!program) {
      finish("");
      return;
    }
    const chunks: Buffer[] = [];
    try {
      // No shell between: the program's parent is pi, which the app watches.
      const child = spawn(program, tool ? ["--tool"] : [], {
        stdio: ["pipe", "pipe", "ignore"],
      });
      child.on("error", () => finish(""));
      child.stdout.on("data", (chunk: Buffer) => chunks.push(chunk));
      // The program ends its input as soon as it has what it needs, and says nothing of it.
      child.stdin.on("error", () => {});
      // `close`, not `exit`: the standard output has been read to its end.
      child.on("close", () => finish(Buffer.concat(chunks).toString("utf8")));
      child.stdin.end(JSON.stringify(payload));
    } catch {
      finish("");
    }
  });
}

/** The keys every report holds, which name the session and its folder. */
function base(event: string, ctx: Context): Record<string, unknown> {
  return {
    event,
    sessionId: ctx.sessionManager.getSessionId(),
    sessionFile: ctx.sessionManager.getSessionFile() ?? null,
    cwd: ctx.cwd,
  };
}

/** The input of a tool call, cut to the keys the app reads: a path or a command. */
function inputOf(event: PiEvent): Record<string, unknown> {
  const input = event.input ?? {};
  switch (event.toolName) {
    case "read":
    case "edit":
    case "write":
      return { path: input.path };
    case "bash":
      return { command: input.command };
    default:
      return {};
  }
}

/** The keys of a `tool_call` or a `tool_result` that both hold. */
function toolKeys(event: PiEvent): Record<string, unknown> {
  return { toolName: event.toolName, toolCallId: event.toolCallId, input: inputOf(event) };
}

/** What each reported event adds to the keys every report holds. */
const added: Record<(typeof REPORTED)[number], (event: PiEvent) => Record<string, unknown>> = {
  session_start: (event) => ({ reason: event.reason }),
  session_shutdown: (event) => ({ reason: event.reason }),
  input: (event) => ({ source: event.source, streamingBehavior: event.streamingBehavior ?? null }),
  agent_start: () => ({}),
  tool_call: toolKeys,
  tool_result: (event) => ({
    ...toolKeys(event),
    isError: event.isError === true,
    // The unified diff pi made of an edit, whose hunks say which lines changed.
    ...(event.toolName === "edit" && typeof event.details?.patch === "string"
      ? { details: { patch: event.details.patch } }
      : {}),
  }),
  agent_settled: (event) => ({ aborted: event.aborted === true }),
};

/**
 * What the extension makes of the app's answer to an `input` report: the user's text, a
 * blank line and the review the app attached, when it gave one; else the message as it is.
 */
function answerToInput(event: PiEvent, answer: string): Record<string, unknown> {
  let review: unknown;
  try {
    review = JSON.parse(answer)?.review;
  } catch {
    review = undefined;
  }
  if (typeof review !== "string") {
    return { action: "continue" };
  }
  return {
    action: "transform",
    text: `${event.text}\n\n${review}`,
    ...(event.images === undefined ? {} : { images: event.images }),
  };
}

/**
 * The extension, as pi calls it once it loaded the file, and again after `/new`. With no
 * YAAR_PI_REPORT set, outside the app, it registers nothing and returns.
 */
export default function yaar(pi: ExtensionAPI): void {
  if (!process.env[REPORT_VARIABLE]) {
    return;
  }

  for (const name of REPORTED) {
    // A handler awaits its own report to its end, and never throws for one that failed.
    pi.on(name as any, async (event: PiEvent, ctx: Context) => {
      const answer = await report({ ...base(name, ctx), ...added[name](event) }, false);
      return name === "input" ? answerToInput(event, answer) : undefined;
    });
  }

  pi.registerTool({
    name: REPLY_TOOL,
    label: "Reply to review thread",
    description:
      "Reply to one review thread of the attached review. Call once per thread. Set addressed=true when your change resolves the comment; leave it false when you disagree or need the reviewer's answer.",
    parameters: Type.Object({
      thread: Type.String({ description: "Thread id, e.g. T1" }),
      body: Type.String({ description: "Your reply to the reviewer, plain text or Markdown" }),
      addressed: Type.Optional(
        Type.Boolean({ description: "true if the comment is addressed by your change" }),
      ),
    }),
    async execute(id: string, params: Record<string, unknown>, _signal: unknown, _onUpdate: unknown, ctx: Context) {
      const answer = await report(
        {
          ...base(REPLY_EVENT, ctx),
          toolCallId: id,
          arguments: { thread: params.thread, body: params.body, addressed: params.addressed },
        },
        true,
      );
      let said: { text?: unknown; error?: unknown } = {};
      try {
        said = JSON.parse(answer) ?? {};
      } catch {
        // No answer at all: the program could not be run.
        throw new Error(answer === "" ? "the app is not running" : "the app did not keep the reply");
      }
      if (typeof said.error === "string") {
        throw new Error(said.error);
      }
      if (typeof said.text !== "string") {
        throw new Error("the app did not keep the reply");
      }
      return { content: [{ type: "text", text: said.text }], details: undefined };
    },
  });
}

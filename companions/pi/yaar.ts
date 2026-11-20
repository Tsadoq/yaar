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

/**
 * Runs the reporting program YAAR_PI_REPORT names now, with `payload` on its standard
 * input, after every report started before it has ended, and gives what it printed once
 * the program has ended: nothing, an empty text, when the program cannot be run or no
 * YAAR_PI_REPORT is set. `tool` passes `--tool`.
 */
export function report(payload: Record<string, unknown>, tool: boolean): Promise<string> {
  void payload;
  void tool;
  throw new Error("not built yet: the report of pi's extension");
}

/**
 * The extension, as pi calls it once it loaded the file, and again after `/new`. With no
 * YAAR_PI_REPORT set, outside the app, it registers nothing and returns.
 */
export default function yaar(pi: ExtensionAPI): void {
  void pi;
  void Type;
  throw new Error("not built yet: pi's extension");
}

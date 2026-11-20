// The extension of the app YAAR for pi: it tells the app what pi does, hands a review the
// user attached in the app to the user's message, and gives the agent the reply tool.
//
// pi loads it from the app's own folder, through the `-e` the app's `pi` wrapper adds; a
// pi started outside the app's shells never sees it. It is TypeScript, as pi requires,
// written with no syntax that type stripping cannot erase, so that Node runs it as it is in
// its test. It decides nothing: every report goes, as it is, to the reporting program the
// wrapper names in YAAR_PI_REPORT, and the app's answer comes back from it. What each
// report holds and what it is answered is written down in the crate doc of
// `yaar-adapter-pi`; this file follows that doc.
//
// It registers no handler for the trust of a folder, nor for any dialog of pi's: the user
// answers them.

import { Type } from "@earendil-works/pi-ai";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

/** The variable the app's wrapper sets to the absolute path of the reporting program. */
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
 * Runs the reporting program with `payload` on its standard input, after every report
 * started before it has ended, and gives what it printed: nothing, an empty text, when
 * the program cannot be run or no YAAR_PI_REPORT is set. `tool` passes `--tool`.
 */
export function report(payload: Record<string, unknown>, tool: boolean): Promise<string> {
  void payload;
  void tool;
  throw new Error("not built yet: the report of pi's extension");
}

/** The extension, as pi calls it once it loaded the file, and again after `/new`. */
export default function yaar(pi: ExtensionAPI): void {
  void pi;
  void Type;
  throw new Error("not built yet: pi's extension");
}

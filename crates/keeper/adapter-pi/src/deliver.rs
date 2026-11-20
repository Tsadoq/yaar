//! The delivery module of pi: how a review rides with the user's message, and pi's command
//! for bringing a session back.
//!
//! pi takes a review of any length inside the user's own message: the extension's `input`
//! handler, answered `{"review": "<text>"}`, returns the user's text, a blank line and the
//! review as the message pi hands the model, whole, at any length, whether pi is idle or
//! queues the message while it works (the crate's doc). Nothing is written to a file and no
//! read has to be allowed: pi asks no permission.

use std::io;
use std::path::Path;

use yaar_agent::{CliSession, Delivery, Rendered};

/// The instruction that goes first in every review delivered to pi: how to answer each
/// thread with the reply tool the extension registers, and nothing that resolves one.
pub const INSTRUCTION: &str = "This is a code review of your changes, attached to the \
user's message. Address each thread below. For each thread, call the tool \
reply_to_thread once, with `thread` set to the id written after \"Thread\" in its \
heading, `body` set to your reply to the reviewer, and `addressed` set to true when \
your change addresses the comment, or false when you disagree or need the reviewer's \
answer. Only the reviewer resolves a thread.";

/// pi's delivery module: the rendered review whole, in the answer the extension's `input`
/// handler reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToPi;

impl Delivery for ToPi {
    fn instruction(&self) -> &'static str {
        INSTRUCTION
    }

    /// The JSON object `{"review": <rendered.text>}`, as bytes: the text whole, whatever
    /// its length, in a JSON string the extension reads back byte for byte. It never
    /// fails and writes nothing.
    fn hand_out(&mut self, rendered: &Rendered<'_>) -> io::Result<Vec<u8>> {
        let _ = rendered;
        todo!("pi's delivery: the answer that carries the review")
    }
}

/// The command that brings `session` back in the user's shell, as pi measured it: `pi
/// --session-dir '<the folder of cli_record>' --session '<id>'` when `cli_record`, the
/// session's file, names a folder, else `pi --session '<id>'`; each word single-quoted for
/// `sh` (a `'` inside written `'\''`).
pub(crate) fn resume(session: &CliSession, cli_record: Option<&Path>) -> String {
    let _ = (session, cli_record);
    todo!("pi's command for bringing a session back")
}

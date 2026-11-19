//! What the two programs say about terminals: which one, how large, what runs in it, and
//! how its program ended.
//!
//! The bytes a user types and the bytes a program writes are not requests and not facts.
//! They travel in two frames of their own, [`crate::Message::Input`] and
//! [`crate::Message::Output`], which nobody answers: a stream has no reply.

use std::fmt;

use crate::LinkError;
use crate::wire::{In, Out};

/// Names one terminal the Keeper holds, on the Link and in whatever is kept about it.
///
/// The Keeper chooses it when it opens the terminal: 32 lowercase hexadecimal digits,
/// never given to another terminal, also after the Keeper restarts. The Window treats it
/// as a name and reads nothing into it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TerminalId(String);

impl TerminalId {
    /// The id with this text.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The text of the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TerminalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The size of a terminal, in character cells: what its program is told it has.
///
/// The Link carries any value. The Keeper takes a 0 as 1, since a terminal holds at least
/// one cell each way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TerminalSize {
    /// How many characters a line holds.
    pub columns: u16,
    /// How many lines the screen holds.
    pub rows: u16,
}

/// What a new terminal runs.
///
/// The resumption of an agent's session is a further kind of the same field, a new
/// variant with a number of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Program {
    /// The user's shell, started as a login shell in the folder of the workspace.
    Shell,
    /// The launcher of an agent CLI: the user's shell, as for [`Program::Shell`], told to
    /// run that CLI as when the user types its name, so that it gets everything a CLI
    /// typed in a shell of the app gets. Refused when the Keeper has no Adapter by that
    /// name.
    Agent {
        /// The Adapter of the CLI, by its name: `claude`.
        agent: String,
    },
}

/// How the program of a terminal ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// It exited by itself, with this status.
    Exited(i32),
    /// A signal ended it: this one.
    Signalled(i32),
}

impl fmt::Display for Ending {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exited(code) => write!(f, "exited with status {code}"),
            Self::Signalled(signal) => write!(f, "ended by signal {signal}"),
        }
    }
}

pub(crate) fn write_size(out: &mut Out, size: TerminalSize) {
    out.u16(size.columns);
    out.u16(size.rows);
}

pub(crate) fn read_size(input: &mut In<'_>) -> Result<TerminalSize, LinkError> {
    Ok(TerminalSize {
        columns: input.u16()?,
        rows: input.u16()?,
    })
}

pub(crate) fn write_program(out: &mut Out, program: &Program) {
    match program {
        Program::Shell => out.u16(1),
        Program::Agent { agent } => {
            out.u16(2);
            out.text(agent);
        }
    }
}

pub(crate) fn read_program(input: &mut In<'_>) -> Result<Program, LinkError> {
    match input.u16()? {
        1 => Ok(Program::Shell),
        2 => Ok(Program::Agent {
            agent: input.text()?,
        }),
        _ => Err(LinkError::Malformed("unknown kind of program")),
    }
}

pub(crate) fn write_ending(out: &mut Out, ending: Ending) {
    let (kind, value) = match ending {
        Ending::Exited(code) => (0, code),
        Ending::Signalled(signal) => (1, signal),
    };
    out.u8(kind);
    out.u32(u32::from_le_bytes(value.to_le_bytes()));
}

pub(crate) fn read_ending(input: &mut In<'_>) -> Result<Ending, LinkError> {
    let kind = input.u8()?;
    let value = i32::from_le_bytes(input.u32()?.to_le_bytes());
    match kind {
        0 => Ok(Ending::Exited(value)),
        1 => Ok(Ending::Signalled(value)),
        _ => Err(LinkError::Malformed("a program ended in an unknown way")),
    }
}

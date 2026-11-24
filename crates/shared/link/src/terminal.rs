use std::fmt;

use crate::LinkError;
use crate::wire::{In, Out};

/// 32 lowercase hexadecimal digits, chosen by the Keeper and never given to another terminal, also
/// after a restart.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TerminalId(String);

impl TerminalId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TerminalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// In character cells. The Link carries any value; the Keeper takes a 0 as 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TerminalSize {
    pub columns: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Program {
    Shell,
    /// The user's shell, told to run the CLI named `agent` as if the user typed it. Refused when
    /// the Keeper has no Adapter by that name.
    Agent {
        agent: String,
    },
    /// As [`Program::Agent`], for a review the user attached: the first session of that CLI that
    /// reports from this terminal takes the review, and its first message carries what is attached.
    AgentFor {
        agent: String,
        review: crate::ReviewId,
    },
    /// Runs the CLI's own command for bringing the session back (`claude --resume '<its CLI id>'`):
    /// the terminal ends with the CLI, the session keeps its id and review. Only for a
    /// [`crate::Session::resumable`] session.
    Resume {
        session: crate::SessionId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    Exited(i32),
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
        Program::AgentFor { agent, review } => {
            out.u16(3);
            out.text(agent);
            out.text(review.as_str());
        }
        Program::Resume { session } => {
            out.u16(4);
            out.text(session.as_str());
        }
    }
}

pub(crate) fn read_program(input: &mut In<'_>) -> Result<Program, LinkError> {
    match input.u16()? {
        1 => Ok(Program::Shell),
        2 => Ok(Program::Agent {
            agent: input.text()?,
        }),
        3 => Ok(Program::AgentFor {
            agent: input.text()?,
            review: crate::ReviewId::new(input.text()?),
        }),
        4 => Ok(Program::Resume {
            session: crate::SessionId::new(input.text()?),
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

//! The Language Server Protocol's frames, addresses and positions, and the shapes of the
//! three answers the Navigator reads; nothing of any one server.
//!
//! A frame is a header of `Content-Length: <n>`, any other header lines, an empty line, all
//! ended by `\r\n`, then `n` bytes of one JSON value. Positions are counted the protocol's
//! default way, lines from 0 and characters in UTF-16 code units; the Navigator's own
//! [`crate::Place`] and [`crate::Location`] count lines from 1 and columns in bytes of the
//! line's UTF-8 text, and the driver converts with [`utf16_column`] and [`byte_column`].

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Writes `message` as one frame, and flushes.
pub fn write_frame(out: &mut impl Write, message: &Value) -> io::Result<()> {
    let _ = (out, message);
    todo!()
}

/// Reads one frame. None at the end of the stream before a frame began; an error for a
/// header with no `Content-Length`, a body that is not JSON, or a stream that ends inside
/// a frame.
pub fn read_frame(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let _ = input;
    todo!()
}

/// The `file://` address of an absolute path, every byte that an address cannot hold as
/// it is written with `%`.
pub fn file_uri(path: &Path) -> String {
    let _ = path;
    todo!()
}

/// The absolute path of a `file://` address; none for another scheme or an address that is
/// not one.
pub fn path_of_uri(uri: &str) -> Option<PathBuf> {
    let _ = uri;
    todo!()
}

/// The column, in UTF-16 code units, of the byte offset `byte` in `line`; past the end of
/// the line, its length.
pub fn utf16_column(line: &str, byte: u32) -> u32 {
    let _ = (line, byte);
    todo!()
}

/// The byte offset of the UTF-16 column `utf16` in `line`; past the end of the line, its
/// length; inside a character, that character's first byte.
pub fn byte_column(line: &str, utf16: u32) -> u32 {
    let _ = (line, utf16);
    todo!()
}

/// The text of a `textDocument/hover` result: the value of a `MarkupContent`, a marked
/// string, or each of an array of them joined by an empty line; none for null or empty.
/// Kept as the server wrote it, Markdown fences and all.
pub fn hover_text(result: &Value) -> Option<String> {
    let _ = result;
    todo!()
}

/// One place of a `textDocument/definition` or `textDocument/references` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spot {
    /// The file.
    pub path: PathBuf,
    /// The line, from 0.
    pub line: u32,
    /// The character, in UTF-16 code units, from 0.
    pub character: u32,
}

/// The places of a definition or references result: a `Location`, an array of them, or an
/// array of `LocationLink`s (their `targetUri` and `targetSelectionRange`), in the server's
/// order; empty for null. A place whose address is not a file is left out.
pub fn spots(result: &Value) -> Vec<Spot> {
    let _ = result;
    todo!()
}

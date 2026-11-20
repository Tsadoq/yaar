//! The Language Server Protocol's frames, addresses and positions, and the shapes of the
//! three answers the Navigator reads; nothing of any one server.
//!
//! A frame is a header of `Content-Length: <n>`, any other header lines, an empty line, all
//! ended by `\r\n`, then `n` bytes of one JSON value. Positions are counted the protocol's
//! default way, lines from 0 and characters in UTF-16 code units; the Navigator's own
//! [`crate::Place`] and [`crate::Location`] count lines from 1 and columns in bytes of the
//! line's UTF-8 text, and the driver converts with [`utf16_column`] and [`byte_column`].

use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The longest header line a frame may have: a line of any length is not a header.
const HEADER_LINE_LIMIT: u64 = 8 * 1024;

/// Writes `message` as one frame, and flushes.
pub fn write_frame(out: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    // One write for the whole frame, so that a writer shared by two threads that each hold
    // its lock for one call never has the header of one frame and the body of another
    // side by side.
    let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    frame.extend_from_slice(&body);
    out.write_all(&frame)?;
    out.flush()
}

/// Reads one frame. None at the end of the stream before a frame began; an error for a
/// header with no `Content-Length`, a body that is not JSON, or a stream that ends inside
/// a frame.
pub fn read_frame(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length: Option<u64> = None;
    let mut first = true;
    loop {
        let mut line = Vec::new();
        let read = input
            .by_ref()
            .take(HEADER_LINE_LIMIT)
            .read_until(b'\n', &mut line)?;
        if read == 0 {
            return if first {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the stream ended inside a frame's header",
                ))
            };
        }
        first = false;
        if line.last() != Some(&b'\n') {
            return Err(if read as u64 >= HEADER_LINE_LIMIT {
                io::Error::new(io::ErrorKind::InvalidData, "a header line is too long")
            } else {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the stream ended inside a frame's header",
                )
            });
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\r', '\n']);
        if text.is_empty() {
            break;
        }
        if let Some((name, value)) = text.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = Some(value.trim().parse().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("the Content-Length {:?} is not a number", value.trim()),
                )
            })?);
        }
    }
    let Some(length) = length else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a frame's header has no Content-Length",
        ));
    };
    // Read what the stream has, up to the length, instead of reserving the length: a
    // length no stream will ever fill costs nothing.
    let mut body = Vec::new();
    input.by_ref().take(length).read_to_end(&mut body)?;
    if (body.len() as u64) < length {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the stream ended inside a frame's body",
        ));
    }
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// The `file://` address of an absolute path, every byte that an address cannot hold as
/// it is written with `%`. A path that is not absolute is a mistake of the caller: it
/// panics.
pub fn file_uri(path: &Path) -> String {
    url::Url::from_file_path(path)
        .unwrap_or_else(|()| panic!("{path:?} is not an absolute path"))
        .into()
}

/// The absolute path of a `file://` address; none for another scheme or an address that is
/// not one.
pub fn path_of_uri(uri: &str) -> Option<PathBuf> {
    url::Url::parse(uri).ok()?.to_file_path().ok()
}

/// The column, in UTF-16 code units, of the byte offset `byte` in `line`; past the end of
/// the line, its length.
pub fn utf16_column(line: &str, byte: u32) -> u32 {
    let mut units = 0;
    for (start, character) in line.char_indices() {
        // A character the offset falls inside is not counted: its column is where it starts.
        if start + character.len_utf8() > byte as usize {
            break;
        }
        units += character.len_utf16() as u32;
    }
    units
}

/// The byte offset of the UTF-16 column `utf16` in `line`; past the end of the line, its
/// length; inside a character, that character's first byte.
pub fn byte_column(line: &str, utf16: u32) -> u32 {
    let mut units = 0;
    for (start, character) in line.char_indices() {
        units += character.len_utf16() as u32;
        if units > utf16 {
            return start as u32;
        }
    }
    line.len() as u32
}

/// The text of a `textDocument/hover` result: the value of a `MarkupContent`, a marked
/// string, or each of an array of them joined by an empty line; none for null or empty.
/// Kept as the server wrote it, Markdown fences and all.
pub fn hover_text(result: &Value) -> Option<String> {
    fn part(value: &Value) -> Option<&str> {
        match value {
            Value::String(text) => Some(text),
            Value::Object(_) => value.get("value").and_then(Value::as_str),
            _ => None,
        }
    }
    let contents = result.get("contents")?;
    let parts: Vec<&str> = match contents {
        Value::Array(items) => items.iter().filter_map(part).collect(),
        one => part(one).into_iter().collect(),
    };
    let parts: Vec<&str> = parts
        .into_iter()
        .filter(|text| !text.trim().is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
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
    fn spot(value: &Value) -> Option<Spot> {
        let (uri, range) = match value.get("targetUri") {
            Some(uri) => (
                uri,
                value
                    .get("targetSelectionRange")
                    .or_else(|| value.get("targetRange"))?,
            ),
            None => (value.get("uri")?, value.get("range")?),
        };
        let start = range.get("start")?;
        Some(Spot {
            path: path_of_uri(uri.as_str()?)?,
            line: u32::try_from(start.get("line")?.as_u64()?).ok()?,
            character: u32::try_from(start.get("character")?.as_u64()?).ok()?,
        })
    }
    match result {
        Value::Array(items) => items.iter().filter_map(spot).collect(),
        Value::Object(_) => spot(result).into_iter().collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    fn framed(message: &Value) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, message).expect("write");
        bytes
    }

    #[test]
    fn a_frame_written_is_a_length_header_an_empty_line_and_the_json() {
        let message = json!({"jsonrpc": "2.0", "id": 1, "method": "shutdown"});
        let body = serde_json::to_vec(&message).unwrap();
        let mut expected = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        expected.extend_from_slice(&body);
        assert_eq!(framed(&message), expected);
    }

    #[test]
    fn a_frame_written_then_read_is_the_message() {
        let message = json!({"text": "été 𝄞 \"quoted\"\r\n", "n": [1, 2, null]});
        let bytes = framed(&message);
        let mut input = Cursor::new(bytes);
        assert_eq!(read_frame(&mut input).unwrap(), Some(message));
        assert_eq!(read_frame(&mut input).unwrap(), None);
    }

    #[test]
    fn the_length_counts_bytes_not_characters() {
        let message = json!({"text": "αβ𝄞"});
        let bytes = framed(&message);
        let header_end = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
        assert_eq!(
            header,
            format!("Content-Length: {}\r\n\r\n", bytes.len() - header_end)
        );
        assert!(bytes.len() - header_end > "{\"text\":\"αβ𝄞\"}".chars().count());
    }

    #[test]
    fn two_frames_back_to_back_are_read_one_after_the_other() {
        let first = json!({"id": 1});
        let second = json!({"id": 2, "result": "second"});
        let mut bytes = framed(&first);
        bytes.extend(framed(&second));
        let mut input = Cursor::new(bytes);
        assert_eq!(read_frame(&mut input).unwrap(), Some(first));
        assert_eq!(read_frame(&mut input).unwrap(), Some(second));
        assert_eq!(read_frame(&mut input).unwrap(), None);
    }

    #[test]
    fn other_header_lines_and_the_case_of_the_length_name_do_not_matter() {
        let body = br#"{"a":1}"#;
        let mut bytes =
            b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\ncontent-length: 7\r\n\r\n"
                .to_vec();
        bytes.extend_from_slice(body);
        assert_eq!(
            read_frame(&mut Cursor::new(bytes)).unwrap(),
            Some(json!({"a": 1}))
        );
    }

    #[test]
    fn a_header_with_no_length_is_refused() {
        let error =
            read_frame(&mut Cursor::new(b"Content-Type: x\r\n\r\n{}".to_vec())).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("Content-Length"), "{error}");
    }

    #[test]
    fn a_length_that_is_no_number_is_refused() {
        let error =
            read_frame(&mut Cursor::new(b"Content-Length: many\r\n\r\n{}".to_vec())).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_body_that_is_not_json_is_refused() {
        let error =
            read_frame(&mut Cursor::new(b"Content-Length: 3\r\n\r\nabc".to_vec())).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_stream_that_ends_inside_a_frame_is_an_error_and_not_the_end() {
        let in_the_body = read_frame(&mut Cursor::new(
            b"Content-Length: 20\r\n\r\n{\"a\":".to_vec(),
        ))
        .unwrap_err();
        assert_eq!(in_the_body.kind(), io::ErrorKind::UnexpectedEof);
        let in_the_header =
            read_frame(&mut Cursor::new(b"Content-Length: 2\r\n".to_vec())).unwrap_err();
        assert_eq!(in_the_header.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn a_length_far_beyond_the_stream_reads_what_there_is_and_fails_without_reserving_it() {
        let error = read_frame(&mut Cursor::new(
            b"Content-Length: 99999999999\r\n\r\n{}".to_vec(),
        ))
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn a_header_line_that_never_ends_is_refused_before_it_fills_memory() {
        let endless = vec![b'x'; 64 * 1024];
        let error = read_frame(&mut Cursor::new(endless)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn an_empty_stream_has_no_frame() {
        assert_eq!(read_frame(&mut Cursor::new(Vec::new())).unwrap(), None);
    }

    #[test]
    fn an_address_of_a_path_holds_nothing_an_address_cannot_and_reads_back() {
        let plain = Path::new("/work/tree/src/main.rs");
        assert_eq!(file_uri(plain), "file:///work/tree/src/main.rs");

        for path in [
            "/work/with space/a.rs",
            "/work/hash#tag/a.rs",
            "/work/question?/a.rs",
            "/work/percent%20/a.rs",
            "/work/été/𝄞/ключ.rs",
        ] {
            let uri = file_uri(Path::new(path));
            assert!(uri.starts_with("file:///"), "{uri}");
            assert!(
                uri.is_ascii() && !uri.contains(' ') && !uri[7..].contains(['#', '?']),
                "{uri}"
            );
            assert_eq!(path_of_uri(&uri), Some(PathBuf::from(path)), "{uri}");
        }
        assert_eq!(file_uri(Path::new("/a b/c#d")), "file:///a%20b/c%23d");
        assert_eq!(file_uri(Path::new("/été")), "file:///%C3%A9t%C3%A9");
    }

    #[test]
    fn an_address_that_is_not_a_file_gives_no_path() {
        assert_eq!(path_of_uri("untitled:Untitled-1"), None);
        assert_eq!(path_of_uri("https://example.com/a.rs"), None);
        assert_eq!(path_of_uri("not an address"), None);
        assert_eq!(path_of_uri(""), None);
        assert_eq!(
            path_of_uri("file:///a/b%20c.rs"),
            Some(PathBuf::from("/a/b c.rs"))
        );
    }

    #[test]
    fn columns_count_utf16_units_and_bytes_both_ways() {
        // α and β are 2 bytes and 1 unit, 𝄞 4 bytes and 2 units, x 1 and 1.
        let line = "αβ𝄞x";
        for (byte, unit) in [(0, 0), (2, 1), (4, 2), (8, 4), (9, 5)] {
            assert_eq!(utf16_column(line, byte), unit, "byte {byte}");
            assert_eq!(byte_column(line, unit), byte, "unit {unit}");
        }
    }

    #[test]
    fn a_column_past_the_end_of_the_line_is_its_length() {
        let line = "αβ𝄞x";
        assert_eq!(utf16_column(line, 99), 5);
        assert_eq!(byte_column(line, 99), 9);
        assert_eq!(utf16_column("", 0), 0);
        assert_eq!(byte_column("", 3), 0);
    }

    #[test]
    fn a_column_inside_a_character_is_where_that_character_starts() {
        let line = "αβ𝄞x";
        // Inside β (byte 3) and inside 𝄞 (bytes 5 to 7): the column of the character.
        assert_eq!(utf16_column(line, 3), 1);
        assert_eq!(utf16_column(line, 5), 2);
        assert_eq!(utf16_column(line, 7), 2);
        // The middle of the surrogate pair: unit 3.
        assert_eq!(byte_column(line, 3), 4);
    }

    #[test]
    fn the_text_of_a_hover_is_kept_as_the_server_wrote_it() {
        let markup =
            json!({"contents": {"kind": "markdown", "value": "```rust\nfn a()\n```\n\nDoc."}});
        assert_eq!(
            hover_text(&markup).as_deref(),
            Some("```rust\nfn a()\n```\n\nDoc.")
        );
        let plain = json!({"contents": {"kind": "plaintext", "value": "standin: total"}});
        assert_eq!(hover_text(&plain).as_deref(), Some("standin: total"));
        let string = json!({"contents": "just text"});
        assert_eq!(hover_text(&string).as_deref(), Some("just text"));
        let marked = json!({"contents": {"language": "ts", "value": "const a: number"}});
        assert_eq!(hover_text(&marked).as_deref(), Some("const a: number"));
    }

    #[test]
    fn the_parts_of_a_hover_array_are_joined_by_an_empty_line() {
        let parts = json!({"contents": [
            {"language": "rust", "value": "fn a()"},
            "Doc.",
            {"kind": "markdown", "value": "More."}
        ]});
        assert_eq!(
            hover_text(&parts).as_deref(),
            Some("fn a()\n\nDoc.\n\nMore.")
        );
    }

    #[test]
    fn a_hover_with_nothing_in_it_has_no_text() {
        assert_eq!(hover_text(&Value::Null), None);
        assert_eq!(hover_text(&json!({"contents": ""})), None);
        assert_eq!(hover_text(&json!({"contents": []})), None);
        assert_eq!(
            hover_text(&json!({"contents": {"kind": "plaintext", "value": ""}})),
            None
        );
        assert_eq!(
            hover_text(&json!({"contents": ["", {"value": "  "}]})),
            None
        );
        assert_eq!(hover_text(&json!({})), None);
        assert_eq!(
            hover_text(&json!({"contents": ["", "kept"]})).as_deref(),
            Some("kept")
        );
    }

    fn location(path: &str, line: u32, character: u32) -> Value {
        json!({
            "uri": file_uri(Path::new(path)),
            "range": {
                "start": {"line": line, "character": character},
                "end": {"line": line, "character": character + 3}
            }
        })
    }

    fn spot(path: &str, line: u32, character: u32) -> Spot {
        Spot {
            path: PathBuf::from(path),
            line,
            character,
        }
    }

    #[test]
    fn the_spots_of_a_location_an_array_and_location_links_are_read_in_the_servers_order() {
        let one = location("/w/a.rs", 4, 7);
        assert_eq!(spots(&one), vec![spot("/w/a.rs", 4, 7)]);

        let several = json!([location("/w/b.rs", 1, 2), location("/w/a.rs", 0, 0)]);
        assert_eq!(
            spots(&several),
            vec![spot("/w/b.rs", 1, 2), spot("/w/a.rs", 0, 0)]
        );

        let links = json!([{
            "originSelectionRange": {"start": {"line": 9, "character": 9}, "end": {"line": 9, "character": 12}},
            "targetUri": file_uri(Path::new("/w/c.rs")),
            "targetRange": {"start": {"line": 3, "character": 0}, "end": {"line": 8, "character": 1}},
            "targetSelectionRange": {"start": {"line": 3, "character": 4}, "end": {"line": 3, "character": 9}}
        }]);
        assert_eq!(spots(&links), vec![spot("/w/c.rs", 3, 4)]);
    }

    #[test]
    fn a_location_link_with_no_selection_range_uses_its_target_range() {
        let link = json!([{
            "targetUri": file_uri(Path::new("/w/c.rs")),
            "targetRange": {"start": {"line": 3, "character": 1}, "end": {"line": 8, "character": 1}}
        }]);
        assert_eq!(spots(&link), vec![spot("/w/c.rs", 3, 1)]);
    }

    #[test]
    fn nothing_and_what_is_not_a_file_leave_no_spot() {
        assert_eq!(spots(&Value::Null), Vec::<Spot>::new());
        assert_eq!(spots(&json!([])), Vec::<Spot>::new());
        let mixed = json!([
            {"uri": "untitled:x", "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}},
            location("/w/kept.rs", 2, 2),
            {"range": {"start": {"line": 0, "character": 0}}},
            "not a place"
        ]);
        assert_eq!(spots(&mixed), vec![spot("/w/kept.rs", 2, 2)]);
    }
}

//! The one entry written into pi's settings after the user's yes, and the kept answer.
//! The entry goes into the file's text, never by serialising parsed settings.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use serde_json::Value;
use yaar_agent::{Consent, ConsentAnswer};

use crate::{AGENT_FOLDER, CONSENT_FILE, EXTENSION, EXTENSION_FILE, SETTINGS_FILE, SETTINGS_KEY};

/// Retries for pi's lock, as pi takes it: 10 tries, 20 ms apart.
const LOCK_TRIES: u32 = 10;
const LOCK_WAIT: Duration = Duration::from_millis(20);

/// None while `home/.pi/agent` is not a folder. A kept yes whose entry is gone from pi's
/// settings stands as `NotAsked`. `problem` says why the settings cannot be read, or that a
/// kept no is still listed.
pub(crate) fn consent(home: &Path, folder: &Path) -> Option<Consent> {
    let agent = home.join(AGENT_FOLDER);
    if !agent.is_dir() {
        return None;
    }
    let settings = agent.join(SETTINGS_FILE);
    let extension = extension_of(folder);
    let kept = kept(folder);

    let (answer, problem) = match kept {
        // A no stands. A still-listed entry (the take-out failed while pi held its lock) is told
        // at every start; an unreadable file says nothing.
        ConsentAnswer::No => match is_listed(&settings, &extension) {
            Ok(true) => (
                ConsentAnswer::No,
                Some(format!(
                    "{}: the extension is still listed there, though your answer is no: \
                     answer no again to take it out",
                    settings.display()
                )),
            ),
            _ => (ConsentAnswer::No, None),
        },
        kept => match is_listed(&settings, &extension) {
            Ok(true) => (kept, None),
            Ok(false) => (ConsentAnswer::NotAsked, None),
            Err(problem) => (ConsentAnswer::NotAsked, Some(problem)),
        },
    };
    Some(Consent {
        question: question(&extension, &settings),
        answer,
        problem,
    })
}

/// A yes adds the entry, a no takes it out, `NotAsked` takes it out and removes the kept answer.
/// A file that cannot be read as pi writes it is left as it was, with `problem` saying so.
pub(crate) fn answer(home: &Path, folder: &Path, answer: ConsentAnswer) -> Option<Consent> {
    let agent = home.join(AGENT_FOLDER);
    if !agent.is_dir() {
        return None;
    }
    let settings = agent.join(SETTINGS_FILE);
    let extension = extension_of(folder);

    let problem = match answer {
        // A yes that was not written keeps nothing.
        ConsentAnswer::Yes => put_in(&settings, &extension)
            .and_then(|()| keep(folder, answer))
            .err(),
        ConsentAnswer::No | ConsentAnswer::NotAsked => {
            let taken = take_out(&settings, &extension);
            let kept = keep(folder, answer);
            taken.err().or(kept.err())
        }
    };

    let mut now = consent(home, folder)?;
    if problem.is_some() {
        now.problem = problem;
    }
    Some(now)
}

/// The path written into the settings: `folder` made absolute, no link resolved.
fn extension_of(folder: &Path) -> PathBuf {
    std::path::absolute(folder)
        .unwrap_or_else(|_| folder.to_owned())
        .join(EXTENSION)
        .join(EXTENSION_FILE)
}

fn question(extension: &Path, settings: &Path) -> String {
    format!(
        "pi can report what it does to the app and take your reviews if the app adds its \
         extension, {}, to the list \"{SETTINGS_KEY}\" of pi's settings, {}. pi then loads it \
         wherever you start pi; outside the app it does nothing. Install it? You can change \
         your answer later.",
        extension.display(),
        settings.display(),
    )
}

fn kept(folder: &Path) -> ConsentAnswer {
    match fs::read_to_string(folder.join(CONSENT_FILE)) {
        Ok(text) => match text.trim() {
            "yes" => ConsentAnswer::Yes,
            "no" => ConsentAnswer::No,
            _ => ConsentAnswer::NotAsked,
        },
        Err(_) => ConsentAnswer::NotAsked,
    }
}

fn keep(folder: &Path, answer: ConsentAnswer) -> Result<(), String> {
    let file = folder.join(CONSENT_FILE);
    let text = match answer {
        ConsentAnswer::Yes => "yes\n",
        ConsentAnswer::No => "no\n",
        ConsentAnswer::NotAsked => {
            return match fs::remove_file(&file) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(format!(
                    "the app could not forget your answer, kept in {} ({error})",
                    file.display()
                )),
                _ => Ok(()),
            };
        }
    };
    fs::create_dir_all(folder)
        .and_then(|()| write_through_a_temporary_file(&file, text.as_bytes(), None))
        .map_err(|error| {
            format!(
                "the app could not keep your answer in {} ({error})",
                file.display()
            )
        })
}

struct Loaded {
    text: Option<String>,
    real: PathBuf,
    permissions: Option<fs::Permissions>,
}

fn load(settings: &Path) -> Result<Loaded, String> {
    let cannot = |what: &str, error: io::Error| {
        format!("{}: the app could not {what} ({error})", settings.display())
    };
    let real = match fs::symlink_metadata(settings) {
        Ok(meta) if meta.is_symlink() => fs::canonicalize(settings)
            .map_err(|error| cannot("follow the link, whose target cannot be reached", error))?,
        Ok(_) => settings.to_owned(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Loaded {
                text: None,
                real: settings.to_owned(),
                permissions: None,
            });
        }
        Err(error) => return Err(cannot("look at it", error)),
    };
    let bytes = fs::read(&real).map_err(|error| cannot("read it", error))?;
    let permissions = fs::metadata(&real).ok().map(|meta| meta.permissions());
    let text =
        String::from_utf8(bytes).map_err(|_| unreadable(settings, "it is not text (UTF-8)"))?;
    Ok(Loaded {
        text: Some(text),
        real,
        permissions,
    })
}

fn unreadable(settings: &Path, why: &str) -> String {
    format!(
        "{}: the app cannot read it as pi writes it ({why}), so it leaves it as it is",
        settings.display()
    )
}

fn is_listed(settings: &Path, extension: &Path) -> Result<bool, String> {
    let Some(text) = load(settings)?.text else {
        return Ok(false);
    };
    let (_, body) = split_mark(&text);
    let layout = read_layout(body).map_err(|why| unreadable(settings, &why))?;
    Ok(extension
        .to_str()
        .is_some_and(|entry| !layout.matches(body, entry).is_empty()))
}

fn put_in(settings: &Path, extension: &Path) -> Result<(), String> {
    let Some(entry) = extension.to_str() else {
        return Err(format!(
            "{}: the extension's path is not text, which a settings file cannot hold",
            extension.display()
        ));
    };
    change(settings, &|text| match text {
        None => Ok(Some(made_as_pi_would(entry))),
        Some(text) => with_entry(text, entry),
    })
}

fn take_out(settings: &Path, extension: &Path) -> Result<(), String> {
    let Some(entry) = extension.to_str() else {
        // A path that is not text cannot have been written, so there is nothing to take out.
        return Ok(());
    };
    change(settings, &|text| match text {
        None => Ok(None),
        Some(text) => without_entry(text, entry),
    })
}

/// Takes the text, none where there is no file; gives the new text, none where nothing
/// changes, or why pi cannot read the text.
type Edit<'a> = dyn Fn(Option<&str>) -> Result<Option<String>, String> + 'a;

/// The settings are read once without the lock first, so a change that is none takes no lock
/// and writes nothing.
fn change(settings: &Path, edit: &Edit<'_>) -> Result<(), String> {
    let edit =
        |loaded: &Loaded| edit(loaded.text.as_deref()).map_err(|why| unreadable(settings, &why));
    if edit(&load(settings)?)?.is_none() {
        return Ok(());
    }
    let _lock = Lock::take(settings)?;
    // What pi wrote while the app waited for its lock is what is changed.
    let loaded = load(settings)?;
    let Some(text) = edit(&loaded)? else {
        return Ok(());
    };
    write_through_a_temporary_file(&loaded.real, text.as_bytes(), loaded.permissions).map_err(
        |error| {
            format!(
                "{}: the app could not write it ({error}), so it is as it was",
                settings.display()
            )
        },
    )
}

/// pi's own lock: the folder `<settings>.lock`, made with `mkdir`, which fails where it exists.
struct Lock(PathBuf);

impl Lock {
    fn take(settings: &Path) -> Result<Lock, String> {
        let mut name = settings.file_name().map(OsString::from).unwrap_or_default();
        name.push(".lock");
        let lock = settings.with_file_name(name);
        for attempt in 0..LOCK_TRIES {
            match fs::create_dir(&lock) {
                Ok(()) => return Ok(Lock(lock)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    if attempt + 1 < LOCK_TRIES {
                        thread::sleep(LOCK_WAIT);
                    }
                }
                Err(error) => {
                    return Err(format!(
                        "the app could not take pi's lock {} ({error})",
                        lock.display()
                    ));
                }
            }
        }
        Err("pi is changing its settings: try again".to_owned())
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}

/// A reader sees the old file or the new one; a failure leaves the old one and no temporary file.
fn write_through_a_temporary_file(
    file: &Path,
    bytes: &[u8],
    permissions: Option<fs::Permissions>,
) -> io::Result<()> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let mut name = OsString::from(".");
    name.push(file.file_name().unwrap_or_default());
    name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = file.with_file_name(name);

    let written = (|| {
        let mut out = fs::File::options()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        out.write_all(bytes)?;
        out.sync_all()?;
        if let Some(permissions) = permissions {
            fs::set_permissions(&temporary, permissions)?;
        }
        fs::rename(&temporary, file)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

fn split_mark(text: &str) -> (&str, &str) {
    match text.strip_prefix('\u{feff}') {
        Some(body) => ("\u{feff}", body),
        None => ("", text),
    }
}

fn made_as_pi_would(entry: &str) -> String {
    format!(
        "{{\n  {}: [\n    {}\n  ]\n}}\n",
        json_string(SETTINGS_KEY),
        json_string(entry)
    )
}

fn json_string(text: &str) -> String {
    serde_json::to_string(text).expect("a string is JSON")
}

fn with_entry(text: &str, entry: &str) -> Result<Option<String>, String> {
    let (mark, body) = split_mark(text);
    let layout = read_layout(body)?;
    if !layout.matches(body, entry).is_empty() {
        return Ok(None);
    }
    let quoted = json_string(entry);
    let key = json_string(SETTINGS_KEY);

    let (at, insert) = match (&layout.list, layout.members.last()) {
        (Some(list), _) => match list.elements.last() {
            Some(last) => (last.end, format!(", {quoted}")),
            None => (list.open + 1, quoted),
        },
        (None, None) => {
            let tail = if layout.close == layout.open + 1 {
                "\n"
            } else {
                ""
            };
            (
                layout.open + 1,
                format!("\n  {key}: [\n    {quoted}\n  ]{tail}"),
            )
        }
        (None, Some(last)) => {
            let lead = &body[last.lead..last.key_start];
            let colon = &body[last.key_end..last.value_start];
            (last.value_end, format!(",{lead}{key}{colon}[{quoted}]"))
        }
    };
    Ok(Some(format!(
        "{mark}{}{insert}{}",
        &body[..at],
        &body[at..]
    )))
}

fn without_entry(text: &str, entry: &str) -> Result<Option<String>, String> {
    let (mark, body) = split_mark(text);
    let mut body = body.to_owned();
    let mut changed = false;
    loop {
        let layout = read_layout(&body)?;
        let Some(&found) = layout.matches(&body, entry).first() else {
            break;
        };
        let list = layout.list.as_ref().expect("a match is in the list");
        let (from, to) = list.span_of(found);
        body.replace_range(from..to, "");
        changed = true;
    }
    Ok(changed.then(|| format!("{mark}{body}")))
}

struct Layout {
    open: usize,
    close: usize,
    members: Vec<Member>,
    list: Option<List>,
}

struct Member {
    lead: usize,
    key_start: usize,
    key_end: usize,
    value_start: usize,
    value_end: usize,
}

struct List {
    open: usize,
    close: usize,
    elements: Vec<Span>,
}

#[derive(Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
}

impl Layout {
    /// Matched by decoded string, however it is spelled in the file.
    fn matches(&self, body: &str, entry: &str) -> Vec<usize> {
        let Some(list) = &self.list else {
            return Vec::new();
        };
        list.elements
            .iter()
            .enumerate()
            .filter(|(_, span)| {
                serde_json::from_str::<String>(&body[span.start..span.end])
                    .is_ok_and(|string| string == entry)
            })
            .map(|(index, _)| index)
            .collect()
    }
}

impl List {
    fn span_of(&self, index: usize) -> (usize, usize) {
        let this = self.elements[index];
        if self.elements.len() == 1 {
            // Alone and right after `[`, only the element goes and any blanks before `]` stay; set
            // apart from `[`, the blanks go too and the list is `[]`.
            return if self.open + 1 == this.start {
                (this.start, this.end)
            } else {
                (self.open + 1, self.close)
            };
        }
        match self.elements.get(index + 1) {
            Some(next) => (this.start, next.start),
            None => (self.elements[index - 1].end, this.end),
        }
    }
}

fn read_layout(body: &str) -> Result<Layout, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| format!("it is not JSON: {error}"))?;
    if !value.is_object() {
        return Err("it is not a JSON object".to_owned());
    }
    let scan = Scan {
        bytes: body.as_bytes(),
        text: body,
    };
    let open = scan.blanks(0);
    let (members, close) = scan.object(open)?;

    let mut list = None;
    for member in &members {
        if scan.string(member.key_start, member.key_end)? != SETTINGS_KEY {
            continue;
        }
        if list.is_some() {
            return Err(format!("it has \"{SETTINGS_KEY}\" more than once"));
        }
        let not_strings = || format!("\"{SETTINGS_KEY}\" is not a list of strings");
        if scan.bytes[member.value_start] != b'[' {
            return Err(not_strings());
        }
        let found = scan.list(member.value_start)?;
        if found
            .elements
            .iter()
            .any(|element| scan.bytes[element.start] != b'"')
        {
            return Err(not_strings());
        }
        list = Some(found);
    }
    Ok(Layout {
        open,
        close,
        members,
        list,
    })
}

struct Scan<'a> {
    bytes: &'a [u8],
    text: &'a str,
}

impl Scan<'_> {
    fn broken<T>() -> Result<T, String> {
        Err("it is laid out in a way the app does not follow".to_owned())
    }

    fn blanks(&self, mut at: usize) -> usize {
        while matches!(self.bytes.get(at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            at += 1;
        }
        at
    }

    fn string(&self, start: usize, end: usize) -> Result<String, String> {
        serde_json::from_str(&self.text[start..end])
            .map_err(|_| "it holds a string pi cannot read".to_owned())
    }

    fn string_end(&self, mut at: usize) -> Result<usize, String> {
        at += 1;
        loop {
            match self.bytes.get(at) {
                Some(b'\\') => at += 2,
                Some(b'"') => return Ok(at + 1),
                Some(_) => at += 1,
                None => return Self::broken(),
            }
        }
    }

    fn value_end(&self, mut at: usize) -> Result<usize, String> {
        match self.bytes.get(at) {
            Some(b'"') => self.string_end(at),
            Some(b'{' | b'[') => {
                let mut depth = 0usize;
                loop {
                    match self.bytes.get(at) {
                        Some(b'"') => at = self.string_end(at)?,
                        Some(b'{' | b'[') => {
                            depth += 1;
                            at += 1;
                        }
                        Some(b'}' | b']') => {
                            depth -= 1;
                            at += 1;
                            if depth == 0 {
                                return Ok(at);
                            }
                        }
                        Some(_) => at += 1,
                        None => return Self::broken(),
                    }
                }
            }
            Some(_) => {
                while !matches!(
                    self.bytes.get(at),
                    None | Some(b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r')
                ) {
                    at += 1;
                }
                Ok(at)
            }
            None => Self::broken(),
        }
    }

    fn object(&self, open: usize) -> Result<(Vec<Member>, usize), String> {
        let mut members = Vec::new();
        let mut at = open + 1;
        loop {
            let lead = at;
            at = self.blanks(at);
            match self.bytes.get(at) {
                Some(b'}') => return Ok((members, at)),
                Some(b'"') => {}
                _ => return Self::broken(),
            }
            let key_start = at;
            let key_end = self.string_end(at)?;
            at = self.blanks(key_end);
            if self.bytes.get(at) != Some(&b':') {
                return Self::broken();
            }
            let value_start = self.blanks(at + 1);
            let value_end = self.value_end(value_start)?;
            members.push(Member {
                lead,
                key_start,
                key_end,
                value_start,
                value_end,
            });
            at = self.blanks(value_end);
            match self.bytes.get(at) {
                Some(b',') => at += 1,
                Some(b'}') => return Ok((members, at)),
                _ => return Self::broken(),
            }
        }
    }

    fn list(&self, open: usize) -> Result<List, String> {
        let mut elements = Vec::new();
        let mut at = open + 1;
        loop {
            at = self.blanks(at);
            match self.bytes.get(at) {
                Some(b']') => {
                    return Ok(List {
                        open,
                        close: at,
                        elements,
                    });
                }
                Some(_) => {}
                None => return Self::broken(),
            }
            let end = self.value_end(at)?;
            elements.push(Span { start: at, end });
            at = self.blanks(end);
            match self.bytes.get(at) {
                Some(b',') => at += 1,
                Some(b']') => {}
                _ => return Self::broken(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &str = "/data/pi/extension/yaar.ts";

    #[test]
    fn every_spelling_of_the_entry_is_taken_out_and_the_rest_stays() {
        let text = r#"{"extensions": ["/a", "\/data\/pi\/extension\/yaar.ts", "/b", "/data/pi/extension/yaar.ts"]}"#;

        let out = without_entry(text, ENTRY).unwrap();

        assert_eq!(out.as_deref(), Some(r#"{"extensions": ["/a", "/b"]}"#));
    }

    #[test]
    fn the_only_entry_leaves_an_empty_list_whether_it_stood_on_a_line_of_its_own_or_not() {
        let alone = |text: &str| without_entry(text, ENTRY).unwrap().unwrap();

        assert_eq!(
            alone(r#"{"extensions": ["/data/pi/extension/yaar.ts"]}"#),
            r#"{"extensions": []}"#
        );
        assert_eq!(
            alone("{\"extensions\": [ \"/data/pi/extension/yaar.ts\" ]}"),
            r#"{"extensions": []}"#
        );
        assert_eq!(
            alone("{\n  \"extensions\": [\n    \"/data/pi/extension/yaar.ts\"\n  ]\n}\n"),
            "{\n  \"extensions\": []\n}\n"
        );
    }

    #[test]
    fn a_text_without_the_entry_is_not_changed() {
        assert_eq!(
            without_entry(r#"{"extensions": ["/a"]}"#, ENTRY).unwrap(),
            None
        );
        assert_eq!(without_entry(r#"{"a": 1}"#, ENTRY).unwrap(), None);
        assert_eq!(
            without_entry(
                r#"{"x": {"extensions": ["/data/pi/extension/yaar.ts"]}}"#,
                ENTRY
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn a_string_that_ends_in_an_escaped_backslash_does_not_hide_the_next_quote() {
        let text = r#"{"a": "x\\", "b": ["]", "}"], "c": [], "extensions": []}"#;

        let out = with_entry(text, ENTRY).unwrap().unwrap();

        assert_eq!(
            out,
            r#"{"a": "x\\", "b": ["]", "}"], "c": [], "extensions": ["/data/pi/extension/yaar.ts"]}"#
        );
    }

    #[test]
    fn a_member_that_is_a_number_a_literal_or_a_nested_value_ends_where_it_ends() {
        for last in ["-1.5e3", "true", "null", "[[1], {\"a\": [2]}]", "\"s\""] {
            let text = format!("{{\"a\": 1, \"z\": {last}}}");

            let out = with_entry(&text, ENTRY).unwrap().unwrap();

            assert_eq!(
                out,
                format!("{{\"a\": 1, \"z\": {last}, \"extensions\": [\"{ENTRY}\"]}}"),
                "{last}"
            );
        }
    }

    #[test]
    fn a_text_pi_would_not_read_is_refused_by_both_edits_with_a_reason() {
        for text in [
            "",
            "[]",
            "{",
            r#"{"extensions": [1]}"#,
            "{// c\n}",
            "{\"a\": 1,}",
        ] {
            assert!(with_entry(text, ENTRY).is_err(), "{text:?}");
            assert!(without_entry(text, ENTRY).is_err(), "{text:?}");
        }
    }

    #[test]
    fn a_byte_order_mark_stays_where_it_is_on_both_edits() {
        let text = "\u{feff}{\"extensions\": []}";

        let with = with_entry(text, ENTRY).unwrap().unwrap();
        let without = without_entry(&with, ENTRY).unwrap().unwrap();

        assert!(with.starts_with("\u{feff}{"));
        assert_eq!(without, text);
    }

    #[test]
    fn the_entry_is_escaped_as_a_json_string() {
        let entry = "/data/\"q\" \\b/\u{e9}\tx";

        let out = with_entry("{}", entry).unwrap().unwrap();

        let value: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["extensions"][0], entry);
        assert_eq!(
            without_entry(&out, entry).unwrap().unwrap(),
            "{\n  \"extensions\": []\n}"
        );
    }
}

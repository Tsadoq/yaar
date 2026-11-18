//! The id of a workspace, in the git folder of its worktree.

use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{ID_FILE, WorkspaceError, WorkspaceId};

/// The id in the git folder of a worktree; none when no id was written there yet, and
/// also when what is there is not an id.
pub(crate) fn read(git_dir: &Path) -> Result<Option<WorkspaceId>, WorkspaceError> {
    let file = git_dir.join(ID_FILE);
    match fs::read_to_string(&file) {
        Ok(content) => {
            let id = content.strip_suffix('\n').unwrap_or(&content);
            Ok(is_id(id).then(|| WorkspaceId::new(id)))
        }
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::InvalidData
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(WorkspaceError::Id(file, e)),
    }
}

/// Draws a new id and writes it into the git folder of a worktree, over whatever id
/// was there. The file is written under another name and then renamed, so that nobody
/// reads half an id.
pub(crate) fn give(git_dir: &Path) -> Result<WorkspaceId, WorkspaceError> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let id = random()?;
    let file = git_dir.join(ID_FILE);
    let unfinished = git_dir.join(format!(
        "{ID_FILE}.{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let written = fs::File::create(&unfinished)
        .and_then(|mut to| {
            to.write_all(format!("{id}\n").as_bytes())?;
            to.sync_all()
        })
        .and_then(|()| fs::rename(&unfinished, &file));
    match written {
        Ok(()) => Ok(WorkspaceId::new(id)),
        Err(e) => {
            let _ = fs::remove_file(&unfinished);
            Err(WorkspaceError::Id(file, e))
        }
    }
}

/// 32 lowercase hexadecimal digits from the random numbers of the system.
pub(crate) fn random() -> Result<String, WorkspaceError> {
    let source = Path::new("/dev/urandom");
    let mut bytes = [0u8; 16];
    fs::File::open(source)
        .and_then(|mut from| from.read_exact(&mut bytes))
        .map_err(|e| WorkspaceError::Id(source.to_owned(), e))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn is_id(text: &str) -> bool {
    text.len() == 32
        && text
            .bytes()
            .all(|digit| digit.is_ascii_digit() || (b'a'..=b'f').contains(&digit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_32_lowercase_hexadecimal_digits_and_two_are_not_the_same() {
        let (one, other) = (random().expect("an id"), random().expect("an id"));
        assert!(is_id(&one) && is_id(&other), "{one} {other}");
        assert_ne!(one, other);

        assert!(!is_id("0123456789ABCDEF0123456789ABCDEF"));
        assert!(!is_id("0123456789abcdef"));
        assert!(!is_id("0123456789abcdef0123456789abcdeg"));
    }
}

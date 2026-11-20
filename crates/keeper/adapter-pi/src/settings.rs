//! The one entry the Adapter writes into pi's own settings after the user's yes, and the
//! answer it keeps: what the crate's doc says under "How pi reaches the app".
//!
//! Every function takes the user's home, under which pi keeps its settings
//! (`home/.pi/agent/settings.json`), and the Adapter's own folder of the app, where the
//! extension lies and the answer is kept. Nothing here reads the environment: a test hands
//! a home of its own, and so no test can reach the user's real settings of pi.

use std::path::Path;

use yaar_agent::{Consent, ConsentAnswer};

/// How the user's leave stands for the pi installed under `home`, as
/// [`yaar_agent::Adapter::consent`] says: none while `home/.pi/agent` is not a folder. Its
/// `answer` is the kept one, but a kept yes whose entry is not in pi's settings is
/// [`ConsentAnswer::NotAsked`]; `problem` says why the settings cannot be read, where they
/// cannot, and is none for a kept no, which needs nothing read. Writes nothing.
pub(crate) fn consent(home: &Path, folder: &Path) -> Option<Consent> {
    let _ = (home, folder);
    todo!("the consent of pi's settings")
}

/// Takes the user's answer, as [`yaar_agent::Adapter::answer_consent`] says: a yes adds
/// the entry, the absolute path of `folder/extension/yaar.ts`, to the top-level
/// `extensions` of pi's settings and keeps `yes`; a no keeps `no` and takes the entry out;
/// [`ConsentAnswer::NotAsked`] takes the entry out and removes the kept answer. The rest of
/// the file stays byte for byte; the file is changed through a temporary file renamed over
/// it, under pi's lock. A settings file that cannot be read as pi writes it is left as it
/// was, and the answer is not kept (a yes) or is kept (a no, ask again) with `problem`
/// saying so. None while `home/.pi/agent` is not a folder: nothing is kept.
pub(crate) fn answer(home: &Path, folder: &Path, answer: ConsentAnswer) -> Option<Consent> {
    let _ = (home, folder, answer);
    todo!("the answer to pi's question")
}

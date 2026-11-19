//! The user's settings, between the Link and the Store: what a Window reads when it
//! connects, and what it keeps when the user sets one.
//!
//! The Store keeps every setting the user set, those of the whole app and those of each
//! workspace; the Link carries the kinds a Window sets so far, which in slice 5 is the form
//! of Flow of a workspace. A setting the Store keeps that the Link has no word for yet is
//! not sent: the settings screen of slice 9 adds the words, and nothing here changes for
//! them but one arm of each mapping.
//!
//! `GetSettings` waits the Keeper's turn, and a kept setting is told as
//! `Fact::SettingChanged` to every Window under the same turn, after the reply of the
//! request that set it: a Window that asks for the settings never takes an older list for a
//! newer one.

use yaar_link::{Fact, Refusal, Reply, Setting};

use crate::wiring::Kept;

/// What the Keeper of the contract answers until the Keeper's package of slice 5 wires the
/// settings: a refusal, so that no Window's thread waits on an answer that never comes.
const NOT_WIRED: &str = "the settings are not wired yet";

/// `Request::GetSettings`: every setting the Store keeps that the Link has a word for, as
/// `Reply::Settings`, in the Store's order.
pub(crate) fn get(kept: &Kept) -> Result<Reply, Refusal> {
    let _ = kept;
    Err(Refusal::Failed {
        reason: NOT_WIRED.to_owned(),
    })
}

/// `Request::SetSetting`: keeps `setting` in the Store, answers `Reply::Done` once it is
/// there, and puts `Fact::SettingChanged` with it into `facts`, for every Window. A
/// setting of a workspace the Keeper does not know is refused with
/// `Refusal::UnknownWorkspace`, and nothing is kept or told.
pub(crate) fn set(kept: &Kept, setting: Setting, facts: &mut Vec<Fact>) -> Result<Reply, Refusal> {
    let _ = (kept, setting, facts);
    Err(Refusal::Failed {
        reason: NOT_WIRED.to_owned(),
    })
}

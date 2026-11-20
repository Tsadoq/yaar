//! Downloads and updates, as the Navigator runs them: only when the user accepted an offer
//! or asked for an update (FR-057, FR-058, FR-063, FR-073). The download itself, its digest
//! and its folder are `download.rs`'s; which servers run is `lifecycle.rs`'s.
//!
//! # A download
//!
//! [`download`] ([`crate::Ask::Download`] of a row): the keys of that row whose state is
//! missing with an offer (`lifecycle::offering`) are shown as
//! [`crate::State::Downloading`] with that offer (`lifecycle::downloading`), and one
//! download of the offer's server, the row's `download` for this machine (`Row::download`),
//! is fetched by [`crate::download::Downloader::fetch`] into
//! [`crate::NavigatorConfig::servers`], on a thread of its own, which sends
//! [`crate::navigator::Event::Downloaded`]. A row with no key offering, a registered row,
//! a row with no download for this machine, and a row whose download is on its way
//! already, download nothing: the ask does nothing at all, and nothing is told.
//!
//! # An update
//!
//! [`update`] ([`crate::Ask::Update`] of a row): the keys of that row whose state shows an
//! update (`lifecycle::updating`) keep running as they are while the newer version is
//! fetched, the same way; nothing else is shown meanwhile. Nothing but this ask ever
//! downloads a newer version (FR-063).
//!
//! # When it ends
//!
//! [`downloaded`]: on success, every key that was downloading or updating for the row is
//! searched again (`lifecycle::search_again`), which stops the server it ran and finds the
//! version now installed as its downloaded candidate (`search::look` reads
//! `download::installed`). On failure, the reason is written to the standard error as one
//! line, and every such key is searched again too: a download shows missing with its offer
//! again, an update keeps the version that ran.
//!
//! The [`crate::download::Downloader`] is made at the first download
//! ([`crate::download::Downloader::new`]); when it cannot be made, the download fails with
//! its sentence as the reason.

use crate::download::{DownloadError, Installed};
use crate::navigator::Core;

/// What this module keeps in [`Core::fetch`]: the downloader once made, and the rows whose
/// download is on its way, with the keys waiting for each.
#[derive(Default)]
pub(crate) struct Fetches {}

/// [`crate::Ask::Download`]: downloads the row's server for every key whose state offers it.
pub(crate) fn download(core: &mut Core, language: &str) {
    let _ = (core, language);
    todo!()
}

/// [`crate::Ask::Update`]: downloads the version the row pins now, for the keys whose state
/// shows an update, and starts it in the place of the one that runs.
pub(crate) fn update(core: &mut Core, language: &str) {
    let _ = (core, language);
    todo!()
}

/// A download ended: the keys that waited for it are searched again.
pub(crate) fn downloaded(
    core: &mut Core,
    language: String,
    outcome: Result<Installed, DownloadError>,
) {
    let _ = (core, language, outcome);
    todo!()
}

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

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use crate::download::{DownloadError, Downloader, Installed};
use crate::lifecycle;
use crate::navigator::{Core, Event, ServerKey};
use crate::registry::Download;

/// What this module keeps in [`Core::fetch`]: the downloader once made, and the rows whose
/// download is on its way, with the keys waiting for each.
#[derive(Default)]
pub(crate) struct Fetches {
    /// The downloader, made once by the first download's own thread (reading the system's
    /// certificates takes a while, and the Navigator's thread waits for nothing), or the
    /// sentence of why it could not be made. Handed to each thread behind an `Arc`.
    downloader: Arc<OnceLock<Result<Downloader, String>>>,
    /// The rows with a download on its way, each with the keys that were shown for it when it
    /// began. A key that is forgotten meanwhile stays in the list, and is left alone when the
    /// download ends, since `lifecycle::search_again` leaves a key it does not know alone.
    on_its_way: BTreeMap<String, Vec<ServerKey>>,
}

/// [`crate::Ask::Download`]: downloads the row's server for every key whose state offers it.
pub(crate) fn download(core: &mut Core, language: &str) {
    if core.fetch.on_its_way.contains_key(language) {
        return;
    }
    let keys = lifecycle::offering(core, language);
    let Some((_, offer)) = keys.first() else {
        return;
    };
    let Some((server, download)) = pinned(core, language, &offer.server) else {
        return;
    };
    let offer = offer.clone();
    let keys: Vec<ServerKey> = keys.into_iter().map(|(key, _)| key).collect();
    for key in &keys {
        lifecycle::downloading(core, key, offer.clone());
    }
    begin(core, language, keys, server, download);
}

/// [`crate::Ask::Update`]: downloads the version the row pins now, for the keys whose state
/// shows an update, and starts it in the place of the one that runs.
pub(crate) fn update(core: &mut Core, language: &str) {
    if core.fetch.on_its_way.contains_key(language) {
        return;
    }
    let keys = lifecycle::updating(core, language);
    let Some((_, offer)) = keys.first() else {
        return;
    };
    let Some((server, download)) = pinned(core, language, &offer.server) else {
        return;
    };
    // The keys that run another server of the row wait for an ask of their own.
    let keys: Vec<ServerKey> = keys
        .iter()
        .filter(|(_, shown)| shown.server == server)
        .map(|(key, _)| key.clone())
        .collect();
    begin(core, language, keys, server, download);
}

/// A download ended: the keys that waited for it are searched again.
pub(crate) fn downloaded(
    core: &mut Core,
    language: String,
    outcome: Result<Installed, DownloadError>,
) {
    let waiting = core.fetch.on_its_way.remove(&language).unwrap_or_default();
    let mut keys = waiting;
    match outcome {
        Ok(_) => {
            // A key made while the download was on its way found nothing, and shows the
            // offer: it finds what is installed now.
            for (key, _) in lifecycle::offering(core, &language) {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
        Err(error) => eprintln!("yaar-navigator: the download for {language} failed: {error}"),
    }
    for key in &keys {
        lifecycle::search_again(core, key);
    }
}

/// The server of the row `language` named `server`, and its download for this machine, when
/// the row is one the app downloads for: not the user's, and with a download for the machine.
fn pinned(core: &Core, language: &str, server: &str) -> Option<(String, Download)> {
    let platform = core.config.platform?;
    let row = core.registry.row(language).filter(|row| !row.registered)?;
    let server = row
        .servers
        .iter()
        .find(|candidate| candidate.name == server)?;
    Some((server.name.clone(), server.download(platform)?.clone()))
}

/// Starts the one download of `download` for the row `language`, on a thread of its own,
/// which sends [`Event::Downloaded`], and notes the `keys` that wait for it.
fn begin(
    core: &mut Core,
    language: &str,
    keys: Vec<ServerKey>,
    server: String,
    download: Download,
) {
    core.fetch.on_its_way.insert(language.to_owned(), keys);
    let downloader = Arc::clone(&core.fetch.downloader);
    let servers = core.config.servers.clone();
    let events = core.events.clone();
    let row = language.to_owned();
    let spawned = std::thread::Builder::new()
        .name("navigator-download".to_owned())
        .spawn(move || {
            let outcome = match downloader.get_or_init(Downloader::new) {
                Ok(downloader) => downloader.fetch(&server, &download, &servers),
                Err(reason) => Err(DownloadError::Network(reason.clone())),
            };
            let _ = events.send(Event::Downloaded {
                language: row,
                outcome,
            });
        });
    if let Err(error) = spawned {
        downloaded(
            core,
            language.to_owned(),
            Err(DownloadError::Network(format!(
                "the download's thread could not be started: {error}"
            ))),
        );
    }
}

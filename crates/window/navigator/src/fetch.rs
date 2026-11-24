use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use crate::download::{DownloadError, Downloader, Installed};
use crate::lifecycle;
use crate::navigator::{Core, Event, ServerKey};
use crate::registry::Download;

#[derive(Default)]
pub(crate) struct Fetches {
    /// Made once by the first download's own thread (reading the system's certificates is slow); a
    /// failure is kept and fails every later download.
    downloader: Arc<OnceLock<Result<Downloader, String>>>,
    /// Rows with a download on its way, each with the keys shown for it when it began. A key
    /// forgotten meanwhile stays listed: `lifecycle::search_again` leaves an unknown key alone.
    on_its_way: BTreeMap<String, Waiting>,
}

#[derive(Default)]
struct Waiting {
    server: String,
    keys: Vec<ServerKey>,
    /// An update's keys keep running their server, and a failure leaves them as they are.
    update: bool,
}

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
    begin(core, language, keys, server, download, false);
}

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
    begin(core, language, keys, server, download, true);
}

pub(crate) fn downloaded(
    core: &mut Core,
    language: String,
    outcome: Result<Installed, DownloadError>,
) {
    let Waiting {
        server,
        mut keys,
        update,
    } = core.fetch.on_its_way.remove(&language).unwrap_or_default();
    match outcome {
        Ok(installed) => {
            // A key made while the download was on its way shows the offer and finds what is
            // installed now; one that tried, started or ran the version installed then has a copy
            // whose folder the download removed.
            let late = lifecycle::offering(core, &language)
                .into_iter()
                .map(|(key, _)| key)
                .chain(lifecycle::on_an_older_copy(
                    core, &language, &server, &installed,
                ));
            for key in late.collect::<Vec<_>>() {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
        Err(error) => {
            eprintln!("yaar-navigator: the download for {language} failed: {error}");
            // An update's keys run the version that ran before; restarting would lose its pending
            // answers.
            if update {
                return;
            }
        }
    }
    for key in &keys {
        lifecycle::search_again(core, key);
    }
}

fn pinned(core: &Core, language: &str, server: &str) -> Option<(String, Download)> {
    let platform = core.config.platform?;
    let row = core.registry.row(language).filter(|row| !row.registered)?;
    let server = row
        .servers
        .iter()
        .find(|candidate| candidate.name == server)?;
    Some((server.name.clone(), server.download(platform)?.clone()))
}

fn begin(
    core: &mut Core,
    language: &str,
    keys: Vec<ServerKey>,
    server: String,
    download: Download,
    update: bool,
) {
    core.fetch.on_its_way.insert(
        language.to_owned(),
        Waiting {
            server: server.clone(),
            keys,
            update,
        },
    );
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

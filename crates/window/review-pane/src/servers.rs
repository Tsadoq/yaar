//! The line of the language server of the code on screen: whether it is ready, starting or
//! missing, the offer to download a missing one, and the question before a server inside a
//! repository first runs (FR-057, FR-059, FR-062, FR-063, FR-067, FR-070).
//!
//! # Which server
//!
//! The code on screen is the file of the line the reviewer is on, or of the file view's
//! line: its server is `Navigation::state_for` of the workspace on screen, that file, and
//! for a removed line the version its old side is read in. The pane observes the model and
//! draws the line again at each change.
//!
//! # The line
//!
//! One line, under the bar and above everything else of the pane, in Changes, in Flow and
//! over the file view alike, under the selector `review-servers`. It stays while its server
//! is known: it is a line of the pane and never a dialog, and nothing the reviewer does but
//! acting on it takes it away (FR-059). It says, for a built-in row and one the user
//! registered alike (FR-070), the row's title and where its server stands, with the text
//! selector `review-servers-state:<ready|starting|missing|asking|downloading>`:
//!
//! | State | Words, under `review-servers-text:<words>` | Controls |
//! | --- | --- | --- |
//! | ready | `<title>: ready` | the update control, when the state shows an update |
//! | starting | `<title>: starting; names are found by text match meanwhile` | |
//! | missing, with an offer | `<title>: no language server found` | [`DOWNLOAD`] `<server> <version>` under `review-servers-download`, key [`crate::DownloadServer`] (`shift-i`) |
//! | missing, no offer | `<title>: no language server found; names are found by text match` | |
//! | asking | `<title>: <program> lies inside the repository <name>. Run language servers from this repository?` | [`ALLOW`] under `review-servers-yes`, [`REFUSE`] under `review-servers-no`, keys [`crate::AllowServers`] (`shift-y`) and [`crate::RefuseServers`] (`shift-q`) |
//! | downloading | `<title>: downloading <server> <version>` | |
//!
//! Each control's drawn text carries the selector `review-servers-<control>:<its words>`,
//! as the question for leave's do, so that a control that does one thing and reads another
//! fails its test.
//!
//! # What the controls do
//!
//! The download control asks `Navigation::download` of the row (FR-058); the update control
//! `Navigation::update`. The question's controls answer through `Navigation::answer_consent`
//! for the repository asked about, which the Keeper keeps as a setting of that repository
//! and tells every Window: a repository with an answer is never asked again, from any of its
//! worktrees (FR-062), and the line shows the next state the Navigator tells. With no
//! question shown, the two keys say so in the notice and send nothing.

use gpui_kit::*;
use yaar_navigator::ServerState;

use crate::ReviewPane;

/// The words of the control that downloads, before the server's name and version.
pub const DOWNLOAD: &str = "Download";

/// The words of the control that updates, before the version.
pub const UPDATE: &str = "Update to";

/// The words of the control that answers yes.
pub const ALLOW: &str = "Run them";

/// The words of the control that answers no.
pub const REFUSE: &str = "Not from this repository";

/// What this module keeps in the pane.
#[derive(Default)]
pub(crate) struct Servers {}

impl ReviewPane {
    /// The state of the server of the code on screen, as the line shows it; none while
    /// there is none.
    pub fn server_on_screen(&self, cx: &App) -> Option<ServerState> {
        let _ = cx;
        todo!()
    }

    /// The line, drawn under the bar; none while no server is known for the code on screen.
    pub(crate) fn render_servers(&self, cx: &Context<Self>) -> Option<Div> {
        let _ = cx;
        None
    }

    /// The Navigation model changed.
    pub(crate) fn servers_changed(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// What [`crate::DownloadServer`] does.
    pub(crate) fn download_server(
        &mut self,
        _: &crate::DownloadServer,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = cx;
    }

    /// What [`crate::UpdateServer`] does.
    pub(crate) fn update_server(
        &mut self,
        _: &crate::UpdateServer,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = cx;
    }

    /// What [`crate::AllowServers`] does.
    pub(crate) fn allow_servers(
        &mut self,
        _: &crate::AllowServers,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = cx;
    }

    /// What [`crate::RefuseServers`] does.
    pub(crate) fn refuse_servers(
        &mut self,
        _: &crate::RefuseServers,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = cx;
    }
}

use gpui_kit::Action;
use yaar_frame::Control;
use yaar_terminal_pane::{
    Clear, ScrollLineDown, ScrollLineUp, ScrollToBottom, ScrollToTop, SelectAll, controls,
};

fn row_of(name: &str) -> Option<Control> {
    controls()
        .into_iter()
        .find(|control| (control.action)().name() == name)
}

fn the_six() -> [(&'static str, &'static str); 6] {
    [
        (Clear.name(), "Clear"),
        (SelectAll.name(), "Select All"),
        (ScrollToTop.name(), "Scroll to Top"),
        (ScrollToBottom.name(), "Scroll to Bottom"),
        (ScrollLineUp.name(), "Scroll Line Up"),
        (ScrollLineDown.name(), "Scroll Line Down"),
    ]
}

#[cfg(target_os = "macos")]
#[test]
fn the_history_and_screen_keys_are_cmd_k_cmd_a_and_the_cmd_scroll_keys_in_a_terminal() {
    use yaar_frame::{Asks, PROGRAM_CONTEXT, Place};

    let keys = [
        (Clear.name(), "cmd-k"),
        (SelectAll.name(), "cmd-a"),
        (ScrollToTop.name(), "cmd-home"),
        (ScrollToBottom.name(), "cmd-end"),
        (ScrollLineUp.name(), "cmd-up"),
        (ScrollLineDown.name(), "cmd-down"),
    ];
    let titles = the_six();
    for ((name, key), (title_name, title)) in keys.into_iter().zip(titles) {
        assert_eq!(name, title_name, "the two lists name the same action");
        let row = row_of(name).unwrap_or_else(|| panic!("{title} has no row"));
        assert_eq!(row.keystroke, key, "the key of {title}");
        assert_eq!(row.title, title, "the title of {title}");
        assert_eq!(row.place, Place::In(PROGRAM_CONTEXT), "where {title} works");
        assert_eq!(row.sends, None, "{title} sends nothing of the Link");
        assert_eq!(row.asks, Asks::Nothing, "{title} asks for nothing");
    }
}

#[cfg(not(target_os = "macos"))]
#[test]
fn off_macos_none_of_the_six_has_a_key() {
    // Every key but two reaches the program in a terminal there, so these have no row.
    for (name, title) in the_six() {
        assert!(row_of(name).is_none(), "{title} has a row on this system");
    }
}

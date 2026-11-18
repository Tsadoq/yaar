//! One terminal as the pane shows it: its screen, drawn by the app's own painter, and the
//! keys, the text and the mouse that go to its program.
//!
//! A key reaches the view after the key bindings: a kept key has run its action by then
//! and never arrives here. Of the rest, the view's key listener sends what
//! [`crate::encode_key`] encodes; a key that types text is left to the view's input
//! handler, which sends the committed text and holds marked text while an input method
//! or a dead key composes. The view sets [`yaar_frame::PROGRAM_CONTEXT`], so that no key
//! of the Frame that is no Cmd key and no kept key fires while it has the focus, and so
//! that the keys the toolkit binds for the whole window reach the program too
//! ([`yaar_frame::release_program_keys`]).
//!
//! The painter lays the grid of cells from the top left corner of the view's element,
//! with no margin: the cell at column `c` and line `l` starts `c` cell widths right of the
//! element's left edge and `l` cell heights below its top edge, and the terminal has as
//! many columns and lines as whole cells fit.

use std::ops::Range;

use gpui_kit::*;
use yaar_link::{Ending, TerminalId};

/// One terminal of the pane.
pub struct TerminalView {
    id: TerminalId,
    // The emulator, the link, the marked text and the rest: the builder of the view
    // gives them their shape.
}

impl TerminalView {
    /// The terminal this view shows.
    pub fn id(&self) -> &TerminalId {
        &self.id
    }

    /// The text of the lines the view draws, top to bottom, each without the blanks at
    /// its end. The painter draws from the same screen.
    pub fn lines(&self) -> Vec<String> {
        todo!()
    }

    /// The text an input method or a dead key is composing, which the view draws
    /// underlined at the cursor and has not sent; none while nothing is composed.
    pub fn marked_text(&self) -> Option<String> {
        todo!()
    }

    /// The title the program gave the terminal; none before it gave one.
    pub fn title(&self) -> Option<String> {
        todo!()
    }

    /// How the program ended; none while it runs. The view still shows its last screen.
    pub fn ending(&self) -> Option<Ending> {
        todo!()
    }

    /// Where the keys go while this view has the focus.
    pub fn focus_handle(&self) -> FocusHandle {
        todo!()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = (window, cx);
        todo!();
        #[allow(unreachable_code)]
        div()
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let _ = (range, adjusted_range, window, cx);
        todo!()
    }

    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let _ = (ignore_disabled_input, window, cx);
        todo!()
    }

    fn marked_text_range(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let _ = (window, cx);
        todo!()
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = (window, cx);
        todo!()
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (range, text, window, cx);
        todo!()
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (range, new_text, new_selected_range, window, cx);
        todo!()
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let _ = (range_utf16, element_bounds, window, cx);
        todo!()
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let _ = (point, window, cx);
        todo!()
    }
}

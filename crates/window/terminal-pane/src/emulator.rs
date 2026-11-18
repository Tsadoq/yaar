//! The screen of one terminal, as its program's output builds it.
//!
//! The emulator core is `alacritty_terminal`. In front of it sits a scanner of the app's
//! own that reads the same bytes first, and does three things the core does not:
//! - it holds the kitty keyboard flags a program pushes or sets to those of
//!   [`crate::IMPLEMENTED_FLAGS`], before the core stores them, so that the core's answer
//!   to the query for the flags names exactly what the encoder implements;
//! - it answers the questions the core leaves unanswered: the terminal's name and
//!   version (`CSI > q`) and the size of a cell in pixels (`CSI 16 t`);
//! - it passes everything else on unchanged, also a sequence cut between two calls of
//!   [`Emulator::advance`].
//!
//! What the terminal answers a program, the core's answers and the scanner's together,
//! waits in [`Emulator::take_replies`] in the order the questions came.

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener};
use yaar_link::TerminalSize;

use crate::modes::Modes;

/// How many lines a terminal keeps above its screen.
pub const HISTORY_LINES: usize = 10_000;

/// One terminal's screen and history, and what the terminal owes its program.
pub struct Emulator {
    // The core, with the listener that collects its answers and its title.
    _core: Term<Told>,
}

/// What the core tells while it reads: its answers to the program, the title. Kept
/// until the emulator hands them on.
#[derive(Clone, Default)]
struct Told {
    // The builder of the emulator gives this its shape.
    _kept: (),
}

impl EventListener for Told {
    fn send_event(&self, event: Event) {
        let _ = event;
        todo!()
    }
}

/// A colour as the program asked for it. The painter turns it into the theme's colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    /// The theme's colour for text.
    Foreground,
    /// The theme's colour for the background.
    Background,
    /// One of the 256 colours of the palette: 0 to 15 the theme's, the rest the fixed
    /// cube and greys.
    Indexed(u8),
    /// A colour given as red, green and blue.
    Rgb(u8, u8, u8),
}

/// How a cell's text is drawn, besides its colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    /// Bold.
    pub bold: bool,
    /// Faint.
    pub dim: bool,
    /// Italic.
    pub italic: bool,
    /// Underlined, curly or straight.
    pub underline: bool,
    /// Struck through.
    pub strike: bool,
    /// Text and background swapped.
    pub inverse: bool,
    /// Not drawn.
    pub hidden: bool,
}

/// One cell of the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// Its character with the marks combined with it; empty in the second cell of a wide
    /// character, which the first one covers.
    pub text: String,
    /// The colour of the text.
    pub fg: Color,
    /// The colour of the cell.
    pub bg: Color,
    /// How the text is drawn.
    pub style: Style,
    /// Whether its character takes this cell and the next.
    pub wide: bool,
}

/// Where the cursor is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// Its line on the screen, counted from 0 at the top.
    pub line: usize,
    /// Its column, counted from 0.
    pub column: usize,
}

/// What the screen shows now: the lines in view, scrolled back into the history as far as
/// the view is, and the cursor where the program put it, when it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    /// The lines in view, top to bottom, each as wide as the terminal.
    pub lines: Vec<Vec<Cell>>,
    /// The cursor; none while the program hides it or the view is scrolled away from it.
    pub cursor: Option<Cursor>,
}

/// One cell of the screen, by its place: the line on the screen as shown now, counted
/// from 0 at the top, and the column, counted from 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CellPlace {
    /// The line.
    pub line: usize,
    /// The column.
    pub column: usize,
}

impl Emulator {
    /// An empty screen of `size`, with [`HISTORY_LINES`] of history, the kitty keyboard
    /// protocol on, and the cell taken as 8 by 16 pixels until [`Emulator::set_cell_size`]
    /// says otherwise.
    pub fn new(size: TerminalSize) -> Emulator {
        let _ = size;
        todo!()
    }

    /// Reads what the program wrote: through the scanner, then the core.
    pub fn advance(&mut self, bytes: &[u8]) {
        let _ = bytes;
        todo!()
    }

    /// The bytes the terminal owes the program, in order, since the last call: its
    /// answers to the program's questions.
    pub fn take_replies(&mut self) -> Vec<u8> {
        todo!()
    }

    /// The size of the screen.
    pub fn size(&self) -> TerminalSize {
        todo!()
    }

    /// Gives the screen a new size. The program learns it from the Keeper, which the pane
    /// tells separately.
    pub fn resize(&mut self, size: TerminalSize) {
        let _ = size;
        todo!()
    }

    /// The size of one cell in pixels, as the painter lays the grid out: what the
    /// terminal answers a program that asks.
    pub fn set_cell_size(&mut self, width: u16, height: u16) {
        let _ = (width, height);
        todo!()
    }

    /// The modes the program set that decide what a key, a click or a paste sends.
    pub fn modes(&self) -> Modes {
        todo!()
    }

    /// What the screen shows now, for the painter.
    pub fn screen(&self) -> Screen {
        todo!()
    }

    /// The text of the lines in view, top to bottom, each without the blanks at its end:
    /// what [`Emulator::screen`] holds, as text.
    pub fn lines(&self) -> Vec<String> {
        todo!()
    }

    /// The title the program gave the terminal last; none before it gave one, and after
    /// it reset it.
    pub fn title(&self) -> Option<String> {
        todo!()
    }

    /// Scrolls the view one screen into the history, or back towards the newest line.
    /// The view never goes past either end.
    pub fn scroll_page(&mut self, up: bool) {
        let _ = up;
        todo!()
    }

    /// Scrolls the view by `lines` into the history when positive, back towards the
    /// newest line when negative, as one step of the wheel does. The view never goes past
    /// either end.
    pub fn scroll_lines(&mut self, lines: i32) {
        let _ = lines;
        todo!()
    }

    /// How many lines the view is scrolled back into the history: 0 shows the newest.
    pub fn scrolled_back(&self) -> usize {
        todo!()
    }

    /// Selects the text from one cell to another, both included, in the order read;
    /// a new selection takes the place of the one before.
    pub fn select(&mut self, from: CellPlace, to: CellPlace) {
        let _ = (from, to);
        todo!()
    }

    /// The selected text, lines joined by line feeds; none with nothing selected.
    pub fn selected_text(&self) -> Option<String> {
        todo!()
    }

    /// Selects nothing.
    pub fn clear_selection(&mut self) {
        todo!()
    }
}

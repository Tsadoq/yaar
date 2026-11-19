//! The screen of one terminal, as its program's output builds it.
//!
//! The emulator core is `alacritty_terminal`. In front of it sits a scanner of the app's
//! own that reads the same bytes first, and does four things the core does not:
//! - it answers the query for the kitty keyboard flags itself, from the flags in force
//!   cut to [`crate::IMPLEMENTED_FLAGS`], so that the answer names exactly what the
//!   encoder implements whatever a program pushed or set (the core's own answer is the
//!   top of the stack of pushes and says nothing of a set);
//! - it answers the questions the core leaves unanswered: the terminal's name and
//!   version (`CSI > q`) and the size of a cell in pixels (`CSI 16 t`);
//! - it makes a push of keyboard flags safe to repeat without end (see [`safe_push`]);
//! - it passes everything else on unchanged, also a sequence cut between two calls of
//!   [`Emulator::advance`].
//!
//! What the terminal answers a program, the core's answers and the scanner's together,
//! waits in [`Emulator::take_replies`] in the order the questions came.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Osc52, TermMode, viewport_to_point};
use alacritty_terminal::vte::ansi::{
    Color as AnsiColor, NamedColor, Processor, Rgb, StdSyncHandler,
};
use yaar_link::TerminalSize;

use crate::modes::{IMPLEMENTED_FLAGS, KeyboardFlags, Modes, MouseMode};

/// How many lines a terminal keeps above its screen.
pub const HISTORY_LINES: usize = 10_000;

/// The fewest columns a screen has: a wide character takes two.
const MIN_COLUMNS: u16 = 2;

/// One terminal's screen and history, and what the terminal owes its program.
pub struct Emulator {
    // The core, with the listener that collects its answers and its title.
    core: Term<Told>,
    // The same listener the core holds: what it kept is read here.
    told: Told,
    // The parser that drives the core, which also holds a synchronized update back.
    parser: Processor<StdSyncHandler>,
    scanner: Scanner,
}

/// What the core tells while it reads: its answers to the program, the title. Kept
/// until the emulator hands them on.
#[derive(Clone, Default)]
struct Told {
    kept: Arc<Mutex<Kept>>,
}

/// What a [`Told`] keeps, and what it needs to answer from: the size of the screen and
/// of a cell, which the core does not hold in pixels.
struct Kept {
    replies: Vec<u8>,
    title: Option<String>,
    columns: u16,
    rows: u16,
    cell_width: u16,
    cell_height: u16,
}

impl Default for Kept {
    fn default() -> Kept {
        Kept {
            replies: Vec::new(),
            title: None,
            columns: 80,
            rows: 24,
            cell_width: 8,
            cell_height: 16,
        }
    }
}

impl Told {
    fn kept(&self) -> MutexGuard<'_, Kept> {
        // A panic while the lock is held leaves plain data behind: carry on with it.
        self.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl EventListener for Told {
    fn send_event(&self, event: Event) {
        let mut kept = self.kept();
        match event {
            Event::PtyWrite(text) => kept.replies.extend_from_slice(text.as_bytes()),
            Event::Title(title) => kept.title = Some(title),
            Event::ResetTitle => kept.title = None,
            Event::TextAreaSizeRequest(format) => {
                let text = format(WindowSize {
                    num_lines: kept.rows,
                    num_cols: kept.columns,
                    cell_width: kept.cell_width,
                    cell_height: kept.cell_height,
                });
                kept.replies.extend_from_slice(text.as_bytes());
            }
            Event::ColorRequest(index, format) => {
                let text = format(palette(index));
                kept.replies.extend_from_slice(text.as_bytes());
            }
            // Nothing else is the emulator's to act on: no bell, no clipboard, no
            // cursor shape, and the pane draws when output arrives.
            _ => {}
        }
    }
}

/// The colour a program that asks for one is told: the fixed palette of xterm for the
/// 16 colours and the cube and greys above them, and a light grey on a dark grey for the
/// text and the background. The painter draws the theme's own colours; a program that
/// asks wants to know whether the background is dark, which this says.
fn palette(index: usize) -> Rgb {
    const BASE: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    const FOREGROUND: Rgb = Rgb {
        r: 0xd4,
        g: 0xd4,
        b: 0xd4,
    };
    const BACKGROUND: Rgb = Rgb {
        r: 0x1e,
        g: 0x1e,
        b: 0x1e,
    };
    match index {
        0..=15 => {
            let (r, g, b) = BASE[index];
            Rgb { r, g, b }
        }
        16..=231 => {
            let step = |n: usize| if n == 0 { 0 } else { (55 + 40 * n) as u8 };
            let n = index - 16;
            Rgb {
                r: step(n / 36),
                g: step(n / 6 % 6),
                b: step(n % 6),
            }
        }
        232..=255 => {
            let grey = (8 + 10 * (index - 232)) as u8;
            Rgb {
                r: grey,
                g: grey,
                b: grey,
            }
        }
        i if i == NamedColor::Background as usize => BACKGROUND,
        _ => FOREGROUND,
    }
}

/// What the scanner makes of the bytes it was given: bytes for the core, in order with
/// the answers the scanner gives itself.
enum Piece {
    /// Bytes for the core.
    Pass(Vec<u8>),
    /// Bytes the terminal owes the program, to follow what came before in the replies.
    Answer(Vec<u8>),
    /// The query for the keyboard flags, which the emulator answers once the core has
    /// read what came before it.
    FlagsQuery,
}

/// The scanner that reads the program's bytes before the core does. It looks at escape
/// sequences that begin with `CSI` and acts on four; the rest it passes on as it got
/// them. A sequence cut at the end of one call waits in `held` for the next.
#[derive(Default)]
struct Scanner {
    // From the ESC that began a sequence, up to what has arrived of it.
    held: Vec<u8>,
}

/// The longest sequence the scanner waits for: the ones it acts on are far shorter, and
/// a longer one is passed on to the core at once, whole. A sequence that starts `CSI >`
/// is held to this length too and its parameters past it are dropped: the core reads no
/// more of a push than its first parameter, and the push must be seen whole to be made
/// safe. A first parameter that does not fit is read as far as it goes. Nothing else is
/// ever cut: a mode reset that runs past this length must reach the core with every
/// mode in it.
const MAX_HELD: usize = 64;

/// What a push of keyboard flags goes to the core as: the push cut to the flags the
/// encoder implements, with a title pushed before it and popped after it. Once a stack
/// of flags holds 4096 entries the core drops its oldest by removing the bottom of the
/// stack of titles instead, which fails when that is empty, and so does the thread of
/// whoever fed it a file of pushes. With a title pushed first the stack is never empty
/// there, and popping it afterwards leaves the titles a program pushed as they were
/// (past 4096 pushes, as the core itself would have it, the oldest title goes).
///
/// `push` is a whole `CSI > ... u`. The core reads the number before its first `;` or `:`
/// and keeps what fits in a byte; a push in a form with anything but digits and those two
/// characters is one the core does not read as a push, and goes on as it is.
fn safe_push(push: &[u8]) -> Vec<u8> {
    let params = &push[3..push.len() - 1];
    let mut bytes = b"\x1b[22t".to_vec();
    if params
        .iter()
        .all(|b| matches!(b, b'0'..=b'9' | b';' | b':'))
    {
        let first = params
            .split(|b| matches!(b, b';' | b':'))
            .next()
            .unwrap_or(&[]);
        // As the parser of the core reads it: 16 bits, saturating.
        let flags = first.iter().fold(0u16, |n, digit| {
            n.saturating_mul(10).saturating_add(u16::from(digit - b'0'))
        });
        bytes.extend_from_slice(
            format!("\x1b[>{}u", flags & u16::from(IMPLEMENTED_FLAGS.0)).as_bytes(),
        );
    } else {
        bytes.extend_from_slice(push);
    }
    bytes.extend_from_slice(b"\x1b[23t");
    bytes
}

/// What a set of keyboard flags, `CSI = flags ; way u`, goes to the core as: the flags cut
/// to those the encoder implements and the way as it was.
///
/// Nothing a test can reach shows this. The core answers the query for the flags from its
/// stack of pushes, which a set does not touch, and the emulator reports the flags in force
/// cut to the implemented ones whatever the core holds. It is here so that the core never
/// stores a flag the encoder does not have, whatever else comes to read it.
///
/// `set` is a whole `CSI = ... u`. The core reads the first two numbers, each before its
/// first `:` and 16 bits wide, and keeps the low byte of the first; a form with anything
/// but digits, `;` and `:`, or with more parameters than the core reads, goes on as it is.
fn safe_set(set: &[u8]) -> Vec<u8> {
    // The most parameters the core's parser keeps; it ignores a sequence with more.
    const MOST_PARAMS: usize = 32;
    let params = &set[3..set.len() - 1];
    if !params
        .iter()
        .all(|b| matches!(b, b'0'..=b'9' | b';' | b':'))
        || params.iter().filter(|b| matches!(b, b';' | b':')).count() >= MOST_PARAMS
    {
        return set.to_vec();
    }
    // The number of a parameter, as the parser of the core reads it: its first part,
    // 16 bits, saturating. A parameter that is not there is none.
    let number = |field: &[u8]| {
        let first = field.split(|&b| b == b':').next().unwrap_or(&[]);
        first.iter().fold(0u16, |n, digit| {
            n.saturating_mul(10).saturating_add(u16::from(digit - b'0'))
        })
    };
    let mut fields = params.split(|&b| b == b';');
    let flags = fields.next().map_or(0, number);
    let mut bytes = format!("\x1b[={}", flags & u16::from(IMPLEMENTED_FLAGS.0)).into_bytes();
    if let Some(way) = fields.next() {
        bytes.extend_from_slice(format!(";{}", number(way)).as_bytes());
    }
    bytes.push(b'u');
    bytes
}

impl Scanner {
    fn scan(&mut self, bytes: &[u8], cell: (u16, u16)) -> Vec<Piece> {
        let mut pieces = Vec::new();
        let mut pass = Vec::new();
        for &byte in bytes {
            self.step(byte, cell, &mut pass, &mut pieces);
        }
        if !pass.is_empty() {
            pieces.push(Piece::Pass(pass));
        }
        pieces
    }

    // The scanner follows the parser of the core through an escape sequence, because a
    // sequence is only safe when it is seen from its ESC to its end: what the parser does
    // with a byte inside one decides where the sequence ends.
    fn step(&mut self, byte: u8, cell: (u16, u16), pass: &mut Vec<u8>, pieces: &mut Vec<Piece>) {
        if self.held.is_empty() {
            if byte == 0x1b {
                self.held.push(byte);
            } else {
                pass.push(byte);
            }
            return;
        }
        match byte {
            // The parser ends the sequence at CAN and SUB, doing what they say.
            0x18 | 0x1a => {
                pass.append(&mut self.held);
                pass.push(byte);
            }
            // And begins another at ESC.
            0x1b => {
                pass.append(&mut self.held);
                self.held.push(byte);
            }
            // Any other control byte it runs at once, and goes on with the sequence.
            0x00..=0x17 | 0x19 | 0x1c..=0x1f => pass.push(byte),
            // It ignores a DEL and what is above ASCII there.
            0x7f..=0xff => {}
            _ if self.held.len() == 1 => {
                if byte == b'[' {
                    self.held.push(byte);
                } else {
                    // Not a CSI: what was held goes on, and this byte starts afresh.
                    pass.append(&mut self.held);
                    self.step(byte, cell, pass, pieces);
                }
            }
            0x20..=0x3f => {
                let cut = self.held[2..].first() == Some(&b'>');
                if cut && self.held.len() >= MAX_HELD {
                    // More of the parameters than the core would read.
                } else {
                    self.held.push(byte);
                    if self.held.len() > MAX_HELD {
                        pass.append(&mut self.held);
                    }
                }
            }
            0x40..=0x7e => {
                self.held.push(byte);
                let sequence = std::mem::take(&mut self.held);
                match act_on(&sequence, cell) {
                    Act::Pass(bytes) => pass.extend_from_slice(&bytes),
                    Act::Answer(answer) => {
                        if !pass.is_empty() {
                            pieces.push(Piece::Pass(std::mem::take(pass)));
                        }
                        pieces.push(Piece::Answer(answer));
                    }
                    Act::FlagsQuery => {
                        if !pass.is_empty() {
                            pieces.push(Piece::Pass(std::mem::take(pass)));
                        }
                        pieces.push(Piece::FlagsQuery);
                    }
                }
            }
        }
    }
}

enum Act {
    Pass(Vec<u8>),
    Answer(Vec<u8>),
    FlagsQuery,
}

/// What the scanner does with one whole `CSI` sequence, given with its introducer and its
/// final byte.
fn act_on(sequence: &[u8], cell: (u16, u16)) -> Act {
    let unchanged = || Act::Pass(sequence.to_vec());
    let last = sequence[sequence.len() - 1];
    let body = &sequence[2..sequence.len() - 1];
    let (marker, params) = match body.first() {
        Some(&m @ (b'<' | b'=' | b'>' | b'?')) => (Some(m), &body[1..]),
        _ => (None, body),
    };
    // Every push, in whatever form the core reads as one.
    if marker == Some(b'>') && last == b'u' {
        return Act::Pass(safe_push(sequence));
    }
    // Every set, cut to the flags the encoder implements before the core stores them.
    if marker == Some(b'=') && last == b'u' {
        return Act::Pass(safe_set(sequence));
    }
    // Every query for the flags in force, in whatever form the core reads as one: it
    // reads the question whatever follows the question mark.
    if marker == Some(b'?')
        && last == b'u'
        && params
            .iter()
            .all(|b| matches!(b, b'0'..=b'9' | b';' | b':'))
    {
        return Act::FlagsQuery;
    }
    let Some(params) = parse_params(params) else {
        return unchanged();
    };
    match (marker, last, params.as_slice()) {
        // The terminal's name and version.
        (Some(b'>'), b'q', [] | [None | Some(0)]) => {
            Act::Answer(format!("\x1bP>|yaar {}\x1b\\", env!("CARGO_PKG_VERSION")).into_bytes())
        }
        // The size of a cell in pixels.
        (None, b't', [Some(16)]) => {
            let (width, height) = cell;
            Act::Answer(format!("\x1b[6;{height};{width}t").into_bytes())
        }
        _ => unchanged(),
    }
}

/// The numbers of a parameter string: an empty one is none. Anything but digits and
/// semicolons makes the whole sequence not the scanner's.
fn parse_params(params: &[u8]) -> Option<Vec<Option<u32>>> {
    if params.is_empty() {
        return Some(Vec::new());
    }
    params
        .split(|&b| b == b';')
        .map(|field| {
            if field.is_empty() {
                Some(None)
            } else if field.iter().all(u8::is_ascii_digit) {
                let mut n = 0u32;
                for digit in field {
                    n = n.saturating_mul(10).saturating_add(u32::from(digit - b'0'));
                }
                Some(Some(n))
            } else {
                None
            }
        })
        .collect()
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
    ///
    /// A screen holds at least two columns and one line: the Keeper takes a 0 as 1, and
    /// the core cannot hold a wide character in one column.
    pub fn new(size: TerminalSize) -> Emulator {
        let told = Told::default();
        let (columns, rows) = (size.columns.max(MIN_COLUMNS), size.rows.max(1));
        {
            let mut kept = told.kept();
            kept.columns = columns;
            kept.rows = rows;
        }
        let config = Config {
            kitty_keyboard: true,
            scrolling_history: HISTORY_LINES,
            osc52: Osc52::Disabled,
            ..Config::default()
        };
        let core = Term::new(config, &dimensions(columns, rows), told.clone());
        Emulator {
            core,
            told,
            parser: Processor::new(),
            scanner: Scanner::default(),
        }
    }

    /// Reads what the program wrote: through the scanner, then the core.
    ///
    /// A program that began a synchronized update (`CSI ? 2026 h`) and went silent has
    /// the screen held back until its time is out; the first call after that ends the
    /// update, with no bytes too, which is how a pane without output makes the screen
    /// catch up.
    pub fn advance(&mut self, bytes: &[u8]) {
        self.end_timed_out_update();
        let cell = {
            let kept = self.told.kept();
            (kept.cell_width, kept.cell_height)
        };
        for piece in self.scanner.scan(bytes, cell) {
            match piece {
                Piece::Pass(bytes) => self.parser.advance(&mut self.core, &bytes),
                Piece::Answer(answer) => {
                    self.end_update();
                    self.told.kept().replies.extend_from_slice(&answer);
                }
                Piece::FlagsQuery => {
                    self.end_update();
                    // The core answers from the stack of pushes alone, and says nothing
                    // of a set. The flags in force are what its mode holds.
                    let flags = self.modes().keyboard.0;
                    let answer = format!("\x1b[?{flags}u");
                    self.told
                        .kept()
                        .replies
                        .extend_from_slice(answer.as_bytes());
                }
            }
        }
        self.end_timed_out_update();
    }

    fn end_timed_out_update(&mut self) {
        let deadline = self.parser.sync_timeout().sync_timeout();
        if deadline.is_some_and(|deadline| deadline <= Instant::now()) {
            self.parser.stop_sync(&mut self.core);
        }
    }

    // A program that asks a question inside a synchronized update is waiting for the
    // answer, and the answer says what the bytes before the question did, so the bytes
    // held back are read now: a terminal answers a question when it is asked.
    fn end_update(&mut self) {
        if self.parser.sync_timeout().sync_timeout().is_some() {
            self.parser.stop_sync(&mut self.core);
        }
    }

    /// The bytes the terminal owes the program, in order, since the last call: its
    /// answers to the program's questions.
    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.told.kept().replies)
    }

    /// The size of the screen.
    pub fn size(&self) -> TerminalSize {
        TerminalSize {
            columns: self.core.columns() as u16,
            rows: self.core.screen_lines() as u16,
        }
    }

    /// Gives the screen a new size. The program learns it from the Keeper, which the pane
    /// tells separately.
    pub fn resize(&mut self, size: TerminalSize) {
        let (columns, rows) = (size.columns.max(MIN_COLUMNS), size.rows.max(1));
        {
            let mut kept = self.told.kept();
            kept.columns = columns;
            kept.rows = rows;
        }
        self.core.resize(dimensions(columns, rows));
    }

    /// The size of one cell in pixels, as the painter lays the grid out: what the
    /// terminal answers a program that asks.
    pub fn set_cell_size(&mut self, width: u16, height: u16) {
        let mut kept = self.told.kept();
        kept.cell_width = width;
        kept.cell_height = height;
    }

    /// The modes the program set that decide what a key, a click or a paste sends.
    pub fn modes(&self) -> Modes {
        let mode = self.core.mode();
        let mut keyboard = 0;
        if mode.contains(TermMode::DISAMBIGUATE_ESC_CODES) {
            keyboard |= KeyboardFlags::DISAMBIGUATE.0;
        }
        if mode.contains(TermMode::REPORT_EVENT_TYPES) {
            keyboard |= KeyboardFlags::REPORT_EVENT_TYPES.0;
        }
        let mouse = if mode.contains(TermMode::MOUSE_MOTION) {
            MouseMode::Motion
        } else if mode.contains(TermMode::MOUSE_DRAG) {
            MouseMode::Drag
        } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            MouseMode::Click
        } else {
            MouseMode::Off
        };
        Modes {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            keyboard: KeyboardFlags(keyboard).intersection(IMPLEMENTED_FLAGS),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            mouse,
            mouse_sgr: mode.contains(TermMode::SGR_MOUSE),
            focus_events: mode.contains(TermMode::FOCUS_IN_OUT),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
        }
    }

    /// What the screen shows now, for the painter.
    pub fn screen(&self) -> Screen {
        let grid = self.core.grid();
        let offset = grid.display_offset();
        let lines = (0..grid.screen_lines())
            .map(|line| {
                let row = &grid[Line(line as i32 - offset as i32)];
                (0..grid.columns())
                    .map(|column| cell(&row[Column(column)]))
                    .collect()
            })
            .collect();
        let hidden = !self.core.mode().contains(TermMode::SHOW_CURSOR) || offset > 0;
        let cursor = (!hidden).then(|| {
            let point = grid.cursor.point;
            Cursor {
                line: point.line.0.max(0) as usize,
                column: point.column.0,
            }
        });
        Screen { lines, cursor }
    }

    /// The text of the lines in view, top to bottom, each without the blanks at its end:
    /// what [`Emulator::screen`] holds, as text.
    pub fn lines(&self) -> Vec<String> {
        self.screen()
            .lines
            .into_iter()
            .map(|line| {
                let text: String = line.iter().map(|cell| cell.text.as_str()).collect();
                text.trim_end().to_string()
            })
            .collect()
    }

    /// The title the program gave the terminal last; none before it gave one, and after
    /// it reset it.
    pub fn title(&self) -> Option<String> {
        self.told.kept().title.clone()
    }

    /// Scrolls the view one screen into the history, or back towards the newest line.
    /// The view never goes past either end.
    pub fn scroll_page(&mut self, up: bool) {
        self.core
            .scroll_display(if up { Scroll::PageUp } else { Scroll::PageDown });
    }

    /// Scrolls the view by `lines` into the history when positive, back towards the
    /// newest line when negative, as one step of the wheel does. The view never goes past
    /// either end.
    pub fn scroll_lines(&mut self, lines: i32) {
        self.core.scroll_display(Scroll::Delta(lines));
    }

    /// How many lines the view is scrolled back into the history: 0 shows the newest.
    pub fn scrolled_back(&self) -> usize {
        self.core.grid().display_offset()
    }

    /// Selects the text from one cell to another, both included, in the order read;
    /// a new selection takes the place of the one before.
    pub fn select(&mut self, from: CellPlace, to: CellPlace) {
        let (first, last) = if from <= to { (from, to) } else { (to, from) };
        let offset = self.core.grid().display_offset();
        let point = |place: CellPlace| {
            viewport_to_point(offset, Point::new(place.line, Column(place.column)))
        };
        let mut selection = Selection::new(SelectionType::Simple, point(first), Side::Left);
        selection.update(point(last), Side::Right);
        self.core.selection = Some(selection);
    }

    /// The selected text, lines joined by line feeds; none with nothing selected.
    pub fn selected_text(&self) -> Option<String> {
        self.core.selection_to_string()
    }

    /// Selects nothing.
    pub fn clear_selection(&mut self) {
        self.core.selection = None;
    }
}

/// The size of a screen as the core asks for it: no more lines than the screen has.
struct Size {
    columns: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

fn dimensions(columns: u16, rows: u16) -> Size {
    Size {
        columns: columns.into(),
        rows: rows.into(),
    }
}

fn color(color: AnsiColor) -> Color {
    match color {
        AnsiColor::Spec(Rgb { r, g, b }) => Color::Rgb(r, g, b),
        AnsiColor::Indexed(index) => Color::Indexed(index),
        AnsiColor::Named(name) => match name as usize {
            index @ 0..=15 => Color::Indexed(index as u8),
            index if index == NamedColor::Background as usize => Color::Background,
            index
                if index >= NamedColor::DimBlack as usize
                    && index <= NamedColor::DimWhite as usize =>
            {
                Color::Indexed((index - NamedColor::DimBlack as usize) as u8)
            }
            _ => Color::Foreground,
        },
    }
}

fn cell(cell: &alacritty_terminal::term::cell::Cell) -> Cell {
    let flags = cell.flags;
    let mut text = String::new();
    if !flags.contains(Flags::WIDE_CHAR_SPACER) {
        text.push(cell.c);
        text.extend(cell.zerowidth().into_iter().flatten());
    }
    Cell {
        text,
        fg: color(cell.fg),
        bg: color(cell.bg),
        style: Style {
            bold: flags.intersects(Flags::BOLD),
            dim: flags.intersects(Flags::DIM),
            italic: flags.intersects(Flags::ITALIC),
            underline: flags.intersects(Flags::ALL_UNDERLINES),
            strike: flags.intersects(Flags::STRIKEOUT),
            inverse: flags.intersects(Flags::INVERSE),
            hidden: flags.intersects(Flags::HIDDEN),
        },
        wide: flags.intersects(Flags::WIDE_CHAR),
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::Keystroke;

    use super::*;
    use crate::keys::{OptionKey, encode_key, encode_release};

    const ESC: &str = "\x1b";

    fn emulator() -> Emulator {
        Emulator::new(TerminalSize {
            columns: 20,
            rows: 5,
        })
    }

    fn replies_to(emulator: &mut Emulator, text: &str) -> String {
        emulator.advance(text.as_bytes());
        String::from_utf8(emulator.take_replies()).expect("replies are text")
    }

    /// The flags the emulator reports for a program that sends `text` first.
    fn flags_after(text: &str) -> String {
        let mut emulator = emulator();
        emulator.advance(text.as_bytes());
        replies_to(&mut emulator, "\x1b[?u")
    }

    fn key(source: &'static str) -> Keystroke {
        Keystroke::parse(source).expect("a keystroke")
    }

    fn modes_with(flags: u8) -> Modes {
        Modes {
            keyboard: KeyboardFlags(flags),
            ..Modes::default()
        }
    }

    // TERM-02
    #[test]
    fn the_query_for_keyboard_flags_answers_a_push_cut_to_the_implemented_flags() {
        assert_eq!(IMPLEMENTED_FLAGS, KeyboardFlags(3));
        for n in 0..=31u32 {
            // A program that sets a flag the terminal does not have must hear that it
            // has not got it: a push of 5 is granted 1, a push of 4 or 8 nothing.
            assert_eq!(
                flags_after(&format!("{ESC}[>{n}u")),
                format!("{ESC}[?{}u", n & 3),
                "after a push of {n}"
            );
        }
    }

    // TERM-02
    #[test]
    fn the_query_for_keyboard_flags_is_answered_alike_in_every_form_the_core_reads_as_one() {
        let long_zeros = format!("{ESC}[?{}u", "0".repeat(70));
        let long_ones = format!("{ESC}[?{}1u", "0".repeat(70));
        for n in 0..=31u32 {
            let push = format!("{ESC}[>{n}u");
            let want = format!("{ESC}[?{}u", n & 3);
            for query in [
                format!("{ESC}[?u"),
                format!("{ESC}[?0u"),
                format!("{ESC}[?1u"),
                format!("{ESC}[?7u"),
                format!("{ESC}[?0;0u"),
                format!("{ESC}[?1:2u"),
                format!("{ESC}[?;u"),
                long_zeros.clone(),
                long_ones.clone(),
            ] {
                let mut emulator = emulator();
                emulator.advance(push.as_bytes());
                assert_eq!(
                    replies_to(&mut emulator, &query),
                    want,
                    "after a push of {n}, asked as {query:?}"
                );
            }
        }
        // The same after a set, and a query cut between two calls.
        let mut emulator = emulator();
        emulator.advance(format!("{ESC}[=5;1u{ESC}[?").as_bytes());
        assert_eq!(replies_to(&mut emulator, "0u"), format!("{ESC}[?1u"));
    }

    // TERM-02
    #[test]
    fn the_query_for_keyboard_flags_answers_a_set_cut_to_the_implemented_flags() {
        for n in 0..=31u32 {
            assert_eq!(
                flags_after(&format!("{ESC}[={n};1u")),
                format!("{ESC}[?{}u", n & 3),
                "after a set of {n}"
            );
            // Without the second parameter a set replaces, as it does for the core.
            assert_eq!(
                flags_after(&format!("{ESC}[={n}u")),
                format!("{ESC}[?{}u", n & 3),
                "after a set of {n} with no way given"
            );
        }
        // Adding and removing flags apply to the granted ones only.
        assert_eq!(
            flags_after(&format!("{ESC}[=1;1u{ESC}[=6;2u")),
            format!("{ESC}[?3u")
        );
        assert_eq!(
            flags_after(&format!("{ESC}[=3;1u{ESC}[=5;3u")),
            format!("{ESC}[?2u")
        );
    }

    // TERM-02
    #[test]
    fn a_pop_brings_back_the_flags_pushed_before() {
        assert_eq!(
            flags_after(&format!("{ESC}[>1u{ESC}[>7u")),
            format!("{ESC}[?3u")
        );
        assert_eq!(
            flags_after(&format!("{ESC}[>1u{ESC}[>7u{ESC}[<u")),
            format!("{ESC}[?1u")
        );
        assert_eq!(
            flags_after(&format!("{ESC}[>2u{ESC}[>1u{ESC}[>7u{ESC}[<2u")),
            format!("{ESC}[?2u")
        );
        assert_eq!(
            flags_after(&format!("{ESC}[>5u{ESC}[<u")),
            format!("{ESC}[?0u")
        );
        // Whatever was pushed first comes back, cut to the granted flags, however the
        // flags pushed on top of it are cut.
        for n in 0..=31u32 {
            for above in [(n + 1) % 32, (n + 2) % 32, (n + 7) % 32] {
                assert_eq!(
                    flags_after(&format!("{ESC}[>{n}u{ESC}[>{above}u{ESC}[<u")),
                    format!("{ESC}[?{}u", n & 3),
                    "after a push of {n} and of {above}, and a pop"
                );
            }
            assert_eq!(
                flags_after(&format!("{ESC}[>{n}u{ESC}[>3u{ESC}[>2u{ESC}[<2u")),
                format!("{ESC}[?{}u", n & 3),
                "after a push of {n}, two more and a pop of two"
            );
            // A pop past the bottom leaves none.
            assert_eq!(
                flags_after(&format!("{ESC}[>{n}u{ESC}[<9u")),
                format!("{ESC}[?0u"),
                "after a push of {n} and a pop of nine"
            );
        }
    }

    // TERM-02
    #[test]
    fn the_modes_hold_the_granted_flags_and_no_others() {
        for n in 0..=31u8 {
            let mut emulator = emulator();
            emulator.advance(format!("{ESC}[>{n}u").as_bytes());
            assert_eq!(
                emulator.modes().keyboard,
                KeyboardFlags(n & 3),
                "after a push of {n}"
            );
        }
    }

    // TERM-02
    #[test]
    fn the_encoder_implements_the_flags_the_terminal_grants() {
        let ctrl_a = key("ctrl-a");
        let encode = |flags: u8| encode_key(&ctrl_a, &modes_with(flags), OptionKey::Meta, false);
        // Flag 1: every key with Ctrl is an escape code of the protocol; flag 0: the
        // control byte.
        assert_eq!(encode(1), Some(b"\x1b[97;5u".to_vec()));
        assert_eq!(encode(0), Some(vec![0x01]));

        let release = |flags: u8| encode_release(&ctrl_a, &modes_with(flags), OptionKey::Meta);
        // Flag 2: the release of a key is sent, with the event type 3.
        assert_eq!(release(3), Some(b"\x1b[97;5:3u".to_vec()));
        assert_eq!(release(1), None);
        assert_eq!(release(0), None);
    }

    // TERM-02
    #[test]
    fn flags_the_encoder_does_not_have_change_nothing_it_sends() {
        let keys: Vec<Keystroke> = vec![
            key("ctrl-a"),
            key("escape"),
            key("shift-enter"),
            key("alt-b"),
            key("a->a"),
            key("f3"),
        ];
        for keystroke in &keys {
            let encode =
                |flags: u8| encode_key(keystroke, &modes_with(flags), OptionKey::Meta, false);
            let release =
                |flags: u8| encode_release(keystroke, &modes_with(flags), OptionKey::Meta);
            for with_flag_1 in [5, 9] {
                assert_eq!(
                    encode(with_flag_1),
                    encode(1),
                    "{keystroke:?} under {with_flag_1}"
                );
                assert_eq!(
                    release(with_flag_1),
                    release(1),
                    "{keystroke:?} under {with_flag_1}"
                );
            }
            for alone in [4, 8, 16, 28] {
                assert_eq!(encode(alone), encode(0), "{keystroke:?} under {alone}");
                assert_eq!(release(alone), release(0), "{keystroke:?} under {alone}");
            }
            for with_flags_1_and_2 in [7, 11] {
                assert_eq!(encode(with_flags_1_and_2), encode(3), "{keystroke:?}");
                assert_eq!(release(with_flags_1_and_2), release(3), "{keystroke:?}");
            }
        }
        // The keys differ between 0 and 1, so the comparison above shows something.
        assert_ne!(
            encode_key(&keys[0], &modes_with(1), OptionKey::Meta, false),
            encode_key(&keys[0], &modes_with(0), OptionKey::Meta, false)
        );
    }

    // TERM-02
    #[test]
    fn flags_a_program_pushed_reach_the_encoder_only_as_granted() {
        let ctrl_a = key("ctrl-a");
        for (pushed, sent) in [
            (5, b"\x1b[97;5u".to_vec()),
            (9, b"\x1b[97;5u".to_vec()),
            (4, vec![0x01]),
            (8, vec![0x01]),
        ] {
            let mut emulator = emulator();
            emulator.advance(format!("{ESC}[>{pushed}u").as_bytes());
            assert_eq!(
                encode_key(&ctrl_a, &emulator.modes(), OptionKey::Meta, false),
                Some(sent),
                "after a push of {pushed}"
            );
        }
    }

    #[test]
    fn a_sequence_cut_between_two_calls_is_read_as_one() {
        let mut emulator = emulator();
        for piece in ["\x1b", "[", ">", "7", "u", "\x1b[", "?", "u"] {
            emulator.advance(piece.as_bytes());
        }
        assert_eq!(
            String::from_utf8(emulator.take_replies()).unwrap(),
            "\x1b[?3u"
        );

        // A cut that ends in something that is no sequence of the scanner's goes on to
        // the core, which makes red text of it.
        emulator.advance(b"\x1b");
        emulator.advance(b"[31mred");
        let screen = emulator.screen();
        assert_eq!(emulator.lines()[0], "red");
        assert_eq!(screen.lines[0][0].fg, Color::Indexed(1));
    }

    #[test]
    fn an_escape_that_starts_no_sequence_is_passed_on_with_what_follows() {
        let mut emulator = emulator();
        emulator.advance(b"a\x1b");
        emulator.advance(b"7b\x1b[4Dc");
        // ESC 7 saves the cursor, and ESC [ 4 D moves left four: the core read both.
        assert_eq!(emulator.lines()[0], "cb");
    }

    #[test]
    fn the_terminal_answers_its_name_and_version() {
        let version = env!("CARGO_PKG_VERSION");
        let expected = format!("{ESC}P>|yaar {version}{ESC}\\");
        for query in ["\x1b[>q", "\x1b[>0q"] {
            assert_eq!(replies_to(&mut emulator(), query), expected, "{query:?}");
        }
        // A query of another kind with the same final byte is not this one.
        assert_eq!(replies_to(&mut emulator(), "\x1b[>1q"), "");
    }

    #[test]
    fn the_terminal_answers_the_size_of_a_cell_in_pixels() {
        let mut emulator = emulator();
        assert_eq!(replies_to(&mut emulator, "\x1b[16t"), "\x1b[6;16;8t");
        emulator.set_cell_size(9, 19);
        assert_eq!(replies_to(&mut emulator, "\x1b[16t"), "\x1b[6;19;9t");
        // The size of the screen in pixels is the core's answer, from the same cell.
        assert_eq!(replies_to(&mut emulator, "\x1b[14t"), "\x1b[4;95;180t");
    }

    #[test]
    fn answers_come_in_the_order_the_questions_came() {
        let mut emulator = emulator();
        emulator.advance(b"ab");
        let replies = replies_to(&mut emulator, "\x1b[6n\x1b[>q\x1b[c\x1b[16t\x1b[6n");
        let version = env!("CARGO_PKG_VERSION");
        assert_eq!(
            replies,
            format!("{ESC}[1;3R{ESC}P>|yaar {version}{ESC}\\{ESC}[?6c{ESC}[6;16;8t{ESC}[1;3R")
        );
        assert_eq!(
            emulator.take_replies(),
            b"",
            "the replies are handed on once"
        );
    }

    #[test]
    fn the_terminal_answers_what_the_core_knows() {
        let mut emulator = emulator();
        // Device attributes, the status of a mode (synchronized output is known), the
        // cursor.
        assert_eq!(replies_to(&mut emulator, "\x1b[c"), "\x1b[?6c");
        assert_eq!(replies_to(&mut emulator, "\x1b[?2026$p"), "\x1b[?2026;2$y");
        assert_eq!(replies_to(&mut emulator, "\x1b[?1004$p"), "\x1b[?1004;2$y");
        emulator.advance(b"\x1b[?1004h");
        assert_eq!(replies_to(&mut emulator, "\x1b[?1004$p"), "\x1b[?1004;1$y");
    }

    #[test]
    fn a_program_asking_for_colours_is_told_a_dark_background() {
        let mut emulator = emulator();
        assert_eq!(
            replies_to(&mut emulator, "\x1b]11;?\x1b\\"),
            "\x1b]11;rgb:1e1e/1e1e/1e1e\x1b\\"
        );
        assert_eq!(
            replies_to(&mut emulator, "\x1b]10;?\x07"),
            "\x1b]10;rgb:d4d4/d4d4/d4d4\x07"
        );
        assert_eq!(
            replies_to(&mut emulator, "\x1b]4;1;?\x07"),
            "\x1b]4;1;rgb:cdcd/0000/0000\x07"
        );
        assert_eq!(
            replies_to(&mut emulator, "\x1b]4;196;?\x07"),
            "\x1b]4;196;rgb:ffff/0000/0000\x07"
        );
        assert_eq!(
            replies_to(&mut emulator, "\x1b]4;244;?\x07"),
            "\x1b]4;244;rgb:8080/8080/8080\x07"
        );
    }

    #[test]
    fn a_program_cannot_reach_the_clipboard() {
        let mut emulator = emulator();
        emulator.advance(b"\x1b]52;c;aGVsbG8=\x07");
        assert_eq!(replies_to(&mut emulator, "\x1b]52;c;?\x07"), "");
    }

    #[test]
    fn the_title_follows_the_program() {
        let mut emulator = emulator();
        assert_eq!(emulator.title(), None);
        emulator.advance(b"\x1b]0;first\x07");
        assert_eq!(emulator.title().as_deref(), Some("first"));
        emulator.advance(b"\x1b]2;second\x1b\\");
        assert_eq!(emulator.title().as_deref(), Some("second"));
        // A title pushed and popped is the one before it: here, none.
        emulator.advance(b"\x1b[23t");
        emulator.advance(b"\x1b]0;third\x07\x1b[22t\x1b]0;fourth\x07");
        assert_eq!(emulator.title().as_deref(), Some("fourth"));
        emulator.advance(b"\x1b[23t");
        assert_eq!(emulator.title().as_deref(), Some("third"));
    }

    /// Every parameter of a mode sequence as a sequence of its own: what a program that
    /// sent its modes one by one would have sent.
    fn one_by_one(params: &str, last: char) -> String {
        params
            .split(';')
            .map(|mode| format!("{ESC}[?{mode}{last}"))
            .collect()
    }

    #[test]
    fn a_mode_sequence_longer_than_the_scanner_holds_does_what_the_same_modes_sent_one_by_one_do() {
        let beginning = "\x1b[?1000h\x1b[?1006h\x1b[?2004h";
        // Modes that change nothing the pane reads, so that the sequence is long and the
        // ones that matter come last.
        let padding = "25;12;7;1004;1015;1005;1001;9;1007;1048;45;66;67;69;80";
        for (last, tail) in [('l', "1003;1002;1006;1000"), ('h', "1000;1003")] {
            let params = format!("{padding};{tail}");
            let long = format!("{ESC}[?{params}{last}");
            assert!(
                long.len() > MAX_HELD,
                "the sequence is {} bytes, no longer than the scanner holds",
                long.len()
            );
            let mut whole = emulator();
            whole.advance(beginning.as_bytes());
            whole.advance(long.as_bytes());
            let mut parts = emulator();
            parts.advance(beginning.as_bytes());
            parts.advance(one_by_one(&params, last).as_bytes());
            assert_eq!(whole.modes(), parts.modes(), "a long `{last}`");
            // The same with the sequence cut in two calls.
            let mut cut = emulator();
            cut.advance(beginning.as_bytes());
            let (first, second) = long.split_at(MAX_HELD - 7);
            cut.advance(first.as_bytes());
            cut.advance(second.as_bytes());
            assert_eq!(cut.modes(), parts.modes(), "a long `{last}`, cut in two");
            if last == 'l' {
                assert_eq!(whole.modes().mouse, MouseMode::Off);
                assert!(!whole.modes().mouse_sgr);
            } else {
                assert_eq!(whole.modes().mouse, MouseMode::Motion);
            }
        }
    }

    #[test]
    fn the_modes_follow_what_the_program_sets() {
        let mut emulator = emulator();
        // As in xterm, the wheel is the arrow keys on the alternate screen until the
        // program says otherwise.
        assert_eq!(
            emulator.modes(),
            Modes {
                alternate_scroll: true,
                ..Modes::default()
            }
        );

        emulator.advance(b"\x1b[?1h\x1b[?2004h\x1b[?1004h\x1b[?1006h\x1b[?1000h");
        let modes = emulator.modes();
        assert!(modes.app_cursor && modes.bracketed_paste && modes.focus_events);
        assert!(modes.mouse_sgr);
        assert_eq!(modes.mouse, MouseMode::Click);

        emulator.advance(b"\x1b[?1002h");
        assert_eq!(emulator.modes().mouse, MouseMode::Drag);
        emulator.advance(b"\x1b[?1003h");
        assert_eq!(emulator.modes().mouse, MouseMode::Motion);
        // The core takes the three as one choice: the last set is the one in force, and
        // resetting it leaves none.
        emulator.advance(b"\x1b[?1003l");
        assert_eq!(emulator.modes().mouse, MouseMode::Off);
        emulator.advance(b"\x1b[?1002h\x1b[?1002l");
        assert_eq!(emulator.modes().mouse, MouseMode::Off);

        assert!(!emulator.modes().alt_screen);
        emulator.advance(b"\x1b[?1049h");
        assert!(emulator.modes().alt_screen);
        emulator.advance(b"\x1b[?1007l");
        assert!(!emulator.modes().alternate_scroll);
        emulator.advance(b"\x1b[?1049l\x1b[?1l\x1b[?2004l\x1b[?1004l\x1b[?1006l");
        let modes = emulator.modes();
        assert!(!modes.app_cursor && !modes.bracketed_paste && !modes.focus_events);
        assert!(!modes.mouse_sgr && !modes.alt_screen);
    }

    #[test]
    fn the_lines_in_view_are_the_text_without_the_blanks_at_their_ends() {
        let mut emulator = emulator();
        emulator.advance("one  \r\n  two\r\n\r\nfour".as_bytes());
        assert_eq!(emulator.lines(), ["one", "  two", "", "four", ""]);
    }

    #[test]
    fn a_wide_character_takes_two_cells_and_one_of_text() {
        let mut emulator = emulator();
        emulator.advance("a\u{4f60}b e\u{301}".as_bytes());
        let screen = emulator.screen();
        let cells = &screen.lines[0];
        assert_eq!(cells[1].text, "\u{4f60}");
        assert!(cells[1].wide);
        assert_eq!(
            cells[2].text, "",
            "the second cell of a wide character is empty"
        );
        assert!(!cells[2].wide);
        assert_eq!(cells[3].text, "b");
        assert_eq!(cells[5].text, "e\u{301}", "a mark stays with its letter");
        assert_eq!(emulator.lines()[0], "a\u{4f60}b e\u{301}");
        assert_eq!(cells.len(), 20, "every line is as wide as the terminal");
    }

    #[test]
    fn cells_carry_their_colours_and_style() {
        let mut emulator = emulator();
        emulator.advance(
            b"\x1b[1;3;4;9;7;31;42mA\x1b[0m\x1b[38;5;200mB\x1b[38;2;1;2;3;48;5;17mC\x1b[0;2;8;91mD",
        );
        let screen = emulator.screen();
        let row = &screen.lines[0];
        assert_eq!(row[0].fg, Color::Indexed(1));
        assert_eq!(row[0].bg, Color::Indexed(2));
        assert_eq!(
            row[0].style,
            Style {
                bold: true,
                italic: true,
                underline: true,
                strike: true,
                inverse: true,
                ..Style::default()
            }
        );
        assert_eq!(
            (row[1].fg, row[1].bg),
            (Color::Indexed(200), Color::Background)
        );
        assert_eq!(row[1].style, Style::default());
        assert_eq!(
            (row[2].fg, row[2].bg),
            (Color::Rgb(1, 2, 3), Color::Indexed(17))
        );
        assert_eq!(row[3].fg, Color::Indexed(9));
        assert!(row[3].style.dim && row[3].style.hidden);
        assert_eq!(row[4].fg, Color::Foreground);
    }

    #[test]
    fn the_cursor_shows_where_the_program_left_it_unless_hidden() {
        let mut emulator = emulator();
        emulator.advance(b"ab\r\nc");
        assert_eq!(
            emulator.screen().cursor,
            Some(Cursor { line: 1, column: 1 })
        );
        emulator.advance(b"\x1b[?25l");
        assert_eq!(emulator.screen().cursor, None);
        emulator.advance(b"\x1b[?25h");
        assert_eq!(
            emulator.screen().cursor,
            Some(Cursor { line: 1, column: 1 })
        );
    }

    fn numbered(emulator: &mut Emulator, count: usize) {
        for n in 1..=count {
            emulator.advance(format!("line {n}\r\n").as_bytes());
        }
    }

    #[test]
    fn the_view_scrolls_into_the_history_and_never_past_either_end() {
        let mut emulator = emulator();
        // 5 lines on screen, 20 written: the screen shows 17 to 20 and an empty line,
        // 16 lines are history.
        numbered(&mut emulator, 20);
        assert_eq!(
            emulator.lines(),
            ["line 17", "line 18", "line 19", "line 20", ""]
        );
        assert_eq!(emulator.scrolled_back(), 0);

        emulator.scroll_lines(2);
        assert_eq!(emulator.scrolled_back(), 2);
        assert_eq!(emulator.lines()[0], "line 15");
        assert_eq!(emulator.screen().cursor, None, "the cursor is not in view");

        emulator.scroll_lines(-1);
        assert_eq!(emulator.scrolled_back(), 1);
        emulator.scroll_page(true);
        assert_eq!(emulator.scrolled_back(), 6);
        emulator.scroll_page(false);
        assert_eq!(emulator.scrolled_back(), 1);

        emulator.scroll_lines(1_000);
        assert_eq!(emulator.scrolled_back(), 16);
        assert_eq!(emulator.lines()[0], "line 1");
        emulator.scroll_page(true);
        assert_eq!(emulator.scrolled_back(), 16);

        emulator.scroll_lines(-1_000);
        assert_eq!(emulator.scrolled_back(), 0);
        emulator.scroll_page(false);
        assert_eq!(emulator.scrolled_back(), 0);
    }

    #[test]
    fn the_history_holds_the_lines_it_is_given_and_no_more() {
        let mut emulator = Emulator::new(TerminalSize {
            columns: 10,
            rows: 2,
        });
        numbered(&mut emulator, HISTORY_LINES + 50);
        emulator.scroll_lines(i32::MAX);
        assert_eq!(emulator.scrolled_back(), HISTORY_LINES);
    }

    #[test]
    fn a_selection_gives_the_text_from_one_cell_to_the_other_both_included() {
        let mut emulator = emulator();
        emulator.advance(b"hello world\r\nsecond line\r\nthird");
        let place = |line, column| CellPlace { line, column };
        assert_eq!(emulator.selected_text(), None);

        emulator.select(place(0, 6), place(0, 10));
        assert_eq!(emulator.selected_text().as_deref(), Some("world"));

        emulator.select(place(0, 6), place(2, 2));
        assert_eq!(
            emulator.selected_text().as_deref(),
            Some("world\nsecond line\nthi")
        );

        // The order the two cells are given in makes no difference.
        emulator.select(place(2, 2), place(0, 6));
        assert_eq!(
            emulator.selected_text().as_deref(),
            Some("world\nsecond line\nthi")
        );

        // A new selection replaces the one before.
        emulator.select(place(1, 0), place(1, 5));
        assert_eq!(emulator.selected_text().as_deref(), Some("second"));

        emulator.clear_selection();
        assert_eq!(emulator.selected_text(), None);
    }

    #[test]
    fn a_selection_follows_its_text_into_the_history() {
        let mut emulator = emulator();
        numbered(&mut emulator, 8);
        // Screen: lines 5 to 8; select the first of them, then let it scroll away.
        emulator.select(
            CellPlace { line: 0, column: 0 },
            CellPlace { line: 0, column: 5 },
        );
        assert_eq!(emulator.selected_text().as_deref(), Some("line 5"));
        numbered(&mut emulator, 3);
        assert_eq!(emulator.lines()[0], "line 8");
        assert_eq!(emulator.selected_text().as_deref(), Some("line 5"));
    }

    #[test]
    fn a_selection_taken_while_scrolled_back_names_the_cells_shown() {
        let mut emulator = emulator();
        numbered(&mut emulator, 20);
        emulator.scroll_lines(3);
        emulator.select(
            CellPlace { line: 0, column: 0 },
            CellPlace { line: 1, column: 6 },
        );
        assert_eq!(
            emulator.selected_text().as_deref(),
            Some("line 14\nline 15")
        );
    }

    #[test]
    fn a_selection_of_a_wide_character_holds_it_once() {
        let mut emulator = emulator();
        emulator.advance("a\u{4f60}b".as_bytes());
        emulator.select(
            CellPlace { line: 0, column: 0 },
            CellPlace { line: 0, column: 3 },
        );
        assert_eq!(emulator.selected_text().as_deref(), Some("a\u{4f60}b"));
    }

    #[test]
    fn a_new_size_is_the_size_and_a_screen_is_never_smaller_than_two_by_one() {
        let mut emulator = emulator();
        assert_eq!(
            emulator.size(),
            TerminalSize {
                columns: 20,
                rows: 5
            }
        );
        emulator.resize(TerminalSize {
            columns: 30,
            rows: 8,
        });
        assert_eq!(
            emulator.size(),
            TerminalSize {
                columns: 30,
                rows: 8
            }
        );
        assert_eq!(emulator.screen().lines.len(), 8);
        assert_eq!(emulator.screen().lines[0].len(), 30);
        // The size the program is told by its own query is the new one.
        assert_eq!(replies_to(&mut emulator, "\x1b[18t"), "\x1b[8;8;30t");

        // The Keeper takes a 0 as 1; a wide character must not bring the emulator down
        // in a screen that narrow.
        let least = TerminalSize {
            columns: 2,
            rows: 1,
        };
        emulator.resize(TerminalSize {
            columns: 0,
            rows: 0,
        });
        assert_eq!(emulator.size(), least);
        emulator.advance("\u{4f60}x\r\ny\u{4f60}\u{4f60}".as_bytes());
        assert_eq!(emulator.lines().len(), 1);
        assert_eq!(
            Emulator::new(TerminalSize {
                columns: 1,
                rows: 3
            })
            .size(),
            TerminalSize {
                columns: 2,
                rows: 3
            }
        );
    }

    #[test]
    fn a_program_that_pushes_flags_without_end_cannot_stop_the_terminal() {
        // The core holds 4096 pushes a screen, and past that it drops the oldest one, by
        // a path that fails when no title was pushed. Output must never do that.
        let mut emulator = emulator();
        let pushes = format!("{ESC}[>1u").repeat(5000);
        emulator.advance(pushes.as_bytes());
        assert_eq!(replies_to(&mut emulator, "\x1b[?u"), "\x1b[?1u");
        // The same on the alternate screen, with the pushes cut anywhere.
        emulator.advance(b"\x1b[?1049h");
        for chunk in format!("{ESC}[>3u").repeat(5000).as_bytes().chunks(7) {
            emulator.advance(chunk);
        }
        assert_eq!(replies_to(&mut emulator, "\x1b[?u"), "\x1b[?3u");
        // A push in a form the scanner does not read, and one too long to be held, are
        // pushes too (a number of a hundred digits is a number).
        let long = format!("{ESC}[>{}1u", "0".repeat(100));
        for _ in 0..5000 {
            emulator.advance(b"\x1b[>1:2u");
            emulator.advance(long.as_bytes());
        }
        let answer = replies_to(&mut emulator, "\x1b[?u");
        assert!(
            answer.starts_with("\x1b[?") && answer.ends_with('u'),
            "answered {answer:?}"
        );
        // The oldest are dropped and the newest kept: pops bring back flags pushed last.
        emulator.advance(b"\x1b[?1049l\x1b[>2u\x1b[>3u\x1b[<u");
        assert_eq!(replies_to(&mut emulator, "\x1b[?u"), "\x1b[?2u");
    }

    #[test]
    fn a_push_the_core_reads_through_other_bytes_cannot_stop_the_terminal_either() {
        // The core runs a control byte inside an escape sequence at once and carries on
        // with the sequence, and it ignores a DEL and a byte of 0x80 and above there. So
        // each of these is a push, however many times it is sent.
        let between: [&[u8]; 8] = [
            b"\x07",
            b"\r",
            b"\n",
            b"\x00",
            b"\x08",
            b"\x1f",
            b"\x7f\x7f",
            b"\xff",
        ];
        for between in between {
            let inside: Vec<u8> = [&b"\x1b[>1"[..], between, b"u"].concat();
            let before: Vec<u8> = [&b"\x1b"[..], between, b"[>1u"].concat();
            for (place, one) in [("inside", inside), ("before the bracket", before)] {
                let mut emulator = emulator();
                emulator.advance(&one.repeat(5000));
                assert_eq!(
                    replies_to(&mut emulator, "\x1b[?u"),
                    "\x1b[?1u",
                    "after pushes with {between:?} {place} them"
                );
            }
        }
        // The same with the bytes cut anywhere.
        let mut emulator = emulator();
        let all = b"\x1b[>1\x07u\x1b[>1\x7fu\x1b\x07[>1u\x1b[>1\xffu".repeat(2000);
        for chunk in all.chunks(5) {
            emulator.advance(chunk);
        }
        assert_eq!(replies_to(&mut emulator, "\x1b[?u"), "\x1b[?1u");
    }

    #[test]
    fn a_control_byte_inside_an_escape_sequence_acts_and_cancel_and_escape_end_the_sequence() {
        // A backspace inside the sequence moves the cursor; the sequence still ends.
        let mut emulator = emulator();
        emulator.advance(b"ab\x1b[>1\x08u");
        emulator.advance(b"X");
        assert_eq!(emulator.lines()[0], "aX");
        assert_eq!(replies_to(&mut emulator, "\x1b[?u"), "\x1b[?1u");
        // CAN and SUB end a sequence: what follows is text, and nothing was pushed.
        for end in ["\x18", "\x1a"] {
            let mut emulator = self::emulator();
            emulator.advance(format!("{ESC}[>1{end}u").as_bytes());
            assert_eq!(emulator.lines()[0], "u");
            assert_eq!(replies_to(&mut emulator, "\x1b[?u"), "\x1b[?0u");
        }
        // A new ESC begins afresh, and the push after it is read.
        let mut emulator = self::emulator();
        emulator.advance(b"\x1b[>3\x1b[>1u");
        assert_eq!(replies_to(&mut emulator, "\x1b[?u"), "\x1b[?1u");
    }

    #[test]
    fn pushing_flags_leaves_the_titles_a_program_pushed_as_they_were() {
        let mut emulator = emulator();
        emulator.advance(b"\x1b]0;one\x07\x1b[22t\x1b]0;two\x07\x1b[22t\x1b]0;three\x07");
        emulator.advance(format!("{ESC}[>1u").repeat(1000).as_bytes());
        assert_eq!(emulator.title().as_deref(), Some("three"));
        emulator.advance(b"\x1b[23t");
        assert_eq!(emulator.title().as_deref(), Some("two"));
        emulator.advance(b"\x1b[23t");
        assert_eq!(emulator.title().as_deref(), Some("one"));
        emulator.advance(b"\x1b[23t");
        assert_eq!(
            emulator.title().as_deref(),
            Some("one"),
            "nothing left to pop"
        );
    }

    #[test]
    fn a_question_asked_inside_a_synchronized_update_is_answered_from_what_came_before_it() {
        let mut emulator = emulator();
        // The push is held back with the rest of the update, and the answer to the query
        // that follows it must still say that it was read.
        assert_eq!(
            replies_to(&mut emulator, "\x1b[?2026h\x1b[>1u\x1b[?u"),
            "\x1b[?1u"
        );
        // The core's own answers and the scanner's come in the order of the questions.
        let version = env!("CARGO_PKG_VERSION");
        let mut emulator = self::emulator();
        emulator.advance(b"ab");
        assert_eq!(
            replies_to(&mut emulator, "\x1b[?2026h\x1b[6n\x1b[>q\x1b[c\x1b[16t"),
            format!("{ESC}[1;3R{ESC}P>|yaar {version}{ESC}\\{ESC}[?6c{ESC}[6;16;8t")
        );
    }

    #[test]
    fn a_synchronized_update_shows_when_it_ends_or_its_time_is_out() {
        let mut emulator = emulator();
        emulator.advance(b"before\x1b[?2026h\r\nduring");
        assert_eq!(emulator.lines()[..2], ["before", ""], "held back");
        emulator.advance(b" more\x1b[?2026l");
        assert_eq!(emulator.lines()[..2], ["before", "during more"]);

        // An update that never ends is shown once its time is out, and the call that
        // makes it so needs no bytes.
        emulator.advance(b"\x1b[?2026h\r\nhung");
        assert_eq!(emulator.lines()[2], "");
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while emulator.lines()[2] != "hung" {
            assert!(Instant::now() < deadline, "the update was never ended");
            std::thread::sleep(std::time::Duration::from_millis(10));
            emulator.advance(b"");
        }
    }
}

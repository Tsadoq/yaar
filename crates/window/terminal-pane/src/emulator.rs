//! `alacritty_terminal` is the core; a scanner of the app's own reads the bytes first: it answers
//! the kitty flags query, `CSI > q` and `CSI 16 t`, makes a flags push safe to repeat
//! ([`safe_push`]) and passes the rest on.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Osc52, TermMode, viewport_to_point};
use alacritty_terminal::vte::ansi::{NamedColor, Processor, Rgb, StdSyncHandler};
use yaar_link::TerminalSize;
use yaar_ui::themes::built_in_light;
use yaar_ui::tokens::TerminalColors;

use crate::draw::{Colour, colour_of};

use crate::modes::{IMPLEMENTED_FLAGS, KeyboardFlags, Modes, MouseMode};

pub use alacritty_terminal::vte::ansi::Color;

pub const HISTORY_LINES: usize = 10_000;

const MIN_COLUMNS: u16 = 2;

pub struct Emulator {
    core: Term<Told>,
    told: Told,
    parser: Processor<StdSyncHandler>,
    scanner: Scanner,
    progress: ProgressScan,
}

#[derive(Clone, Default)]
struct Told {
    kept: Arc<Mutex<Kept>>,
}

struct Kept {
    replies: Vec<u8>,
    colours: TerminalColors,
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
            colours: built_in_light().terminal,
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
                let text = answer_with(requested(index, &kept.colours), &*format);
                kept.replies.extend_from_slice(text.as_bytes());
            }
            _ => {}
        }
    }
}

/// `index` is the core's: 0 to 255 the numbered colours, then foreground 256, background 257,
/// cursor 258.
fn requested(index: usize, colours: &TerminalColors) -> Colour {
    let colour = match u8::try_from(index) {
        Ok(number) => Color::Indexed(number),
        Err(_) if index == NamedColor::Background as usize => Color::Named(NamedColor::Background),
        Err(_) if index == NamedColor::Cursor as usize => Color::Named(NamedColor::Cursor),
        Err(_) => Color::Named(NamedColor::Foreground),
    };
    colour_of(colour, colours)
}

fn answer_with(colour: Colour, format: &dyn Fn(Rgb) -> String) -> String {
    let rgba = gpui_kit::Rgba::from(colour);
    let byte = |channel: f32| (channel.clamp(0., 1.) * 255.).round() as u8;
    // The core's value is made from its text form: a value written field by field is what the app's
    // colour test takes for a colour written in code.
    let value = format!(
        "#{:02x}{:02x}{:02x}",
        byte(rgba.r),
        byte(rgba.g),
        byte(rgba.b)
    )
    .parse()
    .expect("a colour written as #rrggbb");
    format(value)
}

enum Piece {
    Pass(Vec<u8>),
    Answer(Vec<u8>),
    FlagsQuery,
}

#[derive(Default)]
struct Scanner {
    held: Vec<u8>,
}

/// The longest sequence held: a longer one goes to the core whole. A `CSI >` sequence is cut to
/// this length, since the core reads only a push's first parameter. Nothing else is cut: a long
/// mode reset must reach the core with every mode in it.
const MAX_HELD: usize = 64;

/// Once a stack of flags holds 4096 entries the core drops its oldest by removing the bottom of the
/// stack of titles, which panics when that is empty and kills the feeding thread. A title pushed
/// first and popped after keeps it non-empty.
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

/// Cuts a set of keyboard flags to those the encoder implements, so the core never stores
/// another; no test observes it, as the flags query is answered from the flags in force.
/// The core reads two numbers, each before its first `:`, and keeps the low byte of the first.
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
            // The parser ends the sequence at CAN and SUB, and begins another at ESC.
            0x18 | 0x1a => {
                pass.append(&mut self.held);
                pass.push(byte);
            }
            0x1b => {
                pass.append(&mut self.held);
                self.held.push(byte);
            }
            0x00..=0x17 | 0x19 | 0x1c..=0x1f => pass.push(byte),
            0x7f..=0xff => {}
            _ if self.held.len() == 1 => {
                if byte == b'[' {
                    self.held.push(byte);
                } else {
                    pass.append(&mut self.held);
                    self.step(byte, cell, pass, pieces);
                }
            }
            0x20..=0x3f => {
                let cut = self.held[2..].first() == Some(&b'>');
                if cut && self.held.len() >= MAX_HELD {
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

fn act_on(sequence: &[u8], cell: (u16, u16)) -> Act {
    let unchanged = || Act::Pass(sequence.to_vec());
    let last = sequence[sequence.len() - 1];
    let body = &sequence[2..sequence.len() - 1];
    let (marker, params) = match body.first() {
        Some(&m @ (b'<' | b'=' | b'>' | b'?')) => (Some(m), &body[1..]),
        _ => (None, body),
    };
    if marker == Some(b'>') && last == b'u' {
        return Act::Pass(safe_push(sequence));
    }
    if marker == Some(b'=') && last == b'u' {
        return Act::Pass(safe_set(sequence));
    }
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
        (Some(b'>'), b'q', [] | [None | Some(0)]) => {
            Act::Answer(format!("\x1bP>|yaar {}\x1b\\", env!("CARGO_PKG_VERSION")).into_bytes())
        }
        (None, b't', [Some(16)]) => {
            let (width, height) = cell;
            Act::Answer(format!("\x1b[6;{height};{width}t").into_bytes())
        }
        _ => unchanged(),
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// It is this far, in percent from 0 to 100: state 1.
    Normal(u8),
    /// It failed, this far: state 2.
    Error(u8),
    /// It works, with no measure of how far: state 3, as Claude Code writes when a turn starts.
    Indeterminate,
    /// It paused, this far: state 4.
    Paused(u8),
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum At {
    #[default]
    Ground,
    Escape,
    Command,
    TooLong,
}

const MAX_PROGRESS: usize = 32;

/// Reads `OSC 9 ; 4 ; <state> ; <value>` beside the core, ending a command where the core's
/// parser does: at the bell or an ESC. CAN or SUB cancel the command.
#[derive(Default)]
struct ProgressScan {
    at: At,
    command: Vec<u8>,
    reported: Option<Progress>,
}

impl ProgressScan {
    fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            match (self.at, byte) {
                (At::Ground, 0x1b) => self.at = At::Escape,
                (At::Ground, _) => {}
                (At::Escape, b']') => {
                    self.command.clear();
                    self.at = At::Command;
                }
                (At::Escape, 0x1b) => {}
                (At::Escape, _) => self.at = At::Ground,
                (At::Command | At::TooLong, 0x07) => self.end(),
                (At::Command | At::TooLong, 0x1b) => {
                    self.end();
                    self.at = At::Escape;
                }
                (At::Command | At::TooLong, 0x18 | 0x1a) => self.at = At::Ground,
                (At::Command, _) if self.command.len() < MAX_PROGRESS => self.command.push(byte),
                (At::Command, _) => self.at = At::TooLong,
                (At::TooLong, _) => {}
            }
        }
    }

    fn end(&mut self) {
        if self.at == At::Command {
            let command = std::mem::take(&mut self.command);
            self.take(&command);
        }
        self.at = At::Ground;
    }

    fn take(&mut self, command: &[u8]) {
        let mut fields = command.split(|&b| b == b';');
        if fields.next() != Some(b"9") || fields.next() != Some(b"4") {
            return;
        }
        let number = |field: &[u8]| {
            field.iter().all(u8::is_ascii_digit).then(|| {
                field.iter().fold(0u32, |n, digit| {
                    n.saturating_mul(10).saturating_add(u32::from(digit - b'0'))
                })
            })
        };
        let Some(Some(state)) = fields.next().map(number) else {
            return;
        };
        let value = fields.next().and_then(number).unwrap_or(0).min(100) as u8;
        self.reported = match state {
            0 => None,
            1 => Some(Progress::Normal(value)),
            2 => Some(Progress::Error(value)),
            3 => Some(Progress::Indeterminate),
            4 => Some(Progress::Paused(value)),
            _ => return,
        };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub inverse: bool,
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// Empty in the second cell of a wide character, which the first one covers.
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub style: Style,
    pub wide: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    pub lines: Vec<Vec<Cell>>,
    /// The cursor; none while the program hides it or the view is scrolled away from it.
    pub cursor: Option<Cursor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CellPlace {
    pub line: usize,
    pub column: usize,
}

impl Emulator {
    /// A screen holds at least two columns and one line: the Keeper takes a 0 as 1, and the core
    /// cannot hold a wide character in one column.
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
            progress: ProgressScan::default(),
        }
    }

    /// A program that began a synchronized update (`CSI ? 2026 h`) and went silent has the screen
    /// held back until its time is out; the first call after that ends it, with no bytes too.
    pub fn advance(&mut self, bytes: &[u8]) {
        self.end_timed_out_update();
        let cell = {
            let kept = self.told.kept();
            (kept.cell_width, kept.cell_height)
        };
        self.progress.feed(bytes);
        for piece in self.scanner.scan(bytes, cell) {
            match piece {
                Piece::Pass(bytes) => self.parser.advance(&mut self.core, &bytes),
                // A program that asks inside a synchronized update waits for an answer that must
                // follow the bytes before the question, so the update is ended and those bytes are
                // read first.
                Piece::Answer(answer) => {
                    self.end_update();
                    self.told.kept().replies.extend_from_slice(&answer);
                }
                Piece::FlagsQuery => {
                    self.end_update();
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

    fn end_update(&mut self) {
        if self.parser.sync_timeout().sync_timeout().is_some() {
            self.parser.stop_sync(&mut self.core);
        }
    }

    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.told.kept().replies)
    }

    pub fn size(&self) -> TerminalSize {
        TerminalSize {
            columns: self.core.columns() as u16,
            rows: self.core.screen_lines() as u16,
        }
    }

    pub fn resize(&mut self, size: TerminalSize) {
        let (columns, rows) = (size.columns.max(MIN_COLUMNS), size.rows.max(1));
        {
            let mut kept = self.told.kept();
            kept.columns = columns;
            kept.rows = rows;
        }
        self.core.resize(dimensions(columns, rows));
    }

    /// The colours told to `OSC 4` (0 to 15) and `OSC 10`, `11`, `12`: the applied theme's. 16 to
    /// 255 are xterm's whatever the theme.
    pub fn set_colours(&mut self, colours: TerminalColors) {
        self.told.kept().colours = colours;
    }

    pub fn set_cell_size(&mut self, width: u16, height: u16) {
        let mut kept = self.told.kept();
        kept.cell_width = width;
        kept.cell_height = height;
    }

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

    /// State 0 gives none, as Claude Code writes when a turn stops.
    pub fn progress(&self) -> Option<Progress> {
        self.progress.reported
    }

    pub fn title(&self) -> Option<String> {
        self.told.kept().title.clone()
    }

    pub fn scroll_page(&mut self, up: bool) {
        self.core
            .scroll_display(if up { Scroll::PageUp } else { Scroll::PageDown });
    }

    pub fn scroll_lines(&mut self, lines: i32) {
        self.core.scroll_display(Scroll::Delta(lines));
    }

    pub fn scrolled_back(&self) -> usize {
        self.core.grid().display_offset()
    }

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

    pub fn selected_text(&self) -> Option<String> {
        self.core.selection_to_string()
    }

    pub fn clear_selection(&mut self) {
        self.core.selection = None;
    }

    /// Selects the history and the screen down to the last line that holds text or the cursor,
    /// whichever is lower. Gives the marked cells as the view shows them now.
    pub fn select_all(&mut self) -> (CellPlace, CellPlace) {
        let (bottom, offset, last_line, last_column) = {
            let grid = self.core.grid();
            let holds_text = |line: i32| {
                (0..grid.columns()).any(|column| grid[Line(line)][Column(column)].c != ' ')
            };
            let last_text = (0..grid.screen_lines() as i32)
                .rev()
                .find(|&line| holds_text(line));
            let cursor = grid.cursor.point.line.0;
            (
                last_text.map_or(cursor, |line| line.max(cursor)),
                grid.display_offset() as i32,
                grid.screen_lines() as i32 - 1,
                grid.columns() - 1,
            )
        };
        let mut selection = Selection::new(
            SelectionType::Simple,
            Point::new(self.core.topmost_line(), Column(0)),
            Side::Left,
        );
        selection.update(Point::new(Line(bottom), Column(last_column)), Side::Right);
        self.core.selection = Some(selection);
        (
            CellPlace { line: 0, column: 0 },
            CellPlace {
                line: (bottom + offset).clamp(0, last_line) as usize,
                column: last_column,
            },
        )
    }

    pub fn clear_above_the_cursor(&mut self) {
        if self.core.mode().contains(TermMode::ALT_SCREEN) {
            return;
        }
        self.core.selection = None;
        let lines = self.core.screen_lines();
        let above = usize::try_from(self.core.grid().cursor.point.line.0).unwrap_or(0);
        let grid = self.core.grid_mut();
        if above > 0 {
            grid.scroll_up(&(Line(0)..Line(lines as i32)), above);
            grid.cursor.point.line.0 -= above as i32;
        }
        grid.clear_history();
    }

    pub fn scroll_to_top(&mut self) {
        self.core.scroll_display(Scroll::Top);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.core.scroll_display(Scroll::Bottom);
    }
}

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

fn cell(cell: &alacritty_terminal::term::cell::Cell) -> Cell {
    let flags = cell.flags;
    let mut text = String::new();
    if !flags.contains(Flags::WIDE_CHAR_SPACER) {
        text.push(cell.c);
        text.extend(cell.zerowidth().into_iter().flatten());
    }
    Cell {
        text,
        fg: cell.fg,
        bg: cell.bg,
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
    use yaar_ui::themes::{DARK, Themes};

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

    #[test]
    fn the_query_for_keyboard_flags_answers_a_push_cut_to_the_implemented_flags() {
        assert_eq!(IMPLEMENTED_FLAGS, KeyboardFlags(3));
        for n in 0..=31u32 {
            assert_eq!(
                flags_after(&format!("{ESC}[>{n}u")),
                format!("{ESC}[?{}u", n & 3),
                "after a push of {n}"
            );
        }
    }

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
        let mut emulator = emulator();
        emulator.advance(format!("{ESC}[=5;1u{ESC}[?").as_bytes());
        assert_eq!(replies_to(&mut emulator, "0u"), format!("{ESC}[?1u"));
    }

    #[test]
    fn the_query_for_keyboard_flags_answers_a_set_cut_to_the_implemented_flags() {
        for n in 0..=31u32 {
            assert_eq!(
                flags_after(&format!("{ESC}[={n};1u")),
                format!("{ESC}[?{}u", n & 3),
                "after a set of {n}"
            );
            assert_eq!(
                flags_after(&format!("{ESC}[={n}u")),
                format!("{ESC}[?{}u", n & 3),
                "after a set of {n} with no way given"
            );
        }
        assert_eq!(
            flags_after(&format!("{ESC}[=1;1u{ESC}[=6;2u")),
            format!("{ESC}[?3u")
        );
        assert_eq!(
            flags_after(&format!("{ESC}[=3;1u{ESC}[=5;3u")),
            format!("{ESC}[?2u")
        );
    }

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
            assert_eq!(
                flags_after(&format!("{ESC}[>{n}u{ESC}[<9u")),
                format!("{ESC}[?0u"),
                "after a push of {n} and a pop of nine"
            );
        }
    }

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

    #[test]
    fn the_encoder_implements_the_flags_the_terminal_grants() {
        let ctrl_a = key("ctrl-a");
        let encode = |flags: u8| encode_key(&ctrl_a, &modes_with(flags), OptionKey::Meta, false);
        // Flag 1 sends Ctrl keys as protocol escape codes, flag 0 the control byte; flag 2 adds
        // releases, event type 3.
        assert_eq!(encode(1), Some(b"\x1b[97;5u".to_vec()));
        assert_eq!(encode(0), Some(vec![0x01]));

        let release = |flags: u8| encode_release(&ctrl_a, &modes_with(flags), OptionKey::Meta);
        assert_eq!(release(3), Some(b"\x1b[97;5:3u".to_vec()));
        assert_eq!(release(1), None);
        assert_eq!(release(0), None);
    }

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
        assert_ne!(
            encode_key(&keys[0], &modes_with(1), OptionKey::Meta, false),
            encode_key(&keys[0], &modes_with(0), OptionKey::Meta, false)
        );
    }

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

        // A cut that ends in something that is no sequence of the scanner's goes on to the core,
        // which makes red text of it.
        emulator.advance(b"\x1b");
        emulator.advance(b"[31mred");
        let screen = emulator.screen();
        assert_eq!(emulator.lines()[0], "red");
        assert_eq!(screen.lines[0][0].fg, Color::Named(NamedColor::Red));
    }

    #[test]
    fn an_escape_that_starts_no_sequence_is_passed_on_with_what_follows() {
        let mut emulator = emulator();
        emulator.advance(b"a\x1b");
        emulator.advance(b"7b\x1b[4Dc");
        // ESC 7 saves the cursor after `a` and `ESC [ 4 D` moves left: the core read both, so `c`
        // overwrites `a`.
        assert_eq!(emulator.lines()[0], "cb");
    }

    #[test]
    fn the_terminal_answers_its_name_and_version() {
        let version = env!("CARGO_PKG_VERSION");
        let expected = format!("{ESC}P>|yaar {version}{ESC}\\");
        for query in ["\x1b[>q", "\x1b[>0q"] {
            assert_eq!(replies_to(&mut emulator(), query), expected, "{query:?}");
        }
        assert_eq!(replies_to(&mut emulator(), "\x1b[>1q"), "");
    }

    #[test]
    fn the_terminal_answers_the_size_of_a_cell_in_pixels() {
        let mut emulator = emulator();
        assert_eq!(replies_to(&mut emulator, "\x1b[16t"), "\x1b[6;16;8t");
        emulator.set_cell_size(9, 19);
        assert_eq!(replies_to(&mut emulator, "\x1b[16t"), "\x1b[6;19;9t");
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
        assert_eq!(replies_to(&mut emulator, "\x1b[c"), "\x1b[?6c");
        assert_eq!(replies_to(&mut emulator, "\x1b[?2026$p"), "\x1b[?2026;2$y");
        assert_eq!(replies_to(&mut emulator, "\x1b[?1004$p"), "\x1b[?1004;2$y");
        emulator.advance(b"\x1b[?1004h");
        assert_eq!(replies_to(&mut emulator, "\x1b[?1004$p"), "\x1b[?1004;1$y");
    }

    #[test]
    fn a_program_asking_for_colours_is_told_the_themes_and_xterms_for_the_cube() {
        let mut emulator = emulator();
        let dark = Themes::built_in().named(DARK).expect("built in").clone();
        emulator.set_colours(dark.terminal);
        assert_eq!(
            replies_to(&mut emulator, "\x1b]11;?\x1b\\"),
            "\x1b]11;rgb:1717/1717/1717\x1b\\"
        );
        assert_eq!(
            replies_to(&mut emulator, "\x1b]10;?\x07"),
            "\x1b]10;rgb:fafa/fafa/fafa\x07"
        );
        assert_eq!(
            replies_to(&mut emulator, "\x1b]4;1;?\x07"),
            "\x1b]4;1;rgb:f8f8/7171/7171\x07"
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
        emulator.advance(b"\x1b[23t");
        emulator.advance(b"\x1b]0;third\x07\x1b[22t\x1b]0;fourth\x07");
        assert_eq!(emulator.title().as_deref(), Some("fourth"));
        emulator.advance(b"\x1b[23t");
        assert_eq!(emulator.title().as_deref(), Some("third"));
    }

    fn one_by_one(params: &str, last: char) -> String {
        params
            .split(';')
            .map(|mode| format!("{ESC}[?{mode}{last}"))
            .collect()
    }

    #[test]
    fn a_mode_sequence_longer_than_the_scanner_holds_does_what_the_same_modes_sent_one_by_one_do() {
        let beginning = "\x1b[?1000h\x1b[?1006h\x1b[?2004h";
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
        assert_eq!(row[0].fg, Color::Named(NamedColor::Red));
        assert_eq!(row[0].bg, Color::Named(NamedColor::Green));
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
            (Color::Indexed(200), Color::Named(NamedColor::Background))
        );
        assert_eq!(row[1].style, Style::default());
        let Color::Spec(by_value) = row[2].fg else {
            panic!("the text of the third cell is a value, not {:?}", row[2].fg);
        };
        assert_eq!((by_value.r, by_value.g, by_value.b), (1, 2, 3));
        assert_eq!(row[2].bg, Color::Indexed(17));
        assert_eq!(row[3].fg, Color::Named(NamedColor::BrightRed));
        assert!(row[3].style.dim && row[3].style.hidden);
        assert_eq!(row[4].fg, Color::Named(NamedColor::Foreground));
    }

    fn written(count: usize) -> Emulator {
        let mut emulator = emulator();
        let text: Vec<String> = (1..=count).map(|n| n.to_string()).collect();
        emulator.advance(text.join("\r\n").as_bytes());
        emulator
    }

    #[test]
    fn clearing_drops_the_history_and_the_lines_above_the_cursor_and_keeps_the_rest() {
        let mut emulator = written(12);
        assert_eq!(emulator.lines(), ["8", "9", "10", "11", "12"]);
        emulator.scroll_to_top();
        assert_eq!(emulator.scrolled_back(), 7);
        emulator.scroll_to_bottom();

        emulator.clear_above_the_cursor();

        assert_eq!(emulator.lines(), ["12", "", "", "", ""]);
        assert_eq!(
            emulator.screen().cursor.map(|c| (c.line, c.column)),
            Some((0, 2))
        );
        emulator.scroll_to_top();
        assert_eq!(emulator.scrolled_back(), 0, "no history is left");

        emulator.advance(b"!\r\nnext");
        assert_eq!(emulator.lines(), ["12!", "next", "", "", ""]);
    }

    #[test]
    fn clearing_with_the_cursor_above_other_lines_keeps_them_in_place_under_it() {
        let mut emulator = emulator();
        emulator.advance(b"1\r\n2\r\n3\r\n4\r\n5");
        emulator.advance(b"\x1b[3;1H");
        emulator.clear_above_the_cursor();
        assert_eq!(emulator.lines(), ["3", "4", "5", "", ""]);
        assert_eq!(
            emulator.screen().cursor.map(|c| (c.line, c.column)),
            Some((0, 0))
        );
    }

    #[test]
    fn clearing_with_the_cursor_on_the_first_line_drops_only_the_history() {
        let mut emulator = written(12);
        emulator.advance(b"\x1b[H");
        emulator.clear_above_the_cursor();
        assert_eq!(emulator.lines(), ["8", "9", "10", "11", "12"]);
        emulator.scroll_to_top();
        assert_eq!(emulator.scrolled_back(), 0);
    }

    #[test]
    fn clearing_brings_the_view_back_to_the_newest_line_and_takes_the_selection_away() {
        let mut emulator = written(12);
        emulator.scroll_to_top();
        emulator.select(
            CellPlace { line: 0, column: 0 },
            CellPlace { line: 1, column: 0 },
        );
        assert!(emulator.selected_text().is_some());
        emulator.clear_above_the_cursor();
        assert_eq!(emulator.scrolled_back(), 0);
        assert_eq!(emulator.selected_text(), None);
        assert_eq!(emulator.lines()[0], "12");
    }

    #[test]
    fn clearing_on_the_alternate_screen_changes_nothing_and_the_history_is_still_there_after() {
        let mut emulator = written(12);
        emulator.advance(b"\x1b[?1049h\x1b[Halt\r\nscreen");
        let before = emulator.lines();
        emulator.clear_above_the_cursor();
        assert_eq!(emulator.lines(), before);
        emulator.advance(b"\x1b[?1049l");
        assert_eq!(emulator.lines(), ["8", "9", "10", "11", "12"]);
        emulator.scroll_to_top();
        assert_eq!(emulator.scrolled_back(), 7);
    }

    #[test]
    fn the_scroll_keys_stop_at_the_ends_of_the_history() {
        let mut emulator = written(12);
        emulator.scroll_to_bottom();
        assert_eq!(emulator.scrolled_back(), 0);
        emulator.scroll_to_top();
        assert_eq!(emulator.scrolled_back(), 7);
        assert_eq!(emulator.lines()[0], "1");
        emulator.scroll_to_top();
        assert_eq!(emulator.scrolled_back(), 7);
        emulator.scroll_to_bottom();
        assert_eq!(emulator.lines()[0], "8");
    }

    #[test]
    fn selecting_all_takes_the_history_and_the_screen_down_to_the_cursor() {
        let mut emulator = written(12);
        let marked = emulator.select_all();
        assert_eq!(
            marked,
            (
                CellPlace { line: 0, column: 0 },
                CellPlace {
                    line: 4,
                    column: 19
                }
            )
        );
        let expected: Vec<String> = (1..=12).map(|n| n.to_string()).collect();
        assert_eq!(emulator.selected_text(), Some(expected.join("\n")));
    }

    #[test]
    fn selecting_all_stops_at_the_last_line_with_text_or_the_cursor_and_marks_what_is_in_view() {
        let mut emulator = emulator();
        emulator.advance(b"a\r\nb\r\nc");
        emulator.advance(b"\x1b[1;1H");
        let marked = emulator.select_all();
        assert_eq!(emulator.selected_text().as_deref(), Some("a\nb\nc"));
        assert_eq!(marked.1.line, 2);
        emulator.advance(b"\x1b[5;1H");
        let marked = emulator.select_all();
        assert_eq!(marked.1.line, 4);
        assert_eq!(emulator.selected_text().as_deref(), Some("a\nb\nc\n\n"));
    }

    #[test]
    fn selecting_all_in_a_view_scrolled_into_the_history_marks_the_view_as_far_as_it_goes() {
        let mut emulator = written(12);
        emulator.scroll_to_top();
        let marked = emulator.select_all();
        assert_eq!(marked.1.line, 4);
        assert_eq!(
            emulator.selected_text().map(|text| text.lines().count()),
            Some(12)
        );
        emulator.clear_selection();
        assert_eq!(emulator.selected_text(), None);
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

        emulator.select(place(2, 2), place(0, 6));
        assert_eq!(
            emulator.selected_text().as_deref(),
            Some("world\nsecond line\nthi")
        );

        emulator.select(place(1, 0), place(1, 5));
        assert_eq!(emulator.selected_text().as_deref(), Some("second"));

        emulator.clear_selection();
        assert_eq!(emulator.selected_text(), None);
    }

    #[test]
    fn a_selection_follows_its_text_into_the_history() {
        let mut emulator = emulator();
        numbered(&mut emulator, 8);
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
        assert_eq!(replies_to(&mut emulator, "\x1b[18t"), "\x1b[8;8;30t");

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
        // The core holds 4096 pushes a screen, and past that it drops the oldest by a path that
        // panics when no title was pushed.
        let mut emulator = emulator();
        let pushes = format!("{ESC}[>1u").repeat(5000);
        emulator.advance(pushes.as_bytes());
        assert_eq!(replies_to(&mut emulator, "\x1b[?u"), "\x1b[?1u");
        emulator.advance(b"\x1b[?1049h");
        for chunk in format!("{ESC}[>3u").repeat(5000).as_bytes().chunks(7) {
            emulator.advance(chunk);
        }
        assert_eq!(replies_to(&mut emulator, "\x1b[?u"), "\x1b[?3u");
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
        emulator.advance(b"\x1b[?1049l\x1b[>2u\x1b[>3u\x1b[<u");
        assert_eq!(replies_to(&mut emulator, "\x1b[?u"), "\x1b[?2u");
    }

    #[test]
    fn a_push_the_core_reads_through_other_bytes_cannot_stop_the_terminal_either() {
        // The core runs a control byte inside an escape sequence at once and carries on with the
        // sequence, and it ignores a DEL and a byte of 0x80 and above there.
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
        let mut emulator = emulator();
        let all = b"\x1b[>1\x07u\x1b[>1\x7fu\x1b\x07[>1u\x1b[>1\xffu".repeat(2000);
        for chunk in all.chunks(5) {
            emulator.advance(chunk);
        }
        assert_eq!(replies_to(&mut emulator, "\x1b[?u"), "\x1b[?1u");
    }

    #[test]
    fn a_control_byte_inside_an_escape_sequence_acts_and_cancel_and_escape_end_the_sequence() {
        let mut emulator = emulator();
        emulator.advance(b"ab\x1b[>1\x08u");
        emulator.advance(b"X");
        assert_eq!(emulator.lines()[0], "aX");
        assert_eq!(replies_to(&mut emulator, "\x1b[?u"), "\x1b[?1u");
        for end in ["\x18", "\x1a"] {
            let mut emulator = self::emulator();
            emulator.advance(format!("{ESC}[>1{end}u").as_bytes());
            assert_eq!(emulator.lines()[0], "u");
            assert_eq!(replies_to(&mut emulator, "\x1b[?u"), "\x1b[?0u");
        }
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

        emulator.advance(b"\x1b[?2026h\r\nhung");
        assert_eq!(emulator.lines()[2], "");
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while emulator.lines()[2] != "hung" {
            assert!(Instant::now() < deadline, "the update was never ended");
            std::thread::sleep(std::time::Duration::from_millis(10));
            emulator.advance(b"");
        }
    }

    #[test]
    fn a_program_that_reported_no_progress_has_none() {
        let mut emulator = emulator();
        assert_eq!(emulator.progress(), None);
        emulator.advance(b"plain text");
        assert_eq!(emulator.progress(), None);
    }

    #[test]
    fn the_progress_sequence_gives_each_state_with_its_value() {
        let cases: [(&str, Progress); 5] = [
            ("\x1b]9;4;1;42\x07", Progress::Normal(42)),
            ("\x1b]9;4;2;7\x07", Progress::Error(7)),
            ("\x1b]9;4;3;\x07", Progress::Indeterminate),
            ("\x1b]9;4;4;99\x07", Progress::Paused(99)),
            ("\x1b]9;4;1\x1b\\", Progress::Normal(0)),
        ];
        for (text, progress) in cases {
            let mut emulator = emulator();
            emulator.advance(text.as_bytes());
            assert_eq!(emulator.progress(), Some(progress), "{text:?}");
        }
    }

    #[test]
    fn a_progress_of_state_zero_clears_what_was_reported() {
        let mut emulator = emulator();
        emulator.advance(b"\x1b]9;4;3;\x07");
        assert_eq!(emulator.progress(), Some(Progress::Indeterminate));
        emulator.advance(b"\x1b]9;4;0;\x07");
        assert_eq!(emulator.progress(), None);
    }

    #[test]
    fn the_last_progress_of_one_read_is_the_one_kept() {
        let mut emulator = emulator();
        emulator.advance(b"\x1b]9;4;3;\x07work\x1b]9;4;1;50\x07");
        assert_eq!(emulator.progress(), Some(Progress::Normal(50)));
    }

    #[test]
    fn a_progress_sequence_cut_between_two_reads_is_read_as_one() {
        for cut in 1.."\x1b]9;4;1;42\x07".len() {
            let text = "\x1b]9;4;1;42\x07";
            let mut emulator = emulator();
            emulator.advance(&text.as_bytes()[..cut]);
            assert_eq!(emulator.progress(), None, "cut at {cut}: nothing yet");
            emulator.advance(&text.as_bytes()[cut..]);
            assert_eq!(
                emulator.progress(),
                Some(Progress::Normal(42)),
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn a_progress_value_past_a_hundred_is_a_hundred() {
        let mut emulator = emulator();
        emulator.advance(b"\x1b]9;4;1;250\x07");
        assert_eq!(emulator.progress(), Some(Progress::Normal(100)));
        emulator.advance(b"\x1b]9;4;1;99999999999999999999\x07");
        assert_eq!(emulator.progress(), Some(Progress::Normal(100)));
    }

    #[test]
    fn sequences_that_are_no_progress_leave_it_as_it_was() {
        let mut emulator = emulator();
        emulator.advance(b"\x1b]9;4;3;\x07");
        for other in [
            "\x1b]0;a title\x07",
            "\x1b]9;a notification\x07",
            "\x1b]9;4;9;10\x07",
            "\x1b]9;4;x;10\x07",
            "\x1b]99;4;1;10\x07",
            "\x1b]9;4;1;10\x18",
        ] {
            emulator.advance(other.as_bytes());
            assert_eq!(
                emulator.progress(),
                Some(Progress::Indeterminate),
                "{other:?}"
            );
        }
        let mut long = b"\x1b]9;4;1;".to_vec();
        long.extend(std::iter::repeat_n(b'1', 100_000));
        emulator.advance(&long);
        emulator.advance(b"\x07");
        assert_eq!(emulator.progress(), Some(Progress::Indeterminate));
        emulator.advance(b"\x1b]9;4;4;5\x07");
        assert_eq!(emulator.progress(), Some(Progress::Paused(5)));
    }

    #[test]
    fn the_progress_sequence_draws_nothing_and_the_text_around_it_is_drawn() {
        let mut emulator = emulator();
        emulator.advance(b"ab\x1b]9;4;3;\x07cd\x1b]9;4;1;5\x1b\\ef");
        assert_eq!(emulator.lines()[0], "abcdef");
        assert_eq!(emulator.take_replies(), b"");
    }

    #[test]
    fn the_title_still_reaches_the_core_beside_the_progress() {
        let mut emulator = emulator();
        emulator.advance(b"\x1b]9;4;3;\x07\x1b]0;shell\x07");
        assert_eq!(emulator.title().as_deref(), Some("shell"));
        assert_eq!(emulator.progress(), Some(Progress::Indeterminate));
    }
}

//! The painter of one terminal: the screen of the emulator drawn as a grid of cells.
//!
//! It draws what [`crate::Emulator::screen`] holds and nothing else: the cell backgrounds,
//! the text, the selection the view keeps, and the cursor. The grid is laid from the top
//! left corner of the element it is given, with no margin: the cell at column `c` and
//! line `l` starts `c` cell widths right of the left edge and `l` cell heights below the
//! top edge.

use alacritty_terminal::vte::ansi::NamedColor;
use gpui_kit::*;
use yaar_frame::theme::{TerminalColors, indexed, rgb24};

use crate::emulator::{CellPlace, Color, Screen, Style};

/// The colour a program's `colour` is drawn in, under the theme whose terminal colours are
/// `terminal`.
///
/// The names: the text and the background and the cursor are the theme's, and so is each of
/// the sixteen the names red to white and their bright forms stand for. The dim forms of
/// the eight are those eight, since the faint style dims whatever the colour is; the bright
/// and dim forms of the text are the text. The numbers 0 to 15 are the theme's too; 16 to
/// 255 are xterm's cube and greys, and a colour a program gives by its value is that value,
/// whatever the theme ([`yaar_frame::theme::indexed`] and [`yaar_frame::theme::rgb24`]).
pub fn colour_of(colour: Color, terminal: &TerminalColors) -> Hsla {
    match colour {
        Color::Named(name) => match name {
            NamedColor::Background => terminal.background,
            NamedColor::Cursor => terminal.cursor,
            NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::DimForeground => {
                terminal.foreground
            }
            // The sixteen are numbered 0 to 15, and the dim eight come after the text's
            // own names, in the same order as the first eight.
            name => {
                let number = name as usize;
                let first_dim = NamedColor::DimBlack as usize;
                match number {
                    0..=15 => terminal.ansi[number],
                    n if (first_dim..first_dim + 8).contains(&n) => terminal.ansi[n - first_dim],
                    _ => terminal.foreground,
                }
            }
        },
        Color::Indexed(number @ 0..=15) => terminal.ansi[usize::from(number)],
        Color::Indexed(number) => indexed(number),
        Color::Spec(value) => rgb24(value.r, value.g, value.b),
    }
}

/// What the painter needs besides the screen.
pub(crate) struct Grid<'a> {
    /// Where the top left corner of the first cell is.
    pub origin: Point<Pixels>,
    /// The size of one cell.
    pub cell: Size<Pixels>,
    /// The font of the terminal, before bold and italic.
    pub font: Font,
    /// Its size.
    pub font_size: Pixels,
    /// The colours of the theme's terminal.
    pub colours: &'a TerminalColors,
    /// Whether the terminal has the focus: the cursor is a block then, an outline else.
    pub focused: bool,
    /// The cells the user selected, both included, in the order read.
    pub selection: Option<(CellPlace, CellPlace)>,
}

/// A stretch of text drawn in one go, starting at a column.
struct Segment {
    column: usize,
    text: String,
    runs: Vec<TextRun>,
}

/// Draws the screen.
pub(crate) fn paint(screen: &Screen, grid: &Grid, window: &mut Window, cx: &mut App) {
    let colours = grid.colours;
    let at = |line: usize, column: usize| {
        grid.origin
            + point(
                grid.cell.width * column as f32,
                grid.cell.height * line as f32,
            )
    };
    let selected = |line: usize, column: usize| {
        grid.selection.is_some_and(|(from, to)| {
            let (first, last) = if from <= to { (from, to) } else { (to, from) };
            let place = CellPlace { line, column };
            first <= place && place <= last
        })
    };

    let mut backgrounds: Vec<(usize, usize, usize, Hsla)> = Vec::new();
    let mut texts: Vec<(usize, Segment)> = Vec::new();
    let mut under_cursor: Option<(String, Hsla)> = None;
    for (line, cells) in screen.lines.iter().enumerate() {
        let mut open: Option<Segment> = None;
        let mut last_style: Option<(Hsla, Style)> = None;
        for (column, cell) in cells.iter().enumerate() {
            // The second cell of a wide character is covered by the first.
            if cell.text.is_empty() {
                continue;
            }
            let (mut text, mut back) = (colour_of(cell.fg, colours), colour_of(cell.bg, colours));
            let mut has_background = cell.bg != Color::Named(NamedColor::Background);
            if cell.style.inverse {
                std::mem::swap(&mut text, &mut back);
                has_background = true;
            }
            if cell.style.dim {
                text.a *= 0.6;
            }
            if selected(line, column) {
                back = colours.selection;
                has_background = true;
            }
            let width = if cell.wide { 2 } else { 1 };
            if has_background {
                match backgrounds.last_mut() {
                    Some((l, _, end, color)) if *l == line && *end == column && *color == back => {
                        *end = column + width;
                    }
                    _ => backgrounds.push((line, column, column + width, back)),
                }
            }
            if screen
                .cursor
                .is_some_and(|cursor| cursor.line == line && cursor.column == column)
            {
                under_cursor = Some((cell.text.clone(), colours.background));
            }
            let blank = cell.text == " " || cell.style.hidden;
            if blank && !cell.style.underline && !cell.style.strike {
                if let Some(segment) = open.take() {
                    texts.push((line, segment));
                }
                last_style = None;
                continue;
            }
            let shown = if cell.style.hidden { " " } else { &cell.text };
            // Only plain one-byte characters share a stretch of text: anything else
            // is placed by its column, so that a wide or a combined character can
            // never push the cells after it off the grid.
            let plain = shown.len() == 1;
            let contiguous = open
                .as_ref()
                .is_some_and(|segment| segment.column + segment.text.len() == column);
            if !(plain && contiguous)
                && let Some(segment) = open.take()
            {
                texts.push((line, segment));
                last_style = None;
            }
            let segment = open.get_or_insert_with(|| Segment {
                column,
                text: String::new(),
                runs: Vec::new(),
            });
            segment.text.push_str(shown);
            let style = (text, cell.style);
            if last_style == Some(style) {
                segment.runs.last_mut().expect("a run of the open text").len += shown.len();
            } else {
                let mut font = grid.font.clone();
                if cell.style.bold {
                    font.weight = FontWeight::BOLD;
                }
                if cell.style.italic {
                    font.style = FontStyle::Italic;
                }
                segment.runs.push(TextRun {
                    len: shown.len(),
                    font,
                    color: text,
                    background_color: None,
                    underline: cell.style.underline.then_some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(text),
                        wavy: false,
                    }),
                    strikethrough: cell.style.strike.then_some(StrikethroughStyle {
                        thickness: px(1.),
                        color: Some(text),
                    }),
                });
                last_style = Some(style);
            }
            if !plain && let Some(segment) = open.take() {
                texts.push((line, segment));
                last_style = None;
            }
        }
        if let Some(segment) = open.take() {
            texts.push((line, segment));
        }
    }

    for (line, from, to, color) in backgrounds {
        let bounds = Bounds::new(
            at(line, from),
            size(grid.cell.width * (to - from) as f32, grid.cell.height),
        );
        window.paint_quad(fill(bounds, color));
    }
    for (line, segment) in texts {
        let shaped = window.text_system().shape_line(
            segment.text.into(),
            grid.font_size,
            &segment.runs,
            None,
        );
        shaped
            .paint(
                at(line, segment.column),
                grid.cell.height,
                TextAlign::Left,
                None,
                window,
                cx,
            )
            .ok();
    }
    if let Some(cursor) = screen.cursor {
        let bounds = Bounds::new(at(cursor.line, cursor.column), grid.cell);
        if grid.focused {
            window.paint_quad(fill(bounds, colours.cursor));
            // The character under the block is drawn again in the colour of the
            // background, or it would vanish into the block.
            if let Some((text, color)) = under_cursor
                && !text.trim().is_empty()
            {
                let run = TextRun {
                    len: text.len(),
                    font: grid.font.clone(),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let shaped =
                    window
                        .text_system()
                        .shape_line(text.into(), grid.font_size, &[run], None);
                shaped
                    .paint(
                        bounds.origin,
                        grid.cell.height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .ok();
            }
        } else {
            window.paint_quad(outline(bounds, colours.cursor, BorderStyle::Solid));
        }
    }
}

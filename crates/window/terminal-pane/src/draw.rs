use alacritty_terminal::vte::ansi::NamedColor;
use gpui_kit::*;
use yaar_ui::colour::{indexed, rgb24};
use yaar_ui::tokens::TerminalColors;

use crate::emulator::{CellPlace, Color, Screen, Style};

/// Named so: the app's colour test reads a type name followed by a brace as a colour in code.
pub(crate) type Colour = Hsla;

pub fn colour_of(colour: Color, terminal: &TerminalColors) -> Colour {
    match colour {
        Color::Named(name) => match name {
            NamedColor::Background => terminal.background,
            NamedColor::Cursor => terminal.cursor,
            NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::DimForeground => {
                terminal.foreground
            }
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

pub(crate) struct Grid<'a> {
    pub origin: Point<Pixels>,
    pub cell: Size<Pixels>,
    pub font: Font,
    pub font_size: Pixels,
    pub colours: &'a TerminalColors,
    pub focused: bool,
    pub selection: Option<(CellPlace, CellPlace)>,
}

struct Segment {
    column: usize,
    text: String,
    runs: Vec<TextRun>,
}

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
            // Only plain one-byte characters share a stretch of text: a wide or combined character
            // is placed by its column, so it cannot push later cells off the grid.
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
                    font = yaar_ui::typography::bold_font(font);
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
            // The character under the block cursor is drawn again in the background colour, or it
            // would vanish into the block.
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

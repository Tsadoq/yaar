//! The last two tests run on GPUI's test platform and set the system appearance through the
//! Frame's model, as the platform cannot.

mod common;

use alacritty_terminal::vte::ansi::{NamedColor, Rgb};
use gpui_kit::TestAppContext;
use yaar_link::TerminalSize;
use yaar_terminal_pane::{Color, Emulator, colour_of};
use yaar_ui::colour::{hex, indexed, rgb24};
use yaar_ui::themes::{self, Appearance, Themes};
use yaar_ui::tokens::TerminalColors;

use common::{open_panes, terminal, wait_until};

fn own() -> TerminalColors {
    TerminalColors {
        foreground: hex(0x010101),
        background: hex(0x020202),
        cursor: hex(0x030303),
        selection: hex(0x040404),
        ansi: std::array::from_fn(|n| hex(0x100000 + 0x010101 * n as u32)),
    }
}

const NAMES: [NamedColor; 8] = [
    NamedColor::Black,
    NamedColor::Red,
    NamedColor::Green,
    NamedColor::Yellow,
    NamedColor::Blue,
    NamedColor::Magenta,
    NamedColor::Cyan,
    NamedColor::White,
];

const BRIGHT: [NamedColor; 8] = [
    NamedColor::BrightBlack,
    NamedColor::BrightRed,
    NamedColor::BrightGreen,
    NamedColor::BrightYellow,
    NamedColor::BrightBlue,
    NamedColor::BrightMagenta,
    NamedColor::BrightCyan,
    NamedColor::BrightWhite,
];

const DIM: [NamedColor; 8] = [
    NamedColor::DimBlack,
    NamedColor::DimRed,
    NamedColor::DimGreen,
    NamedColor::DimYellow,
    NamedColor::DimBlue,
    NamedColor::DimMagenta,
    NamedColor::DimCyan,
    NamedColor::DimWhite,
];

#[test]
fn the_default_colours_and_the_cursor_are_the_themes_own() {
    let terminal = own();
    for (name, expected) in [
        (NamedColor::Foreground, terminal.foreground),
        (NamedColor::BrightForeground, terminal.foreground),
        (NamedColor::DimForeground, terminal.foreground),
        (NamedColor::Background, terminal.background),
        (NamedColor::Cursor, terminal.cursor),
    ] {
        assert_eq!(
            colour_of(Color::Named(name), &terminal),
            expected,
            "{name:?}"
        );
    }
}

#[test]
fn the_sixteen_are_the_themes_whether_a_program_names_them_or_numbers_them() {
    let terminal = own();
    for (n, (normal, bright)) in NAMES.into_iter().zip(BRIGHT).enumerate() {
        let (number, bright_number) = (n as u8, n as u8 + 8);
        assert_eq!(
            colour_of(Color::Named(normal), &terminal),
            terminal.ansi[n],
            "{normal:?}"
        );
        assert_eq!(
            colour_of(Color::Indexed(number), &terminal),
            terminal.ansi[n],
            "number {number}"
        );
        assert_eq!(
            colour_of(Color::Named(bright), &terminal),
            terminal.ansi[n + 8],
            "{bright:?}"
        );
        assert_eq!(
            colour_of(Color::Indexed(bright_number), &terminal),
            terminal.ansi[n + 8],
            "number {bright_number}"
        );
    }
}

#[test]
fn the_dim_names_are_the_colours_they_dim() {
    let terminal = own();
    for (n, dim) in DIM.into_iter().enumerate() {
        assert_eq!(
            colour_of(Color::Named(dim), &terminal),
            terminal.ansi[n],
            "{dim:?}"
        );
    }
}

#[test]
fn the_numbers_above_fifteen_are_xterms_whatever_the_theme() {
    let terminal = own();
    for number in 16..=255u8 {
        assert_eq!(
            colour_of(Color::Indexed(number), &terminal),
            indexed(number),
            "number {number}"
        );
    }
    assert_eq!(colour_of(Color::Indexed(16), &terminal), hex(0x000000));
    assert_eq!(colour_of(Color::Indexed(196), &terminal), hex(0xff0000));
    assert_eq!(colour_of(Color::Indexed(231), &terminal), hex(0xffffff));
    assert_eq!(colour_of(Color::Indexed(232), &terminal), hex(0x080808));
    assert_eq!(colour_of(Color::Indexed(255), &terminal), hex(0xeeeeee));
}

#[test]
fn a_colour_by_its_value_is_that_value_whatever_the_theme() {
    let terminal = own();
    let by_value = Rgb { r: 1, g: 2, b: 3 };
    assert_eq!(colour_of(Color::Spec(by_value), &terminal), rgb24(1, 2, 3));
    assert_eq!(colour_of(Color::Spec(by_value), &terminal), hex(0x010203));
}

fn answers(emulator: &mut Emulator, question: &[u8]) -> String {
    emulator.advance(question);
    String::from_utf8(emulator.take_replies()).expect("an answer is text")
}

/// The answer for an 8-bit channel: 16 bits a channel, each byte twice.
fn rgb(red: u8, green: u8, blue: u8) -> String {
    format!("rgb:{red:02x}{red:02x}/{green:02x}{green:02x}/{blue:02x}{blue:02x}")
}

fn emulator() -> Emulator {
    Emulator::new(TerminalSize {
        columns: 20,
        rows: 5,
    })
}

#[test]
fn a_program_that_asks_for_its_colours_is_told_the_themes() {
    let mut emulator = emulator();
    emulator.set_colours(own());
    assert_eq!(
        answers(&mut emulator, b"\x1b]10;?\x07"),
        format!("\x1b]10;{}\x07", rgb(1, 1, 1))
    );
    assert_eq!(
        answers(&mut emulator, b"\x1b]11;?\x1b\\"),
        format!("\x1b]11;{}\x1b\\", rgb(2, 2, 2))
    );
    assert_eq!(
        answers(&mut emulator, b"\x1b]12;?\x07"),
        format!("\x1b]12;{}\x07", rgb(3, 3, 3))
    );
    for n in 0..16u8 {
        assert_eq!(
            answers(&mut emulator, format!("\x1b]4;{n};?\x07").as_bytes()),
            format!("\x1b]4;{n};{}\x07", rgb(0x10 + n, n, n)),
            "colour {n}"
        );
    }
    assert_eq!(
        answers(&mut emulator, b"\x1b]4;196;?\x07"),
        format!("\x1b]4;196;{}\x07", rgb(255, 0, 0))
    );
    assert_eq!(
        answers(&mut emulator, b"\x1b]4;244;?\x07"),
        format!("\x1b]4;244;{}\x07", rgb(128, 128, 128))
    );
}

#[test]
fn the_answers_follow_the_colours_handed_last() {
    let mut emulator = emulator();
    let light = Themes::built_in()
        .named(themes::LIGHT)
        .expect("built in")
        .terminal;
    let light_background = answers(&mut emulator, b"\x1b]11;?\x07");
    emulator.set_colours(own());
    let own_background = answers(&mut emulator, b"\x1b]11;?\x07");
    emulator.set_colours(light);
    let again = answers(&mut emulator, b"\x1b]11;?\x07");
    assert_eq!(
        light_background,
        format!("\x1b]11;{}\x07", rgb(255, 255, 255))
    );
    assert_eq!(own_background, format!("\x1b]11;{}\x07", rgb(2, 2, 2)));
    assert_eq!(again, light_background);
}

#[test]
fn every_channel_value_a_theme_holds_is_told_as_it_was_written() {
    // A colour passes through hue, saturation and lightness into a definition: the answer must give
    // back all 8 bits.
    for value in 0..=255u8 {
        let n = u32::from(value);
        for (colour, (red, green, blue)) in [
            (hex(n * 0x010101), (value, value, value)),
            (hex(n << 16), (value, 0, 0)),
            (hex(n << 8), (0, value, 0)),
            (hex(n), (0, 0, value)),
            (hex(n << 16 | 0x80 << 8 | 0x01), (value, 0x80, 0x01)),
        ] {
            let mut emulator = emulator();
            let mut colours = own();
            colours.background = colour;
            emulator.set_colours(colours);
            assert_eq!(
                answers(&mut emulator, b"\x1b]11;?\x07"),
                format!("\x1b]11;{}\x07", rgb(red, green, blue))
            );
        }
    }
}

#[gpui_kit::test]
fn the_terminal_answers_a_question_about_its_colours_from_the_theme_applied(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes(cx);
    let id = terminal('1');
    panes.open_terminal(&id, &mut cx);

    assert_eq!(
        ask(&mut panes, &id, &mut cx),
        format!("\x1b]11;{}\x07", rgb(0xff, 0xff, 0xff))
    );

    panes
        .models
        .looks
        .update(&mut cx, |looks, cx| looks.system_is(Appearance::Dark, cx));
    wait_until(&mut cx, "the dark theme applied", |cx| {
        cx.update(|app| themes::applied(app).name == themes::DARK)
    });
    assert_eq!(
        ask(&mut panes, &id, &mut cx),
        format!("\x1b]11;{}\x07", rgb(0x17, 0x17, 0x17))
    );

    panes
        .models
        .looks
        .update(&mut cx, |looks, cx| looks.system_is(Appearance::Light, cx));
    wait_until(&mut cx, "the light theme applied", |cx| {
        cx.update(|app| themes::applied(app).name == themes::LIGHT)
    });
    assert_eq!(
        ask(&mut panes, &id, &mut cx),
        format!("\x1b]11;{}\x07", rgb(0xff, 0xff, 0xff))
    );
}

#[gpui_kit::test]
fn a_terminal_opened_under_the_dark_theme_answers_with_it_at_once(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_panes(cx);
    panes
        .models
        .looks
        .update(&mut cx, |looks, cx| looks.system_is(Appearance::Dark, cx));
    wait_until(&mut cx, "the dark theme applied", |cx| {
        cx.update(|app| themes::applied(app).name == themes::DARK)
    });
    let id = terminal('1');
    panes.open_terminal(&id, &mut cx);
    // The question comes with the terminal's first bytes, before any frame drew it.
    panes.keeper.output(&id, b"\x1b]10;?\x07");
    let (_, bytes) = panes.keeper.input(&mut cx);
    assert_eq!(
        String::from_utf8(bytes).expect("text"),
        format!("\x1b]10;{}\x07", rgb(0xfa, 0xfa, 0xfa))
    );
}

fn ask(
    panes: &mut common::Panes,
    id: &yaar_link::TerminalId,
    cx: &mut gpui_kit::VisualTestContext,
) -> String {
    panes.keeper.output(id, b"\x1b]11;?\x07");
    let (from, bytes) = panes.keeper.input(cx);
    assert_eq!(&from, id);
    String::from_utf8(bytes).expect("an answer is text")
}

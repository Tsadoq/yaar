//! What a click, a drag or the wheel sends to the program of a terminal.
//!
//! A program that asked for mouse reports gets them in the form it asked for, SGR or the
//! old one, for the actions its mode names. A program that asked for none gets nothing:
//! a drag then selects text in the pane, and the wheel scrolls the history, or sends the
//! arrow keys on the alternate screen when the program asked for that.

use gpui_kit::Modifiers;

use crate::modes::{Modes, MouseMode};

/// A button of the mouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    /// The left one.
    Left,
    /// The middle one, or the wheel pressed.
    Middle,
    /// The right one.
    Right,
}

/// One action of the mouse over a cell of the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    /// A button went down.
    Press(MouseButton),
    /// A button came up.
    Release(MouseButton),
    /// The pointer moved, with this button held or none.
    Move(Option<MouseButton>),
    /// The wheel turned one step up.
    WheelUp,
    /// The wheel turned one step down.
    WheelDown,
}

/// The bytes a mouse action over the cell at `column` and `line`, both counted from 0,
/// sends to the program; none when the program did not ask to be told of it.
///
/// With no mouse reports asked for, on the alternate screen with alternate scroll on, a
/// step of the wheel is one Up or Down arrow, as the cursor mode writes it.
pub fn encode_mouse(
    action: MouseAction,
    column: u16,
    line: u16,
    modifiers: &Modifiers,
    modes: &Modes,
) -> Option<Vec<u8>> {
    if modes.mouse == MouseMode::Off {
        return wheel_as_arrow(action, modes);
    }
    let reported = match action {
        MouseAction::Press(_)
        | MouseAction::Release(_)
        | MouseAction::WheelUp
        | MouseAction::WheelDown => true,
        MouseAction::Move(Some(_)) => matches!(modes.mouse, MouseMode::Drag | MouseMode::Motion),
        MouseAction::Move(None) => modes.mouse == MouseMode::Motion,
    };
    if !reported {
        return None;
    }

    let button = |button: MouseButton| match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    // What a release says in the old form: some button came up, not which.
    let released_in_the_old_form = 3;
    let mut code = match action {
        MouseAction::Press(b) => button(b),
        MouseAction::Release(b) if modes.mouse_sgr => button(b),
        MouseAction::Release(_) => released_in_the_old_form,
        MouseAction::Move(Some(b)) => 32 + button(b),
        MouseAction::Move(None) => 32 + released_in_the_old_form,
        MouseAction::WheelUp => 64,
        MouseAction::WheelDown => 65,
    };
    code += 4 * u32::from(modifiers.shift)
        + 8 * u32::from(modifiers.alt)
        + 16 * u32::from(modifiers.control);

    // Cells are counted from 0 here and from 1 on the wire.
    let (x, y) = (u32::from(column) + 1, u32::from(line) + 1);
    if modes.mouse_sgr {
        let last = if matches!(action, MouseAction::Release(_)) {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{x};{y}{last}").into_bytes());
    }
    // The old form says each number as one byte past 32: what a byte cannot hold, it
    // cannot say.
    let byte = |n: u32| u8::try_from(32 + n).ok();
    Some(vec![0x1b, b'[', b'M', byte(code)?, byte(x)?, byte(y)?])
}

/// The Up or Down arrow a step of the wheel is on the alternate screen, for a program
/// that wants no mouse reports.
fn wheel_as_arrow(action: MouseAction, modes: &Modes) -> Option<Vec<u8>> {
    if !modes.alt_screen || !modes.alternate_scroll {
        return None;
    }
    let letter = match action {
        MouseAction::WheelUp => b'A',
        MouseAction::WheelDown => b'B',
        _ => return None,
    };
    let introducer = if modes.app_cursor { b'O' } else { b'[' };
    Some(vec![0x1b, introducer, letter])
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_MODIFIERS: Modifiers = Modifiers {
        control: false,
        alt: false,
        shift: false,
        platform: false,
        function: false,
    };

    fn modes(mouse: MouseMode, sgr: bool) -> Modes {
        Modes {
            mouse,
            mouse_sgr: sgr,
            ..Modes::default()
        }
    }

    fn encode(action: MouseAction, column: u16, line: u16, modes: &Modes) -> Option<Vec<u8>> {
        encode_mouse(action, column, line, &NO_MODIFIERS, modes)
    }

    use MouseAction::{Move, Press, Release, WheelDown, WheelUp};
    use MouseButton::{Left, Middle, Right};

    #[test]
    fn a_program_that_asked_for_nothing_is_told_nothing() {
        let off = modes(MouseMode::Off, true);
        for action in [
            Press(Left),
            Release(Left),
            Move(Some(Left)),
            Move(None),
            WheelUp,
            WheelDown,
        ] {
            assert_eq!(encode(action, 3, 4, &off), None, "{action:?}");
        }
    }

    #[test]
    fn sgr_reports_say_the_button_and_the_cell_from_one() {
        let sgr = modes(MouseMode::Click, true);
        for (action, expected) in [
            (Press(Left), "\x1b[<0;4;5M"),
            (Press(Middle), "\x1b[<1;4;5M"),
            (Press(Right), "\x1b[<2;4;5M"),
            (Release(Left), "\x1b[<0;4;5m"),
            (Release(Middle), "\x1b[<1;4;5m"),
            (Release(Right), "\x1b[<2;4;5m"),
            (WheelUp, "\x1b[<64;4;5M"),
            (WheelDown, "\x1b[<65;4;5M"),
        ] {
            assert_eq!(
                encode(action, 3, 4, &sgr),
                Some(expected.as_bytes().to_vec()),
                "{action:?}"
            );
        }
        // Cells far to the right and below say themselves in full: the SGR form has no
        // limit of 222.
        assert_eq!(
            encode(Press(Left), 999, 300, &sgr),
            Some(b"\x1b[<0;1000;301M".to_vec())
        );
    }

    #[test]
    fn the_old_form_adds_32_to_each_number() {
        let old = modes(MouseMode::Click, false);
        assert_eq!(
            encode(Press(Left), 0, 0, &old),
            Some(vec![0x1b, b'[', b'M', 32, 33, 33])
        );
        assert_eq!(
            encode(Press(Right), 3, 4, &old),
            Some(vec![0x1b, b'[', b'M', 34, 36, 37])
        );
        assert_eq!(
            encode(Press(Middle), 9, 1, &old),
            Some(vec![0x1b, b'[', b'M', 33, 42, 34])
        );
        // A release does not say which button: it is 3 in every case.
        for button in [Left, Middle, Right] {
            assert_eq!(
                encode(Release(button), 3, 4, &old),
                Some(vec![0x1b, b'[', b'M', 35, 36, 37]),
                "{button:?}"
            );
        }
        assert_eq!(
            encode(WheelUp, 3, 4, &old),
            Some(vec![0x1b, b'[', b'M', 96, 36, 37])
        );
        assert_eq!(
            encode(WheelDown, 3, 4, &old),
            Some(vec![0x1b, b'[', b'M', 97, 36, 37])
        );
    }

    #[test]
    fn the_old_form_cannot_say_a_cell_past_222() {
        let old = modes(MouseMode::Click, false);
        // The last cell the form can name: 222 is 223 from one, which is the byte 255.
        assert_eq!(
            encode(Press(Left), 222, 222, &old),
            Some(vec![0x1b, b'[', b'M', 32, 255, 255])
        );
        assert_eq!(encode(Press(Left), 223, 0, &old), None);
        assert_eq!(encode(Press(Left), 0, 223, &old), None);
        assert_eq!(encode(Press(Left), u16::MAX, u16::MAX, &old), None);
    }

    #[test]
    fn modifiers_add_4_8_and_16() {
        let sgr = modes(MouseMode::Click, true);
        let with = |shift, alt, control| Modifiers {
            shift,
            alt,
            control,
            ..NO_MODIFIERS
        };
        for (modifiers, expected) in [
            (with(true, false, false), "\x1b[<4;1;1M"),
            (with(false, true, false), "\x1b[<8;1;1M"),
            (with(false, false, true), "\x1b[<16;1;1M"),
            (with(true, true, true), "\x1b[<28;1;1M"),
        ] {
            assert_eq!(
                encode_mouse(Press(Left), 0, 0, &modifiers, &sgr),
                Some(expected.as_bytes().to_vec())
            );
        }
        assert_eq!(
            encode_mouse(WheelDown, 0, 0, &with(true, false, true), &sgr),
            Some(b"\x1b[<85;1;1M".to_vec())
        );
        let old = modes(MouseMode::Click, false);
        assert_eq!(
            encode_mouse(Press(Left), 0, 0, &with(false, true, false), &old),
            Some(vec![0x1b, b'[', b'M', 40, 33, 33])
        );
        assert_eq!(
            encode_mouse(Release(Right), 0, 0, &with(false, false, true), &old),
            Some(vec![0x1b, b'[', b'M', 32 + 3 + 16, 33, 33])
        );
    }

    #[test]
    fn a_click_mode_reports_presses_releases_and_the_wheel_and_no_moves() {
        for sgr in [true, false] {
            let click = modes(MouseMode::Click, sgr);
            for action in [Press(Left), Release(Left), WheelUp, WheelDown] {
                assert!(encode(action, 1, 1, &click).is_some(), "{action:?}");
            }
            assert_eq!(encode(Move(Some(Left)), 1, 1, &click), None);
            assert_eq!(encode(Move(None), 1, 1, &click), None);
        }
    }

    #[test]
    fn a_drag_mode_adds_the_moves_with_a_button_held() {
        let drag = modes(MouseMode::Drag, true);
        assert_eq!(
            encode(Move(Some(Left)), 3, 4, &drag),
            Some(b"\x1b[<32;4;5M".to_vec())
        );
        assert_eq!(
            encode(Move(Some(Middle)), 3, 4, &drag),
            Some(b"\x1b[<33;4;5M".to_vec())
        );
        assert_eq!(
            encode(Move(Some(Right)), 3, 4, &drag),
            Some(b"\x1b[<34;4;5M".to_vec())
        );
        assert_eq!(encode(Move(None), 3, 4, &drag), None);
        assert!(encode(Press(Left), 3, 4, &drag).is_some());
        let old = modes(MouseMode::Drag, false);
        assert_eq!(
            encode(Move(Some(Left)), 3, 4, &old),
            Some(vec![0x1b, b'[', b'M', 64, 36, 37])
        );
    }

    #[test]
    fn a_motion_mode_adds_every_move() {
        let motion = modes(MouseMode::Motion, true);
        // A move with no button is 3 and 32 more.
        assert_eq!(
            encode(Move(None), 3, 4, &motion),
            Some(b"\x1b[<35;4;5M".to_vec())
        );
        assert_eq!(
            encode(Move(Some(Left)), 3, 4, &motion),
            Some(b"\x1b[<32;4;5M".to_vec())
        );
        let old = modes(MouseMode::Motion, false);
        assert_eq!(
            encode(Move(None), 3, 4, &old),
            Some(vec![0x1b, b'[', b'M', 67, 36, 37])
        );
        let with_ctrl = Modifiers {
            control: true,
            ..NO_MODIFIERS
        };
        assert_eq!(
            encode_mouse(Move(None), 3, 4, &with_ctrl, &motion),
            Some(b"\x1b[<51;4;5M".to_vec())
        );
    }

    #[test]
    fn on_the_alternate_screen_the_wheel_is_the_arrow_keys_for_a_program_with_no_mouse() {
        let screen = Modes {
            alt_screen: true,
            alternate_scroll: true,
            ..Modes::default()
        };
        assert_eq!(encode(WheelUp, 3, 4, &screen), Some(b"\x1b[A".to_vec()));
        assert_eq!(encode(WheelDown, 3, 4, &screen), Some(b"\x1b[B".to_vec()));
        // The arrows as the cursor mode says them.
        let application = Modes {
            app_cursor: true,
            ..screen
        };
        assert_eq!(
            encode(WheelUp, 3, 4, &application),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            encode(WheelDown, 3, 4, &application),
            Some(b"\x1bOB".to_vec())
        );
        // Only the wheel.
        for action in [Press(Left), Release(Left), Move(None), Move(Some(Left))] {
            assert_eq!(encode(action, 3, 4, &screen), None, "{action:?}");
        }
    }

    #[test]
    fn the_wheel_is_left_to_the_pane_without_the_alternate_screen_or_the_setting() {
        let primary = Modes {
            alternate_scroll: true,
            ..Modes::default()
        };
        assert_eq!(encode(WheelUp, 3, 4, &primary), None);
        let unset = Modes {
            alt_screen: true,
            ..Modes::default()
        };
        assert_eq!(encode(WheelUp, 3, 4, &unset), None);
    }

    #[test]
    fn a_program_that_asked_for_the_mouse_gets_the_wheel_as_buttons_not_arrows() {
        let both = Modes {
            alt_screen: true,
            alternate_scroll: true,
            mouse: MouseMode::Click,
            mouse_sgr: true,
            ..Modes::default()
        };
        assert_eq!(
            encode(WheelUp, 3, 4, &both),
            Some(b"\x1b[<64;4;5M".to_vec())
        );
    }
}

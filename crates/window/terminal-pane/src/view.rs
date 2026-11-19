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
//!
//! The mouse. With the program's mouse mode off, a drag with the left button selects
//! text, and a step of the wheel scrolls the history three lines, or, on the alternate
//! screen with alternate scroll on, is one arrow key. With a mode on, presses, releases,
//! moves and the wheel are sent in the form the program asked for ([`crate::encode_mouse`]);
//! a move that would send the very bytes the last move sent is not sent again, so that a
//! pointer crossing one cell reports it once. One step of the wheel is one line of a
//! [`ScrollDelta::Lines`] event, or the height of a cell in the pixels of a
//! [`ScrollDelta::Pixels`] one, the rest carried to the next event.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_frame::{KeeperLink, PROGRAM_CONTEXT, TerminalOutput};
use yaar_link::{Ending, Request, TerminalId, TerminalSize};

use crate::MouseButton as ReportedButton;
use crate::draw::{self, Grid, Palette};
use crate::{
    CellPlace, Copy, Emulator, MouseAction, MouseMode, OptionKey, Paste, ScrollPageDown,
    ScrollPageUp, encode_focus, encode_key, encode_mouse, encode_paste, encode_release,
};

/// How many lines of history a step of the wheel scrolls while the program takes no
/// mouse reports.
const LINES_A_STEP: i32 = 3;

/// What the view puts before the kept output of a terminal the Keeper cut, in the style
/// that [`TerminalView::cut_band`] looks for: faint and italic.
const CUT_NOTICE: &str = "\x1b[2;3mOlder output is gone.\x1b[0m\r\n";

/// The text of that line.
const CUT_TEXT: &str = "Older output is gone.";

/// What a view tells the pane that holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewEvent {
    /// The columns and rows that fit the view changed, and the Keeper was asked to give
    /// the terminal this size.
    Fitted(TerminalSize),
}

/// What the painter learns while it paints and the listeners read when an event comes:
/// where the grid is and how large a cell is.
#[derive(Default)]
struct Geometry {
    bounds: Cell<Bounds<Pixels>>,
    cell: Cell<Size<Pixels>>,
    /// The size and the cell last given to the emulator, so that a frame that fits the
    /// same grid tells nobody anything.
    fitted: Cell<Option<(TerminalSize, Size<Pixels>)>>,
}

/// One terminal of the pane.
///
/// It holds the emulator that reads what the program writes, and sends the program what
/// is typed, pasted and done with the mouse, and what the terminal answers its
/// questions. It tells the Keeper the size of its grid whenever that changes, and the
/// pane through [`ViewEvent`].
pub struct TerminalView {
    id: TerminalId,
    link: Entity<KeeperLink>,
    emulator: Emulator,
    focus: FocusHandle,
    /// The size the Keeper was last told.
    size: TerminalSize,
    marked: Option<String>,
    ending: Option<Ending>,
    geometry: Rc<Geometry>,
    /// The cells selected, as the painter marks them.
    selection: Option<(CellPlace, CellPlace)>,
    /// What the selection said last: when output changes it, the mark goes.
    selected_text: Option<String>,
    /// Where a drag that selects began.
    drag_from: Option<CellPlace>,
    /// The button whose press the program was told, until it is released.
    reported: Option<ReportedButton>,
    /// The bytes of the last move sent, to keep from sending the same ones twice.
    last_move: Option<Vec<u8>>,
    /// Pixels of the wheel not yet worth a step.
    wheel_rest: f32,
    /// Whether the screen starts with the line that says older output is gone.
    cut: bool,
    attached: bool,
    _output: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ViewEvent> for TerminalView {}

impl TerminalView {
    /// A view of the terminal `id`, whose grid is `size` until the painter fits it to
    /// the room it has. `output` is what the program writes, if the link had a
    /// connection to take it from: the view reads it as it comes. When the Keeper dropped
    /// the oldest of that output, `cut` is true and the screen starts with a line that says
    /// so, before the first byte of it.
    pub(crate) fn new(
        id: TerminalId,
        size: TerminalSize,
        link: Entity<KeeperLink>,
        output: Option<TerminalOutput>,
        cut: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        // One task for the terminal: it waits for output, takes everything that has
        // come by then in one go, and tells the screen once.
        let output = output.map(|output| {
            cx.spawn(async move |view, cx| {
                while let Some(mut bytes) = output.next().await {
                    while let Some(more) = output.ready() {
                        bytes.extend(more);
                    }
                    if view
                        .update(cx, |view, cx| view.take_output(&bytes, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            })
        });
        let mut emulator = Emulator::new(size);
        if cut {
            emulator.advance(CUT_NOTICE.as_bytes());
        }
        Self {
            id,
            link,
            emulator,
            focus: cx.focus_handle(),
            size,
            marked: None,
            ending: None,
            geometry: Rc::default(),
            selection: None,
            selected_text: None,
            drag_from: None,
            reported: None,
            last_move: None,
            wheel_rest: 0.,
            cut,
            attached: false,
            _output: output,
            _subscriptions: Vec::new(),
        }
    }

    /// Where the line that says older output is gone is drawn, as the distance from the
    /// top of the view and its height, while it is in view: it is part of the output, and
    /// scrolls with it. None for a terminal the Keeper kept whole.
    pub(crate) fn cut_band(&self, window: &Window, cx: &App) -> Option<(Pixels, Pixels)> {
        if !self.cut {
            return None;
        }
        let row = self.emulator.screen().lines.iter().position(|line| {
            let text: String = line.iter().map(|cell| cell.text.as_str()).collect();
            let styled = line
                .first()
                .is_some_and(|cell| cell.style.dim && cell.style.italic);
            styled && text.trim_end() == CUT_TEXT
        })?;
        let (_, _, cell) = Self::metrics(window, cx);
        Some((cell.height * row as f32, cell.height))
    }

    /// The terminal this view shows.
    pub fn id(&self) -> &TerminalId {
        &self.id
    }

    /// The text of the lines the view draws, top to bottom, each without the blanks at
    /// its end. The painter draws from the same screen.
    pub fn lines(&self) -> Vec<String> {
        self.emulator.lines()
    }

    /// The text an input method or a dead key is composing, which the view draws
    /// underlined at the cursor and has not sent; none while nothing is composed.
    pub fn marked_text(&self) -> Option<String> {
        self.marked.clone()
    }

    /// The title the program gave the terminal; none before it gave one.
    pub fn title(&self) -> Option<String> {
        self.emulator.title()
    }

    /// How the program ended; none while it runs. The view still shows its last screen.
    pub fn ending(&self) -> Option<Ending> {
        self.ending
    }

    /// Where the keys go while this view has the focus.
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// The program ended: the view keeps its screen and says how.
    pub(crate) fn ended(&mut self, ending: Ending, cx: &mut Context<Self>) {
        self.ending = Some(ending);
        cx.notify();
    }

    /// Sends the program bytes, unless it has ended: the Keeper no longer holds the
    /// terminal.
    fn send(&self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if bytes.is_empty() || self.ending.is_some() {
            return;
        }
        self.link
            .update(cx, |link, _| link.send_input(&self.id, bytes));
    }

    /// Reads what the program wrote, and sends back what the terminal answers.
    fn take_output(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        self.emulator.advance(bytes);
        let replies = self.emulator.take_replies();
        self.send(replies, cx);
        // Output that changes the text under a selection takes the mark away: it would
        // be drawn on text it does not hold.
        if self.selection.is_some() && self.emulator.selected_text() != self.selected_text {
            self.forget_selection();
        }
        cx.notify();
    }

    fn forget_selection(&mut self) {
        self.selection = None;
        self.selected_text = None;
        self.emulator.clear_selection();
    }

    /// What typing or pasting does to what the view shows: the newest line comes back
    /// into view and nothing stays selected.
    fn back_to_the_prompt(&mut self) {
        let back = self.emulator.scrolled_back();
        if back > 0 {
            self.emulator.scroll_lines(-(back as i32));
        }
        self.forget_selection();
    }

    /// The Option setting for the keyboard layout in use.
    fn option_key(cx: &App) -> OptionKey {
        OptionKey::for_layout(cx.keyboard_layout().id(), cfg!(target_os = "macos"))
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let modes = self.emulator.modes();
        let option = Self::option_key(cx);
        let keystroke = &event.keystroke;
        let modifiers = &keystroke.modifiers;
        // A key that types text has no bytes here: it is left alone, and the platform
        // hands its text to the input handler. Its repeats are the exception: macOS
        // would offer the accents of the letter instead of repeating it, and no terminal
        // wants that, so a held key's text is sent here, as the key repeats.
        let bytes = encode_key(keystroke, &modes, option, event.is_held).or_else(|| {
            let text = keystroke.key_char.as_ref()?;
            let plain = !modifiers.alt && !modifiers.control && !modifiers.platform;
            (event.is_held && plain).then(|| text.as_bytes().to_vec())
        });
        if let Some(bytes) = bytes {
            self.back_to_the_prompt();
            self.send(bytes, cx);
            cx.notify();
            cx.stop_propagation();
        }
    }

    fn key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let modes = self.emulator.modes();
        let option = Self::option_key(cx);
        if let Some(bytes) = encode_release(&event.keystroke, &modes, option) {
            self.send(bytes, cx);
            cx.stop_propagation();
        }
    }

    /// The program's terminal gained or lost the focus.
    fn focus_changed(&mut self, focused: bool, cx: &mut Context<Self>) {
        if let Some(bytes) = encode_focus(focused, &self.emulator.modes()) {
            self.send(bytes, cx);
        }
        cx.notify();
    }

    /// What has to be set up once there is a window to set it up in.
    fn attach(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.attached = true;
        let gained = cx.on_focus(&self.focus, window, |view, _, cx| {
            view.focus_changed(true, cx);
        });
        let lost = cx.on_blur(&self.focus, window, |view, _, cx| {
            view.focus_changed(false, cx);
        });
        self._subscriptions.extend([gained, lost]);
    }

    /// Gives the terminal the size that fits the room the painter found, when it is not
    /// the one it has: the screen, the Keeper and the pane are told, once.
    fn fit(&mut self, size: TerminalSize, cell: Size<Pixels>, cx: &mut Context<Self>) {
        self.emulator
            .set_cell_size(f32::from(cell.width) as u16, f32::from(cell.height) as u16);
        // A program that ended has no terminal left to resize.
        if size == self.size || self.ending.is_some() {
            return;
        }
        self.size = size;
        self.emulator.resize(size);
        // Nothing waits for the answer: a refusal leaves the Keeper's terminal at the
        // size it had, and the next change asks again.
        let terminal = self.id.clone();
        self.link.update(cx, |link, cx| {
            link.ask(Request::ResizeTerminal { terminal, size }, cx)
                .detach();
        });
        cx.emit(ViewEvent::Fitted(size));
        cx.notify();
    }

    /// The cell under a point of the window, the nearest one for a point outside.
    fn cell_at(&self, position: Point<Pixels>) -> CellPlace {
        let bounds = self.geometry.bounds.get();
        let cell = self.geometry.cell.get();
        let size = self.emulator.size();
        let along = |offset: Pixels, step: Pixels, count: u16| {
            let n = (f32::from(offset) / f32::from(step).max(1.))
                .floor()
                .max(0.);
            (n as usize).min(usize::from(count.max(1)) - 1)
        };
        CellPlace {
            line: along(position.y - bounds.origin.y, cell.height, size.rows),
            column: along(position.x - bounds.origin.x, cell.width, size.columns),
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let place = self.cell_at(event.position);
        let modes = self.emulator.modes();
        let button = reported_button(event.button);
        let sent = button.and_then(|button| {
            encode_mouse(
                MouseAction::Press(button),
                place.column as u16,
                place.line as u16,
                &event.modifiers,
                &modes,
            )
        });
        if let Some(bytes) = sent {
            self.reported = button;
            self.last_move = None;
            self.send(bytes, cx);
        } else if event.button == gpui_kit::MouseButton::Left && modes.mouse == MouseMode::Off {
            self.forget_selection();
            self.drag_from = Some(place);
            cx.notify();
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let place = self.cell_at(event.position);
        let modes = self.emulator.modes();
        let held = event.pressed_button.and_then(reported_button);
        let sent = encode_mouse(
            MouseAction::Move(held),
            place.column as u16,
            place.line as u16,
            &event.modifiers,
            &modes,
        );
        if let Some(bytes) = sent {
            if self.last_move.as_ref() != Some(&bytes) {
                self.last_move = Some(bytes.clone());
                self.send(bytes, cx);
            }
        } else if let Some(from) = self.drag_from
            && event.pressed_button == Some(gpui_kit::MouseButton::Left)
        {
            self.emulator.select(from, place);
            self.selection = Some((from, place));
            self.selected_text = self.emulator.selected_text();
            cx.notify();
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let place = self.cell_at(event.position);
        let modes = self.emulator.modes();
        let button = reported_button(event.button);
        if self.reported.is_some() && self.reported == button {
            self.reported = None;
            self.last_move = None;
            let bytes = button.and_then(|button| {
                encode_mouse(
                    MouseAction::Release(button),
                    place.column as u16,
                    place.line as u16,
                    &event.modifiers,
                    &modes,
                )
            });
            if let Some(bytes) = bytes {
                self.send(bytes, cx);
            }
        } else if event.button == gpui_kit::MouseButton::Left && self.drag_from.take().is_some() {
            // A click that selected no more than the cell it was in selects nothing.
            if self.selection.is_some_and(|(from, to)| from == to) {
                self.forget_selection();
            }
            cx.notify();
        }
    }

    fn scroll(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let cell = self.geometry.cell.get();
        // Positive is up, towards the older lines.
        let steps = match event.delta {
            ScrollDelta::Lines(lines) if lines.y == 0. => 0,
            ScrollDelta::Lines(lines) => {
                lines.y.abs().round().max(1.) as i32 * lines.y.signum() as i32
            }
            ScrollDelta::Pixels(pixels) => {
                self.wheel_rest += f32::from(pixels.y);
                let height = f32::from(cell.height).max(1.);
                let steps = (self.wheel_rest / height).trunc();
                self.wheel_rest -= steps * height;
                steps as i32
            }
        };
        if steps == 0 {
            return;
        }
        let place = self.cell_at(event.position);
        let modes = self.emulator.modes();
        let action = if steps > 0 {
            MouseAction::WheelUp
        } else {
            MouseAction::WheelDown
        };
        let one = encode_mouse(
            action,
            place.column as u16,
            place.line as u16,
            &event.modifiers,
            &modes,
        );
        match one {
            Some(bytes) => {
                self.last_move = None;
                self.send(bytes.repeat(steps.unsigned_abs() as usize), cx);
            }
            // The program takes no wheel: the history scrolls.
            None => {
                self.forget_selection();
                self.emulator.scroll_lines(steps * LINES_A_STEP);
                cx.notify();
            }
        }
        cx.stop_propagation();
    }

    fn scroll_page(&mut self, up: bool, cx: &mut Context<Self>) {
        self.forget_selection();
        self.emulator.scroll_page(up);
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.emulator.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.paste_text(&text, cx);
        }
    }

    fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        self.back_to_the_prompt();
        let bytes = encode_paste(text, &self.emulator.modes());
        self.send(bytes, cx);
        cx.notify();
    }

    /// The font, its size and the size of one cell.
    pub(crate) fn metrics(window: &Window, cx: &App) -> (Font, Pixels, Size<Pixels>) {
        let theme = cx.theme();
        let font = font(theme.mono_font_family.clone());
        let font_size = theme.mono_font_size;
        let system = window.text_system();
        let id = system.resolve_font(&font);
        let width = system
            .advance(id, font_size, 'm')
            .map(|advance| advance.width)
            .unwrap_or(font_size * 0.6);
        let height = (system.ascent(id, font_size) + system.descent(id, font_size))
            .max(font_size * 1.2)
            .ceil();
        (font, font_size, size(width, height))
    }
}

/// The button of the mouse as the encoders name it; none for one a program is not told
/// of.
fn reported_button(button: gpui_kit::MouseButton) -> Option<ReportedButton> {
    match button {
        gpui_kit::MouseButton::Left => Some(ReportedButton::Left),
        gpui_kit::MouseButton::Middle => Some(ReportedButton::Middle),
        gpui_kit::MouseButton::Right => Some(ReportedButton::Right),
        _ => None,
    }
}

/// How many UTF-16 units a text takes.
fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.attached {
            self.attach(window, cx);
        }
        // A synchronized update the program left open ends in its own time, with no
        // output to say so.
        self.emulator.advance(&[]);

        let (font, font_size, cell) = Self::metrics(window, cx);
        self.geometry.cell.set(cell);
        let theme = cx.theme();
        let (background, foreground, muted, border) = (
            theme.background,
            theme.foreground,
            theme.muted_foreground,
            theme.border,
        );
        let palette = Palette::of(theme);
        let screen = self.emulator.screen();
        let at = screen
            .cursor
            .map(|cursor| (cursor.line, cursor.column))
            .unwrap_or((0, 0));
        let focused = self.focus.is_focused(window);
        let selection = self.selection;
        let geometry = self.geometry.clone();
        let view = cx.entity();
        let focus = self.focus.clone();
        let initial = self.size;

        let marked = self.marked.clone().map(|text| {
            div()
                .debug_selector(|| "terminal-marked-text".to_owned())
                .absolute()
                .left(cell.width * at.1 as f32)
                .top(cell.height * at.0 as f32)
                .h(cell.height)
                .bg(background)
                .text_color(foreground)
                .font_family(font.family.clone())
                .text_size(font_size)
                .whitespace_nowrap()
                .underline()
                .child(text)
        });
        let ended = self.ending.map(|ending| {
            div()
                .debug_selector(|| "terminal-ended".to_owned())
                .absolute()
                .bottom_0()
                .left_0()
                .right_0()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(muted)
                .bg(background)
                .border_t_1()
                .border_color(border)
                .child(format!("The program {ending}."))
        });

        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .key_context(PROGRAM_CONTEXT)
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_key_down(cx.listener(Self::key_down))
            .on_key_up(cx.listener(Self::key_up))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(|view, _: &ScrollPageUp, _, cx| view.scroll_page(true, cx)))
            .on_action(cx.listener(|view, _: &ScrollPageDown, _, cx| view.scroll_page(false, cx)))
            .on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(gpui_kit::MouseButton::Middle, cx.listener(Self::mouse_down))
            .on_mouse_down(gpui_kit::MouseButton::Right, cx.listener(Self::mouse_down))
            .on_mouse_up(gpui_kit::MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up(gpui_kit::MouseButton::Middle, cx.listener(Self::mouse_up))
            .on_mouse_up(gpui_kit::MouseButton::Right, cx.listener(Self::mouse_up))
            .on_mouse_up_out(gpui_kit::MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(gpui_kit::MouseButton::Middle, cx.listener(Self::mouse_up))
            .on_mouse_up_out(gpui_kit::MouseButton::Right, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_scroll(cx.listener(Self::scroll))
            .child(
                canvas(
                    |bounds, _, _| bounds,
                    move |_, bounds, window, cx| {
                        geometry.bounds.set(bounds);
                        // Whole cells only, from the top left corner.
                        let columns = (f32::from(bounds.size.width) / f32::from(cell.width))
                            .floor()
                            .max(1.) as u16;
                        let rows = (f32::from(bounds.size.height) / f32::from(cell.height))
                            .floor()
                            .max(1.) as u16;
                        let fitted = (TerminalSize { columns, rows }, cell);
                        let known = geometry.fitted.get().unwrap_or((initial, Size::default()));
                        if known != fitted {
                            geometry.fitted.set(Some(fitted));
                            let view = view.clone();
                            window.defer(cx, move |_, cx| {
                                view.update(cx, |view, cx| view.fit(fitted.0, fitted.1, cx));
                            });
                        }
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, view.clone()),
                            cx,
                        );
                        let grid = Grid {
                            origin: bounds.origin,
                            cell,
                            font,
                            font_size,
                            palette: &palette,
                            focused,
                            selection,
                        };
                        draw::paint(&screen, &grid, window, cx);
                    },
                )
                .size_full(),
            )
            .children(marked)
            .children(ended)
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        // The view holds no document: only the marked text can be asked for.
        let marked = self.marked.as_ref()?;
        let units: Vec<u16> = marked.encode_utf16().collect();
        let end = range.end.min(units.len());
        let start = range.start.min(end);
        *adjusted_range = Some(start..end);
        Some(String::from_utf16_lossy(&units[start..end]))
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        // The cursor sits after the marked text, or nowhere to speak of.
        let end = self.marked.as_deref().map_or(0, utf16_len);
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_deref().map(|marked| 0..utf16_len(marked))
    }

    /// Ends the composition and sends nothing: what was marked is dropped.
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.marked.take().is_some() {
            cx.notify();
        }
    }

    fn paste(&mut self, item: ClipboardItem, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = item.text() {
            self.paste_text(&text, cx);
        }
    }

    /// The text is final: it goes to the program and the marked text is gone.
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = None;
        if !text.is_empty() {
            self.back_to_the_prompt();
            self.send(text.as_bytes().to_vec(), cx);
        }
        cx.notify();
    }

    /// The text is still being composed: it is kept and drawn at the cursor, and nothing
    /// is sent. An empty text ends the composition.
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        new_text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!new_text.is_empty()).then(|| new_text.to_owned());
        cx.notify();
    }

    /// The cell under the cursor, where the system puts its candidate window.
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let cell = self.geometry.cell.get();
        let cursor = self.emulator.screen().cursor?;
        Some(Bounds::new(
            element_bounds.origin
                + point(
                    cell.width * cursor.column as f32,
                    cell.height * cursor.line as f32,
                ),
            cell,
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

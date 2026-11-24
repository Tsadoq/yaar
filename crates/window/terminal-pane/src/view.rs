//! One terminal as the pane shows it: its screen, drawn by the app's own painter, and the keys,
//! text and mouse that go to its program. A key arrives after the key bindings; text goes through
//! the input handler, which holds marked text while an input method composes.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_frame::theme::{self, TerminalColors};
use yaar_frame::{KeeperLink, PROGRAM_CONTEXT, SettingList, TerminalOutput};
use yaar_link::{Ending, Request, TerminalId, TerminalSize};

use crate::MouseButton as ReportedButton;
use crate::draw::{self, Grid};
use crate::{
    CellPlace, Clear, Copy, Emulator, MouseAction, MouseMode, OptionKey, Paste, ScrollLineDown,
    ScrollLineUp, ScrollPageDown, ScrollPageUp, ScrollToBottom, ScrollToTop, SelectAll,
    encode_focus, encode_key, encode_mouse, encode_paste, encode_release,
};

const LINES_A_STEP: i32 = 3;

/// Written before the kept output of a terminal the Keeper cut, faint and italic: the style
/// `TerminalView::cut_band` looks for.
const CUT_NOTICE: &str = "\x1b[2;3mOlder output is gone.\x1b[0m\r\n";

const CUT_TEXT: &str = "Older output is gone.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewEvent {
    Fitted(TerminalSize),
}

#[derive(Default)]
struct Geometry {
    bounds: Cell<Bounds<Pixels>>,
    cell: Cell<Size<Pixels>>,
    fitted: Cell<Option<(TerminalSize, Size<Pixels>)>>,
}

pub struct TerminalView {
    id: TerminalId,
    link: Entity<KeeperLink>,
    settings: Entity<SettingList>,
    emulator: Emulator,
    colours: TerminalColors,
    focus: FocusHandle,
    size: TerminalSize,
    marked: Option<String>,
    ending: Option<Ending>,
    geometry: Rc<Geometry>,
    selection: Option<(CellPlace, CellPlace)>,
    selected_text: Option<String>,
    drag_from: Option<CellPlace>,
    reported: Option<ReportedButton>,
    last_move: Option<Vec<u8>>,
    wheel_rest: f32,
    cut: bool,
    attached: bool,
    _output: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ViewEvent> for TerminalView {}

impl TerminalView {
    /// `output` is what the program writes, if the link had a connection to take it from. `cut` is
    /// true when the Keeper dropped the oldest output.
    pub(crate) fn new(
        id: TerminalId,
        size: TerminalSize,
        link: Entity<KeeperLink>,
        settings: Entity<SettingList>,
        output: Option<TerminalOutput>,
        cut: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        // Batched: everything that arrived is read at once and the screen notified once.
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
        // A program may ask for its colours before the first frame is drawn.
        let colours = theme::applied(cx).terminal;
        emulator.set_colours(colours);
        if cut {
            emulator.advance(CUT_NOTICE.as_bytes());
        }
        Self {
            id,
            link,
            settings,
            emulator,
            colours,
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

    /// Where the "older output is gone" line is drawn, as (distance from the view's top, height);
    /// None for a terminal the Keeper kept whole.
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

    pub fn id(&self) -> &TerminalId {
        &self.id
    }

    pub fn lines(&self) -> Vec<String> {
        self.emulator.lines()
    }

    pub fn marked_text(&self) -> Option<String> {
        self.marked.clone()
    }

    pub fn scrolled_back(&self) -> usize {
        self.emulator.scrolled_back()
    }

    pub fn title(&self) -> Option<String> {
        self.emulator.title()
    }

    pub fn ending(&self) -> Option<Ending> {
        self.ending
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub(crate) fn ended(&mut self, ending: Ending, cx: &mut Context<Self>) {
        self.ending = Some(ending);
        cx.notify();
    }

    /// Does nothing once the program has ended: the Keeper no longer holds the terminal.
    fn send(&self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if bytes.is_empty() || self.ending.is_some() {
            return;
        }
        self.link
            .update(cx, |link, _| link.send_input(&self.id, bytes));
    }

    fn take_output(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        self.emulator.advance(bytes);
        let replies = self.emulator.take_replies();
        self.send(replies, cx);
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

    fn back_to_the_prompt(&mut self) {
        let back = self.emulator.scrolled_back();
        if back > 0 {
            self.emulator.scroll_lines(-(back as i32));
        }
        self.forget_selection();
    }

    fn option_key(&self, cx: &App) -> OptionKey {
        OptionKey::of_setting(
            self.settings.read(cx).option_key(),
            cx.keyboard_layout().id(),
            cfg!(target_os = "macos"),
        )
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let modes = self.emulator.modes();
        let option = self.option_key(cx);
        let keystroke = &event.keystroke;
        let modifiers = &keystroke.modifiers;
        // A key that types text has no bytes here: the platform hands its text to the input
        // handler. Except repeats: macOS would offer the letter's accents instead of repeating it,
        // so a held key's text is sent here.
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
        let option = self.option_key(cx);
        if let Some(bytes) = encode_release(&event.keystroke, &modes, option) {
            self.send(bytes, cx);
            cx.stop_propagation();
        }
    }

    fn focus_changed(&mut self, focused: bool, cx: &mut Context<Self>) {
        if let Some(bytes) = encode_focus(focused, &self.emulator.modes()) {
            self.send(bytes, cx);
        }
        cx.notify();
    }

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

    fn fit(&mut self, size: TerminalSize, cell: Size<Pixels>, cx: &mut Context<Self>) {
        self.emulator
            .set_cell_size(f32::from(cell.width) as u16, f32::from(cell.height) as u16);
        if size == self.size || self.ending.is_some() {
            return;
        }
        self.size = size;
        self.emulator.resize(size);
        // Nothing waits for the answer: a refusal leaves the Keeper's terminal at its old size and
        // the next change asks again.
        let terminal = self.id.clone();
        self.link.update(cx, |link, cx| {
            link.ask(Request::ResizeTerminal { terminal, size }, cx)
                .detach();
        });
        cx.emit(ViewEvent::Fitted(size));
        cx.notify();
    }

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

    fn scroll_with(&mut self, scroll: fn(&mut Emulator), cx: &mut Context<Self>) {
        self.forget_selection();
        scroll(&mut self.emulator);
        cx.notify();
    }

    fn scroll_by(&mut self, lines: i32, cx: &mut Context<Self>) {
        self.forget_selection();
        self.emulator.scroll_lines(lines);
        cx.notify();
    }

    /// Nothing is sent to the program: a shell that wants its prompt redrawn is asked by the user.
    fn clear(&mut self, _: &Clear, _: &mut Window, cx: &mut Context<Self>) {
        self.forget_selection();
        self.emulator.clear_above_the_cursor();
        cx.notify();
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.forget_selection();
        self.selection = Some(self.emulator.select_all());
        self.selected_text = self.emulator.selected_text();
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

fn reported_button(button: gpui_kit::MouseButton) -> Option<ReportedButton> {
    match button {
        gpui_kit::MouseButton::Left => Some(ReportedButton::Left),
        gpui_kit::MouseButton::Middle => Some(ReportedButton::Middle),
        gpui_kit::MouseButton::Right => Some(ReportedButton::Right),
        _ => None,
    }
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.attached {
            self.attach(window, cx);
        }
        // A synchronized update the program left open ends in its own time, with no output to say
        // so.
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
        // A program asks for its colours with a question the emulator answers at once, so a theme
        // change is handed on at the frame that draws with the new definition.
        let colours = theme::applied(cx).terminal;
        if colours != self.colours {
            self.colours = colours;
            self.emulator.set_colours(colours);
        }
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
            // The terminal theme's background, which a program is told when it asks, not the
            // window's.
            .bg(colours.background)
            // While a terminal has the focus no Frame key fires but Cmd and kept keys, and the
            // toolkit's window-wide keys reach the program.
            .key_context(PROGRAM_CONTEXT)
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_key_down(cx.listener(Self::key_down))
            .on_key_up(cx.listener(Self::key_up))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(|view, _: &ScrollPageUp, _, cx| view.scroll_page(true, cx)))
            .on_action(cx.listener(|view, _: &ScrollPageDown, _, cx| view.scroll_page(false, cx)))
            .on_action(cx.listener(Self::clear))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(|view, _: &ScrollToTop, _, cx| {
                view.scroll_with(Emulator::scroll_to_top, cx)
            }))
            .on_action(cx.listener(|view, _: &ScrollToBottom, _, cx| {
                view.scroll_with(Emulator::scroll_to_bottom, cx)
            }))
            .on_action(cx.listener(|view, _: &ScrollLineUp, _, cx| view.scroll_by(1, cx)))
            .on_action(cx.listener(|view, _: &ScrollLineDown, _, cx| view.scroll_by(-1, cx)))
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
                            colours: &colours,
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
        let end = self.marked.as_deref().map_or(0, utf16_len);
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_deref().map(|marked| 0..utf16_len(marked))
    }

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

    /// Where the system puts its candidate window.
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

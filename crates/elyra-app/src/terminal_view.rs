//! Terminal surfaces: [`TerminalView`] renders one PTY-backed terminal with
//! full TUI support (mouse reporting, selection, IME, cursor styles, links);
//! [`TerminalPanel`] holds several terminals for one thread as tabs.

use crate::preferences::Preferences;
use crate::themes;
use elyra_terminal::{
    CursorShape, GridPos, KeyInput, MouseButton as TermButton, MouseModifiers, Palette, Rgb,
    SelectionKind, Snapshot, Terminal, TerminalEvent, key_to_bytes,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::cell::Cell;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

const PADDING: f32 = 8.;
const BLINK_INTERVAL: Duration = Duration::from_millis(530);

gpui_kit::actions!(
    terminal,
    [NewTerminal, CloseTerminal, NextTerminal, PreviousTerminal]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-t", NewTerminal, Some("TerminalPanel")),
        KeyBinding::new("cmd-shift-w", CloseTerminal, Some("TerminalPanel")),
        KeyBinding::new("cmd-shift-]", NextTerminal, Some("TerminalPanel")),
        KeyBinding::new("cmd-shift-[", PreviousTerminal, Some("TerminalPanel")),
    ]);
}

fn color(rgb: Rgb) -> Hsla {
    gpui_kit::rgb(rgb.to_u32()).into()
}

/// Geometry of the last paint, used to map pointer positions to cells.
#[derive(Clone, Copy, Debug)]
struct Layout {
    origin: Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    columns: usize,
    lines: usize,
    cursor: Option<Bounds<Pixels>>,
}

impl Layout {
    fn grid_pos(&self, position: Point<Pixels>) -> GridPos {
        let x = ((position.x - self.origin.x) / self.cell_width).max(0.);
        let y = ((position.y - self.origin.y) / self.line_height).max(0.);
        GridPos {
            line: (y.floor() as usize).min(self.lines.saturating_sub(1)),
            column: (x.floor() as usize).min(self.columns.saturating_sub(1)),
            right_half: x.fract() > 0.5,
        }
    }
}

fn mouse_mods(modifiers: &Modifiers) -> MouseModifiers {
    MouseModifiers {
        shift: modifiers.shift,
        alt: modifiers.alt,
        ctrl: modifiers.control,
    }
}

fn term_button(button: MouseButton) -> Option<TermButton> {
    match button {
        MouseButton::Left => Some(TermButton::Left),
        MouseButton::Middle => Some(TermButton::Middle),
        MouseButton::Right => Some(TermButton::Right),
        _ => None,
    }
}

/// A shell running in a thread's working directory.
pub struct TerminalView {
    terminal: Option<Arc<Terminal>>,
    error: Option<String>,
    focus: FocusHandle,
    pub title: Option<String>,
    exited: bool,
    marked_text: Option<String>,
    layout: Rc<Cell<Option<Layout>>>,
    selecting: bool,
    blink_on: bool,
    scroll_remainder: f32,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl TerminalView {
    pub fn new(cwd: &Path, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let options = Preferences::global(cx).terminal_options();
        let mut tasks = Vec::new();
        let (terminal, error) = match Terminal::spawn(cwd, 80, 24, options) {
            Ok((terminal, events)) => {
                tasks.push(cx.spawn(async move |this, cx| {
                    while let Ok(event) = events.recv().await {
                        let alive = this
                            .update(cx, |this, cx| this.handle_event(event, cx))
                            .is_ok();
                        if !alive {
                            break;
                        }
                    }
                }));
                (Some(Arc::new(terminal)), None)
            }
            Err(err) => (None, Some(format!("Could not start a shell: {err:#}"))),
        };
        tasks.push(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                let alive = this
                    .update(cx, |this, cx| {
                        if Preferences::global(cx).terminal_cursor_blink || !this.blink_on {
                            this.blink_on = !this.blink_on;
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        }));
        let subscriptions = vec![
            cx.on_focus(&focus, window, |this, _, cx| {
                this.blink_on = true;
                if let Some(terminal) = &this.terminal {
                    terminal.focus_changed(true);
                }
                cx.notify();
            }),
            cx.on_blur(&focus, window, |this, _, cx| {
                if let Some(terminal) = &this.terminal {
                    terminal.focus_changed(false);
                }
                cx.notify();
            }),
            cx.observe_global::<Preferences>(|this, cx| {
                if let Some(terminal) = &this.terminal {
                    terminal.set_options(Preferences::global(cx).terminal_options());
                }
                cx.notify();
            }),
        ];
        Self {
            terminal,
            error,
            focus,
            title: None,
            exited: false,
            marked_text: None,
            layout: Rc::new(Cell::new(None)),
            selecting: false,
            blink_on: true,
            scroll_remainder: 0.,
            _tasks: tasks,
            _subscriptions: subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    pub fn exited(&self) -> bool {
        self.exited
    }

    fn handle_event(&mut self, event: TerminalEvent, cx: &mut Context<Self>) {
        match event {
            TerminalEvent::Wakeup => {}
            TerminalEvent::Title(title) => self.title = Some(title).filter(|t| !t.is_empty()),
            TerminalEvent::ResetTitle => self.title = None,
            TerminalEvent::Bell => {}
            TerminalEvent::ClipboardStore(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            TerminalEvent::Exited => self.exited = true,
        }
        cx.notify();
    }

    fn typed(&mut self, cx: &mut Context<Self>) {
        self.blink_on = true;
        self.scroll_remainder = 0.;
        cx.notify();
    }

    fn copy_selection(&self, cx: &mut Context<Self>) -> bool {
        let Some(text) = self.terminal.as_ref().and_then(|t| t.selection_text()) else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(terminal) = self.terminal.clone() else {
            return;
        };
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if modifiers.platform {
            let handled = match keystroke.key.as_str() {
                "c" if !modifiers.shift => self.copy_selection(cx),
                "v" if !modifiers.shift => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        terminal.paste(&text);
                    }
                    true
                }
                "k" => {
                    terminal.clear();
                    true
                }
                "a" => {
                    terminal.select_all();
                    true
                }
                // macOS line editing conventions.
                "left" => {
                    terminal.write(b"\x01".to_vec());
                    true
                }
                "right" => {
                    terminal.write(b"\x05".to_vec());
                    true
                }
                "backspace" => {
                    terminal.write(b"\x15".to_vec());
                    true
                }
                "up" => {
                    terminal.scroll_page(true);
                    true
                }
                "down" => {
                    terminal.scroll_page(false);
                    true
                }
                _ => false,
            };
            if handled {
                self.typed(cx);
                cx.stop_propagation();
            }
            // Other ⌘ shortcuts belong to the application.
            return;
        }
        if modifiers.shift && !modifiers.control && !modifiers.alt {
            match keystroke.key.as_str() {
                "pageup" | "pagedown" if !terminal.mode().alt_screen() => {
                    terminal.scroll_page(keystroke.key == "pageup");
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                _ => {}
            }
        }
        let mode = terminal.mode();
        let input = KeyInput {
            key: &keystroke.key,
            key_char: keystroke.key_char.as_deref(),
            ctrl: modifiers.control,
            alt: modifiers.alt,
            shift: modifiers.shift,
            app_cursor: mode.app_cursor(),
            option_as_meta: Preferences::global(cx).terminal_option_as_meta,
            alt_screen: mode.alt_screen(),
        };
        if let Some(bytes) = key_to_bytes(&input) {
            terminal.write(bytes);
            self.typed(cx);
            cx.stop_propagation();
        }
        // Printable text continues to the platform input handler (IME).
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus(window, cx);
        let (Some(terminal), Some(layout)) = (self.terminal.clone(), self.layout.get()) else {
            return;
        };
        let pos = layout.grid_pos(event.position);
        let mods = mouse_mods(&event.modifiers);
        if event.button == MouseButton::Left && event.modifiers.platform {
            if let Some(url) = terminal.link_at(pos) {
                cx.open_url(&url);
            }
            return;
        }
        if let Some(button) = term_button(event.button)
            && terminal.mouse_button(button, true, pos, mods)
        {
            return;
        }
        if event.button == MouseButton::Left {
            let kind = match event.click_count {
                2 => SelectionKind::Word,
                n if n >= 3 => SelectionKind::Line,
                _ if event.modifiers.alt => SelectionKind::Block,
                _ => SelectionKind::Simple,
            };
            terminal.start_selection(kind, pos);
            self.selecting = true;
            cx.notify();
        } else if event.button == MouseButton::Middle
            && let Some(text) = terminal.selection_text()
        {
            terminal.paste(&text);
        }
    }

    fn finish_selection(&mut self, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        self.selecting = false;
        let Some(terminal) = &self.terminal else {
            return;
        };
        if !terminal.has_selection() {
            terminal.clear_selection();
        } else if Preferences::global(cx).terminal_copy_on_select {
            self.copy_selection(cx);
        }
        cx.notify();
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let (Some(terminal), Some(layout)) = (self.terminal.clone(), self.layout.get()) {
            let pos = layout.grid_pos(event.position);
            if let Some(button) = term_button(event.button)
                && !self.selecting
            {
                terminal.mouse_button(button, false, pos, mouse_mods(&event.modifiers));
            }
        }
        if event.button == MouseButton::Left {
            self.finish_selection(cx);
        }
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (Some(terminal), Some(layout)) = (self.terminal.clone(), self.layout.get()) else {
            return;
        };
        let pos = layout.grid_pos(event.position);
        if !self.selecting && terminal.mouse_move(pos, mouse_mods(&event.modifiers)) {
            return;
        }
        if self.selecting && event.pressed_button == Some(MouseButton::Left) {
            terminal.update_selection(pos);
            cx.notify();
        }
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (Some(terminal), Some(layout)) = (self.terminal.clone(), self.layout.get()) else {
            return;
        };
        let delta = event.delta.pixel_delta(layout.line_height);
        self.scroll_remainder += delta.y / layout.line_height;
        let lines = self.scroll_remainder.trunc() as i32;
        if lines != 0 {
            self.scroll_remainder -= lines as f32;
            terminal.scroll_wheel(
                lines,
                layout.grid_pos(event.position),
                mouse_mods(&event.modifiers),
            );
            cx.notify();
        }
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_text
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_text = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = None;
        if let Some(terminal) = &self.terminal
            && !text.is_empty()
        {
            terminal.write(text.as_bytes().to_vec());
        }
        self.typed(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = Some(new_text.to_string()).filter(|text| !text.is_empty());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.layout.get().and_then(|layout| layout.cursor)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

struct Metrics {
    cell_width: Pixels,
    line_height: Pixels,
    font_size: Pixels,
    font: Font,
}

struct PaintState {
    focused: bool,
    blink_on: bool,
    palette: Palette,
    selection: Hsla,
    marked_text: Option<String>,
    marked_bg: Hsla,
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let prefs = Preferences::global(cx).clone();
        let palette = themes::terminal_palette(&prefs.theme);
        let focused = self.focus.is_focused(window);
        let base = div()
            .id("terminal")
            .track_focus(&self.focus)
            .key_context("Terminal")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(color(palette.background))
            .text_color(color(palette.foreground))
            .cursor(CursorStyle::IBeam)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_selection(cx)),
            )
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_scroll_wheel(cx.listener(Self::on_scroll));

        let Some(terminal) = self.terminal.clone() else {
            return base
                .p_4()
                .child(self.error.clone().unwrap_or_default())
                .into_any_element();
        };
        let font = Font {
            family: prefs.terminal_font_family().to_string().into(),
            ..Font::default()
        };
        let font_size = px(prefs.terminal_font_size);
        let line_height = px((prefs.terminal_font_size * prefs.terminal_line_height).round());
        let state = PaintState {
            focused,
            blink_on: self.blink_on,
            palette,
            selection: cx.theme().selection.opacity(0.55),
            marked_text: self.marked_text.clone(),
            marked_bg: cx.theme().muted,
        };
        let layout_slot = self.layout.clone();
        let entity = cx.entity();
        let focus = self.focus.clone();
        let exited = self.exited;

        base.child(
            canvas(
                {
                    let terminal = terminal.clone();
                    let layout_slot = layout_slot.clone();
                    move |bounds, window, _cx| {
                        let text_system = window.text_system();
                        let font_id = text_system.resolve_font(&font);
                        let cell_width = text_system
                            .advance(font_id, font_size, 'm')
                            .map(|size| size.width)
                            .unwrap_or(font_size * 0.6);
                        let inner_w = bounds.size.width - px(PADDING * 2.);
                        let inner_h = bounds.size.height - px(PADDING * 2.);
                        let columns = (inner_w / cell_width).floor().max(2.) as usize;
                        let lines = (inner_h / line_height).floor().max(1.) as usize;
                        terminal.resize(
                            columns,
                            lines,
                            f32::from(cell_width) as u16,
                            f32::from(line_height) as u16,
                        );
                        let snapshot = terminal.snapshot(&state.palette);
                        let origin = bounds.origin + point(px(PADDING), px(PADDING));
                        let cursor = snapshot.cursor.map(|cursor| {
                            Bounds::new(
                                point(
                                    origin.x + cell_width * cursor.column as f32,
                                    origin.y + line_height * cursor.line as f32,
                                ),
                                size(cell_width, line_height),
                            )
                        });
                        layout_slot.set(Some(Layout {
                            origin,
                            cell_width,
                            line_height,
                            columns: snapshot.columns,
                            lines: snapshot.screen_lines,
                            cursor,
                        }));
                        (
                            snapshot,
                            Metrics {
                                cell_width,
                                line_height,
                                font_size,
                                font,
                            },
                            state,
                        )
                    }
                },
                move |bounds, (snapshot, metrics, state), window, cx| {
                    window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
                    paint_snapshot(bounds, &snapshot, &metrics, &state, window, cx);
                },
            )
            .size_full(),
        )
        .when(exited, |this| {
            this.child(
                div()
                    .absolute()
                    .bottom_2()
                    .right_3()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Process exited"),
            )
        })
        .into_any_element()
    }
}

fn paint_snapshot(
    bounds: Bounds<Pixels>,
    snapshot: &Snapshot,
    metrics: &Metrics,
    state: &PaintState,
    window: &mut Window,
    cx: &mut App,
) {
    let origin = bounds.origin + point(px(PADDING), px(PADDING));
    let (cell_w, line_h) = (metrics.cell_width, metrics.line_height);
    let background = state.palette.background;

    for (row, runs) in snapshot.lines.iter().enumerate() {
        let y = origin.y + line_h * row as f32;
        for run in runs {
            let x = origin.x + cell_w * run.column as f32;
            let run_bounds = Bounds::new(point(x, y), size(cell_w * run.cells as f32, line_h));
            if run.selected {
                window.paint_quad(fill(run_bounds, state.selection));
            } else if let Some(bg) = run.bg.filter(|bg| *bg != background) {
                window.paint_quad(fill(run_bounds, color(bg)));
            }
            if run.text.trim().is_empty() && !run.underline && !run.strikeout {
                continue;
            }
            let mut font = metrics.font.clone();
            if run.bold {
                font.weight = FontWeight::BOLD;
            }
            if run.italic {
                font.style = FontStyle::Italic;
            }
            let mut fg = color(run.fg);
            if run.dim {
                fg = fg.opacity(0.6);
            }
            let text_run = TextRun {
                len: run.text.len(),
                font,
                color: fg,
                background_color: None,
                underline: run.underline.then_some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(fg),
                    wavy: false,
                }),
                strikethrough: run.strikeout.then_some(StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(fg),
                }),
            };
            let shaped = window.text_system().shape_line(
                run.text.clone().into(),
                metrics.font_size,
                &[text_run],
                (!run.wide).then_some(cell_w),
            );
            let _ = shaped.paint(point(x, y), line_h, TextAlign::Left, None, window, cx);
        }
    }

    let Some(cursor) = snapshot.cursor else {
        return;
    };
    let width = if cursor.wide { cell_w * 2. } else { cell_w };
    let cursor_bounds = Bounds::new(
        point(
            origin.x + cell_w * cursor.column as f32,
            origin.y + line_h * cursor.line as f32,
        ),
        size(width, line_h),
    );
    let cursor_color = color(state.palette.foreground);

    if let Some(marked) = &state.marked_text {
        let shaped = window.text_system().shape_line(
            marked.clone().into(),
            metrics.font_size,
            &[TextRun {
                len: marked.len(),
                font: metrics.font.clone(),
                color: cursor_color,
                background_color: None,
                underline: Some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(cursor_color),
                    wavy: false,
                }),
                strikethrough: None,
            }],
            None,
        );
        let marked_bounds = Bounds::new(cursor_bounds.origin, size(shaped.width, line_h));
        window.paint_quad(fill(marked_bounds, state.marked_bg));
        let _ = shaped.paint(
            cursor_bounds.origin,
            line_h,
            TextAlign::Left,
            None,
            window,
            cx,
        );
        return;
    }

    if !state.focused {
        window.paint_quad(outline(
            cursor_bounds,
            cursor_color.opacity(0.6),
            BorderStyle::Solid,
        ));
        return;
    }
    if cursor.blinking && !state.blink_on {
        return;
    }
    match cursor.shape {
        CursorShape::Block => {
            window.paint_quad(fill(cursor_bounds, cursor_color));
            // Redraw the character under the cursor in the background color.
            if let Some(ch) = char_at(snapshot, cursor.line, cursor.column) {
                let text = ch.to_string();
                let shaped = window.text_system().shape_line(
                    text.clone().into(),
                    metrics.font_size,
                    &[TextRun {
                        len: text.len(),
                        font: metrics.font.clone(),
                        color: color(background),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                );
                let _ = shaped.paint(
                    cursor_bounds.origin,
                    line_h,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        }
        CursorShape::Beam => window.paint_quad(fill(
            Bounds::new(cursor_bounds.origin, size(px(2.), line_h)),
            cursor_color,
        )),
        CursorShape::Underline => window.paint_quad(fill(
            Bounds::new(
                point(
                    cursor_bounds.origin.x,
                    cursor_bounds.origin.y + line_h - px(2.),
                ),
                size(width, px(2.)),
            ),
            cursor_color,
        )),
        CursorShape::HollowBlock => {
            window.paint_quad(outline(cursor_bounds, cursor_color, BorderStyle::Solid))
        }
        CursorShape::Hidden => {}
    }
}

fn char_at(snapshot: &Snapshot, line: usize, column: usize) -> Option<char> {
    let run = snapshot
        .lines
        .get(line)?
        .iter()
        .find(|run| column >= run.column && column < run.column + run.cells)?;
    if run.wide {
        return run.text.chars().next();
    }
    run.text
        .chars()
        .nth(column - run.column)
        .filter(|c| *c != ' ')
}

/// The terminals belonging to one thread, shown as tabs.
pub struct TerminalPanel {
    cwd: PathBuf,
    terminals: Vec<Entity<TerminalView>>,
    active: usize,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl TerminalPanel {
    pub fn new(cwd: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut panel = Self {
            cwd,
            terminals: Vec::new(),
            active: 0,
            focus: cx.focus_handle(),
            _subscriptions: Vec::new(),
        };
        panel.add(window, cx);
        panel
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.cwd.clone();
        let view = cx.new(|cx| TerminalView::new(&cwd, window, cx));
        self._subscriptions
            .push(cx.observe(&view, |_, _, cx| cx.notify()));
        self.terminals.push(view);
        self.active = self.terminals.len() - 1;
        self.focus_active(window, cx);
        cx.notify();
    }

    fn close(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.terminals.len() {
            return;
        }
        self.terminals.remove(index);
        if self.terminals.is_empty() {
            self.add(window, cx);
            return;
        }
        self.active = self.active.min(self.terminals.len() - 1);
        self.focus_active(window, cx);
        cx.notify();
    }

    fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.terminals.len() {
            self.active = index;
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    pub fn focus_active(&self, window: &mut Window, cx: &mut App) {
        if let Some(view) = self.terminals.get(self.active) {
            view.update(cx, |view, cx| view.focus(window, cx));
        }
    }

    fn on_new(&mut self, _: &NewTerminal, window: &mut Window, cx: &mut Context<Self>) {
        self.add(window, cx);
    }

    fn on_close(&mut self, _: &CloseTerminal, window: &mut Window, cx: &mut Context<Self>) {
        self.close(self.active, window, cx);
    }

    fn on_next(&mut self, _: &NextTerminal, window: &mut Window, cx: &mut Context<Self>) {
        let next = (self.active + 1) % self.terminals.len().max(1);
        self.select(next, window, cx);
    }

    fn on_previous(&mut self, _: &PreviousTerminal, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.terminals.len().max(1);
        self.select((self.active + len - 1) % len, window, cx);
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = self.terminals.iter().enumerate().map(|(index, view)| {
            let view = view.read(cx);
            let label = view
                .title
                .clone()
                .unwrap_or_else(|| format!("Terminal {}", index + 1));
            let selected = index == self.active;
            h_flex()
                .id(("terminal-tab", index))
                .flex_none()
                .max_w(px(180.))
                .h(px(26.))
                .pl_2()
                .pr_0p5()
                .gap_1()
                .rounded_md()
                .cursor_pointer()
                .text_xs()
                .when(selected, |this| this.bg(cx.theme().secondary))
                .when(!selected, |this| {
                    this.text_color(cx.theme().muted_foreground)
                })
                .hover(|this| this.bg(cx.theme().secondary.opacity(0.7)))
                .child(
                    div()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .when(view.exited(), |this| this.line_through())
                        .child(label),
                )
                .child(
                    Button::new(("close-terminal", index))
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close(index, window, cx);
                        })),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.select(index, window, cx)))
        });
        let active = self.terminals.get(self.active).cloned();

        v_flex()
            .key_context("TerminalPanel")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_new))
            .on_action(cx.listener(Self::on_close))
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_previous))
            .size_full()
            .child(
                h_flex()
                    .id("terminal-tabs")
                    .px_2()
                    .py_1()
                    .gap_1()
                    .overflow_x_scroll()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .children(tabs)
                    .child(
                        Button::new("new-terminal")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .tooltip("New terminal (⌘T)")
                            .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().children(active))
    }
}

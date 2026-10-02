//! A real PTY-backed terminal: alacritty_terminal provides the shell
//! process, VT parsing and grid; this crate exposes a UI-agnostic snapshot,
//! input, mouse and selection API complete enough for full-screen TUIs.

mod keys;
mod mouse;
mod palette;

pub use keys::{KeyInput, key_to_bytes};
pub use mouse::{MouseButton, MouseModifiers};
pub use palette::{Palette, Rgb};

use alacritty_terminal::event::{Event as AlacEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, CursorShape as AlacShape, CursorStyle, NamedColor};
use anyhow::Result;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub enum TerminalEvent {
    /// New output is available; re-render.
    Wakeup,
    Title(String),
    ResetTitle,
    Bell,
    /// The application asked to copy text to the clipboard (OSC 52).
    ClipboardStore(String),
    Exited,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Block,
    Beam,
    Underline,
    HollowBlock,
    Hidden,
}

impl CursorShape {
    fn to_alacritty(self) -> AlacShape {
        match self {
            CursorShape::Block => AlacShape::Block,
            CursorShape::Beam => AlacShape::Beam,
            CursorShape::Underline => AlacShape::Underline,
            CursorShape::HollowBlock => AlacShape::HollowBlock,
            CursorShape::Hidden => AlacShape::Hidden,
        }
    }

    fn from_alacritty(shape: AlacShape) -> Self {
        match shape {
            AlacShape::Block => CursorShape::Block,
            AlacShape::Beam => CursorShape::Beam,
            AlacShape::Underline => CursorShape::Underline,
            AlacShape::HollowBlock => CursorShape::HollowBlock,
            AlacShape::Hidden => CursorShape::Hidden,
        }
    }
}

/// User-configurable terminal behavior.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalOptions {
    /// Shell program; `None` uses the user's login shell.
    pub shell: Option<String>,
    pub scrollback: usize,
    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
}

impl Default for TerminalOptions {
    fn default() -> Self {
        Self {
            shell: None,
            scrollback: 10_000,
            cursor_shape: CursorShape::Block,
            cursor_blink: false,
        }
    }
}

impl TerminalOptions {
    fn term_config(&self) -> Config {
        Config {
            scrolling_history: self.scrollback,
            default_cursor_style: CursorStyle {
                shape: self.cursor_shape.to_alacritty(),
                blinking: self.cursor_blink,
            },
            ..Config::default()
        }
    }
}

#[derive(Clone)]
struct Listener(
    async_channel::Sender<TerminalEvent>,
    Arc<FairMutex<Option<EventLoopSender>>>,
);

impl Listener {
    fn write(&self, bytes: Vec<u8>) {
        if let Some(sender) = self.1.lock().as_ref() {
            let _ = sender.send(Msg::Input(Cow::Owned(bytes)));
        }
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: AlacEvent) {
        let event = match event {
            AlacEvent::Wakeup => TerminalEvent::Wakeup,
            AlacEvent::Title(title) => TerminalEvent::Title(title),
            AlacEvent::ResetTitle => TerminalEvent::ResetTitle,
            AlacEvent::Bell => TerminalEvent::Bell,
            AlacEvent::ClipboardStore(_, text) => TerminalEvent::ClipboardStore(text),
            AlacEvent::Exit | AlacEvent::ChildExit(_) => TerminalEvent::Exited,
            // Replies the terminal must write back to the PTY (DSR, DA, ...).
            AlacEvent::PtyWrite(text) => {
                self.write(text.into_bytes());
                return;
            }
            // Size reports (CSI 14/16/18 t) are answered with the cell grid.
            AlacEvent::TextAreaSizeRequest(format) => {
                // The size is filled in by the event loop's last resize.
                let size = *LAST_SIZE.lock().unwrap();
                self.write(format(size).into_bytes());
                return;
            }
            _ => return,
        };
        let _ = self.0.try_send(event);
    }
}

static LAST_SIZE: Mutex<WindowSize> = Mutex::new(WindowSize {
    num_lines: 24,
    num_cols: 80,
    cell_width: 8,
    cell_height: 16,
});

#[derive(Clone, Copy, Debug)]
struct Size {
    columns: usize,
    lines: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

/// A run of cells sharing one style, positioned on the cell grid.
#[derive(Clone, Debug, PartialEq)]
pub struct StyledRun {
    /// First grid column of the run.
    pub column: usize,
    /// Number of grid cells the run covers (wide characters cover two).
    pub cells: usize,
    pub text: String,
    pub fg: Rgb,
    /// `None` when the cell uses the default background.
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub dim: bool,
    pub underline: bool,
    pub strikeout: bool,
    /// The run is a single double-width character.
    pub wide: bool,
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub line: usize,
    pub column: usize,
    pub shape: CursorShape,
    pub blinking: bool,
    /// The cursor sits on a double-width character.
    pub wide: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub lines: Vec<Vec<StyledRun>>,
    pub cursor: Option<Cursor>,
    pub columns: usize,
    pub screen_lines: usize,
    pub display_offset: usize,
    pub alt_screen: bool,
    pub has_selection: bool,
}

/// A position on the visible screen: `line` 0 is the top visible row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridPos {
    pub line: usize,
    pub column: usize,
    /// Which half of the cell the pointer is over.
    pub right_half: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    /// Character-wise (single click and drag).
    Simple,
    /// Word-wise (double click).
    Word,
    /// Line-wise (triple click).
    Line,
    /// Rectangular (alt + drag).
    Block,
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    listener: Listener,
    sender: EventLoopSender,
    size: Mutex<Size>,
    options: Mutex<TerminalOptions>,
    /// Pressed mouse button, for drag reporting.
    pressed: Mutex<Option<MouseButton>>,
}

impl Terminal {
    pub fn spawn(
        cwd: &Path,
        columns: usize,
        lines: usize,
        options: TerminalOptions,
    ) -> Result<(Self, async_channel::Receiver<TerminalEvent>)> {
        let size = Size {
            columns: columns.max(2),
            lines: lines.max(1),
        };
        let mut env = HashMap::new();
        env.insert("TERM".to_string(), "xterm-256color".to_string());
        env.insert("COLORTERM".to_string(), "truecolor".to_string());
        env.insert("TERM_PROGRAM".to_string(), "ElyraWorkspace".to_string());
        env.insert(
            "TERM_PROGRAM_VERSION".to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        );
        let shell = options
            .shell
            .as_deref()
            .map(str::trim)
            .filter(|shell| !shell.is_empty())
            .map(|shell| {
                let mut parts = shell.split_whitespace().map(str::to_string);
                let program = parts.next().unwrap_or_default();
                tty::Shell::new(program, parts.collect())
            });
        let pty_options = tty::Options {
            shell,
            working_directory: Some(cwd.to_path_buf()),
            drain_on_exit: true,
            env,
            #[cfg(target_os = "windows")]
            escape_args: true,
        };
        let window_size = window_size(size, 8, 16);
        let pty = tty::new(&pty_options, window_size, 0)?;

        let (tx, rx) = async_channel::unbounded();
        let sender_slot = Arc::new(FairMutex::new(None));
        let listener = Listener(tx, sender_slot.clone());
        let term = Arc::new(FairMutex::new(Term::new(
            options.term_config(),
            &size,
            listener.clone(),
        )));
        let event_loop = EventLoop::new(term.clone(), listener.clone(), pty, true, false)?;
        let sender = event_loop.channel();
        *sender_slot.lock() = Some(sender.clone());
        event_loop.spawn();
        Ok((
            Self {
                term,
                listener,
                sender,
                size: Mutex::new(size),
                options: Mutex::new(options),
                pressed: Mutex::new(None),
            },
            rx,
        ))
    }

    fn send(&self, bytes: impl Into<Vec<u8>>) {
        let bytes = bytes.into();
        let _ = self.sender.send(Msg::Input(Cow::Owned(bytes)));
    }

    /// Write user input to the PTY. Typing snaps the view to the live screen
    /// and clears any selection.
    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        {
            let mut term = self.term.lock();
            term.scroll_display(Scroll::Bottom);
            term.selection = None;
        }
        self.send(bytes);
    }

    pub fn set_options(&self, options: TerminalOptions) {
        let mut current = self.options.lock().unwrap();
        if *current != options {
            self.term.lock().set_options(options.term_config());
            *current = options;
        }
    }

    pub fn resize(&self, columns: usize, lines: usize, cell_width: u16, cell_height: u16) {
        let size = Size {
            columns: columns.max(2),
            lines: lines.max(1),
        };
        {
            let mut current = self.size.lock().unwrap();
            if size.columns == current.columns && size.lines == current.lines {
                return;
            }
            *current = size;
        }
        let window = window_size(size, cell_width, cell_height);
        *LAST_SIZE.lock().unwrap() = window;
        self.term.lock().resize(size);
        let _ = self.sender.send(Msg::Resize(window));
    }

    pub fn size(&self) -> (usize, usize) {
        let size = self.size.lock().unwrap();
        (size.columns, size.lines)
    }

    pub fn mode(&self) -> TermModeFlags {
        TermModeFlags(*self.term.lock().mode())
    }

    pub fn app_cursor_mode(&self) -> bool {
        self.mode().app_cursor()
    }

    pub fn paste(&self, text: &str) {
        if self.mode().bracketed_paste() {
            // Strip ESC so pasted text cannot terminate bracketed paste early.
            self.write(format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', "")));
        } else {
            self.write(text.replace("\r\n", "\r").replace('\n', "\r"));
        }
    }

    /// Report focus changes to applications that asked for them (CSI ? 1004 h).
    pub fn focus_changed(&self, focused: bool) {
        if self.mode().0.contains(TermMode::FOCUS_IN_OUT) {
            self.send(if focused {
                &b"\x1b[I"[..]
            } else {
                &b"\x1b[O"[..]
            });
        }
    }

    /// Clear scrollback and screen (⌘K), keeping the shell running.
    pub fn clear(&self) {
        {
            let mut term = self.term.lock();
            term.selection = None;
            term.grid_mut().clear_history();
        }
        // Form feed makes the shell redraw its prompt on a clean screen.
        self.write(b"\x0c".to_vec());
    }

    // ---- scrolling ------------------------------------------------------

    /// Scroll by `lines` (positive = up/back in history) the way the
    /// application expects: mouse reports when it tracks the mouse, arrow keys
    /// in the alternate screen, otherwise the scrollback.
    pub fn scroll_wheel(&self, lines: i32, pos: GridPos, mods: MouseModifiers) {
        if lines == 0 {
            return;
        }
        let mode = self.mode();
        if mode.mouse_mode() && !mods.shift {
            let button = if lines > 0 {
                MouseButton::WheelUp
            } else {
                MouseButton::WheelDown
            };
            for _ in 0..lines.unsigned_abs().min(10) {
                if let Some(bytes) = mouse::encode(button, true, false, pos, mods, mode) {
                    self.send(bytes);
                }
            }
        } else if mode.alt_screen() && mode.0.contains(TermMode::ALTERNATE_SCROLL) && !mods.shift {
            let key = match (lines > 0, mode.app_cursor()) {
                (true, true) => "\x1bOA",
                (true, false) => "\x1b[A",
                (false, true) => "\x1bOB",
                (false, false) => "\x1b[B",
            };
            self.send(key.repeat(lines.unsigned_abs() as usize));
        } else {
            self.term.lock().scroll_display(Scroll::Delta(lines));
        }
    }

    pub fn scroll_to_bottom(&self) {
        self.term.lock().scroll_display(Scroll::Bottom);
    }

    pub fn scroll_page(&self, up: bool) {
        self.term
            .lock()
            .scroll_display(if up { Scroll::PageUp } else { Scroll::PageDown });
    }

    // ---- mouse reporting --------------------------------------------------

    /// Whether a mouse event at this moment should go to the application
    /// instead of driving selection. Shift always forces local selection.
    pub fn wants_mouse(&self, mods: MouseModifiers) -> bool {
        self.mode().mouse_mode() && !mods.shift
    }

    /// Report a button press or release. Returns false when the application
    /// does not track the mouse.
    pub fn mouse_button(
        &self,
        button: MouseButton,
        pressed: bool,
        pos: GridPos,
        mods: MouseModifiers,
    ) -> bool {
        if !self.wants_mouse(mods) {
            return false;
        }
        *self.pressed.lock().unwrap() = pressed.then_some(button);
        if let Some(bytes) = mouse::encode(button, pressed, false, pos, mods, self.mode()) {
            self.send(bytes);
        }
        true
    }

    /// Report pointer motion. Sent only in drag (1002) or any-motion (1003)
    /// tracking modes.
    pub fn mouse_move(&self, pos: GridPos, mods: MouseModifiers) -> bool {
        if !self.wants_mouse(mods) {
            return false;
        }
        let mode = self.mode();
        let pressed = *self.pressed.lock().unwrap();
        let report = match pressed {
            Some(_) => mode
                .0
                .intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION),
            None => mode.0.contains(TermMode::MOUSE_MOTION),
        };
        if report {
            let button = pressed.unwrap_or(MouseButton::None);
            if let Some(bytes) = mouse::encode(button, true, true, pos, mods, mode) {
                self.send(bytes);
            }
        }
        true
    }

    // ---- selection --------------------------------------------------------

    fn point(&self, term: &Term<Listener>, pos: GridPos) -> (Point, Side) {
        let offset = term.grid().display_offset() as i32;
        let column = pos.column.min(term.columns().saturating_sub(1));
        let line = (pos.line.min(term.screen_lines().saturating_sub(1))) as i32 - offset;
        (
            Point::new(Line(line), Column(column)),
            if pos.right_half {
                Side::Right
            } else {
                Side::Left
            },
        )
    }

    pub fn start_selection(&self, kind: SelectionKind, pos: GridPos) {
        let mut term = self.term.lock();
        let (point, side) = self.point(&term, pos);
        let ty = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Word => SelectionType::Semantic,
            SelectionKind::Line => SelectionType::Lines,
            SelectionKind::Block => SelectionType::Block,
        };
        term.selection = Some(Selection::new(ty, point, side));
    }

    pub fn update_selection(&self, pos: GridPos) {
        let mut term = self.term.lock();
        let (point, side) = self.point(&term, pos);
        if let Some(selection) = term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    pub fn clear_selection(&self) {
        self.term.lock().selection = None;
    }

    pub fn select_all(&self) {
        let mut term = self.term.lock();
        let top = term.grid().topmost_line();
        let bottom = term.grid().bottommost_line();
        let last = term.grid().last_column();
        let mut selection = Selection::new(
            SelectionType::Simple,
            Point::new(top, Column(0)),
            Side::Left,
        );
        selection.update(Point::new(bottom, last), Side::Right);
        term.selection = Some(selection);
    }

    pub fn has_selection(&self) -> bool {
        self.term
            .lock()
            .selection
            .as_ref()
            .is_some_and(|selection| !selection.is_empty())
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term
            .lock()
            .selection_to_string()
            .filter(|text| !text.is_empty())
    }

    // ---- links ------------------------------------------------------------

    /// The OSC 8 hyperlink or plain-text URL under the given position.
    pub fn link_at(&self, pos: GridPos) -> Option<String> {
        let term = self.term.lock();
        let (point, _) = self.point(&term, pos);
        if let Some(link) = term.grid()[point].hyperlink() {
            return Some(link.uri().to_string());
        }
        let row = &term.grid()[point.line];
        let cells: Vec<(usize, char)> = (0..term.columns())
            .filter(|&col| !row[Column(col)].flags.contains(Flags::WIDE_CHAR_SPACER))
            .map(|col| (col, row[Column(col)].c))
            .collect();
        url_at(&cells, point.column.0)
    }

    // ---- rendering --------------------------------------------------------

    pub fn snapshot(&self, palette: &Palette) -> Snapshot {
        let term = self.term.lock();
        let content = term.renderable_content();
        let colors = content.colors;
        let display_offset = content.display_offset;
        let selection = content.selection;
        let columns = term.columns();
        let screen_lines = term.screen_lines();
        let mode = content.mode;
        let mut lines: Vec<Vec<StyledRun>> = vec![Vec::new(); screen_lines];

        for indexed in content.display_iter {
            let cell = &indexed.cell;
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let row = indexed.point.line.0 + display_offset as i32;
            if row < 0 || row as usize >= screen_lines {
                continue;
            }
            let mut fg = palette.resolve(cell.fg, colors, true);
            let mut bg = match cell.bg {
                Color::Named(NamedColor::Background) => None,
                other => Some(palette.resolve(other, colors, false)),
            };
            if cell.flags.contains(Flags::INVERSE) {
                let back = bg.unwrap_or(palette.background);
                bg = Some(fg);
                fg = back;
            }
            if cell.flags.contains(Flags::HIDDEN) {
                fg = bg.unwrap_or(palette.background);
            }
            let wide = cell.flags.contains(Flags::WIDE_CHAR);
            let selected = selection.is_some_and(|range| range.contains(indexed.point));
            let style = StyledRun {
                column: indexed.point.column.0,
                cells: if wide { 2 } else { 1 },
                text: String::new(),
                fg,
                bg,
                bold: cell.flags.contains(Flags::BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
                dim: cell.flags.contains(Flags::DIM),
                underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                strikeout: cell.flags.contains(Flags::STRIKEOUT),
                wide,
                selected,
            };
            let ch = if cell.c == '\0' || cell.c == '\t' {
                ' '
            } else {
                cell.c
            };

            let line = &mut lines[row as usize];
            let extends = line.last().is_some_and(|run| {
                !run.wide
                    && !style.wide
                    && run.column + run.cells == style.column
                    && run.fg == style.fg
                    && run.bg == style.bg
                    && run.bold == style.bold
                    && run.italic == style.italic
                    && run.dim == style.dim
                    && run.underline == style.underline
                    && run.strikeout == style.strikeout
                    && run.selected == style.selected
            });
            if extends {
                let run = line.last_mut().unwrap();
                run.text.push(ch);
                run.cells += 1;
            } else {
                let mut run = style;
                run.text.push(ch);
                line.push(run);
            }
            if let Some(zerowidth) = cell.zerowidth()
                && let Some(run) = line.last_mut()
            {
                run.text.extend(zerowidth);
            }
        }

        let style = term.cursor_style();
        let point = content.cursor.point;
        let shape = CursorShape::from_alacritty(content.cursor.shape);
        let cursor_line = point.line.0 + display_offset as i32;
        let cursor = (mode.contains(TermMode::SHOW_CURSOR)
            && shape != CursorShape::Hidden
            && cursor_line >= 0
            && (cursor_line as usize) < screen_lines)
            .then(|| Cursor {
                line: cursor_line as usize,
                column: point.column.0,
                shape,
                blinking: style.blinking,
                wide: term.grid()[point].flags.contains(Flags::WIDE_CHAR),
            });
        Snapshot {
            lines,
            cursor,
            columns,
            screen_lines,
            display_offset,
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            has_selection: selection.is_some(),
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
        let _ = &self.listener;
    }
}

/// Read-only view of the terminal's mode flags.
#[derive(Clone, Copy, Debug)]
pub struct TermModeFlags(TermMode);

impl TermModeFlags {
    pub fn app_cursor(self) -> bool {
        self.0.contains(TermMode::APP_CURSOR)
    }
    pub fn bracketed_paste(self) -> bool {
        self.0.contains(TermMode::BRACKETED_PASTE)
    }
    pub fn alt_screen(self) -> bool {
        self.0.contains(TermMode::ALT_SCREEN)
    }
    pub fn mouse_mode(self) -> bool {
        self.0.intersects(TermMode::MOUSE_MODE)
    }
    pub(crate) fn sgr_mouse(self) -> bool {
        self.0.contains(TermMode::SGR_MOUSE)
    }
    pub(crate) fn utf8_mouse(self) -> bool {
        self.0.contains(TermMode::UTF8_MOUSE)
    }
}

fn window_size(size: Size, cell_width: u16, cell_height: u16) -> WindowSize {
    WindowSize {
        num_lines: size.lines as u16,
        num_cols: size.columns as u16,
        cell_width,
        cell_height,
    }
}

/// Find a URL spanning `column` in a row of `(column, char)` cells.
fn url_at(cells: &[(usize, char)], column: usize) -> Option<String> {
    const SCHEMES: [&str; 4] = ["https://", "http://", "file://", "mailto:"];
    let text: String = cells.iter().map(|(_, c)| *c).collect();
    let chars: Vec<char> = text.chars().collect();
    let index = cells.iter().position(|(col, _)| *col >= column)?;
    let is_url_char = |c: char| {
        !c.is_whitespace() && !matches!(c, '"' | '\'' | '<' | '>' | '`' | '|' | '{' | '}' | '^')
    };
    if !is_url_char(chars[index]) {
        return None;
    }
    let mut start = index;
    while start > 0 && is_url_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = index + 1;
    while end < chars.len() && is_url_char(chars[end]) {
        end += 1;
    }
    let word: String = chars[start..end].iter().collect();
    let offset = SCHEMES
        .iter()
        .filter_map(|scheme| word.find(scheme))
        .min()?;
    // The scheme must begin at or before the pointer.
    if start + word[..offset].chars().count() > index {
        return None;
    }
    let url = word[offset..].trim_end_matches(['.', ',', ';', ':', ')', ']', '!', '?']);
    Some(url.to_string()).filter(|url| {
        SCHEMES
            .iter()
            .any(|s| url.len() > s.len() && url.starts_with(s))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn screen_text(terminal: &Terminal) -> String {
        terminal
            .snapshot(&Palette::dark())
            .lines
            .iter()
            .map(|line| line.iter().map(|run| run.text.as_str()).collect::<String>() + "\n")
            .collect()
    }

    fn wait_for(terminal: &Terminal, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let text = screen_text(terminal);
            if text.contains(needle) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "missing {needle:?}; screen:\n{text}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn runs_shell_and_renders_output() {
        let (terminal, _events) =
            Terminal::spawn(&std::env::temp_dir(), 80, 24, TerminalOptions::default()).unwrap();
        terminal.write("echo elyra-$((40+2))\r");
        wait_for(&terminal, "elyra-42");
    }

    #[test]
    fn wide_characters_keep_grid_columns() {
        let (terminal, _events) =
            Terminal::spawn(&std::env::temp_dir(), 80, 24, TerminalOptions::default()).unwrap();
        terminal.write("printf '\\xe6\\x97\\xa5\\xe6\\x9c\\xacX\\n'\r");
        wait_for(&terminal, "日本X");
        let snapshot = terminal.snapshot(&Palette::dark());
        let line = snapshot
            .lines
            .iter()
            .find(|line| line.iter().any(|run| run.text == "日"))
            .expect("line with wide chars");
        let x = line.iter().find(|run| run.text.starts_with('X')).unwrap();
        let wide = line.iter().find(|run| run.text == "日").unwrap();
        assert!(wide.wide && wide.cells == 2);
        assert_eq!(
            x.column,
            wide.column + 4,
            "two wide chars take four columns"
        );
    }

    #[test]
    fn selection_and_mouse_modes() {
        let (terminal, _events) =
            Terminal::spawn(&std::env::temp_dir(), 80, 24, TerminalOptions::default()).unwrap();
        terminal.write("clear; echo select-me\r");
        let deadline = Instant::now() + Duration::from_secs(10);
        let row = loop {
            let text = screen_text(&terminal);
            if let Some(row) = text.lines().position(|line| line.starts_with("select-me")) {
                break row;
            }
            assert!(Instant::now() < deadline, "no output; screen:\n{text}");
            std::thread::sleep(Duration::from_millis(50));
        };
        terminal.start_selection(
            SelectionKind::Word,
            GridPos {
                line: row,
                column: 2,
                right_half: false,
            },
        );
        assert_eq!(terminal.selection_text().as_deref(), Some("select-me"));

        assert!(!terminal.mode().mouse_mode());
        // Enable SGR any-motion tracking like vim/htop do.
        terminal.write("printf '\\e[?1003h\\e[?1006h'\r");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !terminal.mode().mouse_mode() {
            assert!(Instant::now() < deadline, "mouse mode not enabled");
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(terminal.wants_mouse(MouseModifiers::default()));
        assert!(!terminal.wants_mouse(MouseModifiers {
            shift: true,
            ..Default::default()
        }));
        terminal.write("printf '\\e[?1003l\\e[?1006l'\r");
    }

    #[test]
    fn finds_urls_under_pointer() {
        let row: Vec<(usize, char)> = "see https://example.com/a?b=1, ok"
            .chars()
            .enumerate()
            .collect();
        assert_eq!(
            url_at(&row, 10).as_deref(),
            Some("https://example.com/a?b=1")
        );
        assert_eq!(url_at(&row, 1), None);
        assert_eq!(url_at(&row, 32), None);
    }
}

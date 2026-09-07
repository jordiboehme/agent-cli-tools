//! Terminal primitives for the full-screen commands: the size of a
//! terminal, the unbuffered input mode curses calls cbreak, and a screen
//! buffer that paints with plain ANSI escapes instead of linking curses.
//!
//! The screen holds two grids of cells, the one being drawn and the one
//! the terminal was last sent, and `flush` walks the difference and emits
//! only the runs that changed, each preceded by an absolute cursor move.
//! That is what keeps a redraw from flickering with no curses window to
//! do it, and it is why nothing here ever writes a newline: every cell
//! lands at a position the writer chose.

use std::io::Write;

/// `_IOR('t', 104, struct winsize)`, the macOS spelling. The libc crate
/// defines no `TIOCGWINSZ` for Apple targets, so it is written out here.
const TIOCGWINSZ: libc::c_ulong = 0x4008_7468;

/// The alternate screen, cleared, with the cursor home, hidden, and
/// automatic margin wrap off so a write into the last cell of the last
/// row cannot scroll the display.
const ENTER: &[u8] = b"\x1b[?1049h\x1b[2J\x1b[H\x1b[?25l\x1b[?7l";

/// The exact undo of `ENTER`, plus a reset of any attribute left over.
const LEAVE: &[u8] = b"\x1b[0m\x1b[?7h\x1b[?25h\x1b[?1049l";

/// A terminal's size in character cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
}

/// The size of the terminal on `fd`. `LINES` and `COLUMNS` override what
/// the kernel reports, as they do for curses, and 24 by 80 stands in when
/// the descriptor is not a terminal at all.
pub fn size(fd: i32) -> Size {
    // SAFETY: TIOCGWINSZ writes one `struct winsize`, which is what the
    // third argument points at. A failure leaves the zeroed value, which
    // the fallbacks below replace.
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::ioctl(fd, TIOCGWINSZ, &raw mut ws) } == 0;
    let mut rows = if ok { ws.ws_row } else { 0 };
    let mut cols = if ok { ws.ws_col } else { 0 };
    if let Some(value) = env_dimension("LINES") {
        rows = value;
    }
    if let Some(value) = env_dimension("COLUMNS") {
        cols = value;
    }
    Size {
        rows: if rows == 0 { 24 } else { rows },
        cols: if cols == 0 { 80 } else { cols },
    }
}

/// A terminal dimension read from the environment: a positive decimal
/// number and nothing else. Anything the C `atoi` would read as zero, an
/// empty value included, leaves the kernel's answer alone.
fn env_dimension(name: &str) -> Option<u16> {
    std::env::var(name)
        .ok()?
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|value| *value > 0)
}

/// A terminal put into cbreak mode for as long as this value lives.
pub struct Raw {
    fd: i32,
    saved: libc::termios,
}

impl Raw {
    /// Characters arrive one at a time and unechoed, while `ISIG` stays
    /// on so Ctrl-C still raises SIGINT the way it does under curses'
    /// cbreak. `None` when `fd` is not a terminal, which leaves the
    /// caller running without keyboard input rather than failing.
    pub fn enable(fd: i32) -> Option<Raw> {
        // SAFETY: tcgetattr fills the one termios it is handed, and only
        // on success, which is what the return value is checked for.
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &raw mut saved) } != 0 {
            return None;
        }
        let mut mode = saved;
        mode.c_lflag &= !(libc::ICANON | libc::ECHO);
        mode.c_cc[libc::VMIN] = 1;
        mode.c_cc[libc::VTIME] = 0;
        // SAFETY: `mode` is a termios read from this same descriptor with
        // two flags cleared, so every other field is one the driver gave.
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw const mode) } != 0 {
            return None;
        }
        Some(Raw { fd, saved })
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        // SAFETY: `saved` is the termios this descriptor carried before
        // `enable` changed it, so restoring it is always valid.
        unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &raw const self.saved) };
    }
}

/// A character's colour: the terminal's own default, or one of the 256
/// palette entries (0-7 the base colours, 8-15 their bright forms,
/// 16-255 the colour cube and the greys).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
}

/// Everything a cell carries besides its character.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attrs {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub blink: bool,
    pub reverse: bool,
    pub fg: Color,
    pub bg: Color,
}

impl Attrs {
    /// The SGR sequence that turns a terminal in any state into this one.
    /// It always starts from a reset, so no cell can inherit an attribute
    /// the writer never asked for.
    fn sgr(&self) -> String {
        let mut text = String::with_capacity(16);
        text.push_str("\x1b[0");
        for (on, code) in [
            (self.bold, "1"),
            (self.dim, "2"),
            (self.italic, "3"),
            (self.underline, "4"),
            (self.blink, "5"),
            (self.reverse, "7"),
        ] {
            if on {
                text.push(';');
                text.push_str(code);
            }
        }
        push_color(&mut text, self.fg, 30);
        push_color(&mut text, self.bg, 40);
        text.push('m');
        text
    }
}

/// One colour's parameters, given the base of its plane: 30 for the
/// foreground, 40 for the background. The default needs nothing, since
/// the sequence already began with a reset.
fn push_color(text: &mut String, color: Color, base: u8) {
    let Color::Indexed(index) = color else {
        return;
    };
    match index {
        0..=7 => text.push_str(&format!(";{}", base + index)),
        8..=15 => text.push_str(&format!(";{}", base + 60 + index - 8)),
        _ => text.push_str(&format!(";{};5;{index}", base + 8)),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Cell {
    ch: char,
    attrs: Attrs,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            ch: ' ',
            attrs: Attrs::default(),
        }
    }
}

/// A grid of cells and the terminal it paints.
pub struct Screen<W: Write> {
    out: W,
    size: Size,
    /// The frame being composed.
    cells: Vec<Cell>,
    /// The frame the terminal is already showing, which is what `flush`
    /// subtracts to find the runs it has to write.
    shown: Vec<Cell>,
    /// Bytes waiting for the next `flush`, so anything written straight
    /// through stays in order with the cells around it.
    pending: Vec<u8>,
}

impl<W: Write> Screen<W> {
    pub fn new(out: W, size: Size) -> Self {
        let count = usize::from(size.rows) * usize::from(size.cols);
        Screen {
            out,
            size,
            cells: vec![Cell::default(); count],
            shown: vec![Cell::default(); count],
            pending: Vec::new(),
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// Switch to the alternate screen and clear it. Both grids start
    /// blank, which is true of the terminal too after the clear, so the
    /// first flush writes only the cells that are not spaces.
    pub fn enter(&mut self) {
        self.pending.extend_from_slice(ENTER);
        self.blank();
        let _ = self.out.write_all(&std::mem::take(&mut self.pending));
        let _ = self.out.flush();
    }

    /// Undo `enter`, leaving the terminal exactly as it was found.
    pub fn leave(&mut self) {
        self.pending.extend_from_slice(LEAVE);
        let _ = self.out.write_all(&std::mem::take(&mut self.pending));
        let _ = self.out.flush();
    }

    /// Adopt a new terminal size. The old contents cannot be mapped onto
    /// the new grid in any way a reader would recognize, so the display
    /// is cleared and both grids start blank again.
    pub fn resize(&mut self, size: Size) {
        self.size = size;
        let count = usize::from(size.rows) * usize::from(size.cols);
        self.cells = vec![Cell::default(); count];
        self.shown = vec![Cell::default(); count];
        self.pending.extend_from_slice(b"\x1b[2J\x1b[H");
    }

    fn blank(&mut self) {
        for cell in self.cells.iter_mut().chain(self.shown.iter_mut()) {
            *cell = Cell::default();
        }
    }

    fn index(&self, row: usize, col: usize) -> Option<usize> {
        if row >= usize::from(self.size.rows) || col >= usize::from(self.size.cols) {
            return None;
        }
        Some(row * usize::from(self.size.cols) + col)
    }

    /// Write one character into the frame being composed. A position off
    /// the grid is dropped, which lets a caller render without bounds
    /// checks of its own.
    pub fn put(&mut self, row: usize, col: usize, ch: char, attrs: Attrs) {
        if let Some(at) = self.index(row, col) {
            self.cells[at] = Cell { ch, attrs };
        }
    }

    /// Blank `row` from `col` to its end.
    pub fn clear_to_end_of_row(&mut self, row: usize, col: usize) {
        for col in col..usize::from(self.size.cols) {
            self.put(row, col, ' ', Attrs::default());
        }
    }

    /// The characters of one row of the frame being composed, which is
    /// what a caller compares against to see whether a redraw changed
    /// anything a reader would notice.
    pub fn row_chars(&self, row: usize) -> Vec<char> {
        let cols = usize::from(self.size.cols);
        match self.index(row, 0) {
            Some(at) => self.cells[at..at + cols].iter().map(|c| c.ch).collect(),
            None => Vec::new(),
        }
    }

    /// Send bytes to the terminal untouched, ordered with the frame they
    /// were written between. The bell is the one thing that has to reach
    /// the terminal without occupying a cell.
    pub fn write_raw(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
    }

    /// Write every run of cells that differs from what the terminal is
    /// showing, then make the whole frame visible at once.
    pub fn flush(&mut self) {
        let rows = usize::from(self.size.rows);
        let cols = usize::from(self.size.cols);
        let mut out = std::mem::take(&mut self.pending);
        // The attribute state the terminal is in, as far as this frame
        // has driven it. Unknown at the start, so the first cell of a
        // frame always carries its own sequence.
        let mut applied: Option<Attrs> = None;
        for row in 0..rows {
            let mut col = 0;
            while col < cols {
                let at = row * cols + col;
                if self.cells[at] == self.shown[at] {
                    col += 1;
                    continue;
                }
                out.extend_from_slice(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes());
                while col < cols {
                    let at = row * cols + col;
                    if self.cells[at] == self.shown[at] {
                        break;
                    }
                    let cell = self.cells[at];
                    if applied != Some(cell.attrs) {
                        out.extend_from_slice(cell.attrs.sgr().as_bytes());
                        applied = Some(cell.attrs);
                    }
                    let mut encoded = [0u8; 4];
                    out.extend_from_slice(cell.ch.encode_utf8(&mut encoded).as_bytes());
                    self.shown[at] = cell;
                    col += 1;
                }
            }
        }
        if !out.is_empty() {
            let _ = self.out.write_all(&out);
            let _ = self.out.flush();
        }
        out.clear();
        self.pending = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(rows: u16, cols: u16) -> Screen<Vec<u8>> {
        let mut screen = Screen::new(Vec::new(), Size { rows, cols });
        screen.enter();
        screen.out.clear();
        screen
    }

    fn painted(screen: &mut Screen<Vec<u8>>) -> String {
        screen.flush();
        String::from_utf8(std::mem::take(&mut screen.out)).unwrap()
    }

    #[test]
    fn a_flush_writes_only_the_cells_that_changed() {
        let mut screen = screen(2, 4);
        screen.put(0, 0, 'a', Attrs::default());
        screen.put(1, 2, 'b', Attrs::default());
        assert_eq!(painted(&mut screen), "\x1b[1;1H\x1b[0ma\x1b[2;3Hb");
        // Nothing moved, so nothing is written.
        assert_eq!(painted(&mut screen), "");
        screen.put(0, 0, 'c', Attrs::default());
        assert_eq!(painted(&mut screen), "\x1b[1;1H\x1b[0mc");
    }

    #[test]
    fn attributes_are_written_once_per_run() {
        let mut screen = screen(1, 4);
        let bold = Attrs {
            bold: true,
            fg: Color::Indexed(1),
            ..Attrs::default()
        };
        screen.put(0, 0, 'a', bold);
        screen.put(0, 1, 'b', bold);
        screen.put(0, 2, 'c', Attrs::default());
        assert_eq!(painted(&mut screen), "\x1b[1;1H\x1b[0;1;31mab\x1b[0mc");
    }

    #[test]
    fn colors_use_the_shortest_spelling_of_each_plane() {
        let sgr = |fg, bg| {
            Attrs {
                fg,
                bg,
                ..Attrs::default()
            }
            .sgr()
        };
        assert_eq!(sgr(Color::Default, Color::Default), "\x1b[0m");
        assert_eq!(sgr(Color::Indexed(3), Color::Indexed(4)), "\x1b[0;33;44m");
        assert_eq!(sgr(Color::Indexed(9), Color::Indexed(15)), "\x1b[0;91;107m");
        assert_eq!(
            sgr(Color::Indexed(200), Color::Indexed(16)),
            "\x1b[0;38;5;200;48;5;16m"
        );
    }

    #[test]
    fn clearing_and_reading_a_row_stay_inside_the_grid() {
        let mut screen = screen(2, 3);
        for col in 0..3 {
            screen.put(0, col, 'x', Attrs::default());
        }
        screen.clear_to_end_of_row(0, 1);
        assert_eq!(screen.row_chars(0), vec!['x', ' ', ' ']);
        assert_eq!(screen.row_chars(9), Vec::<char>::new());
        // Off the grid in either direction is dropped, not a panic.
        screen.put(9, 0, 'y', Attrs::default());
        screen.put(0, 9, 'y', Attrs::default());
        assert_eq!(screen.row_chars(0), vec!['x', ' ', ' ']);
    }

    #[test]
    fn a_resize_clears_the_display_and_starts_over() {
        let mut screen = screen(1, 2);
        screen.put(0, 0, 'a', Attrs::default());
        painted(&mut screen);
        screen.resize(Size { rows: 1, cols: 3 });
        screen.put(0, 2, 'b', Attrs::default());
        assert_eq!(painted(&mut screen), "\x1b[2J\x1b[H\x1b[1;3H\x1b[0mb");
    }
}

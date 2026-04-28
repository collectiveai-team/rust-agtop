//! Shared helpers for `insta` snapshot tests against `ratatui::backend::TestBackend`.

use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};

/// Build a TestBackend, draw with the given closure, return the rendered Buffer.
pub fn render_to_buffer<F>(width: u16, height: u16, mut draw: F) -> Buffer
where
    F: FnMut(&mut ratatui::Frame),
{
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| draw(f)).expect("draw");
    terminal.backend().buffer().clone()
}

/// Convert a ratatui Buffer to a stable, multi-line plain-text string for snapshots.
/// Strips styling. Each line is the visible glyphs for that row.
#[must_use]
pub fn buffer_to_text(buf: &Buffer) -> String {
    let area = buf.area;
    let mut lines = Vec::with_capacity(area.height as usize);
    for y in 0..area.height {
        let mut line = String::with_capacity(area.width as usize);
        for x in 0..area.width {
            let cell = &buf[(x, y)];
            line.push_str(cell.symbol());
        }
        lines.push(line.trim_end().to_string());
    }
    lines.join("\n")
}

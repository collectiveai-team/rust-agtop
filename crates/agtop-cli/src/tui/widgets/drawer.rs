//! Generic anchored overlay container.
// Foundation code for Plan 2.
#![allow(dead_code)]

use ratatui::{
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Clear},
    Frame,
};

use crate::tui::theme_v2::Theme;

/// Anchor positions for drawers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    BottomRight,
    Center,
}

/// Compute the drawer rect inside `area` with `width_pct` × `height_pct` of the area,
/// anchored as specified.
#[must_use]
pub fn rect_for(area: Rect, anchor: Anchor, width_pct: f32, height_pct: f32) -> Rect {
    let w = ((area.width as f32) * width_pct.clamp(0.1, 1.0)).round() as u16;
    let h = ((area.height as f32) * height_pct.clamp(0.1, 1.0)).round() as u16;
    match anchor {
        Anchor::BottomRight => Rect {
            x: area.x + area.width.saturating_sub(w),
            y: area.y + area.height.saturating_sub(h),
            width: w,
            height: h,
        },
        Anchor::Center => Rect {
            x: area.x + (area.width.saturating_sub(w)) / 2,
            y: area.y + (area.height.saturating_sub(h)) / 2,
            width: w,
            height: h,
        },
    }
}

/// Render the drawer's outer block (clears the area + draws border + title).
/// Returns the inner content area for the caller to fill.
pub fn render_chrome(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    theme: &Theme,
) -> Rect {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(title.to_string())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_focused))
        .style(Style::default().bg(theme.bg_overlay));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bottom_right_50x60() {
        let area = Rect::new(0, 0, 100, 30);
        let r = rect_for(area, Anchor::BottomRight, 0.5, 0.6);
        assert_eq!(r.width, 50);
        assert_eq!(r.height, 18);
        assert_eq!(r.x, 50);
        assert_eq!(r.y, 12);
    }

    #[test]
    fn center_anchors_to_middle() {
        let area = Rect::new(0, 0, 100, 30);
        let r = rect_for(area, Anchor::Center, 0.5, 0.6);
        assert_eq!(r.x, 25);
        assert_eq!(r.y, 6);
    }
}

//! Component trait: every renderable, event-handling unit implements this.

use ratatui::{layout::Rect, Frame};

use crate::tui::msg::Msg;
use crate::tui::theme_v2::Theme;

/// An `AppEvent` is the de-multiplexed result of a crossterm event plus
/// internal events (animation tick, data refresh tick, etc.). Defined
/// concretely in `tui::input` (Task 11). For the trait definition we
/// only need the type name; `input` will provide it.
pub use crate::tui::input::AppEvent;

/// A self-contained UI unit. Components compose into screens.
pub trait Component {
    /// Render into the given area. `focused` indicates whether this component
    /// currently has keyboard focus (drives focused-border styling, cursor
    /// visibility, etc.).
    fn render(&self, frame: &mut Frame<'_>, area: Rect, focused: bool, theme: &Theme);

    /// Handle an event. Returns `Some(Msg)` to dispatch to the App's update
    /// loop, `None` if the event was ignored.
    fn handle_event(&mut self, event: &AppEvent) -> Option<Msg>;
}

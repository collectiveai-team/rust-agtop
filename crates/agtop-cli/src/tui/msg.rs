//! Application message bus. Components emit `Msg` from `handle_event`,
//! the App's `update()` consumes them and mutates state.
//!
//! New variants are added as new screens/components are introduced.

/// All things the App can be asked to do.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// Switch to another top-level screen (Dashboard / Aggregation / Config).
    SwitchScreen(ScreenId),
    /// Open the help overlay.
    ShowHelp,
    /// Close any modal/overlay or return focus to the parent panel.
    Escape,
    /// Quit the app.
    Quit,
    /// No-op (placeholder when an event is handled but state doesn't change).
    Noop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenId {
    Dashboard,
    Aggregation,
    Config,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msg_is_clone_and_eq() {
        let a = Msg::SwitchScreen(ScreenId::Dashboard);
        let b = a.clone();
        assert_eq!(a, b);
    }
}

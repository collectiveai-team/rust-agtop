//! Crossterm event → AppEvent translation.

use crossterm::event::{Event, KeyEvent, KeyModifiers, MouseEvent};

/// Domain event consumed by Components. Wraps crossterm primitives plus
/// internal ticks (animation frame, data refresh).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    /// A printable or special key was pressed.
    Key(KeyEvent),
    /// Mouse interaction (click, scroll, drag).
    Mouse(MouseEvent),
    /// Terminal resized to the new (cols, rows).
    Resize(u16, u16),
    /// Animation timer ticked (for pulsing widgets).
    AnimationTick,
    /// Data refresh tick (sessions/quota/process snapshots refreshed).
    DataTick,
}

impl AppEvent {
    /// Translate a raw crossterm event. Returns `None` for events we don't care about.
    #[must_use]
    pub fn from_crossterm(ev: Event) -> Option<Self> {
        match ev {
            Event::Key(k) => Some(Self::Key(k)),
            Event::Mouse(m) => Some(Self::Mouse(m)),
            Event::Resize(c, r) => Some(Self::Resize(c, r)),
            Event::FocusGained | Event::FocusLost | Event::Paste(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState};

    #[test]
    fn key_event_translates() {
        let ke = KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let ev = AppEvent::from_crossterm(Event::Key(ke)).unwrap();
        assert!(matches!(ev, AppEvent::Key(k) if k.code == KeyCode::Char('q')));
    }

    #[test]
    fn resize_event_translates() {
        let ev = AppEvent::from_crossterm(Event::Resize(120, 40)).unwrap();
        assert_eq!(ev, AppEvent::Resize(120, 40));
    }

    #[test]
    fn focus_event_is_dropped() {
        assert!(AppEvent::from_crossterm(Event::FocusGained).is_none());
    }
}

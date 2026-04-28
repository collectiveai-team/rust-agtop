//! Tracks which panel/component currently owns keyboard focus.
//! Plans 2–4 add concrete `FocusId` variants per screen.
// Foundation code for Plans 2-4; not yet wired into the existing TUI.
#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FocusId(pub &'static str);

#[derive(Debug, Default)]
pub struct FocusManager {
    stack: Vec<FocusId>,
}

impl FocusManager {
    /// Currently focused panel. `None` if nothing is focused.
    #[must_use]
    pub fn current(&self) -> Option<FocusId> {
        self.stack.last().copied()
    }

    /// Push a new focus onto the stack (e.g. opening a modal/drawer).
    pub fn push(&mut self, id: FocusId) {
        self.stack.push(id);
    }

    /// Pop the most-recent focus, returning to the previous one.
    pub fn pop(&mut self) -> Option<FocusId> {
        self.stack.pop()
    }

    /// Replace the focus stack with a single id (for top-level screen switches).
    pub fn set_root(&mut self, id: FocusId) {
        self.stack.clear();
        self.stack.push(id);
    }

    /// Returns true if the given id is currently focused.
    #[must_use]
    pub fn is_focused(&self, id: FocusId) -> bool {
        self.current() == Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSIONS: FocusId = FocusId("sessions");
    const INFO: FocusId = FocusId("info_drawer");

    #[test]
    fn empty_focus_manager_has_no_current() {
        let f = FocusManager::default();
        assert_eq!(f.current(), None);
        assert!(!f.is_focused(SESSIONS));
    }

    #[test]
    fn set_root_replaces_stack() {
        let mut f = FocusManager::default();
        f.push(SESSIONS);
        f.push(INFO);
        f.set_root(SESSIONS);
        assert_eq!(f.current(), Some(SESSIONS));
    }

    #[test]
    fn push_and_pop_restore_previous() {
        let mut f = FocusManager::default();
        f.set_root(SESSIONS);
        f.push(INFO);
        assert_eq!(f.current(), Some(INFO));
        let popped = f.pop();
        assert_eq!(popped, Some(INFO));
        assert_eq!(f.current(), Some(SESSIONS));
    }
}

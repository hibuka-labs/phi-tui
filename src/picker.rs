//! Generic picker state machine: the mechanics shared by the `@` mention and `/` skill pickers.
//!
//! Every picker is the same triple: typed prefix, filtered entries, a highlighted
//! selected row, with identical keys (Esc cancels, Up/Down move, Backspace trims the
//! prefix, Enter confirms, printable chars append). Only state operations live
//! here; the App owns the two varying parts (prefix -> entries, and what Enter inserts).

use crossterm::event::KeyCode;

/// The mechanical result of one key press, interpreted by the App layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKey {
    /// A printable char: append it to the prefix.
    Char(char),
    /// Backspace: drop the last prefix char (an empty prefix lets the App close the picker).
    Backspace,
    /// Move the highlight by +1 or -1.
    Move(i32),
    /// Confirm the current selection.
    Confirm,
    /// Cancel the whole picker.
    Cancel,
}

/// Translate a key press into a [`PickerKey`]; unrelated keys give `None` (the caller swallows them).
pub fn picker_key(code: KeyCode) -> Option<PickerKey> {
    use KeyCode::*;
    match code {
        Esc => Some(PickerKey::Cancel),
        Up => Some(PickerKey::Move(-1)),
        Down => Some(PickerKey::Move(1)),
        Backspace => Some(PickerKey::Backspace),
        Enter => Some(PickerKey::Confirm),
        Char(c) => Some(PickerKey::Char(c)),
        _ => None,
    }
}

/// Generic picker state: typed prefix, filtered entries, highlight index.
#[derive(Debug, Clone, Default)]
pub struct Picker<T> {
    pub prefix: String,
    pub entries: Vec<T>,
    pub selected: usize,
}

impl<T> Picker<T> {
    pub fn new() -> Self {
        Self {
            prefix: String::new(),
            entries: Vec::new(),
            selected: 0,
        }
    }

    /// True when no prefix has been typed. A backspace on an empty prefix
    /// should close the picker (removing the `@`/`/` trigger) rather than
    /// delete a prefix char.
    pub fn is_prefix_empty(&self) -> bool {
        self.prefix.is_empty()
    }

    /// Append a char to the prefix and reset the highlight to the top.
    pub fn push_char(&mut self, c: char) {
        self.prefix.push(c);
        self.selected = 0;
    }

    /// Remove the last prefix char and reset the highlight to the top.
    pub fn pop_char(&mut self) {
        self.prefix.pop();
        self.selected = 0;
    }

    /// Move the highlight by `delta` (±1), clamped into the entry list.
    /// No-op when there are no entries.
    pub fn move_selection(&mut self, delta: i32) {
        let n = self.entries.len() as i32;
        if n == 0 {
            return;
        }
        self.selected = (self.selected as i32 + delta).clamp(0, n - 1) as usize;
    }

    /// Clamp `selected` into the current entry range (call after refreshing
    /// `entries`, since a filter can shrink the list).
    pub fn clamp_selection(&mut self) {
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker_with(items: &[&'static str]) -> Picker<&'static str> {
        Picker {
            prefix: String::new(),
            entries: items.to_vec(),
            selected: 0,
        }
    }

    #[test]
    fn push_and_pop_edit_prefix_and_reset_selection() {
        let mut p = picker_with(&["a", "b", "c"]);
        p.selected = 2;
        p.push_char('x');
        assert_eq!(p.prefix, "x");
        assert_eq!(p.selected, 0);
        p.pop_char();
        assert!(p.is_prefix_empty());
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn move_selection_clamps_to_entry_range() {
        let mut p = picker_with(&["a", "b", "c"]);
        p.move_selection(-5);
        assert_eq!(p.selected, 0);
        p.move_selection(10);
        assert_eq!(p.selected, 2);
        // Empty list: no-op.
        let mut empty = Picker::<&str>::new();
        empty.move_selection(1);
        assert_eq!(empty.selected, 0);
    }

    #[test]
    fn clamp_selection_after_shrink() {
        let mut p = picker_with(&["a", "b", "c"]);
        p.selected = 2;
        p.entries = vec!["a"];
        p.clamp_selection();
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn picker_key_maps_bindings_and_ignores_others() {
        use KeyCode::*;
        assert_eq!(picker_key(Esc), Some(PickerKey::Cancel));
        assert_eq!(picker_key(Up), Some(PickerKey::Move(-1)));
        assert_eq!(picker_key(Down), Some(PickerKey::Move(1)));
        assert_eq!(picker_key(Backspace), Some(PickerKey::Backspace));
        assert_eq!(picker_key(Enter), Some(PickerKey::Confirm));
        assert_eq!(picker_key(Char('q')), Some(PickerKey::Char('q')));
        assert_eq!(picker_key(Left), None);
        assert_eq!(picker_key(Home), None);
    }
}

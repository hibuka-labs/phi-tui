//! Generic inline completion, shared by the `@` mention and `/` slash pickers.
//!
//! [`CompletableItem`] abstracts over completion item types, which drops the
//! eight duplicated mention/slash method pairs out of `app.rs`. A new type
//! (a `#` tag picker, say) only has to implement the trait to inherit the keys.
//!
//! # Design
//! - **Cohesive**: all completion logic lives in this module
//! - **Decoupled**: the trait insulates it from concrete types; no dependency on App
//! - **Extensible**: a new completion type is just a `CompletableItem`

use crate::mention::{self, Entry};
use crate::picker::{Picker, PickerKey, picker_key};
use crossterm::event::KeyCode;
use std::path::PathBuf;

/// A completable item: what every completion entry has to be able to do.
pub trait CompletableItem: Clone {
    /// Display name, shown in the list.
    fn display_name(&self) -> &str;

    /// Whether this item matches the query string, for filtering.
    fn matches_query(&self, query: &str) -> bool;
}

/// What the completer asks its caller to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompleterAction<T> {
    /// Keep completing (nothing matched).
    Continue,
    /// The user picked an item.
    Selected(T),
    /// The user cancelled completion.
    Cancelled,
    /// Not our key; hand it to another handler.
    Unhandled(KeyCode),
}

/// Refresh callback: takes a prefix, returns the filtered entries.
///
/// # Arguments
/// * `prefix` - the prefix typed so far
/// * `all_entries` - every available entry
///
/// # Returns
/// The filtered entries.
pub type RefreshFn<T> = Box<dyn Fn(&str, &[T]) -> Vec<T>>;

/// Generic inline completer.
///
/// Wraps the picker state machine and trigger logic behind one keyboard interface.
/// The `trigger` char (`@` or `/`) says which kind of completion this is.
///
/// Custom refresh logic is supported for pickers with their own filtering rules (filesystem browsing, say).
pub struct InlineCompleter<T: CompletableItem> {
    /// Picker state: prefix, entries, selected.
    pub picker: Picker<T>,
    /// Trigger char (`@` or `/`).
    pub trigger: char,
    /// Every available entry, used for filtering.
    all_entries: Vec<T>,
    /// Custom refresh logic, if any.
    refresh_fn: Option<RefreshFn<T>>,
}

impl<T: CompletableItem + std::fmt::Debug> std::fmt::Debug for InlineCompleter<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InlineCompleter")
            .field("picker", &self.picker)
            .field("trigger", &self.trigger)
            .field("all_entries", &self.all_entries)
            .field("refresh_fn", &self.refresh_fn.is_some())
            .finish()
    }
}

impl<T: CompletableItem> InlineCompleter<T> {
    /// Build a completer with the default filtering.
    ///
    /// # Arguments
    /// * `trigger` - the trigger char (`@` or `/`)
    /// * `entries` - every available entry
    pub fn new(trigger: char, entries: Vec<T>) -> Self {
        let mut picker = Picker::new();
        picker.entries = entries.clone();
        Self {
            picker,
            trigger,
            all_entries: entries,
            refresh_fn: None,
        }
    }

    /// Build a completer with custom refresh logic.
    ///
    /// # Arguments
    /// * `trigger` - the trigger char (`@` or `/`)
    /// * `entries` - every available entry
    /// * `refresh_fn` - the custom refresh logic
    pub fn with_refresh_fn(trigger: char, entries: Vec<T>, refresh_fn: RefreshFn<T>) -> Self {
        let mut picker = Picker::new();
        picker.entries = entries.clone();
        Self {
            picker,
            trigger,
            all_entries: entries,
            refresh_fn: Some(refresh_fn),
        }
    }

    /// The currently selected entry.
    pub fn selected_item(&self) -> Option<&T> {
        self.picker.entries.get(self.picker.selected)
    }

    /// The trigger char.
    pub fn trigger(&self) -> char {
        self.trigger
    }

    /// Whether the completer is idle (nothing typed).
    pub fn is_empty(&self) -> bool {
        self.picker.is_prefix_empty()
    }

    /// Handle a key and report what to do.
    ///
    /// The completer's main entry point: every key goes through here.
    pub fn handle_key(&mut self, code: KeyCode) -> CompleterAction<T> {
        match picker_key(code) {
            Some(PickerKey::Cancel) => CompleterAction::Cancelled,
            Some(PickerKey::Move(delta)) => {
                self.picker.move_selection(delta);
                CompleterAction::Continue
            }
            Some(PickerKey::Backspace) => {
                if self.picker.is_prefix_empty() {
                    // An empty prefix means backspace cancels completion.
                    CompleterAction::Cancelled
                } else {
                    self.picker.pop_char();
                    self.refresh_entries();
                    CompleterAction::Continue
                }
            }
            Some(PickerKey::Confirm) => {
                if let Some(item) = self.picker.entries.get(self.picker.selected) {
                    CompleterAction::Selected(item.clone())
                } else {
                    CompleterAction::Cancelled
                }
            }
            Some(PickerKey::Char(c)) => {
                self.picker.push_char(c);
                self.refresh_entries();
                CompleterAction::Continue
            }
            None => CompleterAction::Unhandled(code),
        }
    }

    /// Refresh the filtered entry list.
    ///
    /// Filters `all_entries` by the current prefix into `picker.entries`.
    /// A custom refresh function, when set, takes precedence.
    fn refresh_entries(&mut self) {
        if let Some(ref refresh_fn) = self.refresh_fn {
            // Custom refresh logic.
            self.picker.entries = refresh_fn(&self.picker.prefix, &self.all_entries);
        } else {
            // Default filtering.
            if self.picker.prefix.is_empty() {
                self.picker.entries = self.all_entries.clone();
            } else {
                let query = &self.picker.prefix;
                self.picker.entries = self
                    .all_entries
                    .iter()
                    .filter(|item| item.matches_query(query))
                    .cloned()
                    .collect();
            }
        }
        self.picker.clamp_selection();
    }

    /// Replace every available entry (for dynamic loading).
    pub fn set_entries(&mut self, entries: Vec<T>) {
        self.all_entries = entries;
        self.refresh_entries();
    }

    /// How many entries are currently filtered in.
    pub fn entries_count(&self) -> usize {
        self.picker.entries.len()
    }

    /// The current prefix.
    pub fn prefix(&self) -> &str {
        &self.picker.prefix
    }

    /// Every available entry, for updates from outside.
    pub fn all_entries(&self) -> &[T] {
        &self.all_entries
    }
}

/// Mention completer: the `@` file-path picker's special cases.
///
/// What makes it different:
/// 1. Always prepends a synthetic "use what I typed" entry.
/// 2. Needs `workspace_root` to resolve relative paths.
/// 3. Has to produce the right path string on completion.
pub struct MentionCompleter {
    /// Uses an `InlineCompleter` internally.
    inner: InlineCompleter<Entry>,
    /// Workspace root.
    workspace_root: PathBuf,
    /// The directory currently resolved to.
    current_dir: PathBuf,
}

impl std::fmt::Debug for MentionCompleter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MentionCompleter")
            .field("inner", &self.inner)
            .field("workspace_root", &self.workspace_root)
            .field("current_dir", &self.current_dir)
            .finish()
    }
}

impl MentionCompleter {
    /// Build a `MentionCompleter`.
    pub fn new(workspace_root: PathBuf) -> Self {
        // Placeholder refresh function; the real work is in `refresh_entries`.
        let refresh_fn = Box::new(|_prefix: &str, _entries: &[Entry]| -> Vec<Entry> { Vec::new() });

        let mut completer = Self {
            inner: InlineCompleter::with_refresh_fn('@', Vec::new(), refresh_fn),
            workspace_root: workspace_root.clone(),
            current_dir: workspace_root,
        };
        // Populate the entries up front.
        completer.refresh_entries();
        completer
    }

    /// The currently selected entry.
    pub fn selected_item(&self) -> Option<&Entry> {
        self.inner.selected_item()
    }

    /// The trigger char.
    pub fn trigger(&self) -> char {
        self.inner.trigger()
    }

    /// Whether the completer is idle (nothing typed).
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Handle key input and return an action.
    ///
    /// When a directory is selected, navigate into it instead of selecting directly.
    /// Only returns Selected for files or synthetic entries.
    pub fn handle_key(&mut self, code: KeyCode) -> CompleterAction<Entry> {
        match self.inner.handle_key(code) {
            CompleterAction::Selected(item) => {
                if item.is_dir && !item.synthetic {
                    // Real directory selected → navigate into it
                    let new_prefix = mention::rel_or_abs(&self.workspace_root, &item.path);
                    // Ensure trailing / to indicate directory navigation
                    self.inner.picker.prefix = if new_prefix.ends_with('/') {
                        new_prefix
                    } else {
                        format!("{new_prefix}/")
                    };
                    self.refresh_entries();
                    // Focus the entered directory itself — the synthetic row
                    // at index 0 renders the path we just descended into.
                    // Without this reset the highlight keeps the directory's
                    // old index and lands on an unrelated child.
                    self.inner.picker.selected = 0;
                    CompleterAction::Continue
                } else {
                    // File or synthetic entry → select directly
                    CompleterAction::Selected(item)
                }
            }
            CompleterAction::Continue => {
                // Refresh entries (special logic)
                self.refresh_entries();
                CompleterAction::Continue
            }
            other => other,
        }
    }

    /// Refresh the entries, synthetic one included.
    fn refresh_entries(&mut self) {
        let prefix = self.inner.picker.prefix.clone();
        let (dir, name) = mention::split_prefix(&self.workspace_root, &prefix);
        let mut entries = mention::list_entries(&dir, &name);

        // Prepend the synthetic "use what I typed" entry -- it is always first.
        let full = if name.is_empty() {
            dir.clone()
        } else {
            dir.join(&name)
        };
        entries.insert(
            0,
            Entry {
                name: mention::rel_or_abs(&self.workspace_root, &full),
                path: full,
                is_dir: false,
                synthetic: true,
            },
        );

        self.current_dir = dir;
        self.inner.picker.entries = entries;
        self.inner.picker.clamp_selection();
    }

    /// Replace the workspace root.
    pub fn set_workspace_root(&mut self, root: PathBuf) {
        self.workspace_root = root;
    }

    /// How many entries are currently filtered in.
    pub fn entries_count(&self) -> usize {
        self.inner.entries_count()
    }

    /// The current prefix.
    pub fn prefix(&self) -> &str {
        self.inner.prefix()
    }

    /// The directory currently resolved to.
    pub fn current_dir(&self) -> &PathBuf {
        &self.current_dir
    }

    /// The workspace root.
    pub fn workspace_root(&self) -> &PathBuf {
        &self.workspace_root
    }

    /// The entry list, for rendering.
    pub fn entries(&self) -> &[Entry] {
        &self.inner.picker.entries
    }

    /// The selected index, for rendering.
    pub fn selected_index(&self) -> usize {
        self.inner.picker.selected
    }

    /// The text to insert into the composer on completion.
    pub fn finish_text(&self) -> String {
        self.inner
            .selected_item()
            .map(|e| mention::rel_or_abs(&self.workspace_root, &e.path))
            .unwrap_or_else(|| mention::rel_or_abs(&self.workspace_root, &self.current_dir))
    }
}

/// Slash completer: the `/` skill picker's special cases.
///
/// What makes it different:
/// 1. Entries come from `skill_summaries` (loaded dynamically).
/// 2. Filtering matches on both name and description.
pub struct SlashCompleter {
    /// Uses an `InlineCompleter` internally.
    inner: InlineCompleter<(String, String)>,
}

impl std::fmt::Debug for SlashCompleter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlashCompleter")
            .field("inner", &self.inner)
            .finish()
    }
}

impl SlashCompleter {
    /// Build a `SlashCompleter`.
    pub fn new(entries: Vec<(String, String)>) -> Self {
        Self {
            inner: InlineCompleter::new('/', entries),
        }
    }

    /// The currently selected entry.
    pub fn selected_item(&self) -> Option<&(String, String)> {
        self.inner.selected_item()
    }

    /// The trigger char.
    pub fn trigger(&self) -> char {
        self.inner.trigger()
    }

    /// Whether the completer is idle (nothing typed).
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Handle a key and report what to do.
    pub fn handle_key(&mut self, code: KeyCode) -> CompleterAction<(String, String)> {
        self.inner.handle_key(code)
    }

    /// Replace the entry list.
    pub fn set_entries(&mut self, entries: Vec<(String, String)>) {
        self.inner.set_entries(entries);
    }

    /// How many entries are currently filtered in.
    pub fn entries_count(&self) -> usize {
        self.inner.entries_count()
    }

    /// The current prefix.
    pub fn prefix(&self) -> &str {
        self.inner.prefix()
    }

    /// The entry list, for rendering.
    pub fn entries(&self) -> &[(String, String)] {
        &self.inner.picker.entries
    }

    /// The selected index, for rendering.
    pub fn selected_index(&self) -> usize {
        self.inner.picker.selected
    }
}

/// `CompletableItem` for `mention::Entry`.
impl CompletableItem for crate::mention::Entry {
    fn display_name(&self) -> &str {
        &self.name
    }

    fn matches_query(&self, query: &str) -> bool {
        // Plain prefix match, same as the original mention logic.
        self.name.to_lowercase().starts_with(&query.to_lowercase())
    }
}

/// `CompletableItem` for `(String, String)` -- the slash picker's rows.
///
/// The first element is the name, the second the description.
impl CompletableItem for (String, String) {
    fn display_name(&self) -> &str {
        &self.0
    }

    fn matches_query(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        // Name prefix match (always on): typing "r" only matches skills starting with r.
        if self.0.to_lowercase().starts_with(&q) {
            return true;
        }
        // Description substring match (CJK queries only): matches by wording.
        // Pure ASCII queries skip it, so "rev" cannot match "Request a code review".
        if !q.is_ascii() && q.chars().count() >= 2 {
            return self.1.to_lowercase().contains(&q);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test completion item.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestItem {
        name: String,
        desc: String,
    }

    impl TestItem {
        fn new(name: &str, desc: &str) -> Self {
            Self {
                name: name.to_string(),
                desc: desc.to_string(),
            }
        }
    }

    impl CompletableItem for TestItem {
        fn display_name(&self) -> &str {
            &self.name
        }

        fn matches_query(&self, query: &str) -> bool {
            let q = query.to_lowercase();
            self.name.to_lowercase().contains(&q) || self.desc.to_lowercase().contains(&q)
        }
    }

    fn test_entries() -> Vec<TestItem> {
        vec![
            TestItem::new("file1.rs", "Rust source"),
            TestItem::new("file2.txt", "Text file"),
            TestItem::new("dir1", "Directory"),
            TestItem::new("main.rs", "Main entry"),
        ]
    }

    #[test]
    fn new_creates_empty_completer() {
        let completer = InlineCompleter::new('@', test_entries());
        assert_eq!(completer.trigger(), '@');
        assert!(completer.is_empty());
        assert_eq!(completer.entries_count(), 4);
    }

    #[test]
    fn push_char_filters_entries() {
        let mut completer = InlineCompleter::new('@', test_entries());

        // Typing 'f' should filter down to file1.rs and file2.txt.
        let action = completer.handle_key(KeyCode::Char('f'));
        assert_eq!(action, CompleterAction::Continue);
        assert_eq!(completer.entries_count(), 2);
        assert_eq!(completer.prefix(), "f");
    }

    #[test]
    fn backspace_removes_char() {
        let mut completer = InlineCompleter::new('@', test_entries());

        // Type 'f' first.
        completer.handle_key(KeyCode::Char('f'));
        assert_eq!(completer.entries_count(), 2);

        // Backspace should restore the full list.
        let action = completer.handle_key(KeyCode::Backspace);
        assert_eq!(action, CompleterAction::Continue);
        assert_eq!(completer.entries_count(), 4);
        assert!(completer.is_empty());
    }

    #[test]
    fn backspace_on_empty_cancels() {
        let mut completer = InlineCompleter::new('@', test_entries());

        let action = completer.handle_key(KeyCode::Backspace);
        assert_eq!(action, CompleterAction::Cancelled);
    }

    #[test]
    fn esc_cancels() {
        let mut completer = InlineCompleter::new('@', test_entries());

        let action = completer.handle_key(KeyCode::Esc);
        assert_eq!(action, CompleterAction::Cancelled);
    }

    #[test]
    fn arrow_keys_move_selection() {
        let mut completer = InlineCompleter::new('@', test_entries());

        // Down should move the selection.
        let action = completer.handle_key(KeyCode::Down);
        assert_eq!(action, CompleterAction::Continue);
        assert_eq!(completer.picker.selected, 1);

        // Up should move it back.
        let action = completer.handle_key(KeyCode::Up);
        assert_eq!(action, CompleterAction::Continue);
        assert_eq!(completer.picker.selected, 0);
    }

    #[test]
    fn enter_selects_current_item() {
        let mut completer = InlineCompleter::new('@', test_entries());

        // Select the first one.
        let action = completer.handle_key(KeyCode::Enter);
        assert_eq!(
            action,
            CompleterAction::Selected(TestItem::new("file1.rs", "Rust source"))
        );
    }

    #[test]
    fn enter_with_empty_entries_cancels() {
        let mut completer = InlineCompleter::<TestItem>::new('@', vec![]);

        let action = completer.handle_key(KeyCode::Enter);
        assert_eq!(action, CompleterAction::Cancelled);
    }

    #[test]
    fn set_entries_updates_list() {
        let mut completer = InlineCompleter::new('@', test_entries());
        assert_eq!(completer.entries_count(), 4);

        // Replace the entries.
        let new_entries = vec![TestItem::new("new.rs", "New file")];
        completer.set_entries(new_entries);
        assert_eq!(completer.entries_count(), 1);
    }

    #[test]
    fn unhandled_key_returns_unhandled() {
        let mut completer = InlineCompleter::new('@', test_entries());

        // Tab should come back Unhandled.
        let action = completer.handle_key(KeyCode::Tab);
        assert_eq!(action, CompleterAction::Unhandled(KeyCode::Tab));
    }

    #[test]
    fn selected_item_returns_current() {
        let mut completer = InlineCompleter::new('@', test_entries());

        // Selects the first entry by default.
        assert_eq!(
            completer.selected_item(),
            Some(&TestItem::new("file1.rs", "Rust source"))
        );

        // After moving, it reports the new one.
        completer.handle_key(KeyCode::Down);
        assert_eq!(
            completer.selected_item(),
            Some(&TestItem::new("file2.txt", "Text file"))
        );
    }

    #[test]
    fn selected_item_on_empty_returns_none() {
        let completer = InlineCompleter::<TestItem>::new('@', vec![]);
        assert_eq!(completer.selected_item(), None);
    }

    #[test]
    fn custom_refresh_fn_is_used() {
        // Custom refresh: only entries starting with "file".
        let refresh_fn = Box::new(|prefix: &str, entries: &[TestItem]| -> Vec<TestItem> {
            if prefix.is_empty() {
                entries.to_vec()
            } else {
                entries
                    .iter()
                    .filter(|e| e.name.starts_with("file"))
                    .cloned()
                    .collect()
            }
        });

        let mut completer = InlineCompleter::with_refresh_fn('@', test_entries(), refresh_fn);

        // Typing "x" should still return only file1.rs and file2.txt (custom logic).
        completer.handle_key(KeyCode::Char('x'));
        assert_eq!(completer.entries_count(), 2);
        assert_eq!(completer.prefix(), "x");
    }

    #[test]
    fn all_entries_accessor_works() {
        let completer = InlineCompleter::new('@', test_entries());
        assert_eq!(completer.all_entries().len(), 4);
    }

    #[test]
    fn mention_completer_creates_with_workspace_root() {
        let root = PathBuf::from("/tmp/test");
        let completer = MentionCompleter::new(root.clone());
        assert_eq!(completer.trigger(), '@');
        assert!(completer.is_empty());
        assert_eq!(completer.workspace_root(), &root);
    }

    /// Throwaway temp dir (same pattern as the mention.rs tests).
    fn scratch(tag: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("phi-tui-completer-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn enter_on_directory_descends_and_focuses_it() {
        let root = scratch("nav");
        std::fs::create_dir_all(root.join("a_dir")).unwrap();
        std::fs::write(root.join("z.txt"), "x").unwrap();

        let mut m = MentionCompleter::new(root.clone());
        // [synthetic, a_dir, z.txt] — arrow onto the directory.
        m.handle_key(KeyCode::Down);
        assert_eq!(m.entries()[1].name, "a_dir");
        assert_eq!(m.selected_index(), 1);

        // Enter descends into a_dir; focus lands on the synthetic row (the
        // directory itself), not the directory's old index.
        assert!(matches!(
            m.handle_key(KeyCode::Enter),
            CompleterAction::Continue
        ));
        assert_eq!(m.prefix(), "a_dir/");
        assert_eq!(m.selected_index(), 0);
        assert!(m.entries()[0].synthetic);
    }

    #[test]
    fn slash_completer_creates_with_entries() {
        let entries = vec![
            ("skill1".to_string(), "Description 1".to_string()),
            ("skill2".to_string(), "Description 2".to_string()),
        ];
        let completer = SlashCompleter::new(entries);
        assert_eq!(completer.trigger(), '/');
        assert!(completer.is_empty());
        assert_eq!(completer.entries_count(), 2);
    }
}

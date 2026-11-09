//! Transcript mouse selection and the right-click copy menu.
//!
//! Extracted out of the `App` god-object (previously the `selection`/`context_menu`
//! fields, the selection branch of `handle_mouse`, and
//! `selection_text`/`clear_selection`). Hit-testing screen coordinates to line
//! numbers depends on the render output ([`crate::visual::VisualMap`]), so that
//! stays in App; this module only tracks what is selected and what the menu
//! shows.
//!
//! Selection is character-precise: a [`Selection`] spans [`CharPoint`]s (visual
//! row + display-cell column, snapped so a wide character is always taken
//! whole), copy slices the selected cells out of each row, and soft-wrap seams
//! rejoin what the wrap split (see [`crate::visual::Seam`]) — what you see is
//! what you copy.

use std::time::{Duration, Instant};

use crossterm::event::{MouseButton, MouseEventKind};
use unicode_width::UnicodeWidthChar;

use crate::visual::{Seam, VisualMap};

/// A position in the transcript: visual-row index + display-cell column,
/// always snapped to a character's first cell (a double-wide CJK character is
/// whole or absent — never half).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharPoint {
    pub row: usize,
    pub col: usize,
}

/// A character-precise selection (inclusive at both ends): `anchor` is the
/// drag start and `head` the current end; they may be in either order, so
/// [`Selection::bounds`] normalizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: CharPoint,
    pub head: CharPoint,
}

impl Selection {
    /// Normalized bounds with `lo <= hi` (row-major order).
    fn bounds(self) -> (CharPoint, CharPoint) {
        if (self.anchor.row, self.anchor.col) <= (self.head.row, self.head.col) {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }
}

/// Display width of `text` in terminal cells (ASCII = 1, wide CJK = 2,
/// combining marks = 0).
pub fn text_width(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Snap cell column `col` to the FIRST cell of the character it falls on,
/// clamped to the row width past the end. Zero-width characters (combining
/// marks) are not hit-test targets — they belong to their base character.
pub fn snap_start(text: &str, col: usize) -> usize {
    let mut cell = 0usize;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if w == 0 {
            continue;
        }
        if col < cell + w {
            return cell;
        }
        cell += w;
    }
    cell
}

/// The exclusive end cell of the character `col` falls on — the whole wide
/// character is taken. Past-the-end columns clamp to the row width.
pub fn snap_end(text: &str, col: usize) -> usize {
    let mut cell = 0usize;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if w == 0 {
            continue;
        }
        if col < cell + w {
            return cell + w;
        }
        cell += w;
    }
    cell
}

/// The characters covering display-cell range `[start, end)`, wide-char
/// snapped at both ends (no half characters). A zero-width combining mark
/// copies whenever its base character does.
pub fn slice_cells(text: &str, start: usize, end: usize) -> String {
    let start = snap_start(text, start);
    let end = snap_end(text, end.saturating_sub(1).min(text_width(text)));
    let end = end.max(start);
    let mut out = String::new();
    let mut cell = 0usize;
    let mut last_in = false;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        let c_end = cell + w;
        let included = if w == 0 {
            last_in // a combining mark follows its base character
        } else {
            c_end > start && cell < end
        };
        if included {
            out.push(c);
        }
        last_in = included;
        cell = c_end;
    }
    out
}

/// What kind of run a character belongs to (the double-click unit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunClass {
    /// ASCII/`_` word characters.
    Word,
    /// Wide CJK characters (ideographs, kana, full-width forms).
    Cjk,
    /// Whitespace.
    Space,
    /// Everything else.
    Punct,
}

fn run_class(c: char, prev: RunClass) -> RunClass {
    let w = c.width().unwrap_or(0);
    if w == 0 {
        return prev; // a combining mark extends the run it sits in
    }
    if c.is_alphanumeric() || c == '_' {
        if w >= 2 {
            RunClass::Cjk
        } else {
            RunClass::Word
        }
    } else if c.is_whitespace() {
        RunClass::Space
    } else {
        RunClass::Punct
    }
}

/// The run of like characters under cell column `col` — the double-click
/// selection unit. Classes: alphanumerics + `_` form a word run, wide CJK
/// characters form a CJK run, whitespace and punctuation each form their own
/// run ("each picks its own run"). Returns display-cell bounds `(lo, hi)` with
/// `hi` exclusive, wide characters whole.
pub fn word_run(text: &str, col: usize) -> (usize, usize) {
    let mut chars: Vec<(RunClass, usize, usize)> = Vec::new(); // (class, cell, width)
    let mut cell = 0usize;
    let mut prev = RunClass::Word;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        let class = run_class(c, prev);
        prev = class;
        chars.push((class, cell, w));
        cell += w;
    }
    if chars.is_empty() {
        return (0, 0);
    }
    // The character under `col`; a column past the end lands on the last one.
    let mut idx = chars.len() - 1;
    for (i, &(_, c0, w)) in chars.iter().enumerate() {
        if w > 0 && col < c0 + w {
            idx = i;
            break;
        }
    }
    let class = chars[idx].0;
    let mut lo = idx;
    while lo > 0 && chars[lo - 1].0 == class {
        lo -= 1;
    }
    let mut hi = idx;
    while hi + 1 < chars.len() && chars[hi + 1].0 == class {
        hi += 1;
    }
    (chars[lo].1, chars[hi].1 + chars[hi].2)
}

/// Multi-click classifier: crossterm reports presses without a click count.
/// Presses on the same cell inside [`CLICK_CHAIN_WINDOW`] count up (1 → 2 →
/// 3); anything else — a different cell or a window gap — restarts at 1.
/// Counts clamp at 3 (the largest selection unit, a line).
#[derive(Debug, Default)]
pub struct ClickChain {
    cell: Option<(usize, usize)>,
    count: u8,
    last: Option<Instant>,
}

/// The window in which successive presses count as one multi-click.
pub const CLICK_CHAIN_WINDOW: Duration = Duration::from_millis(500);

impl ClickChain {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a press at `cell` and return the running click count (≥ 1).
    pub fn click(&mut self, cell: (usize, usize), now: Instant) -> u8 {
        let continuing = self.cell == Some(cell)
            && self
                .last
                .is_some_and(|t| now.saturating_duration_since(t) <= CLICK_CHAIN_WINDOW);
        self.count = if continuing {
            (self.count + 1).min(3)
        } else {
            1
        };
        self.cell = Some(cell);
        self.last = Some(now);
        self.count
    }

    /// Forget the chain (a press that missed the transcript, a drag).
    pub fn reset(&mut self) {
        self.cell = None;
        self.count = 0;
        self.last = None;
    }
}

/// What a press means once classified by the click chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressKind {
    /// A single click (or a miss).
    Single,
    /// A double click: select the word run under the cursor.
    Double,
    /// A triple click: select the whole logical line.
    Triple,
}

/// Right-click copy menu: anchor cell (top-left) and highlighted item index
/// (0 = copy, 1 = cancel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextMenu {
    pub x: u16,
    pub y: u16,
    pub selected: usize,
}

/// Context-menu popup geometry, shared between rendering and mouse hit-testing
/// so a click lands exactly on the drawn popup.
pub const CONTEXT_MENU_W: u16 = 12;
pub const CONTEXT_MENU_H: u16 = 4; // border + two items

/// Clamp the menu's top-left so the fixed-size popup stays fully on screen.
pub fn context_menu_pos(x: u16, y: u16, area_w: u16, area_h: u16) -> (u16, u16) {
    (
        x.min(area_w.saturating_sub(CONTEXT_MENU_W)),
        y.min(area_h.saturating_sub(CONTEXT_MENU_H)),
    )
}

/// What a click means while the right-click menu is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuClick {
    /// The copy row was clicked.
    Copy,
    /// The dismiss row (or anywhere outside) was clicked; hand back to normal selection.
    Dismiss,
}

/// Mouse selection + copy-menu state machine.
#[derive(Debug, Default)]
pub struct SelectionState {
    pub selection: Option<Selection>,
    pub context_menu: Option<ContextMenu>,
    clicks: ClickChain,
}

impl SelectionState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn context_menu(&self) -> Option<&ContextMenu> {
        self.context_menu.as_ref()
    }

    pub fn clear(&mut self) {
        self.selection = None;
    }

    pub fn menu_is_open(&self) -> bool {
        self.context_menu.is_some()
    }

    pub fn menu_selected(&self) -> usize {
        self.context_menu.map_or(0, |m| m.selected)
    }

    /// Move the menu highlight by `delta` (±1); item 1 is the last item.
    pub fn menu_move(&mut self, delta: i32) {
        let Some(m) = self.context_menu.as_mut() else {
            return;
        };
        if delta < 0 {
            m.selected = m.selected.saturating_sub(1);
        } else {
            m.selected = (m.selected + 1).min(1);
        }
    }

    /// Register a press at `pt` with the click chain and report what the click
    /// means. A press that missed the transcript resets the chain and reports
    /// [`PressKind::Single`].
    pub fn press(&mut self, pt: Option<CharPoint>, now: Instant) -> PressKind {
        match pt {
            Some(p) => match self.clicks.click((p.row, p.col), now) {
                2 => PressKind::Double,
                3.. => PressKind::Triple,
                _ => PressKind::Single,
            },
            None => {
                self.clicks.reset();
                PressKind::Single
            }
        }
    }

    /// Forget the click chain (a drag supersedes multi-click selection).
    pub fn reset_clicks(&mut self) {
        self.clicks.reset();
    }

    /// Left-press: drop any open menu and anchor a new selection at `pt`
    /// (`None` = the press missed the transcript — clears the selection).
    pub fn anchor(&mut self, pt: Option<CharPoint>) {
        self.context_menu = None;
        self.selection = pt.map(|p| Selection { anchor: p, head: p });
    }

    /// Left-drag: move the selection head to `pt` (no-op when no selection).
    pub fn extend(&mut self, pt: Option<CharPoint>) {
        if let Some(pt) = pt
            && let Some(sel) = self.selection.as_mut()
        {
            sel.head = pt;
        }
    }

    /// Select exactly `a..=b` (the word/line picks behind double/triple
    /// click), replacing any previous selection.
    pub fn select_range(&mut self, a: CharPoint, b: CharPoint) {
        self.context_menu = None;
        self.selection = Some(Selection { anchor: a, head: b });
    }

    /// Right-press: open the copy menu when a selection is active.
    pub fn open_menu(&mut self, x: u16, y: u16) {
        if self.selection.is_some() {
            self.context_menu = Some(ContextMenu { x, y, selected: 0 });
        }
    }

    /// While the menu is open, a click (either button) inside the drawn popup
    /// activates the item under the cursor. Returns `None` when the click fell
    /// outside the popup (the caller falls through to normal selection).
    pub fn menu_click(
        &mut self,
        kind: MouseEventKind,
        x: u16,
        y: u16,
        area_w: u16,
        area_h: u16,
    ) -> Option<MenuClick> {
        let menu = self.context_menu?;
        let (mx, my) = context_menu_pos(menu.x, menu.y, area_w, area_h);
        if x >= mx
            && x < mx + CONTEXT_MENU_W
            && y >= my
            && y < my + CONTEXT_MENU_H
            && matches!(
                kind,
                MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Down(MouseButton::Right)
            )
        {
            let clicked_copy = y.saturating_sub(my + 1) == 0;
            self.context_menu = None;
            return Some(if clicked_copy {
                MenuClick::Copy
            } else {
                MenuClick::Dismiss
            });
        }
        None
    }

    /// The active selection as normalized, `total`-clamped bounds (the head
    /// row clamped into range), or `None` when empty/stale. Rows are visual
    /// line indices (post-markdown-expansion), not raw transcript indices.
    /// A head whose rows vanished (content shrank under a live selection)
    /// extends to the end of the surviving last row — nearest living content,
    /// the same rule `translate_index` applies to anchors.
    pub fn range(&self, total: usize) -> Option<(CharPoint, CharPoint)> {
        let sel = self.selection?;
        let (lo, mut hi) = sel.bounds();
        if total == 0 || lo.row >= total {
            return None;
        }
        if hi.row >= total {
            hi.row = total - 1;
            hi.col = usize::MAX;
        }
        Some((lo, hi))
    }

    /// True when visual line `i` falls inside the active selection.
    pub fn is_selected(&self, i: usize, total: usize) -> bool {
        self.range(total)
            .is_some_and(|(lo, hi)| lo.row <= i && i <= hi.row)
    }

    /// The cell-column range `[lo, hi)` to highlight on visual row `i`
    /// (`hi` may be [`usize::MAX`] = to the row's end; wide characters snap
    /// whole). `row_text` is the row's displayed plain text.
    pub fn selected_cols(&self, i: usize, total: usize, row_text: &str) -> Option<(usize, usize)> {
        let (lo, hi) = self.range(total)?;
        if i < lo.row || i > hi.row {
            return None;
        }
        let cols = if lo.row == hi.row {
            (snap_start(row_text, lo.col), snap_end(row_text, hi.col))
        } else if i == lo.row {
            (snap_start(row_text, lo.col), usize::MAX)
        } else if i == hi.row {
            (0, snap_end(row_text, hi.col))
        } else {
            (0, usize::MAX)
        };
        Some(cols)
    }

    /// The selected cells joined as plain text (what-you-see-is-what-you-copy).
    /// `fallback` is used when `visual` is empty (tests, before the first
    /// render): one entry per raw line, hard breaks between. With a map, each
    /// row is sliced to the selected cells and soft-wrap seams rejoin what the
    /// wrap split ([`Seam::joiner`]).
    pub fn text(&self, fallback: &[impl AsRef<str>], visual: &VisualMap) -> String {
        let use_visual = !visual.is_empty();
        let total = if use_visual {
            visual.len()
        } else {
            fallback.len()
        };
        let Some((lo, hi)) = self.range(total) else {
            return String::new();
        };
        let row_text = |i: usize| -> &str {
            if use_visual {
                visual.texts().get(i).map(String::as_str).unwrap_or("")
            } else {
                fallback.get(i).map(AsRef::as_ref).unwrap_or("")
            }
        };
        let mut out = String::new();
        for i in lo.row..=hi.row {
            let row = row_text(i);
            let start = if i == lo.row {
                snap_start(row, lo.col)
            } else {
                0
            };
            let end = if i == hi.row {
                snap_end(row, hi.col)
            } else {
                text_width(row)
            };
            if i > lo.row {
                // Adjacent rows of one output line rejoin at their soft-wrap
                // seam; anything else is a hard break.
                let seam = visual.seam_at(i);
                let same_line = use_visual
                    && visual.output_at(i).is_some()
                    && visual.output_at(i) == visual.output_at(i - 1);
                out.push_str(if same_line && seam != Seam::Break {
                    seam.joiner()
                } else {
                    "\n"
                });
            }
            out.push_str(&slice_cells(row, start, end));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("line {i}")).collect()
    }

    /// Typed empty visual map: the "no render yet" fallback case.
    fn empty_map() -> VisualMap {
        VisualMap::new()
    }

    fn pt(row: usize, col: usize) -> CharPoint {
        CharPoint { row, col }
    }

    // ---- character geometry ----

    /// protects: cell columns snap to character heads/ends, so a wide CJK
    /// character is always taken whole at either edge of a slice.
    /// fails_when: a slice starts or ends mid-character (half a glyph copies).
    /// no_claim: how the renderer paints the highlight.
    #[test]
    fn wide_chars_snap_whole_at_both_edges() {
        // "你好ab": 你(0..2) 好(2..4) a(4..5) b(5..6)
        assert_eq!(snap_start("你好ab", 3), 2, "mid 好 snaps back to its head");
        assert_eq!(snap_end("你好ab", 3), 4, "mid 好 ends past its tail");
        assert_eq!(slice_cells("你好ab", 3, 5), "好a");
        assert_eq!(slice_cells("你好ab", 2, 4), "好");
        // A zero-width combining mark never splits off its base character.
        let e_acute = "e\u{301}x"; // e(0..1) mark(1..1) x(1..2)
        assert_eq!(slice_cells(e_acute, 0, 1), "e\u{301}");
        assert_eq!(slice_cells(e_acute, 1, 2), "x");
    }

    /// protects: a selection span slices the first row from its column, the
    /// last row to its column, and takes middle rows whole.
    /// fails_when: whole rows copy when only part of the first/last row was
    /// dragged over (the original whole-line bug).
    /// no_claim: the seam joins between rows (covered below).
    #[test]
    fn drag_slices_first_and_last_rows() {
        let mut s = SelectionState::new();
        let mut m = VisualMap::new();
        for (i, t) in ["alpha beta", "gamma", "delta epsilon"]
            .into_iter()
            .enumerate()
        {
            m.push_mapped(i, t.to_string());
        }
        s.anchor(Some(pt(0, 6))); // at 'b' of beta
        s.extend(Some(pt(2, 4))); // at 'a' of delta
        assert_eq!(s.text(&Vec::<String>::new(), &m), "beta\ngamma\ndelta");
        // The head character is inclusive: dropping on the space before
        // "epsilon" takes the space too.
        s.extend(Some(pt(2, 5)));
        assert_eq!(s.text(&Vec::<String>::new(), &m), "beta\ngamma\ndelta ");
    }

    /// protects: a degenerate selection (click without drag) copies exactly
    /// the one character under the cursor.
    /// fails_when: a click silently selects a whole line again.
    /// no_claim: what the product does with a click beyond anchoring.
    #[test]
    fn click_without_drag_selects_one_char() {
        let mut s = SelectionState::new();
        let mut m = VisualMap::new();
        m.push_mapped(0, "hello".to_string());
        s.anchor(Some(pt(0, 1)));
        assert_eq!(s.text(&Vec::<String>::new(), &m), "e");
    }

    // ---- seams ----

    /// protects: soft-wrap seams rejoin without a fabricated line break —
    /// a whitespace wrap re-inserts the space it ate, a char-run wrap glues
    /// tight, and hard breaks keep exactly one `\n`.
    /// fails_when: a wrapped sentence copies with a mid-sentence `\n`, or the
    /// wrap's eaten space vanishes and words glue ("helloworld").
    /// no_claim: which width the wrap chose to break at.
    #[test]
    fn seams_rejoin_soft_wraps() {
        let mut s = SelectionState::new();
        let mut m = VisualMap::new();
        // One logical line "hello worldwide" wrapped at width 5: the wrap
        // after "hello" ate a breaking space (join with " "), the wrap inside
        // "worldwide" ate nothing (join with ""); "hard" is a second logical
        // line — a hard break.
        m.push_seamed(0, "hello".to_string(), Seam::Break);
        m.push_seamed(0, "world".to_string(), Seam::Space);
        m.push_seamed(0, "wide".to_string(), Seam::Glue);
        m.push_seamed(1, "hard".to_string(), Seam::Break);
        s.anchor(Some(pt(0, 0)));
        s.extend(Some(pt(3, 4)));
        assert_eq!(s.text(&Vec::<String>::new(), &m), "hello worldwide\nhard");
    }

    /// protects: rows of DIFFERENT output lines never rejoin at a soft seam,
    /// and rows without a recorded seam fall back to `\n`.
    /// fails_when: a seam leaks across an output-line boundary and eats a
    /// hard break. no_claim: the renderer's seam bookkeeping.
    #[test]
    fn seam_never_leaks_across_output_lines() {
        let mut s = SelectionState::new();
        let mut m = VisualMap::new();
        m.push_seamed(0, "one".to_string(), Seam::Break);
        m.push_seamed(1, "two".to_string(), Seam::Glue); // same-line rule fails
        s.anchor(Some(pt(0, 0)));
        s.extend(Some(pt(1, 2)));
        assert_eq!(s.text(&Vec::<String>::new(), &m), "one\ntwo");
    }

    // ---- selected_cols ----

    /// protects: the highlight interval is cell-precise — the first row from
    /// the anchor column, the last row to the head column, middle rows whole.
    /// fails_when: every selected row is painted end to end (the old
    /// whole-line highlight). no_claim: how spans are split while painting.
    #[test]
    fn selected_cols_cuts_the_end_rows() {
        let mut s = SelectionState::new();
        s.anchor(Some(pt(2, 3)));
        s.extend(Some(pt(4, 2)));
        assert_eq!(s.selected_cols(2, 10, "abcdefg"), Some((3, usize::MAX)));
        assert_eq!(s.selected_cols(3, 10, "xy"), Some((0, usize::MAX)));
        assert_eq!(s.selected_cols(4, 10, "abcdefg"), Some((0, 3)));
        assert_eq!(s.selected_cols(1, 10, "abcdefg"), None);
        assert_eq!(s.selected_cols(5, 10, "abcdefg"), None);
    }

    // ---- word runs ----

    /// protects: double-click picks exactly the run under the cursor, one
    /// class per run (word / CJK / space / punctuation).
    /// fails_when: a run crosses classes (a word swallows its trailing dot).
    /// no_claim: CJK dictionary segmentation (runs only, by design).
    #[test]
    fn word_run_picks_one_class_per_run() {
        assert_eq!(
            word_run("foo.bar baz", 1),
            (0, 3),
            "word run stops at the dot"
        );
        assert_eq!(word_run("foo.bar baz", 3), (3, 4), "punctuation run");
        assert_eq!(word_run("foo.bar baz", 5), (4, 7), "word after the dot");
        assert_eq!(word_run("foo.bar baz", 7), (7, 8), "space run");
        // 你好世界 is one CJK run; the ASCII tail is a word run.
        let mixed = "你好世界 abc";
        assert_eq!(word_run(mixed, 2), (0, 8), "CJK run takes the whole run");
        assert_eq!(word_run(mixed, 9), (9, 12), "ASCII run after the space");
        // Underscore joins a word run.
        assert_eq!(word_run("a_b.c", 2), (0, 3));
    }

    // ---- click chain ----

    /// protects: presses on one cell inside the 500 ms window count up to a
    /// triple click; a different cell or a window gap restarts at 1.
    /// fails_when: a slow second click becomes a double click, or a click on
    /// another cell continues the chain. no_claim: what the product maps each
    /// count to (dispatch is the product layer's).
    #[test]
    fn click_chain_counts_within_the_window() {
        let t0 = Instant::now();
        let mut chain = ClickChain::new();
        assert_eq!(chain.click((3, 4), t0), 1);
        assert_eq!(
            chain.click((3, 4), t0 + Duration::from_millis(100)),
            2,
            "same cell inside the window"
        );
        assert_eq!(chain.click((3, 4), t0 + Duration::from_millis(250)), 3);
        assert_eq!(
            chain.click((3, 4), t0 + Duration::from_millis(350)),
            3,
            "clamped"
        );
        // A window gap restarts.
        assert_eq!(
            chain.click((3, 4), t0 + Duration::from_millis(1000)),
            1,
            "500 ms later restarts the count"
        );
        // A different cell restarts, even well inside the window.
        assert_eq!(
            chain.click((3, 4), t0 + Duration::from_millis(1100)),
            2,
            "same cell continues the fresh chain"
        );
        assert_eq!(
            chain.click((9, 9), t0 + Duration::from_millis(1150)),
            1,
            "different cell restarts"
        );
    }

    /// protects: `press` classifies single/double/triple through the chain
    /// and a miss resets it.
    /// fails_when: classification lives outside the state machine and drifts
    /// from it. no_claim: the selection each kind produces (product dispatch).
    #[test]
    fn press_classifies_the_chain() {
        let mut s = SelectionState::new();
        let t0 = Instant::now();
        assert_eq!(s.press(Some(pt(1, 1)), t0), PressKind::Single);
        assert_eq!(
            s.press(Some(pt(1, 1)), t0 + Duration::from_millis(100)),
            PressKind::Double
        );
        assert_eq!(
            s.press(Some(pt(1, 1)), t0 + Duration::from_millis(200)),
            PressKind::Triple
        );
        // A miss resets: the next press is a single again.
        s.press(None, t0 + Duration::from_millis(250));
        assert_eq!(
            s.press(Some(pt(1, 1)), t0 + Duration::from_millis(300)),
            PressKind::Single
        );
    }

    // ---- existing state-machine behavior, rewritten to the new model ----

    #[test]
    fn drag_selects_and_text_joins() {
        let mut s = SelectionState::new();
        let mut m = VisualMap::new();
        for t in lines(10) {
            m.push_mapped(m.len(), t);
        }
        s.anchor(Some(pt(2, 0)));
        s.extend(Some(pt(5, 99)));
        assert_eq!(
            s.selection,
            Some(Selection {
                anchor: pt(2, 0),
                head: pt(5, 99)
            })
        );
        assert_eq!(
            s.text(&Vec::<String>::new(), &m),
            "line 2\nline 3\nline 4\nline 5"
        );
    }

    #[test]
    fn drag_up_normalizes_bounds() {
        let mut s = SelectionState::new();
        let mut m = VisualMap::new();
        for t in lines(10) {
            m.push_mapped(m.len(), t);
        }
        s.anchor(Some(pt(5, 99)));
        s.extend(Some(pt(2, 0)));
        assert!(s.is_selected(3, 10));
        assert_eq!(
            s.text(&Vec::<String>::new(), &m),
            "line 2\nline 3\nline 4\nline 5"
        );
    }

    #[test]
    fn anchor_outside_transcript_clears() {
        let mut s = SelectionState::new();
        s.anchor(Some(pt(0, 0)));
        assert!(s.selection.is_some());
        s.anchor(None); // click below the pane
        assert!(s.selection.is_none());
    }

    #[test]
    fn text_clamps_stale_indices() {
        let mut s = SelectionState::new();
        s.anchor(Some(pt(0, 0)));
        s.extend(Some(pt(5, 0)));
        assert_eq!(s.text(&lines(1), &empty_map()), "line 0");
    }

    #[test]
    fn open_menu_requires_selection() {
        let mut s = SelectionState::new();
        s.open_menu(3, 4);
        assert!(!s.menu_is_open());
        s.anchor(Some(pt(0, 0)));
        s.open_menu(3, 4);
        assert_eq!(
            s.context_menu(),
            Some(&ContextMenu {
                x: 3,
                y: 4,
                selected: 0
            })
        );
    }

    #[test]
    fn menu_move_and_close() {
        let mut s = SelectionState::new();
        s.anchor(Some(pt(0, 0)));
        s.open_menu(0, 0);
        s.menu_move(1);
        assert_eq!(s.menu_selected(), 1);
        s.menu_move(1);
        assert_eq!(s.menu_selected(), 1, "clamped at last item");
        s.menu_move(-1);
        assert_eq!(s.menu_selected(), 0);
        s.context_menu = None;
        assert!(!s.menu_is_open());
    }

    #[test]
    fn menu_click_hit_test() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut s = SelectionState::new();
        s.anchor(Some(pt(0, 0)));
        s.open_menu(0, 0);
        // Click the copy row (row y+1) -> Copy.
        assert_eq!(
            s.menu_click(MouseEventKind::Down(MouseButton::Left), 1, 1, 20, 20),
            Some(MenuClick::Copy)
        );
        assert!(!s.menu_is_open());
        // Re-open; click the dismiss row (y+2) -> Dismiss.
        s.open_menu(0, 0);
        assert_eq!(
            s.menu_click(MouseEventKind::Down(MouseButton::Left), 1, 2, 20, 20),
            Some(MenuClick::Dismiss)
        );
        // Click far outside the popup → None (falls through to selection).
        s.open_menu(0, 0);
        assert_eq!(
            s.menu_click(MouseEventKind::Down(MouseButton::Left), 15, 15, 20, 20),
            None
        );
        assert!(s.menu_is_open());
    }

    /// protects: `select_range` (the word/line picks) sets exactly the span
    /// asked for and closes any open menu.
    /// fails_when: the pick drifts to whole rows or leaves the menu open.
    /// no_claim: which cells the product asked for.
    #[test]
    fn select_range_sets_the_exact_span() {
        let mut s = SelectionState::new();
        let mut m = VisualMap::new();
        m.push_mapped(0, "hello world".to_string());
        s.open_menu(0, 0);
        s.select_range(pt(0, 6), pt(0, 10));
        assert!(!s.menu_is_open());
        assert_eq!(s.text(&Vec::<String>::new(), &m), "world");
    }
}

//! Visual-row mapping: renderer row bookkeeping + block-aware scroll anchoring.
//!
//! A renderer flattens committed [`crate::lines::OutputLine`]s into *visual*
//! rows (markdown expansion, detail blocks, wrapping): one output line can
//! own a whole block of rows, and some rows belong to no output line at all
//! (streaming tail, live panels). `VisualMap` keeps that structure as one
//! aligned pair of arrays and owns the pure mechanics on top of it:
//!
//! - **anchor arithmetic** — re-resolve the window head across reflows while
//!   preserving its offset INSIDE a multi-row block (fold/expand, resize
//!   re-wrap). Resolving to the block's first row instead flings the view a
//!   whole block upward and undoes every one-row scroll-down;
//! - **selection text** — the plain-text twin of each row, for copy.
//!
//! Row content (which rows exist, their styling, their wording) stays with the
//! product renderer; this type only keeps the books. Products push rows in
//! render order and read the mapping back for hit-testing, anchoring and copy.

/// Output index for rows with no committed output line: streaming tail, live
/// thought rows, panel placeholders. Not selectable, not anchorable.
pub const UNSELECTABLE: usize = usize::MAX;

/// Where the window head sat inside its block on the previous frame: the
/// owning output line's index plus the head's row offset within that line's
/// visual block (`0` = the block's first row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockAnchor {
    pub output: usize,
    pub intra: usize,
}

/// The visual→output mapping one render pass produces.
#[derive(Debug, Default, Clone)]
pub struct VisualMap {
    to_output: Vec<usize>,
    texts: Vec<String>,
}

impl VisualMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(n: usize) -> Self {
        Self {
            to_output: Vec::with_capacity(n),
            texts: Vec::with_capacity(n),
        }
    }

    /// Append one visual row belonging to committed output line `output_idx`.
    pub fn push_mapped(&mut self, output_idx: usize, text: String) {
        self.to_output.push(output_idx);
        self.texts.push(text);
    }

    /// Append one visual row that belongs to no output line (streaming tail,
    /// live panel placeholder): [`UNSELECTABLE`], never anchors selection.
    pub fn push_unselectable(&mut self, text: String) {
        self.push_mapped(UNSELECTABLE, text);
    }

    pub fn len(&self) -> usize {
        self.texts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.texts.is_empty()
    }

    /// The output index a visual row maps to (`None` out of range).
    pub fn output_at(&self, visual_idx: usize) -> Option<usize> {
        self.to_output.get(visual_idx).copied()
    }

    /// Plain text of every row, in visual order (copy source).
    pub fn texts(&self) -> &[String] {
        &self.texts
    }

    /// Capture where the window head sits: its output line + offset inside
    /// that line's visual block. `None` for unselectable heads (streaming
    /// tails, panel placeholders) and out-of-range heads — the caller falls
    /// back to index-holding (right for tail-only growth).
    pub fn anchor_at(&self, head: usize) -> Option<BlockAnchor> {
        let out = *self.to_output.get(head)?;
        if out == UNSELECTABLE {
            return None;
        }
        let block_top = self.to_output[..head]
            .iter()
            .rposition(|&i| i != out)
            .map(|p| p + 1)
            .unwrap_or(0);
        Some(BlockAnchor {
            output: out,
            intra: head - block_top,
        })
    }

    /// Re-resolve a [`BlockAnchor`] against THIS map (the post-reflow one):
    /// same output line, same offset inside its block — clamped when the
    /// block shrank (fold), exact when unchanged (a strict no-op), which is
    /// what keeps one-row scrolls one row long. `None` when the line is gone
    /// or unmapped: the caller falls back to index-holding.
    pub fn resolve_anchor(&self, anchor: BlockAnchor) -> Option<usize> {
        if anchor.output == UNSELECTABLE {
            return None;
        }
        let top = self.to_output.iter().position(|&i| i == anchor.output)?;
        let block_len = self.to_output[top..]
            .iter()
            .take_while(|&i| *i == anchor.output)
            .count();
        Some(top + anchor.intra.min(block_len.saturating_sub(1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` rows all belonging to output line `out`, preceded by one row of
    /// output line 0 so the block does not start at the very top.
    fn map_with_block(n: usize, out: usize) -> VisualMap {
        let mut m = VisualMap::new();
        m.push_mapped(0, "lead".into());
        for i in 0..n {
            m.push_mapped(out, format!("block row {i}"));
        }
        m
    }

    #[test]
    fn push_keeps_texts_and_indices_aligned() {
        let mut m = VisualMap::new();
        m.push_mapped(7, "a".into());
        m.push_unselectable("b".into());
        assert_eq!(m.len(), 2);
        assert_eq!(m.output_at(0), Some(7));
        assert_eq!(m.output_at(1), Some(UNSELECTABLE));
        assert_eq!(m.texts(), ["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn anchor_resolve_is_noop_on_unchanged_map() {
        let m = map_with_block(5, 1);
        // Head mid-block: lead row + 3 rows into the 5-row block.
        let a = m.anchor_at(4).expect("mid-block head anchors");
        assert_eq!(a, BlockAnchor { output: 1, intra: 3 });
        assert_eq!(m.resolve_anchor(a), Some(4), "unchanged map: strict no-op");
    }

    #[test]
    fn intra_block_offset_survives_reflow() {
        let before = map_with_block(5, 1);
        let a = before.anchor_at(4).expect("anchors"); // intra = 3

        // After an expand: 2 rows grew ABOVE, the block is now 8 rows.
        let mut after = VisualMap::new();
        for i in 0..3 {
            after.push_mapped(0, format!("above {i}"));
        }
        for i in 0..8 {
            after.push_mapped(1, format!("block row {i}"));
        }
        // top = 3, intra = 3 -> head at 6, NOT the block top at 3.
        assert_eq!(after.resolve_anchor(a), Some(6));
    }

    #[test]
    fn resolve_clamps_when_block_shrinks() {
        let before = map_with_block(5, 1);
        let a = before.anchor_at(4).expect("anchors"); // intra = 3

        // Folded: the same output line is now a single summary row.
        let mut after = VisualMap::new();
        after.push_mapped(0, "lead".into());
        after.push_mapped(1, "summary".into());
        assert_eq!(after.resolve_anchor(a), Some(1), "clamps into the 1-row block");
    }

    #[test]
    fn unselectable_rows_never_anchor() {
        let mut m = VisualMap::new();
        m.push_unselectable("tail".into());
        m.push_mapped(3, "real".into());
        assert_eq!(m.anchor_at(0), None);
        assert_eq!(
            m.resolve_anchor(BlockAnchor {
                output: UNSELECTABLE,
                intra: 0
            }),
            None
        );
        // Mapped row still anchors normally.
        assert!(m.anchor_at(1).is_some());
    }

    #[test]
    fn resolve_is_none_when_output_line_disappears() {
        let before = map_with_block(5, 1);
        let a = before.anchor_at(4).expect("anchors");
        let mut after = VisualMap::new();
        after.push_mapped(0, "lead".into());
        after.push_mapped(2, "other".into());
        assert_eq!(after.resolve_anchor(a), None);
    }

    #[test]
    fn single_row_block_resolves_to_its_row() {
        let mut m = VisualMap::new();
        m.push_mapped(0, "a".into());
        m.push_mapped(1, "b".into());
        let a = m.anchor_at(1).expect("anchors");
        assert_eq!(a, BlockAnchor { output: 1, intra: 0 });
        assert_eq!(m.resolve_anchor(a), Some(1));
    }
}

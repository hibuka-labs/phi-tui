//! Pure transcript→row algorithms: block rhythm and the fold preview ladder.
//!
//! No styles, no copy, no agent semantics. The product decides what a line
//! means, what its meta row says ([`MetaHead`](crate::lines::MetaHead)), and
//! how many payload rows a tier wants on screen; these functions only decide
//! where the blank spacer rows go and which rows of a folded payload are
//! still owed the reader.

use crate::lines::{LineKind, MetaHead, OutputLine};

/// Should `line` open a new visual block — that is, does a blank spacer row
/// belong between `prev` and `line`?
///
/// The transcript's rhythm is block-scoped: blank rows separate *blocks*,
/// never rows inside one. A block's continuation rows are identified by
/// carrying no `original` of their own (see [`OutputLine::original`]) while
/// sharing the previous row's kind; anything else starts a block.
pub fn opens_block<S>(prev: &OutputLine<S>, line: &OutputLine<S>) -> bool {
    match line.kind {
        // A result row belongs to its invocation. Consecutive results
        // (parallel tools) stay one run so the attachments read as a group.
        // A result that folds its payload is still *that call's* result — the
        // fold is presentation, not a new subject — so this arm runs before
        // the structured-block rule below.
        LineKind::ToolResult => !matches!(prev.kind, LineKind::Tool | LineKind::ToolResult),
        // A denial sitting directly under its call is the call's own outcome;
        // otherwise (turn error, warning) it is its own block.
        LineKind::Error => prev.kind != LineKind::Tool,
        kind => {
            // A structured block (diff, folded payload) is whole in itself.
            if line.detail.is_some() {
                return true;
            }
            let multi_row = matches!(
                kind,
                LineKind::User
                    | LineKind::Normal
                    | LineKind::Thought
                    | LineKind::System
                    | LineKind::Plan
            );
            !(multi_row && line.original.is_none() && prev.kind == kind)
        }
    }
}

/// How much of a folded payload a tier puts on screen — the ladder's rungs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preview {
    /// A fixed preview budget of payload rows.
    Rows(usize),
    /// The whole payload. Full expansion also restores a truncated head, so
    /// "expand" can never still be hiding text.
    All,
}

/// Which payload rows of a [`LineDetail::Folded`] block a tier still owes the
/// reader. Pure row bookkeeping — wrapping, indenting and painting stay with
/// the caller.
#[derive(Debug, PartialEq, Eq)]
pub struct FoldBody<'a> {
    /// `raw`'s first line, when the meta row shows it *truncated* and the
    /// caller is showing the whole payload. Draw it **before** [`Self::rows`]
    /// so the expanded block keeps the payload's own order (order may never
    /// change).
    pub restore_head: Option<&'a str>,
    /// Payload rows the meta row does not already display, up to `shown`.
    pub rows: Vec<&'a str>,
    /// Rows beyond the preview budget — what a `... +N lines` tail counts.
    pub hidden: usize,
}

/// Fold preview ladder: split a folded payload into what a tier shows and
/// what it hides.
///
/// When the meta row already displays the first line ([`MetaHead::Whole`] /
/// [`MetaHead::Abbreviated`]) the body starts one row later — the first line
/// is content, and content is never drawn twice.
pub fn folded_body<'a>(raw: &'a str, meta_head: MetaHead, preview: Preview) -> FoldBody<'a> {
    let rows: Vec<&str> = raw.lines().collect();
    let meta_owns_head = meta_head != MetaHead::None && !rows.is_empty();
    let body = if meta_owns_head {
        &rows[1..]
    } else {
        rows.as_slice()
    };
    let take = match preview {
        Preview::Rows(n) => n.min(body.len()),
        Preview::All => body.len(),
    };
    let hidden = body.len() - take;
    let restore_head = match (meta_head, preview) {
        (MetaHead::Abbreviated, Preview::All) => rows.first().copied(),
        _ => None,
    };
    FoldBody {
        restore_head,
        rows: body[..take].to_vec(),
        hidden,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lines::{LineDetail, OutputLine};

    fn line(kind: LineKind, text: &str) -> OutputLine {
        OutputLine::new(text, kind)
    }

    #[test]
    fn tool_result_groups_under_its_call() {
        let call = line(LineKind::Tool, "* spawn_agent a");
        let result = line(LineKind::ToolResult, "  < done");
        assert!(!opens_block(&call, &result));
        // ... and consecutive results stay one run.
        let result2 = line(LineKind::ToolResult, "  < done");
        assert!(!opens_block(&result, &result2));
        // A result after prose is its own block.
        let prose = line(LineKind::Normal, "hello");
        assert!(opens_block(&prose, &result));
    }

    #[test]
    fn error_under_a_call_is_the_calls_outcome() {
        let call = line(LineKind::Tool, "* edit_file");
        let denied = line(LineKind::Error, "!! denied");
        assert!(!opens_block(&call, &denied));
        let prose = line(LineKind::Normal, "hello");
        assert!(opens_block(&prose, &denied));
    }

    #[test]
    fn continuation_rows_stay_inside_their_block() {
        let mut first = line(LineKind::Normal, "one");
        first.original = Some("one two".into());
        let mut second = line(LineKind::Normal, "two");
        // No `original` of its own → continuation.
        second.original = None;
        assert!(!opens_block(&first, &second));
        // A fresh block (its own `original`) does open.
        let mut third = line(LineKind::Normal, "three");
        third.original = Some("three".into());
        assert!(opens_block(&second, &third));
    }

    #[test]
    fn folded_payload_always_opens_a_block() {
        let prose = line(LineKind::Normal, "hello");
        let mut fold = line(LineKind::ToolResult, "  < head");
        fold.detail = Some(LineDetail::Folded {
            raw: "head\nbody".into(),
            line_count: 2,
            char_count: 9,
            meta_head: MetaHead::Whole,
        });
        assert!(opens_block(&prose, &fold));
    }

    #[test]
    fn body_skips_the_head_the_meta_shows() {
        let raw = "head\na\nb";
        let whole = folded_body(raw, MetaHead::Whole, Preview::All);
        assert_eq!(whole.rows, vec!["a", "b"]);
        assert_eq!(whole.hidden, 0);
        assert_eq!(whole.restore_head, None);

        let none = folded_body(raw, MetaHead::None, Preview::All);
        assert_eq!(none.rows, vec!["head", "a", "b"]);
    }

    #[test]
    fn shown_budget_reports_the_hidden_tail() {
        let raw = "head\na\nb\nc";
        let fold = folded_body(raw, MetaHead::Whole, Preview::Rows(2));
        assert_eq!(fold.rows, vec!["a", "b"]);
        assert_eq!(fold.hidden, 1);
        // A budget past the end is clamped, never more hidden than there is.
        let all = folded_body(raw, MetaHead::Whole, Preview::Rows(99));
        assert_eq!(all.rows.len(), 3);
        assert_eq!(all.hidden, 0);
    }

    #[test]
    fn abbreviated_head_returns_only_at_full_expansion() {
        let raw = "a very long head line\nbody";
        let partial = folded_body(raw, MetaHead::Abbreviated, Preview::Rows(1));
        assert_eq!(partial.restore_head, None);
        assert_eq!(partial.rows, vec!["body"]);

        // Even a tiny payload is "full expansion" only at `Preview::All` —
        // a budget that happens to cover the body must not restore the head,
        // or the preview tiers would grow inconsistent with each other.
        let lucky = folded_body(raw, MetaHead::Abbreviated, Preview::Rows(99));
        assert_eq!(lucky.restore_head, None);

        let full = folded_body(raw, MetaHead::Abbreviated, Preview::All);
        assert_eq!(full.restore_head, Some("a very long head line"));
        assert_eq!(full.rows, vec!["body"]);
    }

    #[test]
    fn truncated_head_is_not_restored_when_the_body_is_empty() {
        // A one-line payload abbreviated into the meta row: every tier shows
        // just the meta row; only full expansion pays back the full line.
        let raw = "a very long head line that never fit";
        let compact = folded_body(raw, MetaHead::Abbreviated, Preview::Rows(0));
        assert_eq!(compact.restore_head, None);
        assert!(compact.rows.is_empty());

        let full = folded_body(raw, MetaHead::Abbreviated, Preview::All);
        assert_eq!(full.restore_head, Some("a very long head line that never fit"));
    }

    #[test]
    fn empty_payload_is_empty_everywhere() {
        let fold = folded_body("", MetaHead::Whole, Preview::All);
        assert!(fold.rows.is_empty());
        assert_eq!(fold.hidden, 0);
        assert_eq!(fold.restore_head, None);
    }
}

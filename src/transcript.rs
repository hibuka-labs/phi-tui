//! Committed transcript buffer: output lines, the plan block, and re-wrapping.
//!
//! Extracted from the `App` god-object (previously `output`/`plan_range`/
//! `wrap_width` + `push_system`/`push_banner`/`push_user`/`rewrap_output`).
//! The owner owns one instance: runtime events commit lines into it, and the
//! renderer reads them back for display / selection / copy.
//!
//! `S` is the style token carried by styled lines (see [`OutputLine`]); the
//! transcript itself is style-token agnostic.
//!
//! Generic capability note: `replace_plan` / `clear_plan` implement a
//! "tracked, replaceable block" — a contiguous range of lines that a later
//! full-replace removes so the fresh block can re-append at the tail, and
//! that a turn boundary can reset so the next block appends. `LineKind::Plan`
//! (the update_plan tool's product semantics) is one consumer of this
//! capability; the semantics of *what* a block means stay with the caller.
//! Each removal shifts the output indices below it, so the shifts are
//! recorded ([`Transcript::take_index_moves`]) for cross-frame row identity
//! (scroll anchors — see `crate::visual::BlockAnchor::translate`).

use std::collections::VecDeque;
use std::ops::Range;

use crate::lines::{LineKind, OutputLine};
use crate::wrap::wrap;

/// Display rows a wrapped block occupies at `width`: the wrap itself, plus
/// the per-row prefix the producers commit (User lines carry `> ` on the head
/// row and an indented tail). The transcript stores exactly these rows — head
/// with `original`, tail without — so re-wrap can recognize the previous
/// wrap's tail by exact match instead of guessing from row shape.
fn wrapped_display_rows(original: &str, width: usize, kind: LineKind) -> Vec<String> {
    wrap(original, width)
        .into_iter()
        .enumerate()
        .map(|(j, wrapped)| {
            if kind == LineKind::User {
                if j == 0 {
                    format!("> {wrapped}")
                } else {
                    format!("  {wrapped}")
                }
            } else {
                wrapped
            }
        })
        .collect()
}

/// Default columns the output buffer wraps at. Overridden at runtime to match
/// the terminal width (see [`Transcript::set_wrap_width`]).
pub const DEFAULT_WRAP_WIDTH: usize = 100;

/// The committed side of the transcript: every `OutputLine` the owner's event
/// stream (and user turns) have produced, plus the tracked anchored block and
/// the current wrap width.
#[derive(Debug)]
pub struct Transcript<S = ()> {
    pub output: Vec<OutputLine<S>>,
    /// Start of the current anchored block (the plan block) in `output`. The
    /// block's contract is "full block replaces previous": a later update
    /// removes this range and appends the fresh block at the tail (the plan
    /// must stay visible at the flow end), and tracks the new position here.
    /// Cleared at each turn start.
    plan_range: Option<Range<usize>>,
    /// Output-index ranges removed since the last [`Self::take_index_moves`],
    /// each in the numbering current at its splice, chronological. A
    /// consumer that captured a row identity before these splices (the
    /// render seam's scroll anchor) translates it back to life through them.
    index_moves: Vec<Range<usize>>,
    /// Dynamic wrap width for output lines, updated each frame to match the
    /// terminal width. Keeps output and composer widths aligned.
    wrap_width: usize,
}

impl<S> Transcript<S> {
    pub fn new() -> Self {
        Self {
            output: Vec::new(),
            plan_range: None,
            index_moves: Vec::new(),
            wrap_width: DEFAULT_WRAP_WIDTH,
        }
    }

    pub fn len(&self) -> usize {
        self.output.len()
    }

    pub fn is_empty(&self) -> bool {
        self.output.is_empty()
    }

    /// Append one committed line.
    pub fn push(&mut self, line: OutputLine<S>) {
        self.output.push(line);
    }

    /// Append committed lines (e.g. flushed streaming text).
    pub fn extend(&mut self, lines: Vec<OutputLine<S>>) {
        self.output.extend(lines);
    }

    /// Current output wrap width (columns).
    pub fn wrap_width(&self) -> usize {
        self.wrap_width
    }

    /// Update the output wrap width. When the width changes, committed output
    /// lines with an `original` source are re-wrapped at the new width.
    pub fn set_wrap_width(&mut self, width: usize) {
        let width = width.max(1);
        if width == self.wrap_width {
            return;
        }
        let old_width = self.wrap_width;
        self.wrap_width = width;
        self.rewrap_output(old_width);
    }

    /// Append a system/banner line (welcome, workspace, log path).
    pub fn push_system(&mut self, text: &str) {
        for (i, line) in wrapped_display_rows(text, self.wrap_width, LineKind::System)
            .into_iter()
            .enumerate()
        {
            self.output.push(OutputLine {
                spans: None,
                original: if i == 0 { Some(text.to_string()) } else { None },
                detail: None,
                text: line,
                kind: LineKind::System,
                tool_state: None,
            });
        }
    }

    /// Append one pre-styled line verbatim (a generic "styled header" entry,
    /// e.g. a startup banner's runs): **no** soft-wrap (the wordmark must stay
    /// whole; ratatui clips on narrow terminals), no re-wrap on width change
    /// (`original: None`). The plain `text` keeps copy/selection style-blind;
    /// styled runs arrive as byte-range `spans`, resolved by the renderer.
    pub fn push_styled_line(&mut self, text: String, spans: Vec<crate::lines::SpanSpec<S>>) {
        self.output.push(OutputLine {
            text,
            kind: LineKind::System,
            spans: Some(spans),
            original: None,
            detail: None,
            tool_state: None,
        });
    }

    /// Echo the user's submitted message into the transcript, so the frame log
    /// and output buffer retain the human side of the conversation (the JSONL
    /// turn log already stores `user_input`, but the rendered transcript didn't
    /// show it). `>` on the first line, indented continuations after.
    pub fn push_user(&mut self, text: &str) {
        for (i, display) in wrapped_display_rows(text, self.wrap_width, LineKind::User)
            .into_iter()
            .enumerate()
        {
            self.output.push(OutputLine {
                spans: None,
                original: if i == 0 { Some(text.to_string()) } else { None },
                detail: None,
                text: display,
                kind: LineKind::User,
                tool_state: None,
            });
        }
    }

    /// Forget the tracked plan block. Called at each turn start so the next
    /// plan appends instead of replacing the previous turn's block.
    pub fn clear_plan(&mut self) {
        self.plan_range = None;
    }

    /// Replace the previous anchored block (the plan block): the old block is
    /// removed and the fresh one appended at the tail — a plan update moves
    /// the plan to the flow end so it stays visible (the caller always sends
    /// the full block). When no block is tracked yet, the new block is
    /// appended (first-block behavior, unchanged).
    ///
    /// The removal shifts every output index below the old block, so it is
    /// recorded for [`Self::take_index_moves`] — scroll anchors captured
    /// before this splice translate through those moves instead of jumping.
    pub fn replace_plan(&mut self, block: Vec<OutputLine<S>>) {
        let new_len = block.len();
        if let Some(range) = self.plan_range.take() {
            self.output.drain(range.clone());
            self.index_moves.push(range);
        }
        let start = self.output.len();
        self.output.extend(block);
        self.plan_range = Some(start..start + new_len);
    }

    /// Drain the output-index ranges removed since the last call (each in
    /// the numbering current at its splice, chronological). The render seam
    /// calls this once per frame and translates its scroll anchor through
    /// them (see `crate::visual::BlockAnchor::translate`).
    pub fn take_index_moves(&mut self) -> Vec<Range<usize>> {
        std::mem::take(&mut self.index_moves)
    }

    /// Re-wrap all committed output lines that have an `original` source.
    /// Called when the terminal width changes so output lines follow the new
    /// width instead of staying at the old wrap width.
    ///
    /// A wrapped block is a head row (`original: Some`) plus the tail rows its
    /// previous wrap produced (`original: None`, always directly after the
    /// head). The tail is display residue of the old width: regenerating the
    /// block at the new width replaces it, so it must be dropped first or the
    /// transcript keeps stale word-mid fragments below the fresh rows (the
    /// long-notice glitch — a full-width wrapped notice with a duplicate
    /// fragment row underneath, recurring on every resize). Tail rows are
    /// recognized by exact match against the wrap at `old_width` and bounded
    /// by that wrap's row count, so a standalone row that merely sits next to
    /// a head (same kind, `original: None`) is never eaten.
    fn rewrap_output(&mut self, old_width: usize) {
        let mut rest: VecDeque<OutputLine<S>> = std::mem::take(&mut self.output).into();
        let mut new: Vec<OutputLine<S>> = Vec::with_capacity(rest.len());
        while let Some(line) = rest.pop_front() {
            let Some(original) = line.original.clone() else {
                new.push(line);
                continue;
            };
            if original.is_empty() || line.spans.is_some() {
                // Banner lines or empty originals: keep as-is.
                new.push(line);
                continue;
            }
            // Normal AI prose: keep as a single line; the renderer handles
            // markdown parsing and wrapping at display time.
            if line.kind == LineKind::Normal {
                new.push(line);
                continue;
            }
            let kind = line.kind;
            // Drop the previous wrap's tail rows before regenerating them.
            let prior = wrapped_display_rows(&original, old_width, kind);
            for expected in prior.iter().skip(1) {
                let is_tail = rest.front().is_some_and(|next| {
                    next.original.is_none()
                        && next.spans.is_none()
                        && next.kind == kind
                        && next.text == *expected
                });
                if !is_tail {
                    break;
                }
                rest.pop_front();
            }
            for (j, display) in wrapped_display_rows(&original, self.wrap_width, kind)
                .into_iter()
                .enumerate()
            {
                new.push(OutputLine {
                    text: display,
                    kind,
                    spans: None,
                    original: if j == 0 { Some(original.clone()) } else { None },
                    detail: None,
                    tool_state: None,
                });
            }
        }
        self.output = new;
    }
}

impl<S> Default for Transcript<S> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lines::LineKind;

    fn texts(t: &Transcript) -> Vec<&str> {
        t.output.iter().map(|l| l.text.as_str()).collect()
    }

    fn plan_line(text: &str) -> OutputLine {
        OutputLine {
            spans: None,
            original: None,
            detail: None,
            text: text.to_string(),
            kind: LineKind::Plan,
            tool_state: None,
        }
    }

    #[test]
    fn push_system_wraps_and_marks_first_line_original() {
        let mut t = Transcript::new();
        t.set_wrap_width(10);
        t.push_system("hello world");
        assert_eq!(texts(&t), vec!["hello worl", "d"]);
        assert_eq!(t.output[0].original.as_deref(), Some("hello world"));
        assert!(t.output[1].original.is_none());
        assert_eq!(t.output[0].kind, LineKind::System);
    }

    #[test]
    fn push_user_prefixes_continuation_lines() {
        let mut t = Transcript::new();
        t.set_wrap_width(10);
        t.push_user("abcdefghijk");
        assert_eq!(texts(&t), vec!["> abcdefghij", "  k"]);
        assert_eq!(t.output[0].original.as_deref(), Some("abcdefghijk"));
    }

    #[test]
    fn set_wrap_width_rewraps_committed_lines() {
        let mut t = Transcript::new();
        t.set_wrap_width(20);
        t.push_user("abcdefghijk");
        assert_eq!(texts(&t), vec!["> abcdefghijk"]);
        // Narrower terminal: the user line re-wraps at the new width.
        t.set_wrap_width(10);
        assert_eq!(texts(&t), vec!["> abcdefghij", "  k"]);
        // Same width again: no-op, lines untouched.
        t.set_wrap_width(10);
        assert_eq!(texts(&t), vec!["> abcdefghij", "  k"]);
    }

    /// protects: a plan update MOVES the block to the tail — the old block's
    /// rows vanish exactly (no residue, no blank row where it sat) and the
    /// fresh block is the flow's last rows; the removal is recorded so a
    /// scroll anchor can translate through it. fails_when: the second block
    /// lands anywhere but the tail, an old row survives, or the shift goes
    /// unrecorded. no_claim: what the plan block's rows say, or where the
    /// viewport sits afterwards.
    #[test]
    fn replace_plan_repastes_at_tail_without_residue() {
        let mut t = Transcript::new();
        t.push_user("hello");
        t.replace_plan(vec![plan_line("📋 A1"), plan_line("📋 A2")]);
        t.push_user("world");
        t.replace_plan(vec![plan_line("📋 B")]);
        // Line-by-line: the old block left nothing behind at its position
        // (between the two user lines) and the new block is the tail.
        assert_eq!(texts(&t), vec!["> hello", "> world", "📋 B"]);
        // The splice that moved the block is the recorded index move: rows
        // 1..3 (the old block, pre-splice numbering) vanished.
        assert_eq!(t.take_index_moves(), vec![1..3]);
        assert!(t.take_index_moves().is_empty(), "moves drain once");
    }

    /// protects: the first block simply appends — no removal happens before
    /// a block is tracked. fails_when: a first plan eats existing lines or
    /// lands anywhere but the tail. no_claim: later updates (covered by the
    /// re-paste test).
    #[test]
    fn replace_plan_first_block_appends() {
        let mut t = Transcript::new();
        t.push_user("hello");
        t.replace_plan(vec![plan_line("📋 A")]);
        assert_eq!(texts(&t), vec!["> hello", "📋 A"]);
    }

    /// protects: a multi-row block moves whole, and a height change between
    /// old and new block (3 rows out, 1 row in) is just delete + append.
    /// fails_when: rows of the old block survive, or the new block's rows
    /// interleave with the old position. no_claim: what any row says.
    #[test]
    fn replace_plan_multiline_block_moves_whole() {
        let mut t = Transcript::new();
        t.replace_plan(vec![
            plan_line("📋 A1"),
            plan_line("📋 A2"),
            plan_line("📋 A3"),
        ]);
        t.push_user("mid");
        t.replace_plan(vec![plan_line("📋 B1")]);
        assert_eq!(texts(&t), vec!["> mid", "📋 B1"]);
    }

    /// protects: a turn boundary resets only the tracking — the previous
    /// turn's block stays where it was, and the next block appends at the
    /// tail (with interleaved content in between). fails_when: the old
    /// block moves, disappears, or the new block replaces it. no_claim:
    /// what triggers the boundary (`clear_plan`'s caller is the turn start).
    #[test]
    fn clear_plan_keeps_the_old_block_and_appends_the_next() {
        let mut t = Transcript::new();
        t.replace_plan(vec![plan_line("📋 A")]);
        t.push_user("next turn");
        t.clear_plan();
        t.replace_plan(vec![plan_line("📋 B")]);
        assert_eq!(texts(&t), vec!["📋 A", "> next turn", "📋 B"]);
    }

    #[test]
    fn normal_lines_are_not_rewrapped() {
        // AI prose keeps one raw line (markdown rendering happens at display
        // time), so a width change must not split it.
        let mut t = Transcript::new();
        t.set_wrap_width(5);
        t.push(OutputLine {
            spans: None,
            original: Some("prose with markdown **bold**".to_string()),
            detail: None,
            text: "prose with markdown **bold**".to_string(),
            kind: LineKind::Normal,
            tool_state: None,
        });
        t.set_wrap_width(20);
        assert_eq!(texts(&t), vec!["prose with markdown **bold**"]);
    }

    #[test]
    fn rewrap_drops_the_previous_wraps_stale_tail() {
        // The long-notice glitch: a block that already spans rows at the old
        // width must not leave its stale tail below the re-wrapped head.
        let mut t = Transcript::new();
        t.set_wrap_width(10);
        t.push_system("0123456789abc");
        assert_eq!(texts(&t), vec!["0123456789", "abc"]);
        // Narrower: exactly the fresh wrap — the old "abc" tail is gone.
        t.set_wrap_width(7);
        assert_eq!(texts(&t), vec!["0123456", "789abc"]);
        // Wider again: the old "789abc" tail must not survive either.
        t.set_wrap_width(13);
        assert_eq!(texts(&t), vec!["0123456789abc"]);
    }

    #[test]
    fn rewrap_drops_the_previous_wraps_stale_tail_on_user_lines() {
        // User rows carry the `> `/`  ` prefix — the tail match has to use the
        // same display rows the producer committed.
        let mut t = Transcript::new();
        t.set_wrap_width(10);
        t.push_user("0123456789ab");
        assert_eq!(texts(&t), vec!["> 0123456789", "  ab"]);
        t.set_wrap_width(8);
        assert_eq!(texts(&t), vec!["> 01234567", "  89ab"]);
    }

    #[test]
    fn rewrap_drops_the_previous_wraps_stale_tail_for_wide_chars() {
        // CJK rows wrap on display width, so the tail fragments split mid-
        // phrase — exactly the acceptance symptom (a full-width notice with a
        // word-mid duplicate fragment underneath).
        let mut t = Transcript::new();
        t.set_wrap_width(10);
        let notice = "版本发布通知：插件插座已就绪，欢迎试用";
        t.push_system(notice);
        let at_10 = wrap(notice, 10);
        assert_eq!(
            texts(&t),
            at_10.iter().map(String::as_str).collect::<Vec<_>>()
        );
        t.set_wrap_width(6);
        let at_6 = wrap(notice, 6);
        assert_eq!(
            texts(&t),
            at_6.iter().map(String::as_str).collect::<Vec<_>>()
        );
    }

    #[test]
    fn rewrap_keeps_standalone_rows_next_to_a_head() {
        // Standalone rows (tool lines, status lines) share a head's kind and
        // carry `original: None` — only the old wrap's exact tail is residue.
        let mut t = Transcript::new();
        t.set_wrap_width(10);
        t.push_system("0123456789abc");
        t.push(OutputLine {
            spans: None,
            original: None,
            detail: None,
            text: "standalone status".to_string(),
            kind: LineKind::System,
            tool_state: None,
        });
        t.set_wrap_width(7);
        assert_eq!(texts(&t), vec!["0123456", "789abc", "standalone status"]);
    }
}

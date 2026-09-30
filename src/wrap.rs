//! Text-wrapping helpers for the TUI: hard-wrap at display columns (CJK-aware)
//! and an incremental accumulator for the streaming tail.
//!
//! Extracted from `app.rs` so the streaming state machine and the renderer can
//! both use them without reaching into the `App` god-object.

use unicode_width::UnicodeWidthChar;

/// Hard-wrap text to `width` display columns: split on existing newlines, then
/// break over-long runs at char boundaries. Each char's width is its terminal
/// column count (ASCII = 1, wide CJK = 2, combining marks = 0), so a line of
/// Chinese wraps at the same visual width as a line of English.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for raw in text.split('\n') {
        if raw.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        let mut col = 0usize;
        for c in raw.chars() {
            let cw = c.width().unwrap_or(0);
            // Break before a char that would overflow the line, unless the line
            // is still empty (a single over-wide char still gets its own line).
            if col + cw > width && !line.is_empty() {
                out.push(std::mem::take(&mut line));
                col = 0;
            }
            line.push(c);
            col += cw;
        }
        out.push(line);
    }
    out
}

/// Collapse whitespace and truncate to a single line of at most `max` chars.
/// ASCII `...` (not `…`): CJK fonts render `…` double-width while callers
/// budget single-width cells — the drift pushes rows off-screen.
pub fn one_line(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        flat.chars().take(max.saturating_sub(3)).collect::<String>() + "..."
    }
}

/// Which end of a string an elision keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elide {
    /// Keep the front: `description prefix...`.
    Head,
    /// Keep the back: `.../bg_wake_tests.rs`. With `sep`, whole `sep`-separated
    /// components are kept (right to left) so the tail stops on a boundary.
    Tail { sep: Option<char> },
}

/// Truncate `s` to at most `max_cols` display columns, marking the cut with
/// ASCII `...` (CJK fonts render `…` double-width — see [`one_line`]).
///
/// Width-aware (CJK = 2 columns, combining marks = 0) and char-boundary safe:
/// the returned string never splits a char and never exceeds `max_cols`. A
/// zero budget yields an empty string — the only thing that fits.
pub fn elide(s: &str, max_cols: usize, mode: Elide) -> String {
    const DOTS: &str = "...";
    const DOTS_W: usize = 3;
    if max_cols == 0 {
        return String::new();
    }
    if display_width(s) <= max_cols {
        return s.to_string();
    }
    if max_cols <= DOTS_W {
        return ".".repeat(max_cols);
    }
    let budget = max_cols - DOTS_W;
    match mode {
        Elide::Head => format!("{}{DOTS}", take_width_prefix(s, budget)),
        Elide::Tail { sep } => format!("{DOTS}{}", take_tail(s, budget, sep)),
    }
}

/// Right-pad `s` with spaces out to `cols` display columns — the padding
/// counterpart to [`elide`], for laying fixed-width table columns out.
///
/// Pads by terminal columns, not chars: `format!("{:<w$}", s)` counts chars,
/// so a CJK cell (2 columns per char) overshoots and shoves the next column
/// out of alignment. Never truncates — pass [`elide`] first if the text must
/// also fit; text already wider than `cols` comes back unchanged.
pub fn pad_cols(s: &str, cols: usize) -> String {
    let used = display_width(s);
    if used >= cols {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(cols - used))
    }
}

/// Terminal columns used by `s`.
fn display_width(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Longest char-boundary prefix of `s` that fits in `budget` columns.
fn take_width_prefix(s: &str, budget: usize) -> &str {
    let mut used = 0usize;
    let mut end = 0usize;
    for (i, c) in s.char_indices() {
        let cw = c.width().unwrap_or(0);
        if used + cw > budget {
            break;
        }
        used += cw;
        end = i + c.len_utf8();
    }
    &s[..end]
}

/// Tail of `s` for the elision, at most `budget` columns. With `sep`, whole
/// components are taken right-to-left (joined by `sep`, leading `sep`
/// included); at least one non-empty component must fit — a trailing `sep`
/// yields an empty final component that would otherwise satisfy the fit check
/// on its own and collapse the row to a bare `sep`. When no component fits,
/// falls back to a plain char-level tail that fills the budget.
fn take_tail(s: &str, budget: usize, sep: Option<char>) -> String {
    if let Some(sep) = sep
        && budget >= 2
    {
        // One column is reserved for the leading `sep` after the dots.
        let comp_budget = budget - 1;
        let mut parts: Vec<&str> = Vec::new();
        let mut used = 0usize;
        for comp in s.split(sep).rev() {
            let extra = display_width(comp)
                + if parts.is_empty() {
                    0
                } else {
                    sep.width().unwrap_or(0)
                };
            if used + extra > comp_budget {
                break;
            }
            used += extra;
            parts.push(comp);
        }
        if parts.iter().any(|p| !p.is_empty()) {
            parts.reverse();
            let mut out = String::from(sep);
            out.push_str(&parts.join(&sep.to_string()));
            return out;
        }
    }
    // Degrade: fill the budget with the plain tail.
    let mut used = 0usize;
    let mut start = s.len();
    for (i, c) in s.char_indices().rev() {
        let cw = c.width().unwrap_or(0);
        if used + cw > budget {
            break;
        }
        used += cw;
        start = i;
    }
    s[start..].to_string()
}

/// Incremental hard-wrap accumulator for the streaming tail.
///
/// The output pane renders the live (uncommitted) streaming text every frame.
/// Re-wrapping the whole tail each frame is O(total) per frame — a perf cliff on
/// long answers (30KB+ of streamed text in debug builds) that manifests as a
/// frozen scroll. This accumulates the wrapped form as deltas arrive, so
/// rendering is O(new bytes) per frame instead of O(total). Its output is
/// byte-for-byte identical to [`wrap`].
#[derive(Debug)]
pub struct WrapCache {
    width: usize,
    /// Wrapped lines; the last element is always the current (partial) line.
    lines: Vec<String>,
    /// Display columns currently used by the last line.
    col: usize,
}

impl WrapCache {
    pub fn new(width: usize) -> Self {
        Self {
            width: width.max(1),
            lines: vec![String::new()],
            col: 0,
        }
    }

    pub fn reset(&mut self) {
        self.lines.clear();
        self.lines.push(String::new());
        self.col = 0;
    }

    pub fn extend(&mut self, text: &str) {
        for c in text.chars() {
            if c == '\n' {
                self.lines.push(String::new());
                self.col = 0;
                continue;
            }
            let cw = c.width().unwrap_or(0);
            // Break before a char that would overflow the line, unless the line
            // is still empty (a single over-wide char still gets its own line) —
            // mirrors `wrap`.
            let last_empty = self.lines.last().map_or(true, |l| l.is_empty());
            if self.col + cw > self.width && !last_empty {
                self.lines.push(String::new());
                self.col = 0;
            }
            self.lines.last_mut().expect("non-empty").push(c);
            self.col += cw;
        }
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_preserves_blank_lines_and_hard_wraps() {
        let lines = wrap("abc\ndefghij", 3);
        assert_eq!(lines, vec!["abc", "def", "ghi", "j"]);
        let empty = wrap("", 3);
        assert_eq!(empty, vec![""]);
    }

    #[test]
    fn wrap_multi_line() {
        let lines = wrap("one\ntwo", 100);
        assert_eq!(lines, vec!["one", "two"]);
    }

    #[test]
    fn wrap_counts_wide_cjk_as_two_columns() {
        // Four wide glyphs are 8 columns; at width 4 they split in half.
        assert_eq!(wrap("你好世界", 4), vec!["你好", "世界"]);
        // A wide glyph straddling the boundary is pushed to the next line.
        assert_eq!(wrap("a你b", 3), vec!["a你", "b"]);
        // ASCII is unchanged: width-1 glyphs wrap exactly as before.
        assert_eq!(wrap("abcdefgh", 3), vec!["abc", "def", "gh"]);
    }

    #[test]
    fn wrap_cache_matches_wrap_whole_and_incremental() {
        let cases = [
            "abc\ndefghij",
            "one\ntwo",
            "你好世界",
            "a你b",
            "abcdefgh",
            "a\n\nb",
            "trailing newline\n",
            "",
            "exactlywidth",
        ];
        for &s in &cases {
            for width in [1usize, 2, 3, 4, 100] {
                let expected = wrap(s, width);
                // Whole-string extend.
                let mut whole = WrapCache::new(width);
                whole.extend(s);
                assert_eq!(
                    whole.lines(),
                    expected.as_slice(),
                    "whole-extend {s:?} @{width}"
                );
                // Char-by-char extend (true incrementality).
                let mut incr = WrapCache::new(width);
                for c in s.chars() {
                    let mut buf = [0u8; 4];
                    incr.extend(c.encode_utf8(&mut buf));
                }
                assert_eq!(
                    incr.lines(),
                    expected.as_slice(),
                    "char-extend {s:?} @{width}"
                );
            }
        }
    }

    #[test]
    fn elide_keeps_short_strings_intact() {
        assert_eq!(elide("src/main.rs", 20, Elide::Head), "src/main.rs");
        assert_eq!(
            elide("src/main.rs", 20, Elide::Tail { sep: Some('/') }),
            "src/main.rs"
        );
        // Exactly at budget: untouched.
        assert_eq!(elide("abcde", 5, Elide::Head), "abcde");
    }

    #[test]
    fn elide_head_keeps_front() {
        assert_eq!(elide("abcdefghij", 8, Elide::Head), "abcde...");
    }

    #[test]
    fn elide_counts_wide_cjk_as_two_columns() {
        // Four wide glyphs = 8 columns; budget 7 = "..." (3) + 2 glyphs (4).
        assert_eq!(elide("你好世界", 7, Elide::Head), "你好...");
        // Interior budget is 2: "ab" fills it exactly, so one 2-wide glyph is dropped
        // whole rather than squeezed into one column.
        assert_eq!(elide("ab你好", 5, Elide::Head), "ab...");
    }

    #[test]
    fn elide_tail_prefers_whole_path_components() {
        let p = "src/ui/handlers/runtime/bg_wake_tests.rs"; // 40 columns
        // 28 = "..." (3) + "/runtime/bg_wake_tests.rs" (25).
        assert_eq!(
            elide(p, 28, Elide::Tail { sep: Some('/') }),
            ".../runtime/bg_wake_tests.rs"
        );
        // 20 = "..." (3) + "/bg_wake_tests.rs" (17).
        assert_eq!(
            elide(p, 20, Elide::Tail { sep: Some('/') }),
            ".../bg_wake_tests.rs"
        );
    }

    #[test]
    fn elide_tail_degrades_when_no_component_fits() {
        let p = "src/ui/handlers/runtime/bg_wake_tests.rs";
        // Budget 12: even the 16-column basename is too wide, so the cut lands
        // mid-component and fills the budget.
        let out = elide(p, 12, Elide::Tail { sep: Some('/') });
        assert_eq!(out, "..._tests.rs");
        assert_eq!(unicode_width::UnicodeWidthStr::width(out.as_str()), 12);
    }

    #[test]
    fn elide_degenerate_budgets_never_panic() {
        assert_eq!(elide("abcdef", 3, Elide::Head), "...");
        assert_eq!(elide("abcdef", 1, Elide::Head), ".");
        assert_eq!(elide("你好", 1, Elide::Tail { sep: None }), ".");
    }

    #[test]
    fn elide_returns_empty_for_a_zero_budget() {
        // A 0-column budget holds nothing. Clamping the budget up to 1 handed
        // back a 1-column `.`, which exceeds `max_cols` and breaks the
        // "never exceeds `max_cols`" contract.
        assert_eq!(elide("abcdef", 0, Elide::Head), "");
        assert_eq!(elide("abcdef", 0, Elide::Tail { sep: Some('/') }), "");
        assert_eq!(elide("", 0, Elide::Head), "");
    }

    #[test]
    fn pad_cols_pads_in_display_columns_not_chars() {
        assert_eq!(pad_cols("ab", 5), "ab   ");
        // 2 CJK chars = 4 columns, so a 5-column cell needs 1 space — not 3.
        assert_eq!(pad_cols("你好", 5), "你好 ");
        // Zero-width text is padded from column 0.
        assert_eq!(pad_cols("", 3), "   ");
        // Already at or over budget: unchanged. `pad_cols` never truncates —
        // that is `elide`'s job, and slicing here would panic mid-char.
        assert_eq!(pad_cols("abcdef", 3), "abcdef");
        assert_eq!(pad_cols("你好你好", 3), "你好你好");
    }

    #[test]
    fn elide_tail_ignores_a_trailing_separator() {
        // A trailing `sep` yields an empty final component costing 0 columns.
        // Letting it alone satisfy the component check collapsed the row to a
        // bare `".../"`; it must fall through to the char-level tail instead.
        assert_eq!(
            elide("superlongname/", 9, Elide::Tail { sep: Some('/') }),
            "...gname/"
        );
        // When a real component does fit, the trailing `sep` is still kept.
        assert_eq!(
            elide("aaaa/bbbb/cccccccc/", 14, Elide::Tail { sep: Some('/') }),
            ".../cccccccc/"
        );
    }
}

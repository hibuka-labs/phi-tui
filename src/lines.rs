//! The transcript line model: the committed output unit and its structured
//! variants, plus the generic styled byte-range.
//!
//! Moved verbatim out of `app.rs` (the state root) so widget modules never
//! need to import the state root — the import direction is locked by the
//! compiler, not by convention.

/// A styled byte-range within a plain-text line. `start`/`len` are byte
/// offsets into the concatenated text, so copy/selection and frame capture
/// stay style-blind.
///
/// `S` is the product's own style token (phimint uses its `BannerStyle`);
/// the line model itself is style-token agnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanSpec<S> {
    pub start: usize,
    pub len: usize,
    pub style: S,
}

/// Visual kind of an output line (mapped to a ratatui style in render.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Normal,
    Thought,
    Plan,
    Tool,
    Done,
    ToolResult,
    Error,
    System,
    Cancelled,
    Approval,
    User,
}

/// Structured detail rendered as a multi-line visual block (inline diff,
/// folded thought segment). Attached to `OutputLine.detail`; `None` renders
/// `text` alone.
#[derive(Debug, Clone)]
pub enum LineDetail {
    /// Inline diff for `edit_file` / `write_file`.
    Diff { path: String, hunks: Vec<DiffHunk> },
    /// A folded thinking segment: full raw text plus counts precomputed at
    /// flush time. The renderer shows a one-line summary by default; the
    /// product's expand toggle re-wraps `raw` at the current width.
    Thought {
        raw: String,
        /// Wrapped line count at flush-time width (cosmetic, shown in the
        /// summary; may drift after resize — expanded rendering re-wraps).
        line_count: usize,
        /// Total characters in `raw` (precomputed for the token estimate in
        /// titles/summaries; never re-derived at render time).
        char_count: usize,
    },
}

/// A diff hunk: a group of related changes with a unified-diff header.
#[derive(Debug, Clone)]
pub struct DiffHunk {
    /// `"@@ -a,b +c,d @@"` header line.
    pub header: String,
    pub lines: Vec<DiffLine>,
    /// Which edit (0-based) this hunk came from. Used to apply real file line offsets.
    pub edit_index: usize,
}

/// A single line within a diff hunk.
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
    /// Line number in the old file (1-based). `None` for Add lines.
    pub old_line: Option<u32>,
    /// Line number in the new file (1-based). `None` for Del lines.
    pub new_line: Option<u32>,
}

/// Whether a diff line is an addition, deletion, or unchanged context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Add,
    Del,
    Context,
}

/// A committed transcript line. `S` is the style token carried in `spans`
/// (default `()` = never styled, which keeps unstyled call sites
/// zero-change); a product pins its own token, e.g. `OutputLine<BannerStyle>`.
#[derive(Debug, Clone)]
pub struct OutputLine<S = ()> {
    pub text: String,
    pub kind: LineKind,
    /// Optional styled byte-ranges into `text` (startup banner runs). `None`
    /// means the whole line takes the `kind` style. `text` is always the plain
    /// concatenation, so copy/selection and the frame capture stay style-blind.
    pub spans: Option<Vec<SpanSpec<S>>>,
    /// The original (unwrapped) text for this logical block. When the terminal
    /// width changes, lines with a non-empty `original` are re-wrapped. Lines
    /// without `original` (banners, tool calls, plan blocks) are kept as-is.
    /// For multi-line originals (user text, thoughts), the `original` is stored
    /// only on the *first* output line of the block; subsequent lines have
    /// `original = None` and are replaced during re-wrap.
    pub original: Option<String>,
    /// Structured detail for multi-line blocks (inline diff, folded thought).
    /// When `Some`, the renderer expands this into a visual block instead of
    /// rendering `text` alone.
    pub detail: Option<LineDetail>,
}

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineKind {
    #[default]
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
/// folded text block). Attached to `OutputLine.detail`; `None` renders
/// `text` alone.
#[derive(Debug, Clone)]
pub enum LineDetail {
    /// Inline diff for `edit_file` / `write_file`.
    Diff { path: String, hunks: Vec<DiffHunk> },
    /// A folded text block: full raw text plus counts precomputed at commit
    /// time. The renderer shows a summary line by default; the product's
    /// expand toggle re-wraps `raw` at the current width.
    ///
    /// Carries a whole thinking segment or a whole tool result — the two are
    /// the same mechanism (a dense block that is summarized by default and
    /// expanded on demand). Which summary line to draw, and how many preview
    /// rows to show, is the product's call; the block model only keeps the
    /// text, the counts, and where the meta row stands relative to the head.
    Folded {
        raw: String,
        /// Line count at commit-time width (cosmetic, shown in the summary;
        /// may drift after resize — expanded rendering re-wraps).
        line_count: usize,
        /// Total characters in `raw` (precomputed for the token estimate in
        /// thinking summaries; never re-derived at render time).
        char_count: usize,
        /// Whether the meta row already displays `raw`'s first line — see
        /// [`MetaHead`]. Recorded at commit time because the product is the
        /// only layer that knows what it wrote on the meta row.
        meta_head: MetaHead,
    },
}

/// How a [`LineDetail::Folded`] block's meta row relates to `raw`'s first
/// line, recorded at commit time by the product.
///
/// Two mechanical questions hang on this and nothing else: which rows of
/// `raw` the payload body still owes the reader (a first line the meta
/// already displays is not drawn twice), and whether full expansion must
/// restore a truncated first line before the body. The renderer derives
/// nothing from line kinds — a meta row is whatever the product built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MetaHead {
    /// The meta row is a label only (a tool invocation, a count summary); the
    /// body owns the whole payload, first row included.
    #[default]
    None,
    /// The meta row displays `raw`'s first line whole; the body starts at the
    /// second row.
    Whole,
    /// The meta row displays `raw`'s first line truncated; the body starts at
    /// the second row, and full expansion restores the first row — in the
    /// payload's own order, before the body.
    Abbreviated,
}

/// Live state of a tool invocation line.
///
/// Mechanism only: the line model records where a call is in its lifecycle so
/// the renderer can update the row in place (`ToolCallStarted` → `Running`,
/// `ToolCallFinished` → `Done`/`Failed`/`Denied`). Mapping a state to a glyph
/// and a colour is the product's job — the line model never names a symbol,
/// so the CJK width-safety rules stay a product-layer concern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolState {
    /// Accepted but not started (e.g. queued behind a batch).
    Queued,
    /// In flight. The product may animate this state's glyph.
    #[default]
    Running,
    /// Finished successfully.
    Done,
    /// Finished with an error.
    Failed,
    /// Refused by approval — never ran.
    Denied,
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
    /// Structured detail for multi-line blocks (inline diff, folded text).
    /// When `Some`, the renderer expands this into a visual block instead of
    /// rendering `text` alone.
    pub detail: Option<LineDetail>,
    /// Lifecycle state of a tool invocation line (`None` for everything else).
    ///
    /// Accounting only — it lets the renderer redraw this row's leading glyph
    /// as the call moves through its lifecycle, and lets `ToolCallFinished`
    /// find the row to update in place. Glyphs and colours are the product's.
    pub tool_state: Option<ToolState>,
}

impl<S> Default for OutputLine<S> {
    fn default() -> Self {
        Self {
            text: String::new(),
            kind: LineKind::Normal,
            spans: None,
            original: None,
            detail: None,
            tool_state: None,
        }
    }
}

impl<S> OutputLine<S> {
    /// A plain text line of the given kind: no spans, no rewrap original, no
    /// structured detail, no tool state.
    pub fn new(text: impl Into<String>, kind: LineKind) -> Self {
        Self {
            text: text.into(),
            kind,
            ..Default::default()
        }
    }
}

# Changelog

All notable changes to phi-tui.

## [Unreleased]

## [0.5.0] - 2026-11-14

### Added
- **`LineKind::Notice` — an amber line for "look at this".** A red line means
  something broke; a warning that means "this needs you" is a different
  message and now wears its own face, so red keeps meaning "this failed".

### Changed
- **The markdown renderer moved into a module** (`markdown/table.rs`).
  Internal reorganization only — no API change, no behavior change.



## [0.4.0] - 2026-11-09

### Added
- **Selection is a character, not a line.** `selection.rs` models a
  selection point as a character anchor (`CharPoint { row, col }`) that snaps
  to the first cell of the glyph under the pointer, a click chain (single,
  double, triple), word runs, and block bounds — combining marks belong to
  their base character. `visual.rs` gained a three-dimensional seam model
  (`Break`, `Space`, `Glue`) so a wrapped line knows how its cells join.
- **`HistoryRing` — a fish-style history ring** with a draft slot, wrapping in
  both directions.

### Changed
- **A plan block that updates moves to the tail of the output.** The old
  block is removed (no residue, no blank line) and the new one is appended;
  the viewport absorbs the row shift through the content anchor, so a
  scrolled screen does not move.

## [0.3.0] — 2026-10-18

### Added
- Thought-fold duration is tracked from the first delta to the commit.

### Fixed
- Resize rewrap drops the previous wrap's stale tail exactly: old wrap
  continuation fragments no longer bleed into the new block (standalone rows
  next to a head are kept; wide chars handled).

## [0.2.0] — 2026-10-07

### Added

- Span-aware `wrap_line`, folded blocks, layout module
- `PopupList` widget with display-width elision
- Thinking-panel kit + VisualMap extraction
- Viewport: scroll-to-top pin + half-screen page step
- Mention: navigate into directories instead of selecting them

### Fixed

- Elide honors a zero budget; add `pad_cols`
- Completer Enter on a directory focuses its row
- CJK-width-safe output; smarter skill completer

## [0.1.0] — 2026-09-06

### Added

- Initial release: chat-style TUI components for LLM terminals, extracted
  from the phimint TUI (`src/ui/`) as a framework-free component crate.
- `lines`: the transcript line model (`OutputLine<S>`, `LineKind`,
  `ToolDetail`, `DiffHunk`) and the generic styled byte-range `SpanSpec<S>`.
- `transcript`: committed transcript buffer with an anchored, replaceable
  block (`replace_plan` / `clear_plan`) and width-change re-wrapping.
- `stream`: streaming-tail state machine (`TextDelta`/`ThoughtDelta`
  accumulation, incremental wrap cache, whole-line flush on structural
  events).
- `wrap`: CJK-aware hard-wrap (`wrap` / `one_line`) and the incremental
  `WrapCache` (O(new bytes) per frame instead of O(total)).
- `viewport`: scroll state machine (offset, follow-bottom, window range).
- `selection`: mouse-range selection + right-click copy menu state, taking
  plain text only (style- and line-model-blind).
- `markdown`: pulldown-cmark → ratatui `Line` renderer (markerless headings,
  horizontal rules, inline formatting, tables, LaTeX → Unicode).
- `picker` / `completer` / `mention`: the shared completion state machine and
  the `@` path completer (pure fs, no terminal dependency).
- `input`: multi-line `Composer` (Shift+Enter newline, paste-safe).
- `diff`: hand-written LCS line diff → hunks with unified headers.
- Examples: `chat-demo` (mini chat TUI), `picker-demo`.

[Unreleased]: https://github.com/hibuka-labs/phi-tui/compare/0.1.0...HEAD
[0.1.0]: https://github.com/hibuka-labs/phi-tui/releases/tag/v0.1.0

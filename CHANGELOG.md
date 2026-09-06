# Changelog

All notable changes to phi-tui.

## [Unreleased]

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

# phi-tui

[![Crates.io](https://img.shields.io/crates/v/phi-tui.svg)](https://crates.io/crates/phi-tui)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

Chat-style TUI components for LLM terminals, built on ratatui + crossterm.
Everything a "human types at an LLM" terminal product needs: a committed
transcript with an anchored replaceable block, a streaming-tail state
machine, CJK-aware wrapping, a scroll viewport, mouse selection + copy, a
markdown renderer, and `@`/`/` completion pickers.

**No agent semantics.** phi-tui doesn't know what an approval, a tool call,
or a sub-agent is — those live in your product layer. Zero framework
dependencies; `ratatui` / `crossterm` / `pulldown-cmark` / `unicode-width` /
`tracing` only.

## Components

| Module | What it gives you |
|---|---|
| `lines` | The line model: `OutputLine<S>`, `LineKind`, `ToolDetail`, `DiffHunk`, generic `SpanSpec<S>` styled byte-ranges |
| `transcript` | Committed transcript buffer + anchored replaceable block + re-wrap on width change |
| `stream` | Streaming-tail state machine: deltas in, whole lines out, O(new bytes) rendering |
| `wrap` | CJK-aware hard-wrap (`wrap` / `one_line`) + incremental `WrapCache` |
| `viewport` | Scroll state: offset, follow-bottom, visible window range |
| `selection` | Mouse-range selection + right-click copy menu (plain text in, plain text out) |
| `markdown` | pulldown-cmark → `Vec<ratatui::Line>`: markerless headings, rules, inline styles, tables, LaTeX → Unicode |
| `picker` / `completer` / `mention` | Shared completion state machine + `@` path completer (pure fs) |
| `input` | Multi-line `Composer` (Shift+Enter newline, paste-safe) |
| `diff` | Hand-written LCS line diff → unified hunks |

The line model is generic over your style token: `OutputLine<S = ()>` —
use `OutputLine<()>` for plain text, or pin your own token (colors, semantic
roles) in `spans: Option<Vec<SpanSpec<S>>>` as byte ranges over the plain
text, so copy/selection and frame capture stay style-blind.

## Usage

```toml
[dependencies]
phi-tui = "0.1"
```

```rust
use phi_tui::transcript::Transcript;
use phi_tui::viewport::Viewport;

let mut t: Transcript = Transcript::new();
t.push_user("hello");
let mut vp = Viewport::new();
let window = vp.window_range(t.len(), 10); // rows to draw
```

Commit streaming deltas as whole lines:

```rust
use phi_tui::stream::StreamState;
use phi_tui::transcript::{Transcript, DEFAULT_WRAP_WIDTH};

let mut t: Transcript = Transcript::new();
let mut st: StreamState = StreamState::new(DEFAULT_WRAP_WIDTH);
st.push_text("hel", None);
st.push_text("lo", None);
let lines = st.flush(); // → one committed `OutputLine` "hello"
t.extend(lines);
```

See `examples/chat-demo.rs` for a ~200-line end-to-end chat TUI
(`cargo run --example chat-demo`) and `examples/picker-demo.rs` for the
completion state machine.

## Design notes

- **Style-blind plain text**: `OutputLine.text` is always the plain
  concatenation; styling travels as `SpanSpec { start, len, style }` byte
  ranges, so copy/selection and frame capture never see styles.
- **Anchored replaceable block**: `Transcript::replace_plan` / `clear_plan`
  implement "full block replaces previous, in place" — the pattern behind
  live-updating plan/todo widgets.
- **State machines, thin rendering**: every component is a plain state
  machine or pure function over data; rendering happens in your frame loop.

## License

MIT — see [LICENSE](LICENSE).

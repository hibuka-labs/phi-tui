//! `phi-tui` — chat-style TUI components for LLM terminals.
//!
//! The parts any "human types at an LLM" terminal product needs: a committed
//! transcript buffer with an anchored replaceable block, a streaming-tail
//! state machine, CJK-aware wrapping, a scroll viewport, mouse selection +
//! copy, a markdown renderer, and the `@`/`/` completion pickers.
//!
//! Built for the ratatui 0.30 + crossterm 0.28 ecosystem. Zero framework
//! dependencies — no agent semantics live here (no approvals, tools, or
//! sub-agents); products wire those on top.
//!
//! The line model ([`lines::OutputLine`]) is generic over the product's style
//! token `S` (`OutputLine<S = ()>`); styled byte-ranges travel as
//! [`lines::SpanSpec<S>`] offsets into the plain text, so copy/selection and
//! frame capture stay style-blind.
//!
//! Minimal example — commit lines and read the visible window:
//!
//! ```
//! use phi_tui::transcript::Transcript;
//! use phi_tui::viewport::Viewport;
//!
//! let mut t: Transcript = Transcript::new();
//! t.push_user("hello");
//! let mut vp = Viewport::new();
//! let window = vp.window_range(t.len(), 10);
//! assert!(window.end <= t.len());
//! ```

pub mod completer;
pub mod diff;
pub mod input;
pub mod lines;
pub mod markdown;
pub mod mention;
pub mod picker;
pub mod selection;
pub mod stream;
pub mod tail_panel;
pub mod transcript;
pub mod viewport;
pub mod visual;
pub mod wrap;

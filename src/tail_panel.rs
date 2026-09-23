//! Stateless auto-scrolling tail window: a bordered panel showing the LAST
//! lines of an append-only wrapped buffer. The "scrolling" is a plain
//! `last(N)` slice — no scroll state to own — which is what makes this
//! reusable: any product holding a wrapped tail (LLM thinking, log tail,
//! child output) feeds it in and gets a live observation window.
//!
//! Contract: callers pre-wrap `lines` at (or under) the panel's inner width;
//! wider lines are hard-clipped by `Paragraph` (no ellipsis) — never a layout
//! break.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Widget},
};

/// The visible slice: the last `max` lines. No padding when the buffer is
/// shorter — callers size the panel for the steady state.
pub fn tail_window(lines: &[String], max: usize) -> &[String] {
    let start = lines.len().saturating_sub(max);
    &lines[start..]
}

/// Bordered auto-scrolling observation window over a wrapped tail.
/// Stateless: construct per frame and `f.render_widget(panel, area)`.
pub struct TailPanel<'a> {
    pub title: Line<'a>,
    /// Pre-wrapped at (or under) the panel's inner width; wider lines are
    /// hard-clipped by `Paragraph` without an ellipsis.
    pub lines: &'a [String],
    /// Style for the content lines (typically dim / italic).
    pub line_style: Style,
    /// Style for the border block (typically DarkGray).
    pub border_style: Style,
}

impl Widget for TailPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner_h = area.height.saturating_sub(2) as usize; // top + bottom border
        let window = tail_window(self.lines, inner_h);
        let body: Vec<Line> = window
            .iter()
            .map(|l| Line::from(Span::styled(l.as_str(), self.line_style)))
            .collect();
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(self.border_style)
            .title(self.title);
        Paragraph::new(body).block(block).render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;

    fn buffer_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        let buf = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn tail_window_returns_last_n() {
        let lines: Vec<String> = (0..10).map(|i| format!("l{i}")).collect();
        assert_eq!(tail_window(&lines, 3), ["l7".to_string(), "l8".to_string(), "l9".to_string()]);
        assert_eq!(tail_window(&lines, 20).len(), 10);
        assert!(tail_window(&lines, 0).is_empty());
        assert!(tail_window(&[], 4).is_empty());
    }

    #[test]
    fn panel_draws_title_border_and_last_lines() {
        let backend = ratatui::backend::TestBackend::new(30, 6); // inner height 4
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let lines: Vec<String> = (0..10).map(|i| format!("l{i}")).collect();
        terminal
            .draw(|f| {
                f.render_widget(
                    TailPanel {
                        title: Line::from("thinking · 3s"),
                        lines: &lines,
                        line_style: Style::default().add_modifier(Modifier::DIM),
                        border_style: Style::default(),
                    },
                    f.area(),
                );
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("thinking · 3s"), "title missing:\n{text}");
        assert!(text.contains("╭"), "top border missing (ratatui 0.30 Rounded renders ╭ corner, not ┌):\n{text}");
        assert!(text.contains("l9"), "last line visible:\n{text}");
        assert!(!text.contains("l0"), "early lines hidden:\n{text}");
        assert!(!text.contains("l5"), "beyond-window lines hidden:\n{text}");
    }

    #[test]
    fn panel_empty_buffer_renders_border_only() {
        let backend = ratatui::backend::TestBackend::new(20, 4);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let empty: Vec<String> = Vec::new();
        terminal
            .draw(|f| {
                f.render_widget(
                    TailPanel {
                        title: Line::from("t"),
                        lines: &empty,
                        line_style: Style::default(),
                        border_style: Style::default(),
                    },
                    f.area(),
                );
            })
            .unwrap();
        // Must not panic; border present.
        let text = buffer_text(&terminal);
        assert!(text.contains("╰"), "bottom border missing (ratatui 0.30 Rounded renders ╰ corner, not └):\n{text}");
    }

    #[test]
    fn panel_clips_overwide_lines_within_borders() {
        let backend = ratatui::backend::TestBackend::new(12, 4);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let lines = vec!["x".repeat(100), "short".to_string()];
        terminal
            .draw(|f| {
                f.render_widget(
                    TailPanel {
                        title: Line::from("t"),
                        lines: &lines,
                        line_style: Style::default(),
                        border_style: Style::default(),
                    },
                    f.area(),
                );
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("short"), "later line visible after clip:\n{text}");
        assert!(text.contains('╭') && text.contains('╰'), "borders intact:\n{text}");
    }
}

//! Markdown table layout: box-drawing borders, a width budget, and
//! cell-level wrapping.
//!
//! A table is the one markdown shape that draws its own frame, so it is the
//! one shape that can break the frame: a cell wider than the pane makes the
//! box wider than the pane, and the transcript's soft-wrap then shreds the
//! borders into fragments. So the layout takes a width budget: columns
//! shrink until the box fits, and the cell text wraps *inside* the cell.
//!
//! Measurement uses the `unicode-width` crate — the same measure the rest of
//! the crate wraps with ([`crate::wrap`]), so the borders land where the
//! terminal puts them. A hand-rolled "CJK is two columns" table (which
//! ignored emoji) put every border one column off its own row.

use pulldown_cmark::Alignment;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::wrap::wrap_line;

/// Minimum content width of a table column.
pub(crate) const MIN_COL_WIDTH: usize = 3;

/// Collected table content (the writer fills it while the parser runs).
pub(crate) struct TableState {
    pub(crate) alignments: Vec<Alignment>,
    pub(crate) header: Vec<Vec<Vec<Span<'static>>>>,
    pub(crate) rows: Vec<Vec<Vec<Span<'static>>>>,
    pub(crate) current_row: Vec<Vec<Span<'static>>>,
    pub(crate) current_cell: Vec<Span<'static>>,
    pub(crate) in_header: bool,
    pub(crate) row_is_header: bool,
}

/// Which border line to draw.
enum Border {
    Top,
    Separator,
    Bottom,
}

/// Render a complete table with box-drawing borders.
///
/// `width_budget` is the pane width the box must fit in. `None` keeps the
/// natural (unbounded) layout.
pub(crate) fn render(ts: &TableState, width_budget: Option<usize>) -> Vec<Line<'static>> {
    let num_cols = ts.alignments.len();
    if num_cols == 0 {
        return Vec::new();
    }

    let mut col_widths = natural_widths(ts, num_cols);
    if let Some(budget) = width_budget {
        shrink_to_budget(&mut col_widths, budget, num_cols);
    }

    let mut result = Vec::new();
    result.push(border_line(&col_widths, Border::Top));
    for row in &ts.header {
        result.extend(build_row(row, &col_widths, &ts.alignments));
    }
    result.push(border_line(&col_widths, Border::Separator));
    for row in &ts.rows {
        result.extend(build_row(row, &col_widths, &ts.alignments));
    }
    result.push(border_line(&col_widths, Border::Bottom));
    result
}

/// The widest cell per column (header and body), floored at
/// [`MIN_COL_WIDTH`].
fn natural_widths(ts: &TableState, num_cols: usize) -> Vec<usize> {
    let mut col_widths = vec![0usize; num_cols];
    for row in ts.header.iter().chain(ts.rows.iter()) {
        for (col, cell) in row.iter().enumerate() {
            if col < num_cols {
                let w = cell_width(cell);
                col_widths[col] = col_widths[col].max(w);
            }
        }
    }
    for w in &mut col_widths {
        *w = (*w).max(MIN_COL_WIDTH);
    }
    col_widths
}

/// Shrink the columns until the whole box fits `budget`.
///
/// The widest column gives up the space first (a long prose cell is the one
/// worth folding; a short label column is not). When even the minimum
/// layout is too wide, every column drops to the minimum and the pane
/// clips the rest — a table that cannot fit is still a table.
fn shrink_to_budget(col_widths: &mut [usize], budget: usize, num_cols: usize) {
    // The box costs three columns of chrome per column (a border and two
    // padding spaces) plus one closing border.
    let avail = budget.saturating_sub(3 * num_cols + 1);
    if avail < MIN_COL_WIDTH * num_cols {
        for w in col_widths.iter_mut() {
            *w = MIN_COL_WIDTH;
        }
        return;
    }
    loop {
        let sum: usize = col_widths.iter().sum();
        if sum <= avail {
            return;
        }
        let excess = sum - avail;
        let Some(idx) = col_widths
            .iter()
            .enumerate()
            .filter(|(_, w)| **w > MIN_COL_WIDTH)
            .max_by_key(|(_, w)| **w)
            .map(|(i, _)| i)
        else {
            return;
        };
        let room = col_widths[idx] - MIN_COL_WIDTH;
        let take = excess.min(room);
        col_widths[idx] -= take;
    }
}

/// Display width of a cell's spans.
fn cell_width(cell: &[Span<'static>]) -> usize {
    cell.iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
}

/// The border line (`┌───┬───┐` / `├───┼───┤` / `└───┴───┘`).
fn border_line(col_widths: &[usize], kind: Border) -> Line<'static> {
    let (left, joint, right) = match kind {
        Border::Top => ("┌", "┬", "┐"),
        Border::Separator => ("├", "┼", "┤"),
        Border::Bottom => ("└", "┴", "┘"),
    };
    let border = Style::default().fg(Color::DarkGray);
    let mut spans = vec![Span::styled(left, border)];
    for (i, &w) in col_widths.iter().enumerate() {
        spans.push(Span::styled("─".repeat(w + 2), border));
        if i < col_widths.len() - 1 {
            spans.push(Span::styled(joint, border));
        }
    }
    spans.push(Span::styled(right, border));
    Line::from(spans)
}

/// One table row: the cells are wrapped to their column width, and the row
/// is as tall as its tallest cell.
fn build_row(
    row: &[Vec<Span<'static>>],
    col_widths: &[usize],
    alignments: &[Alignment],
) -> Vec<Line<'static>> {
    let cells: Vec<Vec<Line<'static>>> = col_widths
        .iter()
        .enumerate()
        .map(|(col, &width)| {
            let spans = row.get(col).map(|c| c.as_slice()).unwrap_or(&[]);
            wrap_line(Line::from(spans.to_vec()), width)
        })
        .collect();
    let height = cells.iter().map(|c| c.len()).max().unwrap_or(1);

    let border = Style::default().fg(Color::DarkGray);
    let mut result = Vec::with_capacity(height);
    for row_idx in 0..height {
        let mut spans = vec![Span::styled("│", border)];
        for (col, &width) in col_widths.iter().enumerate() {
            let content = cells[col].get(row_idx).cloned().unwrap_or_default();
            let content_width: usize = content
                .spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            let pad = width.saturating_sub(content_width);
            let alignment = alignments.get(col).copied().unwrap_or(Alignment::None);
            let (left_pad, right_pad) = match alignment {
                Alignment::Center => (pad / 2, pad - pad / 2),
                Alignment::Right => (pad, 0),
                _ => (0, pad),
            };

            spans.push(Span::styled(" ", Style::default()));
            if left_pad > 0 {
                spans.push(Span::styled(" ".repeat(left_pad), Style::default()));
            }
            spans.extend(content.spans);
            if right_pad > 0 {
                spans.push(Span::styled(" ".repeat(right_pad), Style::default()));
            }
            spans.push(Span::styled(" ", Style::default()));
            spans.push(Span::styled("│", border));
        }
        result.push(Line::from(spans));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::render_markdown_with_width;

    /// The plain text of a rendered line.
    fn plain(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// Every rendered line's display width.
    fn widths(lines: &[Line<'_>]) -> Vec<usize> {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                    .sum()
            })
            .collect()
    }

    #[test]
    fn a_wide_table_fits_the_budget_and_keeps_its_borders() {
        let md = "| 项 | 结果 |\n|---|---|\n| **第 2 步早退** | ✅ 早退。首次 `task_output(wait=true)` 立即返回 `[waiting for input]` 块 |\n";
        let lines = render_markdown_with_width(md, Some(40));
        let text: Vec<String> = lines.iter().map(plain).collect();

        // The box fits: no line is wider than the budget.
        assert!(
            widths(&lines).iter().all(|w| *w <= 40),
            "box overflows the budget: {:?}",
            widths(&lines)
        );
        // Every visual row of the box still carries its borders.
        let box_rows: Vec<&String> = text.iter().filter(|l| l.starts_with('│')).collect();
        assert!(
            box_rows.len() >= 4,
            "rows must be wrapped, not clipped: {text:?}"
        );
        for row in &box_rows {
            assert!(row.ends_with('│'), "row lost its right border: {row:?}");
        }
        // The border rows are all the same width — the frame is still a frame.
        let border_rows: Vec<&String> = text
            .iter()
            .filter(|l| l.starts_with('┌') || l.starts_with('├') || l.starts_with('└'))
            .collect();
        assert!(border_rows.len() == 3, "three border lines: {text:?}");
        let first = UnicodeWidthStr::width(border_rows[0].as_str());
        for row in &border_rows {
            assert_eq!(
                UnicodeWidthStr::width(row.as_str()),
                first,
                "border lines differ in width: {text:?}"
            );
        }
    }

    #[test]
    fn every_row_of_a_box_is_the_same_width() {
        // The emoji is the wide character that used to drift the border.
        let md = "| a | b |\n|---|---|\n| ✅ ok | plain |\n| wide | 宽宽宽宽宽 |\n";
        let lines = render_markdown_with_width(md, Some(30));
        let widths = widths(&lines);
        let first = widths[0];
        assert!(
            widths.iter().all(|w| *w == first),
            "rows drift: {widths:?} / {:?}",
            lines.iter().map(plain).collect::<Vec<_>>()
        );
    }

    #[test]
    fn no_budget_keeps_the_natural_width() {
        let md = "| 项 | 结果 |\n|---|---|\n| early | a fairly long result cell |\n";
        let lines = render_markdown_with_width(md, None);
        assert!(widths(&lines).iter().all(|w| *w == widths(&lines)[0]));
        assert!(widths(&lines)[0] > 30, "natural layout is not squeezed");
    }
}

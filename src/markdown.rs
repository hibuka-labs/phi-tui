//! Markdown → ratatui `Line` rendering (custom pulldown-cmark state machine).
//!
//! Unlike tui-markdown (which is a "source viewer" that keeps `#` markers),
//! this renderer produces rich output: headings are styled without markers,
//! horizontal rules become separator lines, inline formatting is applied,
//! tables get box-drawing borders, and LaTeX math is converted to ASCII
//! display forms. It is self-contained — no dependency on any app state root —
//! so the TUI and any future renderer can reuse it.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

mod table;

use table::TableState;

/// Extract plain text from a ratatui `Line` (concatenate all span contents).
pub fn line_plain_text(line: &Line<'_>) -> String {
    let mut s = String::new();
    for span in &line.spans {
        s.push_str(&span.content);
    }
    s
}

/// Convert common LaTeX commands to ASCII display forms.
///
/// ASCII only — the previous Unicode-symbol targets (→ × ≤ α …) are
/// East-Asian-Ambiguous or CJK-font-double-width, and CJK terminals render
/// them 2 cells wide while the layout counts 1, so math-heavy text pushed
/// rows off-screen. Readability loses a little; layout safety wins.
fn latex_to_unicode(latex: &str) -> String {
    let mut s = latex.to_string();
    // Common symbols (order matters: longer patterns first)
    s = s.replace("\\rightarrow", "->");
    s = s.replace("\\leftarrow", "<-");
    s = s.replace("\\Rightarrow", "=>");
    s = s.replace("\\Leftarrow", "<=");
    s = s.replace("\\times", "x");
    s = s.replace("\\div", "/");
    s = s.replace("\\pm", "+/-");
    s = s.replace("\\mp", "-/+");
    s = s.replace("\\leq", "<=");
    s = s.replace("\\geq", ">=");
    s = s.replace("\\neq", "!=");
    s = s.replace("\\approx", "~=");
    s = s.replace("\\equiv", "===");
    s = s.replace("\\cdot", "*");
    s = s.replace("\\ldots", "...");
    s = s.replace("\\cdots", "...");
    s = s.replace("\\infty", "inf");
    s = s.replace("\\partial", "d");
    s = s.replace("\\nabla", "grad");
    s = s.replace("\\sum", "sum");
    s = s.replace("\\prod", "prod");
    s = s.replace("\\int", "int");
    s = s.replace("\\alpha", "alpha");
    s = s.replace("\\beta", "beta");
    s = s.replace("\\gamma", "gamma");
    s = s.replace("\\delta", "delta");
    s = s.replace("\\epsilon", "epsilon");
    s = s.replace("\\theta", "theta");
    s = s.replace("\\lambda", "lambda");
    s = s.replace("\\mu", "mu");
    s = s.replace("\\pi", "pi");
    s = s.replace("\\sigma", "sigma");
    s = s.replace("\\phi", "phi");
    s = s.replace("\\omega", "omega");
    s = s.replace("\\Delta", "Delta");
    s = s.replace("\\Sigma", "Sigma");
    s = s.replace("\\Omega", "Omega");
    // Display commands: remove the command, keep the content
    s = s.replace("\\boxed{", "");
    s = s.replace("\\boxed(", "("); // \boxed(...) → (...)
    // Size/style commands: just remove
    for cmd in &[
        "\\large",
        "\\Large",
        "\\LARGE",
        "\\huge",
        "\\Huge",
        "\\small",
        "\\normalsize",
        "\\bfseries",
        "\\itshape",
        "\\textbf{",
        "\\textit{",
        "\\underline{",
    ] {
        s = s.replace(*cmd, "");
    }
    // \sqrt{n} → sqrt(n)
    while let Some(start) = s.find("\\sqrt{") {
        if let Some(end) = s[start + 6..].find('}') {
            let inner = &s[start + 6..start + 6 + end];
            let replacement = format!("sqrt({})", inner);
            s = format!(
                "{}{}{}",
                &s[..start],
                replacement,
                &s[start + 6 + end + 1..]
            );
        } else {
            break;
        }
    }
    // Remove remaining braces used for grouping: {x} → x
    // Only remove simple single-char braces to avoid breaking nested expressions
    s = s.replace("\\mathbf{", "");
    s = s.replace("\\mathrm{", "");
    s = s.replace("\\text{", "");
    // Clean up remaining single braces
    let mut result = String::new();
    let mut depth = 0i32;
    for ch in s.chars() {
        match ch {
            '{' => {
                if depth == 0 {
                    depth += 1;
                    continue; // skip opening brace
                }
                depth += 1;
                result.push(ch);
            }
            '}' => {
                depth -= 1;
                if depth < 0 {
                    depth = 0; // unmatched close brace
                    continue;
                }
                if depth == 0 {
                    continue; // skip closing brace
                }
                result.push(ch);
            }
            _ => result.push(ch),
        }
    }
    result
}

/// Render markdown text using pulldown-cmark with custom styling.
///
/// Unlike tui-markdown (which is a "source viewer" that keeps `#` markers),
/// this renderer produces rich output: headings are styled without markers,
/// horizontal rules become separator lines, and inline formatting is applied.
pub fn render_markdown(text: &str) -> Vec<Line<'static>> {
    render_markdown_with_width(text, None)
}

/// Render markdown, laying tables out to fit `max_width` terminal columns.
///
/// A table is the one markdown shape that draws its own frame, so it is the
/// one shape that can overflow the pane: without a budget, a long cell makes
/// the box wider than the screen and the transcript's soft-wrap shreds the
/// borders. With a budget the columns shrink and the cell text wraps inside
/// the cell, so the box stays a box. `None` keeps the natural layout.
pub fn render_markdown_with_width(text: &str, max_width: Option<usize>) -> Vec<Line<'static>> {
    let parser = Parser::new_ext(text, Options::all());
    let mut writer = MarkdownWriter::new(max_width);
    for event in parser {
        writer.process_event(event);
    }
    writer.finish()
}

/// State machine for rendering pulldown-cmark events into ratatui Lines.
struct MarkdownWriter {
    lines: Vec<Line<'static>>,
    current_line: Line<'static>,
    inline_style: Style,
    in_code_block: bool,
    code_lang: String,
    needs_newline: bool,
    table_state: Option<TableState>,
    /// The width a rendered table must fit in (`None` = no budget).
    table_width: Option<usize>,
}

impl MarkdownWriter {
    fn new(table_width: Option<usize>) -> Self {
        Self {
            lines: Vec::new(),
            current_line: Line::default(),
            inline_style: Style::default(),
            in_code_block: false,
            code_lang: String::new(),
            needs_newline: false,
            table_state: None,
            table_width,
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_current_line();
        self.lines
    }

    fn flush_current_line(&mut self) {
        if !self.current_line.spans.is_empty() {
            self.lines.push(std::mem::take(&mut self.current_line));
        }
    }

    fn push_line(&mut self, line: Line<'static>) {
        self.flush_current_line();
        self.lines.push(line);
    }

    fn push_blank_line(&mut self) {
        self.flush_current_line();
        self.lines.push(Line::default());
    }

    fn process_event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(code) => self.inline_code(&code),
            // Math events: render LaTeX as Unicode symbols (cyan, like inline code)
            Event::InlineMath(math) => {
                let span = Span::styled(latex_to_unicode(&math), Style::default().fg(Color::Cyan));
                if let Some(ref mut ts) = self.table_state {
                    ts.current_cell.push(span);
                } else {
                    self.current_line.spans.push(span);
                }
            }
            Event::DisplayMath(math) => {
                // Display math: render on its own line(s)
                self.flush_current_line();
                let converted = latex_to_unicode(&math);
                for line in converted.lines() {
                    self.push_line(Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(Color::Cyan),
                    )));
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                self.flush_current_line();
            }
            Event::Rule => {
                self.flush_current_line();
                if !self.lines.is_empty() {
                    self.push_blank_line();
                }
                self.push_line(make_horizontal_rule());
                self.needs_newline = true;
            }
            _ => {}
        }
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush_current_line();
                if self.needs_newline {
                    self.push_blank_line();
                    self.needs_newline = false;
                }
                let style = heading_style(level);
                self.inline_style = style;
                // No # marker — just apply the style to content.
            }
            Tag::Paragraph => {
                if self.needs_newline {
                    self.push_blank_line();
                    self.needs_newline = false;
                }
            }
            Tag::CodeBlock(kind) => {
                self.flush_current_line();
                if self.needs_newline {
                    self.push_blank_line();
                    self.needs_newline = false;
                }
                self.in_code_block = true;
                self.code_lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(lang) => lang.to_string(),
                    _ => String::new(),
                };
                // Code block header
                let header = if self.code_lang.is_empty() {
                    "  │".to_string()
                } else {
                    format!("  │ [{}]", self.code_lang)
                };
                self.push_line(Line::from(Span::styled(
                    header,
                    Style::default().fg(Color::DarkGray),
                )));
            }
            Tag::List(_) => {
                self.flush_current_line();
                if self.needs_newline {
                    self.push_blank_line();
                    self.needs_newline = false;
                }
            }
            Tag::Item => {
                self.flush_current_line();
                self.current_line
                    .spans
                    .push(Span::styled("  - ", Style::default().fg(Color::DarkGray)));
            }
            Tag::BlockQuote(_) => {
                self.flush_current_line();
                self.current_line
                    .spans
                    .push(Span::styled("  │ ", Style::default().fg(Color::DarkGray)));
            }
            Tag::Table(alignments) => {
                self.flush_current_line();
                if self.needs_newline {
                    self.push_blank_line();
                    self.needs_newline = false;
                }
                self.table_state = Some(TableState {
                    alignments,
                    header: Vec::new(),
                    rows: Vec::new(),
                    current_row: Vec::new(),
                    current_cell: Vec::new(),
                    in_header: false,
                    row_is_header: false,
                });
            }
            Tag::TableHead => {
                if let Some(ref mut ts) = self.table_state {
                    ts.in_header = true;
                    // pulldown-cmark 0.13: header cells are directly inside TableHead
                    // without a TableRow wrapper, so start collecting cells now
                    ts.current_row = Vec::new();
                }
            }
            Tag::TableRow => {
                if let Some(ref mut ts) = self.table_state {
                    ts.current_row = Vec::new();
                    // Capture header state at row start
                    ts.row_is_header = ts.in_header;
                }
            }
            Tag::TableCell => {
                if let Some(ref mut ts) = self.table_state {
                    ts.current_cell = Vec::new();
                }
            }
            Tag::Strong => {
                self.inline_style = self.inline_style.add_modifier(Modifier::BOLD);
            }
            Tag::Emphasis => {
                self.inline_style = self.inline_style.add_modifier(Modifier::ITALIC);
            }
            Tag::Strikethrough => {
                self.inline_style = self.inline_style.add_modifier(Modifier::CROSSED_OUT);
            }
            Tag::Link { dest_url, .. } => {
                // Links: cyan underline
                self.inline_style = self
                    .inline_style
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::UNDERLINED);
                // Store URL for potential tooltip/hover (future)
                let _ = dest_url;
            }
            Tag::Image { dest_url, .. } => {
                // Images: show as [Image: url]
                self.current_line.spans.push(Span::styled(
                    "[Image: ",
                    Style::default().fg(Color::DarkGray),
                ));
                let _ = dest_url;
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.flush_current_line();
                self.inline_style = Style::default();
                self.needs_newline = true;
            }
            TagEnd::Paragraph => {
                self.flush_current_line();
                self.needs_newline = true;
            }
            TagEnd::CodeBlock => {
                self.flush_current_line();
                self.in_code_block = false;
                self.code_lang.clear();
                self.needs_newline = true;
            }
            TagEnd::List(_) => {
                self.flush_current_line();
                self.needs_newline = true;
            }
            TagEnd::Item => {
                self.flush_current_line();
            }
            TagEnd::BlockQuote(_) => {
                self.flush_current_line();
            }
            TagEnd::Table => {
                self.flush_current_line();
                if let Some(ts) = self.table_state.take() {
                    let table_lines = table::render(&ts, self.table_width);
                    for line in table_lines {
                        self.push_line(line);
                    }
                }
                self.needs_newline = true;
            }
            TagEnd::TableHead => {
                if let Some(ref mut ts) = self.table_state {
                    // Save header row collected directly from TableHead
                    let row = std::mem::take(&mut ts.current_row);
                    if !row.is_empty() {
                        ts.header.push(row);
                    }
                    ts.in_header = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(ref mut ts) = self.table_state {
                    let row = std::mem::take(&mut ts.current_row);
                    if ts.row_is_header {
                        ts.header.push(row);
                    } else {
                        ts.rows.push(row);
                    }
                }
            }
            TagEnd::TableCell => {
                if let Some(ref mut ts) = self.table_state {
                    let cell = std::mem::take(&mut ts.current_cell);
                    ts.current_row.push(cell);
                }
            }
            TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough => {
                self.inline_style = Style::default();
            }
            TagEnd::Link => {
                self.inline_style = Style::default();
            }
            TagEnd::Image => {
                self.current_line
                    .spans
                    .push(Span::styled("]", Style::default().fg(Color::DarkGray)));
            }
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        if self.in_code_block {
            // Code block content: monospace, dimmed
            for line in text.lines() {
                self.push_line(Line::from(Span::styled(
                    format!("  │ {line}"),
                    Style::default().fg(Color::Gray),
                )));
            }
        } else if let Some(ref mut ts) = self.table_state {
            // Table cell content
            let style = self.inline_style;
            ts.current_cell.push(Span::styled(text.to_string(), style));
        } else {
            let style = self.inline_style;
            self.current_line
                .spans
                .push(Span::styled(text.to_string(), style));
        }
    }

    fn inline_code(&mut self, code: &str) {
        // Inline code: cyan color (like Claude Code), no backticks, no background
        let span = Span::styled(code.to_string(), Style::default().fg(Color::Cyan));
        if let Some(ref mut ts) = self.table_state {
            ts.current_cell.push(span);
        } else {
            self.current_line.spans.push(span);
        }
    }
}

fn heading_style(level: HeadingLevel) -> Style {
    match level {
        HeadingLevel::H1 => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        HeadingLevel::H2 => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        HeadingLevel::H3 => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::ITALIC),
    }
}

/// Render a horizontal rule as a styled separator line.
fn make_horizontal_rule() -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(60),
        Style::default().fg(Color::DarkGray),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_markdown_heading_and_hr() {
        // Test our custom pulldown-cmark renderer.
        let cases: Vec<(&str, &str)> = vec![
            ("h1", "# 标题"),
            ("h2", "## 标题"),
            ("h3", "### 🔑 核心设计亮点"),
            ("hr ---", "---"),
            ("hr ***", "***"),
            ("bold", "**粗体** 和 `code`"),
            ("setext", "标题\n---"),
        ];

        for (label, input) in &cases {
            let lines = render_markdown(input);
            println!("\n=== {label} === input: {input:?}");
            for (i, line) in lines.iter().enumerate() {
                let parts: Vec<String> = line
                    .spans
                    .iter()
                    .map(|s| {
                        let mods = format!("{:?}", s.style.add_modifier);
                        format!("[{:?} {}]", s.content, mods)
                    })
                    .collect();
                println!("  line[{i}]: {}", parts.join(" "));
            }
        }

        // Verify specific behaviors:
        // Heading: no # prefix
        let lines = render_markdown("### 🔑 核心设计亮点");
        let text = line_plain_text(&lines[0]);
        assert!(
            !text.starts_with("###"),
            "heading should not have # prefix: {text}"
        );
        assert!(text.contains("🔑 核心设计亮点"));

        // Horizontal rule
        let lines = render_markdown("---");
        let text = line_plain_text(&lines[0]);
        assert!(text.chars().all(|c| c == '─'), "hr should be ───: {text:?}");

        // Bold
        let lines = render_markdown("**粗体**");
        let text = line_plain_text(&lines[0]);
        assert!(text.contains("粗体"));
        let has_bold = lines[0]
            .spans
            .iter()
            .any(|s| s.style.add_modifier.contains(Modifier::BOLD));
        assert!(has_bold, "bold text should have BOLD modifier");

        // Table with header
        let table_md = "| Header1 | Header2 |\n|---------|--------|\n| Cell1   | Cell2  |";
        let lines = render_markdown(table_md);
        println!("\n=== Table rendering ===");
        for (i, line) in lines.iter().enumerate() {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            println!("  line[{i}]: {text:?}");
        }
        // Should have top border, header row, separator, body row, bottom border = 5 lines
        assert!(
            lines.len() >= 5,
            "table should have at least 5 lines, got {}",
            lines.len()
        );
        // Header row should not be empty
        let header_text: String = lines[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            header_text.contains("Header1"),
            "header should contain 'Header1', got: {header_text:?}"
        );
    }

    #[test]
    fn latex_to_unicode_conversion() {
        assert_eq!(latex_to_unicode("\\sqrt{9}"), "sqrt(9)");
        assert_eq!(latex_to_unicode("\\sqrt{4}"), "sqrt(4)");
        assert_eq!(latex_to_unicode("2 \\times 3"), "2 x 3");
        assert_eq!(latex_to_unicode("6 \\div 2"), "6 / 2");
        assert_eq!(latex_to_unicode("\\pi r^2"), "pi r^2");
        assert_eq!(latex_to_unicode("a \\neq b"), "a != b");
        assert_eq!(latex_to_unicode("\\alpha + \\beta"), "alpha + beta");
        assert_eq!(latex_to_unicode("\\sqrt{4} \\times 9"), "sqrt(4) x 9");
        assert_eq!(
            latex_to_unicode("$(\\sqrt{4} \\times 9 - 10) \\times 3$"),
            "$(sqrt(4) x 9 - 10) x 3$"
        );
    }

    #[test]
    fn math_latex_rendering() {
        // Inline math should render with ASCII forms (Unicode symbols are
        // CJK-double-width and break the terminal layout).
        let lines = render_markdown("计算步骤：$\\sqrt{4}=2$，$2\\times9=18$");
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        // Should have converted forms, not raw LaTeX
        assert!(text.contains("sqrt(4)=2"), "should convert sqrt: {text}");
        assert!(text.contains("2x9=18"), "should convert times: {text}");
        // Math spans should be cyan
        let has_cyan = lines[0]
            .spans
            .iter()
            .any(|s| s.style.fg == Some(Color::Cyan));
        assert!(has_cyan, "math should be cyan colored");

        // Table with math cells
        let table_md = "| 步骤 | 运算 | 结果 |\n|------|------|------|\n| 1 | $\\sqrt{4}$ | $2$ |";
        let lines = render_markdown(table_md);
        let body_line = &lines[3]; // 0: top, 1: header, 2: separator, 3: body, 4: bottom
        let body_text: String = body_line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            body_text.contains("sqrt(4)"),
            "table should convert sqrt: {body_text}"
        );
    }
}

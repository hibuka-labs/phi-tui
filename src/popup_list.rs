//! Popup list band: the completion pickers' rows rendered as a floating band
//! anchored above the composer.
//!
//! Mechanism only — band geometry, `Clear`, scroll window, selection gutter,
//! highlight, frame and separator. Row content (markers, wording, colours) is
//! built by the product and passed in.
//!
//! Style contract (the pattern every phi-tui widget follows):
//!
//! 1. **Typed style**: knobs live in a `XxxStyle` struct with enums for
//!    choices (`WidthSpec`) — widgets never parse strings from a config file.
//! 2. **Defaults**: `XxxStyle::default()` is a complete, presentable look.
//! 3. **Pure injection**: a widget is `f(content, state, style)`; it reads no
//!    globals and no config. Apps parse config and build the style value, so
//!    theming later is an app-side factory with zero widget changes.
//! 4. **Extract on second use**: shared mechanics (band maths, `Clear`, scroll
//!    windows) stay inside the widget until a second widget family needs them.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

/// Columns the selection gutter occupies (`▸ ` or `  `).
pub const GUTTER_W: usize = 2;

/// How wide a popup band is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidthSpec {
    /// Match the anchor rect (the composer) — the default.
    Fill,
    /// Fixed column count, clamped to the terminal.
    Fixed(u16),
}

/// A popup band's look. `PopupStyle::default()` is the frameless, full-width
/// band with a bottom separator — the design default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PopupStyle {
    /// Draw a rounded border (and the `title`) around the band.
    pub frame: bool,
    /// Band width source.
    pub width: WidthSpec,
    /// Maximum rows shown (content shorter than this shrinks the band).
    pub height: u16,
    /// Draw a dim full-width rule under the rows.
    pub separator: bool,
    /// Patched onto every span of the selected row.
    pub highlight: Style,
}

impl Default for PopupStyle {
    fn default() -> Self {
        Self {
            frame: false,
            width: WidthSpec::Fill,
            height: 9,
            separator: true,
            // Selection chrome is background + emphasis only. A `fg` here would
            // be patched onto every span of the row and flatten the product's
            // own two-tone (a bright name beside a dim description) — see
            // `phimint`'s `slash_lines`, which colours the description itself.
            highlight: Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        }
    }
}

impl PopupStyle {
    /// Framed, fixed-width variant: border on, separator off (the border is
    /// the boundary). The old popups' geometry, without their header rows.
    pub fn framed(width: u16, height: u16) -> Self {
        Self {
            frame: true,
            width: WidthSpec::Fixed(width),
            height,
            separator: false,
            ..Default::default()
        }
    }

    /// Row-content width for a band `band_w` columns wide: the band minus the
    /// frame's two border columns. Row builders must budget their text against
    /// THIS rather than [`band_width`], because `render_at` draws the rows
    /// inside `Block::inner` — budgeting against the outer band makes every row
    /// up to 2 columns too wide once `frame` is on, and `Paragraph` hard-clips
    /// the overflow instead of the elision marking the cut.
    pub fn content_width(&self, band_w: u16) -> u16 {
        band_w.saturating_sub(u16::from(self.frame) * 2)
    }

    /// Rows of chrome the band adds around the content (border + separator).
    fn chrome(&self) -> u16 {
        u16::from(self.frame) * 2 + u16::from(self.separator)
    }

    /// Total band height for `rows` content rows, including frame/separator.
    /// The content floor is 1: `height: 0` must not shrink the band to bare
    /// chrome (a framed band would be an empty border box).
    fn band_height(&self, rows: u16) -> u16 {
        rows.min(self.height).max(1) + self.chrome()
    }
}

/// Band width for `style` under `anchor`, clamped to `term`.
pub fn band_width(anchor: Rect, term: Rect, spec: &WidthSpec) -> u16 {
    match *spec {
        // `Fill` matches the anchor (the composer). The composer spans the full
        // terminal width, and the spec's default is "same width as the input
        // box", so no margin is deducted here — only a defensive cap at
        // `term.width` so a stale anchor rect can never overflow.
        WidthSpec::Fill => anchor.width.min(term.width).max(1),
        // A fixed width that exceeds the terminal keeps a 2-column margin.
        WidthSpec::Fixed(w) => w.min(term.width.saturating_sub(2)).max(1),
    }
}

/// The rows to show: a `height`-row window that keeps `selected` visible,
/// centred the same way the pre-widget popups were.
pub fn visible_window(total: usize, height: usize, selected: usize) -> std::ops::Range<usize> {
    let height = height.max(1);
    if total <= height {
        return 0..total;
    }
    let start = if selected < height / 2 {
        0
    } else {
        (selected - height / 2).min(total - height)
    };
    start..start + height
}

/// Band rect for a `height`-row band (content + frame + separator, computed by
/// the caller), anchored above `anchor` (the composer). The band never exceeds
/// `term.height - 2` and always leaves one row at the top.
pub fn popup_rect(anchor: Rect, term: Rect, width: u16, height: u16) -> Rect {
    let height = height.min(term.height.saturating_sub(2));
    let x = anchor.x.min(term.width.saturating_sub(width));
    let y = anchor.y.saturating_sub(height).max(1);
    Rect::new(x, y, width, height)
}

/// The band itself: rows + selection gutter, drawn above an anchor rect.
pub struct PopupList<'a> {
    /// Product-built rows; the widget adds the gutter and the highlight.
    pub rows: Vec<Line<'a>>,
    /// Index of the highlighted row.
    pub selected: usize,
    /// Band look.
    pub style: PopupStyle,
    /// Frame title, drawn only when `style.frame` is set.
    pub title: Option<String>,
}

impl PopupList<'_> {
    /// Draw the band anchored above `anchor` (the composer rect).
    pub fn render_at(&self, f: &mut Frame, anchor: Rect) {
        let term = f.area();
        // A band needs 2 columns (width ≥ 1) and 3 rows (one row of content +
        // the single row of headroom `popup_rect` reserves). `draw()` has no
        // composer fit guard, so this is the only thing standing between a
        // degenerate terminal and an out-of-bounds rect.
        if self.rows.is_empty() || term.width < 2 || term.height < 3 {
            return;
        }
        let width = band_width(anchor, term, &self.style.width);
        // `popup_rect` clamps the band to `term.height - 2`, so a window sized
        // from `style.height` alone would have its tail clipped by `Paragraph`
        // on a short terminal — hiding the selected row and the separator.
        let capacity = term
            .height
            .saturating_sub(2)
            .saturating_sub(self.style.chrome());
        let window = visible_window(
            self.rows.len(),
            (self.style.height.min(capacity)).max(1) as usize,
            self.selected,
        );
        let rect = popup_rect(
            anchor,
            term,
            width,
            self.style.band_height(window.len() as u16),
        );
        f.render_widget(Clear, rect);

        let block = if self.style.frame {
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(self.title.clone().unwrap_or_default())
        } else {
            Block::default()
        };
        let inner = block.inner(rect);

        let mut lines: Vec<Line> = Vec::with_capacity(window.len() + 1);
        for (n, row) in self.rows[window.clone()].iter().enumerate() {
            let selected = window.start + n == self.selected;
            let mut spans: Vec<Span> = Vec::with_capacity(row.spans.len() + 1);
            spans.push(Span::raw(if selected { "▸ " } else { "  " }));
            for span in &row.spans {
                let mut span = span.clone();
                if selected {
                    span.style = span.style.patch(self.style.highlight);
                }
                spans.push(span);
            }
            lines.push(Line::from(spans));
        }

        if self.style.separator {
            lines.push(Line::from(Span::styled(
                "─".repeat(inner.width as usize),
                Style::default().fg(Color::DarkGray),
            )));
        }

        f.render_widget(Paragraph::new(lines).block(block), rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::text::Line;
    // `Color`, `Modifier`, `Style`, `Rect` come in through `use super::*`.

    fn term() -> Rect {
        Rect::new(0, 0, 100, 30)
    }

    fn composer() -> Rect {
        Rect::new(0, 26, 100, 4)
    }

    #[test]
    fn band_width_fills_anchor_or_fixes_and_clamps() {
        assert_eq!(band_width(composer(), term(), &WidthSpec::Fill), 100);
        assert_eq!(band_width(composer(), term(), &WidthSpec::Fixed(64)), 64);
        // Fixed wider than the terminal is clamped with a 2-column margin.
        assert_eq!(band_width(composer(), term(), &WidthSpec::Fixed(999)), 98);
    }

    #[test]
    fn visible_window_centres_on_selection_and_clamps_at_ends() {
        // Everything fits: full range regardless of selection.
        assert_eq!(visible_window(5, 9, 3), 0..5);
        // Selection near the top: window pinned to the start.
        assert_eq!(visible_window(50, 9, 1), 0..9);
        // Selection mid-list: centred (4 rows above, so the row sits mid-window).
        assert_eq!(visible_window(50, 9, 20), 16..25);
        // Selection at the end: window pinned to the last page.
        assert_eq!(visible_window(50, 9, 49), 41..50);
    }

    #[test]
    fn popup_rect_sits_above_the_anchor_within_the_terminal() {
        // `height` is the whole band: 3 content rows + 1 separator row = 4, and
        // the bottom edge lands on the composer's top row (y = 26).
        let r = popup_rect(composer(), term(), 100, 4);
        assert_eq!((r.x, r.y, r.width, r.height), (0, 22, 100, 4));
        // Tall content is clamped to term.height - 2 and pushed down to y = 1.
        let tall = popup_rect(composer(), term(), 100, 29);
        assert_eq!((tall.y, tall.height), (1, 28));
        // Never leaves column 0 / row 0.
        let narrow = popup_rect(Rect::new(0, 1, 10, 2), Rect::new(0, 0, 10, 30), 10, 5);
        assert_eq!(narrow.y, 1);
    }

    #[test]
    fn popup_rect_degenerates_on_a_tiny_terminal() {
        let r = popup_rect(composer(), Rect::new(0, 0, 100, 2), 100, 5);
        assert_eq!(r.height, 0);
    }

    /// Flatten a test buffer into text, one line per row.
    fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
        let area = buf.area();
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(area.x + x, area.y + y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn draw_list(list: &PopupList, anchor: Rect) -> String {
        let mut term = Terminal::new(TestBackend::new(40, 12)).unwrap();
        term.draw(|f| list.render_at(f, anchor)).unwrap();
        buffer_text(term.backend().buffer())
    }

    #[test]
    fn default_style_is_frameless_full_width_with_separator() {
        let style = PopupStyle::default();
        assert!(!style.frame);
        assert_eq!(style.width, WidthSpec::Fill);
        assert_eq!(style.height, 9);
        assert!(style.separator);
        // Highlight is background + emphasis only: an `fg` here would be patched
        // onto every span and flatten the product's two-tone selected row.
        assert_eq!(style.highlight.fg, None);
        assert_eq!(style.highlight.bg, Some(Color::DarkGray));
        assert!(style.highlight.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn highlight_keeps_the_rows_own_foreground() {
        // A selected row must stay two-tone: the widget patches `highlight` over
        // each span's style, so any `fg` in the highlight would win over the
        // span's own colour and make a dim description as bright as the name.
        let list = PopupList {
            rows: vec![Line::from(vec![
                Span::styled("name", Style::default().fg(Color::White)),
                Span::styled("desc", Style::default().fg(Color::DarkGray)),
            ])],
            selected: 0,
            style: PopupStyle::default(),
            title: None,
        };
        let mut term = Terminal::new(TestBackend::new(40, 12)).unwrap();
        term.draw(|f| list.render_at(f, Rect::new(0, 9, 40, 3)))
            .unwrap();
        let buf = term.backend().buffer();
        let area = buf.area();
        let find = |sym: &str| {
            (0..area.height)
                .flat_map(|y| (0..area.width).map(move |x| (x, y)))
                .map(|(x, y)| &buf[(x, y)])
                .find(|c| c.symbol() == sym)
                .map(|c| (c.fg, c.bg, c.modifier))
                .unwrap_or_else(|| panic!("`{sym}` not drawn"))
        };
        let (name_fg, name_bg, name_mod) = find("n");
        let (desc_fg, ..) = find("d");
        // Prove the highlight path actually ran (otherwise the fgs below hold
        // trivially and the test would pass on a broken selection).
        assert_eq!(name_bg, Color::DarkGray, "row not highlighted");
        assert!(name_mod.contains(Modifier::BOLD), "row not emphasised");
        // Each span keeps its own fg under the highlight.
        assert_eq!(name_fg, Color::White, "name fg clobbered");
        assert_eq!(desc_fg, Color::DarkGray, "desc fg clobbered");
    }

    #[test]
    fn framed_preset_is_framed_fixed_width_without_separator() {
        let style = PopupStyle::framed(64, 9);
        assert!(style.frame);
        assert_eq!(style.width, WidthSpec::Fixed(64));
        assert!(!style.separator);
    }

    #[test]
    fn content_width_drops_the_frame_border_columns() {
        // Row builders budget against `content_width`, not the outer band: a
        // framed band spends 2 columns on its border.
        assert_eq!(PopupStyle::default().content_width(40), 40);
        assert_eq!(PopupStyle::framed(30, 9).content_width(30), 28);
        // `separator` costs height, never width.
        let mut style = PopupStyle::framed(30, 9);
        style.separator = true;
        assert_eq!(style.content_width(30), 28);
        // Saturates instead of underflowing on a band too narrow for a border.
        assert_eq!(PopupStyle::framed(1, 9).content_width(1), 0);
    }

    #[test]
    fn frameless_band_draws_gutter_separator_and_highlight() {
        let list = PopupList {
            rows: vec![Line::from("alpha"), Line::from("beta")],
            selected: 1,
            style: PopupStyle::default(),
            title: None,
        };
        // Composer band sits at y = 9..12; the popup grows upward from y = 9.
        let text = draw_list(&list, Rect::new(0, 9, 40, 3));
        assert!(
            text.contains("  alpha"),
            "unselected gutter missing:\n{text}"
        );
        assert!(text.contains("▸ beta"), "selected gutter missing:\n{text}");
        assert!(text.contains('─'), "separator missing:\n{text}");
        assert!(
            !text.contains('╭'),
            "frameless style drew a border:\n{text}"
        );
    }

    #[test]
    fn zero_height_still_draws_a_content_row() {
        // `height: 0` (from a hand-built style, or a config that skipped the
        // clamp) must not shrink the band to bare chrome.
        let mut style = PopupStyle::default();
        style.height = 0;
        let list = PopupList {
            rows: vec![Line::from("alpha"), Line::from("beta")],
            selected: 0,
            style,
            title: None,
        };
        let text = draw_list(&list, Rect::new(0, 9, 40, 3));
        assert!(text.contains("▸ alpha"), "content row missing:\n{text}");
    }

    #[test]
    fn framed_band_draws_border_and_title() {
        let list = PopupList {
            rows: vec![Line::from("alpha")],
            selected: 0,
            style: PopupStyle::framed(20, 9),
            title: Some("skills".into()),
        };
        let text = draw_list(&list, Rect::new(0, 9, 40, 3));
        assert!(
            text.contains('╭') && text.contains('╰'),
            "border missing:\n{text}"
        );
        assert!(text.contains("skills"), "title missing:\n{text}");
    }

    #[test]
    fn empty_rows_draw_nothing() {
        let list = PopupList {
            rows: Vec::new(),
            selected: 0,
            style: PopupStyle::default(),
            title: None,
        };
        let text = draw_list(&list, Rect::new(0, 9, 40, 3));
        assert!(text.trim().is_empty(), "expected blank frame, got:\n{text}");
    }

    #[test]
    fn render_at_is_a_noop_on_a_degenerate_terminal() {
        let list = PopupList {
            rows: vec![Line::from("alpha")],
            selected: 0,
            style: PopupStyle::default(),
            title: None,
        };
        // 1 column / 2 rows: the guard returns before any rect maths.
        for (w, h) in [(1u16, 12u16), (40u16, 2u16)] {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let anchor = Rect::new(0, h.saturating_sub(3), w, 3);
            term.draw(|f| list.render_at(f, anchor)).unwrap();
        }
    }

    #[test]
    fn scroll_window_keeps_the_selected_row_visible() {
        let rows: Vec<Line> = (0..20).map(|i| Line::from(format!("row{i}"))).collect();
        let list = PopupList {
            rows,
            selected: 15,
            style: PopupStyle::default(),
            title: None,
        };
        let text = draw_list(&list, Rect::new(0, 9, 40, 3));
        assert!(text.contains("▸ row15"), "selected row off-window:\n{text}");
        assert!(!text.contains("row0"), "window did not scroll:\n{text}");
    }

    #[test]
    fn short_terminal_keeps_the_selected_row_visible() {
        // A 10-row terminal clamps the band to 8 rows via `popup_rect`. Sizing
        // the window from `style.height` alone (9) would push the last row and
        // the separator past the clip, hiding the highlight.
        let rows: Vec<Line> = (0..20).map(|i| Line::from(format!("row{i}"))).collect();
        let list = PopupList {
            rows,
            selected: 19,
            style: PopupStyle::default(),
            title: None,
        };
        let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
        term.draw(|f| list.render_at(f, Rect::new(0, 7, 40, 3)))
            .unwrap();
        let text = buffer_text(term.backend().buffer());
        assert!(text.contains("▸ row19"), "selected row clipped:\n{text}");
        assert!(text.contains('─'), "separator clipped:\n{text}");
    }
}

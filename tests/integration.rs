//! Cross-module integration tests: the full "commit → wrap → scroll" chain
//! and the markdown/selection copy consistency invariant.

use phi_tui::lines::{LineKind, OutputLine};
use phi_tui::markdown::{line_plain_text, render_markdown};
use phi_tui::selection::SelectionState;
use phi_tui::transcript::Transcript;
use phi_tui::viewport::Viewport;
use phi_tui::wrap::wrap;

/// transcript + wrap + viewport full chain: committed lines re-wrap on width
/// change, and the viewport window always indexes into exactly those lines.
#[test]
fn transcript_wrap_viewport_chain() {
    let mut t: Transcript = Transcript::new();
    t.push_user("abcdefghij klmnopqrst uvwxyz");
    let before = t.len();

    // Narrow terminal: the committed user line re-wraps.
    t.set_wrap_width(10);
    let lines: Vec<String> = t.output.iter().map(|l| l.text.clone()).collect();
    assert!(t.len() > before, "narrow wrap splits into more lines");
    assert!(lines.iter().all(|l| l.chars().count() <= 12), "prefix adds ≤2 cols: {lines:?}");

    // The viewport shows the bottom `height` lines of exactly this transcript.
    let mut vp = Viewport::new();
    let height = 1.min(t.len());
    let window = vp.window_range(t.len(), height);
    assert_eq!(window, (t.len() - height)..t.len());
    for i in window.clone() {
        assert!(t.output.get(i).is_some(), "window index {i} out of range");
    }

    // Scrolling up moves the window; scrolling back re-pins to the bottom.
    assert!(vp.scroll_up(1));
    let scrolled = vp.window_range(t.len(), height);
    assert!(scrolled.end <= window.end || t.len() <= height);
    assert!(vp.scroll_down(1));
    assert!(vp.follow_bottom);
}

/// Markdown expansion text vs selection copy consistency: the plain text the
/// renderer displays per visual line is exactly what a full-range selection
/// copies back (what-you-see-is-what-you-copy).
#[test]
fn markdown_visual_text_matches_selection_copy() {
    let src = "# Title\n\nplain **bold** tail\n\n- one\n- two\n";
    let rendered = render_markdown(src);
    assert!(!rendered.is_empty());

    let visual: Vec<String> = rendered.iter().map(line_plain_text).collect();
    // Markdown markers are gone from the display text.
    assert_eq!(visual[0], "Title", "heading renders without the # marker");
    assert!(visual.iter().all(|l| !l.contains('#') || l.starts_with("Title")));

    // A selection spanning every visual line copies exactly the displayed text.
    let mut sel = SelectionState::new();
    sel.anchor(Some(0));
    sel.extend(Some(visual.len() - 1));
    let copied = sel.text(&Vec::<String>::new(), &visual);
    assert_eq!(copied, visual.join("\n"));

    // The raw-line fallback path (before the first render) agrees per line.
    let raw: Vec<String> = visual.clone();
    let no_visual: Vec<String> = Vec::new();
    let copied_fallback = sel.text(&raw, &no_visual);
    assert_eq!(copied_fallback, visual.join("\n"));
}

/// A flushed streaming tail commits as one Normal line carrying the original
/// source, and lands in the transcript through the same `push` path the
/// product uses — original preserved for later re-wrap.
#[test]
fn stream_flush_commits_into_transcript() {
    use phi_tui::stream::StreamState;

    let mut t: Transcript = Transcript::new();
    let mut st: StreamState = StreamState::new(100);
    let flushed = st.push_text("hello **world**", None);
    assert_eq!(flushed.len(), 0, "no flush until a structural event");
    let flushed = st.flush();
    assert_eq!(flushed.len(), 1);
    assert_eq!(flushed[0].kind, LineKind::Normal);
    t.extend(flushed);
    assert_eq!(t.output[0].original.as_deref(), Some("hello **world**"));
    assert_eq!(t.output[0].text, "hello **world**");
    assert!(t.output[0].spans.is_none(), "streamed prose carries no styled spans");
}

/// The anchored replaceable block: a second full-plan push splices the first
/// in place, and clearing makes the next push append.
#[test]
fn anchored_block_replaces_in_place_then_appends() {
    fn plan(text: &str) -> OutputLine {
        OutputLine {
            text: text.to_string(),
            kind: LineKind::Plan,
            spans: None,
            original: None,
            detail: None,
        }
    }
    let mut t: Transcript = Transcript::new();
    t.push(OutputLine { text: "before".into(), kind: LineKind::Normal, spans: None, original: None, detail: None });
    t.replace_plan(vec![plan("plan v1")]);
    t.replace_plan(vec![plan("plan v2a"), plan("plan v2b")]);
    let texts: Vec<&str> = t.output.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, vec!["before", "plan v2a", "plan v2b"], "replaced in place");
    t.clear_plan();
    t.replace_plan(vec![plan("plan v3")]);
    let texts: Vec<&str> = t.output.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, vec!["before", "plan v2a", "plan v2b", "plan v3"], "appends after clear");
}

/// wrap is CJK-aware; one_line truncates by char count with an ellipsis.
#[test]
fn wrap_is_cjk_aware_and_one_line_truncates() {
    use unicode_width::UnicodeWidthStr;

    // CJK glyphs are 2 columns each: 4 glyphs split in half at width 4.
    assert_eq!(wrap("你好世界", 4), vec!["你好", "世界"]);
    // Every wrapped piece stays within the display-column budget.
    let line = "中文内容每两个字符占四列宽";
    for piece in wrap(line, 6) {
        assert!(piece.width() <= 6, "piece exceeds 6 columns: {piece:?}");
    }
    // one_line truncates by chars (not columns) and marks the cut.
    let clipped = phi_tui::wrap::one_line(line, 6);
    assert_eq!(clipped, "中文内容每两…");
}

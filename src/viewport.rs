//! Scroll viewport state: `scroll_offset` / `follow_bottom` and the visible-window maths.
//!
//! Extracted out of the `App` god-object (previously two fields on App plus
//! `scroll_up` / `scroll_down` / `window_range`). App holds one instance: the
//! render layer asks for the visible window each frame and the wheel drives it.

use std::ops::Range;

/// The scrollable output viewport: how far the transcript is scrolled up from
/// the bottom, whether it is pinned to the newest lines, and the last known
/// rendered size (used to clamp the offset to the useful range).
#[derive(Debug)]
pub struct Viewport {
    /// Lines scrolled up from the bottom (`0` while following the bottom).
    pub scroll_offset: usize,
    /// Whether the output view is pinned to the newest lines.
    pub follow_bottom: bool,
    /// Pinned to the very top of the content (resume replay landing spot).
    /// `window_range` shows `0..height` while set; any manual scroll clears
    /// it by converting the pin into an equivalent `scroll_offset`.
    pub pin_top: bool,
    /// Last known rendered line count (updated by `render_output` each frame).
    pub rendered_total: usize,
    /// Last known output-pane height in rows (updated by `render_output`).
    /// `0` before the first render — scroll_up then cannot clamp yet.
    pub viewport_height: usize,
}

impl Viewport {
    pub fn new() -> Self {
        Self {
            scroll_offset: 0,
            follow_bottom: true,
            pin_top: false,
            rendered_total: 0,
            viewport_height: 0,
        }
    }

    /// Record the last rendered total + pane height (from `render_output`).
    /// Head-index variant of [`Self::set_visible_anchored`] (`preferred_head`
    /// = `None`).
    pub fn set_visible(&mut self, total: usize, height: usize) {
        self.set_visible_anchored(total, height, None);
    }

    /// Record the last rendered total + pane height, keeping the reader's
    /// place while the flow reflows underneath them.
    ///
    /// `preferred_head` is the flow row (in the NEW numbering) that should sit
    /// at the window top — the caller resolves it from the previous frame's
    /// visual→output mapping, so mid-transcript reflows (Ctrl+O fold/expand,
    /// resize re-wrap) keep the place by CONTENT, not by raw index. `None`
    /// falls back to holding the raw head index: right for tail-only mutation
    /// (streaming commits, thinking-panel placeholder rows) and for tail rows
    /// the caller cannot map to an output line.
    ///
    /// Same-frame manual scrolls compose: they adjust `scroll_offset` first,
    /// and this preserves their new head through the size change. Landing on
    /// offset `0` re-enters follow-bottom — visually at the bottom but not
    /// following would strand the next tail growth outside the window.
    pub fn set_visible_anchored(
        &mut self,
        total: usize,
        height: usize,
        preferred_head: Option<usize>,
    ) {
        if !self.follow_bottom && !self.pin_top && self.viewport_height > 0 {
            let new_max = total.saturating_sub(height);
            let target_head = match preferred_head {
                Some(head) => head,
                None => {
                    let prev_max = self.rendered_total.saturating_sub(self.viewport_height);
                    prev_max - self.scroll_offset.min(prev_max)
                }
            };
            self.scroll_offset = new_max.saturating_sub(target_head);
            if self.scroll_offset == 0 {
                self.follow_bottom = true;
            }
        }
        self.rendered_total = total;
        self.viewport_height = height;
    }

    /// Scroll up by `step` lines. Returns `true` if the viewport moved.
    /// Once the viewport knows its rendered size, the offset is clamped to the
    /// maximum useful offset (past the top of the content).
    pub fn scroll_up(&mut self, step: usize) -> bool {
        if self.pin_top {
            // Already showing the head; convert the pin into its equivalent
            // offset so the clamp below keeps the view where it is.
            self.pin_top = false;
            self.scroll_offset = self.max_offset();
        }
        let old = self.scroll_offset;
        self.follow_bottom = false;
        self.scroll_offset += step;
        // Clamp to the maximum useful offset using the last known rendered
        // line count and pane height. This prevents scroll_offset from
        // growing past the top of content.
        if self.viewport_height > 0 {
            let max_offset = self.rendered_total.saturating_sub(self.viewport_height);
            if self.scroll_offset > max_offset {
                self.scroll_offset = max_offset;
            }
        }
        tracing::debug!(
            old_offset = old,
            new_offset = self.scroll_offset,
            rendered_total = self.rendered_total,
            viewport_height = self.viewport_height,
            "scroll_up"
        );
        self.scroll_offset != old
    }

    /// Scroll down by `step` lines. Returns `true` if the viewport moved;
    /// re-enters follow-bottom when the scroll reaches the bottom.
    pub fn scroll_down(&mut self, step: usize) -> bool {
        if self.follow_bottom {
            tracing::debug!("scroll_down: already at bottom");
            return false;
        }
        if self.pin_top {
            // Leave the top pin: start from an equivalent offset (the head)
            // so the subtraction below moves one real page down.
            self.pin_top = false;
            self.scroll_offset = self.max_offset();
        }
        let old = self.scroll_offset;
        if self.scroll_offset <= step {
            self.scroll_offset = 0;
            self.follow_bottom = true;
        } else {
            self.scroll_offset -= step;
        }
        tracing::debug!(
            old_offset = old,
            new_offset = self.scroll_offset,
            follow_bottom = self.follow_bottom,
            "scroll_down"
        );
        true
    }

    /// Pin the viewport to the very top of the content: `window_range` shows
    /// `0..height` while the pin holds; manual scrolls convert it into an
    /// equivalent offset via [`Self::max_offset`]. Used after a resume
    /// replay: the user re-enters the session to READ the history, so the
    /// view starts at its head (not the tail).
    pub fn scroll_to_top(&mut self) {
        self.follow_bottom = false;
        self.pin_top = true;
        self.scroll_offset = 0;
    }

    /// Snap back to the newest lines (re-enter follow-bottom). Used when the
    /// user sends a message while scrolled up, so the reply streams into
    /// view instead of below the fold.
    pub fn scroll_to_bottom(&mut self) {
        self.follow_bottom = true;
        self.pin_top = false;
        self.scroll_offset = 0;
    }

    /// Offset that puts the window at the very top of the content
    /// (`start = 0`), given the last known rendered size.
    fn max_offset(&self) -> usize {
        self.rendered_total.saturating_sub(self.viewport_height)
    }

    /// Lines a PageUp/PageDown press moves: half a screen. One line per
    /// press makes multi-hundred-line scrollback (a resumed conversation)
    /// effectively unnavigable — 900 presses to walk the history.
    pub fn page_step(&self) -> usize {
        (self.viewport_height / 2).max(1)
    }

    /// The `[start, end)` range of `total` lines to show in a window of
    /// `height` rows, honoring `follow_bottom` and `scroll_offset`.
    pub fn window_range(&self, total: usize, height: usize) -> Range<usize> {
        if total <= height {
            tracing::debug!(total, height, "window_range: content fits, 0..total");
            return 0..total;
        }
        if self.pin_top {
            tracing::debug!(total, height, "window_range: pinned to top");
            return 0..height;
        }
        if self.follow_bottom {
            let start = total - height;
            tracing::debug!(total, height, start, "window_range: follow_bottom");
            return start..total;
        }
        let max_offset = total - height;
        let offset = self.scroll_offset.min(max_offset);
        let start = max_offset - offset;
        tracing::debug!(
            total,
            height,
            raw_scroll_offset = self.scroll_offset,
            max_offset,
            clamped_offset = offset,
            start,
            end = (start + height).min(total),
            "window_range"
        );
        start..(start + height).min(total)
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_range_follows_bottom_when_pinned() {
        let v = Viewport::new();
        assert_eq!(v.window_range(100, 30), 70..100);
        // Content fits: 0..total regardless of scroll.
        assert_eq!(v.window_range(10, 30), 0..10);
    }

    #[test]
    fn window_range_honors_scroll_offset() {
        let mut v = Viewport::new();
        v.follow_bottom = false;
        v.scroll_offset = 10;
        assert_eq!(v.window_range(100, 30), 60..90);
        // Past the top clamps to the first `height` lines.
        v.scroll_offset = 1000;
        assert_eq!(v.window_range(100, 30), 0..30);
    }

    /// Resume replay pins the view to the conversation head: the first
    /// render shows 0..height, and PgDn moves one real page down from there.
    #[test]
    fn scroll_to_top_pins_window_to_head() {
        let mut v = Viewport::new();
        v.scroll_to_top();
        assert!(!v.follow_bottom);
        assert_eq!(v.window_range(942, 54), 0..54);
        // PgDn from the pin: one real page down, still not following.
        v.set_visible(942, 54);
        assert!(v.scroll_down(20));
        assert!(!v.follow_bottom);
        assert_eq!(v.window_range(942, 54), 20..74);
        // PgUp back to the top: one page up lands exactly at the head.
        assert!(v.scroll_up(20));
        assert_eq!(v.window_range(942, 54), 0..54);
        // Further PgUp at the head is a no-op (clamped, view unchanged).
        assert!(!v.scroll_up(20), "already at the head: no movement");
        assert_eq!(v.window_range(942, 54), 0..54);
    }

    /// Sending a message while scrolled up snaps back to the newest lines.
    #[test]
    fn scroll_to_bottom_reenters_follow() {
        let mut v = Viewport::new();
        v.scroll_to_top();
        v.scroll_to_bottom();
        assert!(v.follow_bottom);
        assert!(!v.pin_top);
        assert_eq!(v.scroll_offset, 0);
        assert_eq!(v.window_range(100, 30), 70..100);
    }

    /// PgUp/PgDn move half a screen per press (never zero): one line per
    /// press made a resumed multi-hundred-line history unnavigable.
    #[test]
    fn page_step_is_half_viewport_with_floor_of_one() {
        let mut v = Viewport::new();
        assert_eq!(v.page_step(), 1, "degenerate height still moves");
        v.viewport_height = 41;
        assert_eq!(v.page_step(), 20);
        v.viewport_height = 1;
        assert_eq!(v.page_step(), 1);
    }

    #[test]
    fn scroll_up_leaves_follow_bottom() {
        let mut v = Viewport::new();
        assert!(v.follow_bottom);
        assert!(v.scroll_up(1));
        assert!(!v.follow_bottom);
        assert_eq!(v.scroll_offset, 1);
    }

    #[test]
    fn scroll_up_clamps_to_rendered_top() {
        let mut v = Viewport::new();
        v.set_visible(100, 30);
        v.follow_bottom = false;
        v.scroll_offset = 60;
        // 60 + 1 would exceed max_offset (100-30=70)? No: 61 <= 70, stays.
        assert!(v.scroll_up(1));
        assert_eq!(v.scroll_offset, 61);
        // Past the top clamps to the first pane.
        v.scroll_offset = 69;
        assert!(v.scroll_up(5));
        assert_eq!(v.scroll_offset, 70);
        assert!(!v.scroll_up(1), "already at the top: no movement");
    }

    #[test]
    fn scroll_down_reenters_follow_bottom() {
        let mut v = Viewport::new();
        v.scroll_up(2);
        assert_eq!(v.scroll_offset, 2);
        // Down one step: still scrolled up.
        assert!(v.scroll_down(1));
        assert_eq!(v.scroll_offset, 1);
        assert!(!v.follow_bottom);
        // Down the remaining step: back at bottom.
        assert!(v.scroll_down(1));
        assert!(v.follow_bottom);
        assert_eq!(v.scroll_offset, 0);
        // Already at bottom: no-op.
        assert!(!v.scroll_down(1));
    }

    #[test]
    fn set_visible_holds_head_when_tail_grows() {
        let mut v = Viewport::new();
        v.set_visible(100, 20);
        v.scroll_up(10); // head at 70
        assert_eq!(v.window_range(100, 20), 70..90);
        v.set_visible(108, 20); // +8 rows appended at the tail
        assert_eq!(
            v.window_range(108, 20),
            70..90,
            "history view must not jump when the tail grows"
        );
    }

    #[test]
    fn set_visible_holds_head_when_tail_shrinks() {
        let mut v = Viewport::new();
        v.set_visible(100, 20);
        v.scroll_up(10);
        v.set_visible(93, 20); // -7 rows (e.g. thinking-panel rows collapse)
        assert_eq!(v.window_range(93, 20), 70..90);
    }

    #[test]
    fn set_visible_follow_bottom_keeps_tailing() {
        let mut v = Viewport::new();
        v.set_visible(100, 20);
        v.set_visible(108, 20);
        assert_eq!(v.window_range(108, 20), 88..108);
    }

    #[test]
    fn same_frame_scroll_and_tail_growth_compose() {
        let mut v = Viewport::new();
        v.set_visible(100, 20);
        v.scroll_up(5); // head at 75
        v.set_visible(108, 20); // growth absorbed, scroll kept
        assert_eq!(v.window_range(108, 20), 75..95);
    }

    #[test]
    fn set_visible_anchored_prefers_content_head() {
        let mut v = Viewport::new();
        v.set_visible(100, 20);
        v.scroll_up(10); // head at 70
        // Middle reflow: the row that was at 70 is now at 75.
        v.set_visible_anchored(120, 20, Some(75));
        assert_eq!(v.window_range(120, 20), 75..95);
    }

    #[test]
    fn shrink_clamp_reenters_follow_bottom() {
        let mut v = Viewport::new();
        v.set_visible(100, 20);
        v.scroll_up(3); // barely scrolled
        v.set_visible(80, 20); // huge tail collapse clamps the offset to 0
        assert_eq!(v.scroll_offset, 0);
        assert!(
            v.follow_bottom,
            "at the bottom must mean following, or the next tail growth strands"
        );
    }
}

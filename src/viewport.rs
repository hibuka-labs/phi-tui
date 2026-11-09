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
    /// Frozen window head while a follow-bottom view absorbs a tail shrink
    /// (a thinking panel collapsing 8 placeholder rows into a 1-row summary
    /// is a net −7): re-pinning to the bottom would drag the read content
    /// down by the delta, so the head stays put and the vacated rows refill
    /// as new records append. Released back to following once the flow
    /// covers the window again (or the held row is gone); any manual scroll
    /// converts it to a plain offset.
    pub held_head: Option<usize>,
    /// Set by [`Self::scroll_to_bottom`], consumed by the next
    /// [`Self::set_visible_anchored`]. An explicit jump to the bottom leaves
    /// no reading position to protect: a same-frame tail shrink must not
    /// resurrect the abandoned head as a hold.
    jumped_to_bottom: bool,
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
            held_head: None,
            jumped_to_bottom: false,
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
    ///
    /// A tail shrink under a following view is absorbed by [`Self::held_head`]
    /// instead: the head freezes and the vacated rows refill as the flow
    /// grows, so a block collapsing never drags the visible content down.
    pub fn set_visible_anchored(
        &mut self,
        total: usize,
        height: usize,
        preferred_head: Option<usize>,
    ) {
        self.set_visible_inner(total, height, preferred_head, true);
    }

    /// [`Self::set_visible_anchored`] for a frame whose totals changed by a
    /// mid-flow MOVE (rows removed mid-buffer and re-appended at the tail —
    /// the plan block's re-paste). A move is never a tail shrink, so the
    /// tail-shrink hold must not engage even when the move nets rows out:
    /// under follow-bottom the view stays pinned to the bottom (the
    /// re-pasted block is the point — it must be visible), and a scrolled-up
    /// reader is kept in place by the caller's translated `preferred_head`.
    /// An EXISTING hold (from a real tail shrink) is maintained as usual.
    pub fn set_visible_moved(
        &mut self,
        total: usize,
        height: usize,
        preferred_head: Option<usize>,
    ) {
        self.set_visible_inner(total, height, preferred_head, false);
    }

    fn set_visible_inner(
        &mut self,
        total: usize,
        height: usize,
        preferred_head: Option<usize>,
        tail_shrink_holds: bool,
    ) {
        let just_jumped = std::mem::take(&mut self.jumped_to_bottom);
        if let Some(held) = self.held_head {
            // Hold maintenance. Release back to following when the flow
            // covers the window again (`total == held + height` renders
            // exactly the held window, so the release itself never jumps),
            // when the held row is gone, or when the content fits anyway.
            // Otherwise keep the reader's rows frozen — the content anchor
            // wins (the held row may itself have moved), the raw head
            // otherwise.
            if total >= held.saturating_add(height) || held >= total || total <= height {
                self.held_head = None;
                self.follow_bottom = true;
            } else if let Some(head) = preferred_head {
                self.held_head = Some(head);
            }
        } else if tail_shrink_holds
            && !just_jumped
            && self.follow_bottom
            && !self.pin_top
            && self.viewport_height > 0
            && total < self.rendered_total
            && total > height
        {
            // Tail shrink while following the bottom: hold the head (content
            // anchor preferred, raw previous start otherwise) instead of
            // re-pinning — the collapse must not move what is on screen.
            // `follow_bottom` drops until the hold releases: a frozen head is
            // by definition not at the newest lines.
            let old_start = self.rendered_total.saturating_sub(self.viewport_height);
            self.held_head = Some(preferred_head.unwrap_or(old_start));
            self.follow_bottom = false;
        } else if !self.follow_bottom && !self.pin_top && self.viewport_height > 0 {
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
        if let Some(held) = self.held_head {
            // Walk the held head up one notch at a time; the refilling gap
            // under it folds into an ordinary offset once the head crosses
            // back into the expressible range — no jump in either regime.
            let max_offset = self.max_offset();
            let new_head = held.saturating_sub(step);
            if new_head <= max_offset {
                self.held_head = None;
                self.follow_bottom = false;
                self.scroll_offset = max_offset - new_head;
            } else {
                self.held_head = Some(new_head);
            }
            return new_head != held;
        }
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
        if self.held_head.is_some() {
            // The hold sits at the flow tail: the rows below it are the
            // shrink still refilling. Nothing lies further down.
            return false;
        }
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
        self.held_head = None;
        self.follow_bottom = false;
        self.pin_top = true;
        self.scroll_offset = 0;
    }

    /// Snap back to the newest lines (re-enter follow-bottom). Used when the
    /// user sends a message while scrolled up, so the reply streams into
    /// view instead of below the fold.
    pub fn scroll_to_bottom(&mut self) {
        self.held_head = None;
        self.follow_bottom = true;
        self.pin_top = false;
        self.scroll_offset = 0;
        self.jumped_to_bottom = true;
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
        if let Some(held) = self.held_head {
            // Held tail-shrink: freeze the head and show through the flow
            // end — the short tail (blank rows below the content) is the
            // shrink refilling as new records append. A head past the flow
            // end (the row is gone; maintenance releases it next frame)
            // degrades to the last whole window, never an empty one.
            let start = if held < total {
                held
            } else {
                total.saturating_sub(height)
            };
            tracing::debug!(total, height, held, start, "window_range: held_head");
            return start..(start + height).min(total);
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

    /// A thinking-panel collapse under a following view (8 placeholder rows
    /// folding into a 1-row summary is a net −7) must not move what is on
    /// screen: the head freezes at its content row and the vacated rows
    /// refill as new records append. Re-pinning to the bottom would have
    /// dragged every visible row down by the delta.
    #[test]
    fn follow_bottom_shrink_holds_head_and_refills() {
        let mut v = Viewport::new();
        v.set_visible(100, 30); // following: window 70..100
        assert_eq!(v.window_range(100, 30), 70..100);
        v.set_visible_anchored(93, 30, Some(70)); // collapse −7
        assert_eq!(
            v.window_range(93, 30),
            70..93,
            "the collapse keeps every visible row in place"
        );
        v.set_visible_anchored(96, 30, Some(70)); // +3 rows streamed in
        assert_eq!(v.window_range(96, 30), 70..96);
        v.set_visible_anchored(100, 30, Some(70)); // the gap is refilled
        assert_eq!(v.window_range(100, 30), 70..100);
        v.set_visible(104, 30); // released: tailing again, no jump at release
        assert_eq!(v.window_range(104, 30), 74..104);
    }

    /// Without a content anchor (tail rows the caller cannot map) the raw
    /// previous head is held — same no-jump contract.
    #[test]
    fn follow_bottom_shrink_holds_raw_head_without_anchor() {
        let mut v = Viewport::new();
        v.set_visible(100, 30);
        v.set_visible(93, 30); // set_visible = anchored(None), −7 still holds
        assert_eq!(v.window_range(93, 30), 70..93);
    }

    /// Scrolling up out of a hold walks the head up; the refilling gap folds
    /// into an ordinary offset once the window is whole again — no jump in
    /// either regime.
    #[test]
    fn scroll_up_from_hold_folds_gap_then_offsets() {
        let mut v = Viewport::new();
        v.set_visible(100, 30);
        v.set_visible_anchored(93, 30, Some(70)); // held at 70, gap of 7
        assert!(v.scroll_up(1));
        assert_eq!(v.window_range(93, 30), 69..93);
        for _ in 0..5 {
            assert!(v.scroll_up(1));
        }
        assert_eq!(v.window_range(93, 30), 64..93);
        assert!(v.scroll_up(1)); // 63 is the last whole window: plain offset
        assert_eq!(v.window_range(93, 30), 63..93);
        assert!(v.scroll_up(1));
        assert_eq!(v.window_range(93, 30), 62..92);
    }

    /// The hold sits at the flow tail — scrolling down finds nothing, and an
    /// explicit jump to the bottom releases the hold (and its gap) at once.
    #[test]
    fn hold_is_at_the_tail_until_released() {
        let mut v = Viewport::new();
        v.set_visible(100, 30);
        v.set_visible_anchored(93, 30, Some(70));
        assert!(!v.scroll_down(10), "nothing lies below a tail hold");
        assert_eq!(v.window_range(93, 30), 70..93);
        v.scroll_to_bottom();
        assert!(v.follow_bottom);
        assert_eq!(v.window_range(93, 30), 63..93);
    }

    /// A hold whose head row is gone (the flow was cut away below it)
    /// releases to following instead of stranding a mostly blank pane.
    #[test]
    fn vanished_hold_releases_to_follow() {
        let mut v = Viewport::new();
        v.set_visible(100, 30);
        v.set_visible_anchored(93, 30, Some(70));
        v.set_visible_anchored(50, 30, Some(49));
        assert!(v.follow_bottom);
        assert_eq!(v.window_range(50, 30), 20..50);
    }

    /// An explicit jump to the bottom protects nothing but the bottom: a
    /// same-frame tail shrink must not resurrect the abandoned reading
    /// position as a hold.
    #[test]
    fn sink_then_same_frame_shrink_keeps_the_bottom() {
        let mut v = Viewport::new();
        v.set_visible(100, 30);
        v.scroll_up(40); // reading history at head 30
        v.scroll_to_bottom();
        v.set_visible_anchored(93, 30, Some(30)); // collapse in the same frame
        assert!(v.follow_bottom);
        assert_eq!(v.window_range(93, 30), 63..93);
    }

    /// protects: a mid-flow move that nets rows out (the plan block's
    /// re-paste: shorter block replaced at the tail) keeps a following view
    /// pinned to the bottom — the re-pasted block is the point of the
    /// feature and must be visible. fails_when: the move is mistaken for a
    /// tail shrink and freezes the head with a blank gap under the new
    /// block. no_claim: what the block says (its content is the tool's).
    #[test]
    fn follow_bottom_move_shrink_stays_pinned() {
        let mut v = Viewport::new();
        v.set_visible(100, 30);
        assert_eq!(v.window_range(100, 30), 70..100);
        // Move: −5 rows mid-flow, +2 appended at the tail → net −3.
        v.set_visible_moved(97, 30, None);
        assert!(v.follow_bottom, "a move must not freeze a following view");
        assert_eq!(v.window_range(97, 30), 67..97, "still tailing");
    }

    /// protects: a hold engaged by a real tail shrink survives a later move
    /// and follows its translated head — the reader's frozen rows stay put
    /// while the flow re-pastes underneath. fails_when: the move releases
    /// the hold (re-pinning drags the read content) or holds a stale head.
    /// no_claim: when the hold should release (the release rules are
    /// `set_visible_anchored`'s).
    #[test]
    fn move_maintains_an_existing_hold() {
        let mut v = Viewport::new();
        v.set_visible(100, 30);
        v.set_visible_anchored(93, 30, Some(70)); // real tail shrink, held at 70
        assert_eq!(v.held_head, Some(70));
        // A plan re-paste moves rows: the held content now lives at 66.
        v.set_visible_moved(90, 30, Some(66));
        assert_eq!(v.held_head, Some(66), "hold follows the content");
        assert!(!v.follow_bottom);
        assert_eq!(v.window_range(90, 30), 66..90);
    }
}

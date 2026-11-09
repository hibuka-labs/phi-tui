//! Input-history ring for a composer — mechanism only (no files, no product
//! policy): fish-style recall where the draft you were typing is a slot in
//! the ring, so `Up` past the oldest entry finds it back and `Up` again wraps
//! to the newest. Both directions cycle; `Down` from idle does nothing (the
//! caller keeps its cursor meaning).
//!
//! Recency order (oldest → newest): `entries[0] … entries[n-1]`, then the
//! draft slot `n`. `up` walks older (newest → … → oldest → draft → newest),
//! `down` walks newer (oldest → … → draft → oldest).

/// Cap on remembered entries. The store's file cap is the same number; the
/// ring never needs more than one screenful of recall either way.
pub const HISTORY_CAP: usize = 500;

#[derive(Debug, Default)]
pub struct HistoryRing {
    /// Sent inputs, oldest first, capped at [`HISTORY_CAP`].
    entries: Vec<String>,
    /// `Some(i)` while cycling: an index into `entries`, or `entries.len()`
    /// for the draft slot. `None` = idle.
    pos: Option<usize>,
    /// The composer text captured when the cycle started — what the draft
    /// slot returns.
    draft: String,
    /// What the ring last placed into the composer. When the composer no
    /// longer holds it, the user edited: the next `up` starts a fresh cycle
    /// around their new text, and `down` leaves the composer alone.
    placed: Option<String>,
}

impl HistoryRing {
    pub fn new() -> Self {
        Self::default()
    }

    /// A ring pre-filled from a store (oldest first); the newest
    /// [`HISTORY_CAP`] entries win.
    pub fn from_entries(entries: Vec<String>) -> Self {
        let mut entries = entries;
        if entries.len() > HISTORY_CAP {
            entries.drain(..entries.len() - HISTORY_CAP);
        }
        Self {
            entries,
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether a recall cycle is live (a `down` would move it).
    pub fn cycling(&self) -> bool {
        self.pos.is_some()
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Record a sent input. Blank entries are noise; a repeat of the newest
    /// entry is not a new fact (shell `ignoredups`). Recording always breaks
    /// the cycle — the ring is idle again.
    pub fn push(&mut self, text: &str) {
        self.pos = None;
        self.placed = None;
        if text.trim().is_empty() || self.entries.last().is_some_and(|e| e == text) {
            return;
        }
        if self.entries.len() == HISTORY_CAP {
            self.entries.remove(0);
        }
        self.entries.push(text.to_string());
    }

    /// Older recall (wraparound through the draft slot). `current` is the
    /// composer's text now. `None` only when there is no history.
    pub fn up(&mut self, current: &str) -> Option<String> {
        self.recall(current, false)
    }

    /// Newer recall (wraparound through the draft slot). `None` when idle or
    /// when the composer was edited since the ring placed something in it —
    /// `down` never hijacks text it did not put there.
    pub fn down(&mut self, current: &str) -> Option<String> {
        self.recall(current, true)
    }

    fn recall(&mut self, current: &str, newer: bool) -> Option<String> {
        let n = self.entries.len();
        if n == 0 {
            return None;
        }
        let pos = if self.placed.as_deref() != Some(current) {
            // Fresh (or the user edited a recalled entry): `up` opens a new
            // cycle at the newest entry around what is in the composer now.
            if newer {
                return None;
            }
            self.draft = current.to_string();
            n - 1
        } else {
            match self.pos {
                // Cycling; `placed == current` makes `pos` live.
                Some(p) => {
                    if newer {
                        if p == n { 0 } else { p + 1 }
                    } else if p == 0 {
                        n
                    } else {
                        p - 1
                    }
                }
                None => n - 1,
            }
        };
        self.pos = Some(pos);
        let text = if pos == n {
            self.draft.clone()
        } else {
            self.entries[pos].clone()
        };
        self.placed = Some(text.clone());
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring() -> HistoryRing {
        let mut r = HistoryRing::new();
        r.push("A");
        r.push("B");
        r.push("C");
        r
    }

    #[test]
    fn push_records_and_skips_noise() {
        let mut r = HistoryRing::new();
        r.push("   ");
        r.push("A");
        r.push("A"); // consecutive duplicate: ignoredups
        assert_eq!(r.entries(), &["A".to_string()]);
        r.push("B");
        assert_eq!(r.entries(), &["A".to_string(), "B".to_string()]);
    }

    #[test]
    fn up_walks_older_and_finds_the_draft_back() {
        let mut r = ring();
        // Idle + a draft in the composer: the first up takes the newest.
        assert_eq!(r.up("draft").as_deref(), Some("C"));
        assert_eq!(r.up("C").as_deref(), Some("B"));
        assert_eq!(r.up("B").as_deref(), Some("A"));
        // Past the oldest the draft is in the ring — "循环找回".
        assert_eq!(r.up("A").as_deref(), Some("draft"));
        // And around to the newest again.
        assert_eq!(r.up("draft").as_deref(), Some("C"));
    }

    #[test]
    fn down_walks_newer_through_the_draft() {
        let mut r = ring();
        assert_eq!(r.up("draft").as_deref(), Some("C"));
        assert_eq!(r.up("C").as_deref(), Some("B"));
        assert_eq!(r.up("B").as_deref(), Some("A"));
        // Newer again: B, C, then the draft slot, then around to the oldest.
        assert_eq!(r.down("A").as_deref(), Some("B"));
        assert_eq!(r.down("B").as_deref(), Some("C"));
        assert_eq!(r.down("C").as_deref(), Some("draft"));
        assert_eq!(r.down("draft").as_deref(), Some("A"));
    }

    #[test]
    fn down_is_inert_until_up_starts_a_cycle() {
        let mut r = ring();
        assert_eq!(r.down("draft"), None);
        assert!(!r.cycling());
    }

    #[test]
    fn editing_breaks_the_cycle() {
        let mut r = ring();
        assert_eq!(r.up("draft").as_deref(), Some("C"));
        // The user edits the recalled entry: `down` leaves it alone...
        assert_eq!(r.down("C with edits"), None);
        // ...and the next `up` opens a fresh cycle around the new text.
        assert_eq!(r.up("C with edits").as_deref(), Some("C"));
        assert_eq!(r.up("C").as_deref(), Some("B"));
        assert_eq!(r.up("B").as_deref(), Some("A"));
        assert_eq!(r.up("A").as_deref(), Some("C with edits"));
    }

    #[test]
    fn push_breaks_the_cycle() {
        let mut r = ring();
        assert_eq!(r.up("draft").as_deref(), Some("C"));
        r.push("sent");
        assert!(!r.cycling());
        assert_eq!(r.up("sent").as_deref(), Some("sent"));
    }

    #[test]
    fn the_ring_caps_at_history_cap() {
        let mut r = HistoryRing::new();
        for i in 0..HISTORY_CAP + 10 {
            r.push(&format!("e{i}"));
        }
        assert_eq!(r.entries().len(), HISTORY_CAP);
        assert_eq!(r.entries()[0], format!("e{}", 10));
        assert_eq!(
            r.entries()[HISTORY_CAP - 1],
            format!("e{}", HISTORY_CAP + 9)
        );
    }

    #[test]
    fn empty_ring_recalls_nothing() {
        let mut r = HistoryRing::new();
        assert_eq!(r.up("draft"), None);
        assert_eq!(r.down("draft"), None);
    }
}

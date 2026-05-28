//! Virtual scrolling helper for large lists.

/// Virtual scrolling helper for large lists
///
/// Calculates which items should be visible based on scroll position
/// and viewport size, enabling efficient rendering of large lists.
#[derive(Debug, Clone)]
pub struct VirtualScroller {
    /// Total number of items
    total_items: usize,
    /// Height of each item in pixels
    item_height: f64,
    /// Height of the viewport in pixels
    viewport_height: f64,
    /// Current scroll offset in pixels
    scroll_offset: f64,
    /// Number of items to render above/below visible area (buffer)
    overscan: usize,
}

impl VirtualScroller {
    /// Creates a new virtual scroller
    #[must_use]
    pub const fn new(total_items: usize, item_height: f64, viewport_height: f64) -> Self {
        Self {
            total_items,
            item_height,
            viewport_height,
            scroll_offset: 0.0,
            overscan: 5,
        }
    }

    /// Sets the overscan (buffer) count
    #[must_use]
    pub const fn with_overscan(mut self, overscan: usize) -> Self {
        self.overscan = overscan;
        self
    }

    /// Updates the scroll offset
    pub const fn set_scroll_offset(&mut self, offset: f64) {
        self.scroll_offset = offset.max(0.0);
    }

    /// Updates the viewport height
    pub const fn set_viewport_height(&mut self, height: f64) {
        self.viewport_height = height.max(0.0);
    }

    /// Updates the total item count
    pub const fn set_total_items(&mut self, count: usize) {
        self.total_items = count;
    }

    /// Gets the range of visible items (`start_index`, `end_index`)
    #[must_use]
    #[expect(
        clippy::cast_sign_loss,
        reason = "value is non-negative by construction in this code path"
    )]
    pub fn visible_range(&self) -> (usize, usize) {
        if self.total_items == 0 || self.item_height <= 0.0 {
            return (0, 0);
        }

        let first_visible = (self.scroll_offset / self.item_height).floor() as usize;
        let visible_count = (self.viewport_height / self.item_height).ceil() as usize + 1;

        let start = first_visible.saturating_sub(self.overscan);
        let end = (first_visible + visible_count + self.overscan).min(self.total_items);

        (start, end)
    }

    /// Gets the total scrollable height
    #[must_use]
    pub fn total_height(&self) -> f64 {
        self.total_items as f64 * self.item_height
    }

    /// Gets the offset for a specific item index
    #[must_use]
    pub fn item_offset(&self, index: usize) -> f64 {
        index as f64 * self.item_height
    }

    /// Checks if an item is currently visible
    #[must_use]
    pub fn is_visible(&self, index: usize) -> bool {
        let (start, end) = self.visible_range();
        index >= start && index < end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_virtual_scroller() {
        let mut scroller = VirtualScroller::new(100, 30.0, 300.0);

        // At top, should show items 0-15 (10 visible + 1 extra + 5 overscan below)
        let (start, end) = scroller.visible_range();
        assert_eq!(start, 0);
        assert!(end <= 16);
        assert!(end > 0);

        // Scroll down significantly (past the overscan buffer)
        scroller.set_scroll_offset(300.0); // 10 items down
        let (start, end) = scroller.visible_range();
        // start should be 10 - 5 (overscan) = 5
        assert!(start >= 5);
        assert!(end > start);

        // Check visibility
        assert!(scroller.is_visible(start));
        assert!(scroller.is_visible(end - 1));
        assert!(!scroller.is_visible(end + 10));
    }

    #[test]
    fn test_virtual_scroller_empty() {
        let scroller = VirtualScroller::new(0, 30.0, 300.0);
        let (start, end) = scroller.visible_range();
        assert_eq!(start, 0);
        assert_eq!(end, 0);
    }

    #[test]
    fn test_virtual_scroller_total_height() {
        let scroller = VirtualScroller::new(100, 30.0, 300.0);
        assert!((scroller.total_height() - 3000.0).abs() < f64::EPSILON);
    }
}

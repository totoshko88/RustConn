//! Color pool management for split containers
//!
//! This module provides color allocation for split containers, ensuring each
//! container has a visually distinct color for identification. Colors are
//! allocated from a predefined palette and can be released back to the pool.
//!
//! When all colors are exhausted, the pool wraps around and reuses colors
//! while still tracking which colors are currently allocated.

use std::collections::HashSet;

use super::types::ColorId;

/// Standard color palette for split containers.
///
/// These colors are chosen to be visually distinct and accessible in both
/// light and dark themes. Each color is represented as an RGB tuple.
///
/// The palette includes:
/// - Blue (0x3584e4)
/// - Green (0x2ec27e)
/// - Orange (0xff7800)
/// - Purple (0x9141ac)
/// - Cyan (0x00b4d8)
/// - Red (0xe01b24)
pub const SPLIT_COLORS: &[(u8, u8, u8)] = &[
    (0x35, 0x84, 0xe4), // Blue
    (0x2e, 0xc2, 0x7e), // Green
    (0xff, 0x78, 0x00), // Orange
    (0x91, 0x41, 0xac), // Purple
    (0x00, 0xb4, 0xd8), // Cyan
    (0xe0, 0x1b, 0x24), // Red
];

/// Manages color allocation for split containers.
///
/// The `ColorPool` maintains a set of available colors and tracks which
/// colors are currently allocated. When a color is needed, it allocates
/// the next available color. When all colors are exhausted, it wraps
/// around and reuses colors from the beginning of the palette.
///
/// # Example
///
/// ```
/// use rustconn_core::split::{ColorPool, ColorId};
///
/// let mut pool = ColorPool::new();
///
/// // Allocate a color
/// let color1 = pool.allocate();
/// assert_eq!(color1, ColorId::new(0));
///
/// // Allocate another color
/// let color2 = pool.allocate();
/// assert_eq!(color2, ColorId::new(1));
///
/// // Release the first color
/// pool.release(color1);
///
/// // The released color becomes available again
/// // (though the next allocation will continue from where it left off
/// // until wrap-around occurs)
/// ```
#[derive(Debug)]
pub struct ColorPool {
    /// Currently allocated colors
    allocated: HashSet<ColorId>,
    /// Next color index to try allocating
    next_index: u8,
    /// Total number of colors in the palette
    palette_size: u8,
}

impl ColorPool {
    /// Creates a new color pool with the standard palette.
    ///
    /// The pool starts with all colors available and will allocate
    /// starting from index 0.
    #[must_use]
    pub fn new() -> Self {
        Self {
            allocated: HashSet::new(),
            next_index: 0,
            palette_size: SPLIT_COLORS.len() as u8,
        }
    }

    /// Allocates the next available color.
    ///
    /// This method attempts to find an unallocated color starting from
    /// the current position. If all colors are allocated, it wraps around
    /// and returns the next color in sequence (which will already be
    /// allocated, but this allows continued operation).
    ///
    /// # Returns
    ///
    /// The allocated `ColorId`. The color is marked as allocated in the pool.
    ///
    /// # Example
    ///
    /// ```
    /// use rustconn_core::split::ColorPool;
    ///
    /// let mut pool = ColorPool::new();
    /// let color = pool.allocate();
    /// // color is now allocated and tracked by the pool
    /// ```
    pub fn allocate(&mut self) -> ColorId {
        // Try to find an unallocated color
        let start_index = self.next_index;
        loop {
            let color = ColorId::new(self.next_index);
            self.next_index = (self.next_index + 1) % self.palette_size;

            // Use insert() which returns true if the value was newly inserted
            if self.allocated.insert(color) {
                return color;
            }

            // If we've checked all colors and they're all allocated,
            // wrap around and return the next one anyway
            if self.next_index == start_index {
                // All colors are allocated, just use the current one
                // (it's already in allocated set)
                return color;
            }
        }
    }

    /// Returns a color to the pool.
    ///
    /// After releasing, the color becomes available for future allocations.
    /// If the color was not allocated, this is a no-op.
    ///
    /// # Arguments
    ///
    /// * `color` - The color ID to release back to the pool
    ///
    /// # Example
    ///
    /// ```
    /// use rustconn_core::split::ColorPool;
    ///
    /// let mut pool = ColorPool::new();
    /// let color = pool.allocate();
    /// pool.release(color);
    /// // color is now available for reallocation
    /// ```
    pub fn release(&mut self, color: ColorId) {
        self.allocated.remove(&color);
    }

    /// Returns the number of currently allocated colors.
    ///
    /// # Example
    ///
    /// ```
    /// use rustconn_core::split::ColorPool;
    ///
    /// let mut pool = ColorPool::new();
    /// assert_eq!(pool.allocated_count(), 0);
    ///
    /// let _ = pool.allocate();
    /// assert_eq!(pool.allocated_count(), 1);
    /// ```
    #[must_use]
    pub fn allocated_count(&self) -> usize {
        self.allocated.len()
    }

    /// Returns the total number of colors in the palette.
    ///
    /// # Example
    ///
    /// ```
    /// use rustconn_core::split::ColorPool;
    ///
    /// let pool = ColorPool::new();
    /// assert_eq!(pool.palette_size(), 6);
    /// ```
    #[must_use]
    pub const fn palette_size(&self) -> u8 {
        self.palette_size
    }

    /// Checks if a specific color is currently allocated.
    ///
    /// # Arguments
    ///
    /// * `color` - The color ID to check
    ///
    /// # Returns
    ///
    /// `true` if the color is currently allocated, `false` otherwise.
    ///
    /// # Example
    ///
    /// ```
    /// use rustconn_core::split::{ColorPool, ColorId};
    ///
    /// let mut pool = ColorPool::new();
    /// let color = pool.allocate();
    ///
    /// assert!(pool.is_allocated(color));
    /// pool.release(color);
    /// assert!(!pool.is_allocated(color));
    /// ```
    #[must_use]
    pub fn is_allocated(&self, color: ColorId) -> bool {
        self.allocated.contains(&color)
    }

    /// Returns the RGB values for a given color ID.
    ///
    /// # Arguments
    ///
    /// * `color` - The color ID to look up
    ///
    /// # Returns
    ///
    /// The RGB tuple for the color, or `None` if the color index is out of bounds.
    ///
    /// # Example
    ///
    /// ```
    /// use rustconn_core::split::{ColorPool, ColorId};
    ///
    /// let color = ColorId::new(0);
    /// let rgb = ColorPool::get_rgb(color);
    /// assert_eq!(rgb, Some((0x35, 0x84, 0xe4))); // Blue
    /// ```
    #[must_use]
    pub fn get_rgb(color: ColorId) -> Option<(u8, u8, u8)> {
        SPLIT_COLORS.get(color.index() as usize).copied()
    }
}

impl Default for ColorPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_pool_has_no_allocated_colors() {
        let pool = ColorPool::new();
        assert_eq!(pool.allocated_count(), 0);
    }

    #[test]
    fn palette_size_matches_constant() {
        let pool = ColorPool::new();
        assert_eq!(pool.palette_size() as usize, SPLIT_COLORS.len());
    }

    #[test]
    fn allocate_returns_sequential_colors() {
        let mut pool = ColorPool::new();

        let color0 = pool.allocate();
        let color1 = pool.allocate();
        let color2 = pool.allocate();

        assert_eq!(color0, ColorId::new(0));
        assert_eq!(color1, ColorId::new(1));
        assert_eq!(color2, ColorId::new(2));
    }

    #[test]
    fn allocate_marks_color_as_allocated() {
        let mut pool = ColorPool::new();

        let color = pool.allocate();
        assert!(pool.is_allocated(color));
        assert_eq!(pool.allocated_count(), 1);
    }

    #[test]
    fn release_makes_color_available() {
        let mut pool = ColorPool::new();

        let color = pool.allocate();
        assert!(pool.is_allocated(color));

        pool.release(color);
        assert!(!pool.is_allocated(color));
        assert_eq!(pool.allocated_count(), 0);
    }

    #[test]
    fn release_unallocated_color_is_noop() {
        let mut pool = ColorPool::new();
        let color = ColorId::new(0);

        // Should not panic or cause issues
        pool.release(color);
        assert_eq!(pool.allocated_count(), 0);
    }

    #[test]
    fn allocate_wraps_around_when_exhausted() {
        let mut pool = ColorPool::new();
        let palette_size = pool.palette_size();

        // Allocate all colors
        for i in 0..palette_size {
            let color = pool.allocate();
            assert_eq!(color.index(), i);
        }

        assert_eq!(pool.allocated_count(), palette_size as usize);

        // Next allocation should wrap around
        let wrapped_color = pool.allocate();
        // It should return a color (wrap-around behavior)
        assert!(wrapped_color.index() < palette_size);
    }

    #[test]
    fn allocate_skips_allocated_colors() {
        let mut pool = ColorPool::new();

        // Allocate first two colors
        let color0 = pool.allocate();
        let _color1 = pool.allocate();

        // Release the first color
        pool.release(color0);

        // Allocate two more - should get color2, then wrap to color0
        let color2 = pool.allocate();
        assert_eq!(color2, ColorId::new(2));

        // Continue allocating until we need to reuse
        let color3 = pool.allocate();
        let color4 = pool.allocate();
        let color5 = pool.allocate();

        assert_eq!(color3, ColorId::new(3));
        assert_eq!(color4, ColorId::new(4));
        assert_eq!(color5, ColorId::new(5));

        // Now allocate again - should wrap and find color0 (which was released)
        let reused = pool.allocate();
        assert_eq!(reused, ColorId::new(0));
    }

    #[test]
    fn get_rgb_returns_correct_colors() {
        assert_eq!(
            ColorPool::get_rgb(ColorId::new(0)),
            Some((0x35, 0x84, 0xe4))
        ); // Blue
        assert_eq!(
            ColorPool::get_rgb(ColorId::new(1)),
            Some((0x2e, 0xc2, 0x7e))
        ); // Green
        assert_eq!(
            ColorPool::get_rgb(ColorId::new(2)),
            Some((0xff, 0x78, 0x00))
        ); // Orange
        assert_eq!(
            ColorPool::get_rgb(ColorId::new(3)),
            Some((0x91, 0x41, 0xac))
        ); // Purple
        assert_eq!(
            ColorPool::get_rgb(ColorId::new(4)),
            Some((0x00, 0xb4, 0xd8))
        ); // Cyan
        assert_eq!(
            ColorPool::get_rgb(ColorId::new(5)),
            Some((0xe0, 0x1b, 0x24))
        ); // Red
    }

    #[test]
    fn get_rgb_returns_none_for_invalid_index() {
        assert_eq!(ColorPool::get_rgb(ColorId::new(6)), None);
        assert_eq!(ColorPool::get_rgb(ColorId::new(255)), None);
    }

    #[test]
    fn default_creates_new_pool() {
        let pool = ColorPool::default();
        assert_eq!(pool.allocated_count(), 0);
        assert_eq!(pool.palette_size() as usize, SPLIT_COLORS.len());
    }

    #[test]
    fn multiple_allocate_release_cycles() {
        let mut pool = ColorPool::new();

        // First cycle
        let c1 = pool.allocate();
        let c2 = pool.allocate();
        pool.release(c1);
        pool.release(c2);

        assert_eq!(pool.allocated_count(), 0);

        // Second cycle - should start from where we left off (index 2)
        // but find c1 and c2 available when wrapping
        let c3 = pool.allocate();
        let c4 = pool.allocate();

        // c3 should be index 2 (continuing from where we left off)
        assert_eq!(c3, ColorId::new(2));
        assert_eq!(c4, ColorId::new(3));
    }

    #[test]
    fn split_colors_constant_has_expected_length() {
        assert_eq!(SPLIT_COLORS.len(), 6);
    }
}

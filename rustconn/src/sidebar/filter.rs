//! Filter logic for the sidebar.
//!
//! The former per-protocol pill buttons (and their `create_filter_button` /
//! `connect_filter_button` helpers) were replaced by a single "Filter"
//! `GtkMenuButton` whose popover holds a `GtkCheckButton` per protocol
//! (GNOME HIG §2a). The checkbox handlers reuse
//! [`crate::sidebar::search::update_search_with_filters`] directly, so no
//! button-construction helper remains in this module.

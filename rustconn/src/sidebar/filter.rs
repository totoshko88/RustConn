//! Filter logic for the sidebar
use gtk4::prelude::*;
use gtk4::Button;

/// Creates a protocol filter button with icon only (no text label)
///
/// # Arguments
/// * `icon_name` - GTK icon name for the button
/// * `tooltip` - Tooltip text for the button
/// * `protocol` - Protocol name used for accessible label
///
/// # Accessibility
/// Sets proper accessible label and role for screen readers.
pub fn create_filter_button(protocol: &str, icon_name: &str, tooltip: &str) -> Button {
    let button = Button::new();
    let icon = gtk4::Image::from_icon_name(icon_name);
    icon.set_pixel_size(16);
    button.set_child(Some(&icon));
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("flat");
    button.add_css_class("filter-button");

    // Accessibility: set descriptive label for screen readers
    let accessible_label = format!("Filter by {protocol} protocol");
    button.update_property(&[gtk4::accessible::Property::Label(&accessible_label)]);

    button
}

/// Connects a filter button to the toggle handler
///
/// This helper reduces code duplication when setting up filter button click handlers.
pub fn connect_filter_button<F>(button: &Button, toggle_handler: F)
where
    F: Fn(&Button) + 'static,
{
    button.connect_clicked(move |btn| {
        toggle_handler(btn);
    });
}

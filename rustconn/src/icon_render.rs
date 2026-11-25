//! Rendering rules for user-supplied icon values.
//!
//! Connections, groups and templates store their icon in a single string field
//! that may hold either an emoji glyph or a GTK icon name. Deciding which of
//! the two it is has to happen identically everywhere a row is drawn, otherwise
//! the same value shows up in one view and disappears in another.
//!
//! Use [`rustconn_core::dialog_utils::is_glyph_icon`] to pick the branch, and
//! [`theme_icon_or`] for the icon-name branch so a name the current theme does
//! not carry falls back to a visible default instead of empty space. That case
//! is routine in a Flatpak sandbox: the GNOME runtime ships only the Adwaita
//! theme, so names the user found in a host icon browser cannot be resolved.

/// Returns `true` when the current icon theme can resolve `name`.
///
/// Answers `true` when no display is available (headless unit runs), so a
/// caller never silently swaps a user's icon for the default in that case.
#[must_use]
pub fn theme_has_icon(name: &str) -> bool {
    gtk4::gdk::Display::default().is_none_or(|display| {
        let theme = gtk4::IconTheme::for_display(&display);
        theme.has_icon(name)
    })
}

/// Returns `name` when the icon theme can resolve it, otherwise `fallback`.
///
/// Keeps a row legible when a stored icon name is missing from the active
/// theme, which the icon widget would otherwise render as blank space.
#[must_use]
pub fn theme_icon_or<'a>(name: &'a str, fallback: &'a str) -> &'a str {
    if theme_has_icon(name) {
        name
    } else {
        tracing::debug!(icon = %name, %fallback, "icon name not in theme, using fallback");
        fallback
    }
}

/// Returns the tab-bar icon for a connection's custom icon value.
///
/// Renders the one string `Connection::icon` stores — an emoji/Unicode glyph or
/// a GTK icon name — the same way as the sidebar row, so a connection shows the
/// same mark on its tab as in the saved list. An empty value and a name the
/// active theme lacks fall back to the protocol's icon; a glyph is drawn as
/// text, which a `ThemedIcon` cannot carry.
#[must_use]
pub fn tab_icon(custom_icon: &str, protocol_icon: &str) -> gtk4::gio::Icon {
    use gtk4::glib::object::Cast;

    let custom = custom_icon.trim();
    if custom.is_empty() {
        return gtk4::gio::ThemedIcon::new(protocol_icon).upcast();
    }
    if rustconn_core::dialog_utils::is_glyph_icon(custom) {
        return glyph_paintable(custom)
            .unwrap_or_else(|| gtk4::gio::ThemedIcon::new(protocol_icon).upcast());
    }
    gtk4::gio::ThemedIcon::new(theme_icon_or(custom, protocol_icon)).upcast()
}

/// Draws a single glyph as a 32×32 paintable for the tab bar.
///
/// Cairo's toy text API, the same route the status overlays use, so the glyph
/// picks up the system font the sidebar label would. `None` when a pixbuf or
/// cairo context cannot be created (the caller then falls back to a themed
/// icon rather than an empty tab mark).
fn glyph_paintable(glyph: &str) -> Option<gtk4::gio::Icon> {
    use gtk4::glib::object::Cast;

    const SIZE: i32 = 32;
    let surface =
        gtk4::cairo::ImageSurface::create(gtk4::cairo::Format::ARgb32, SIZE, SIZE).ok()?;
    let cr = gtk4::cairo::Context::new(&surface).ok()?;
    // Theme foreground, so a monochrome glyph is legible in light and dark
    // alike (colour emoji carry their own ink); the overlay surfaces use the
    // same pair.
    let (_, fg) = crate::utils::placeholder_surface_colors();
    cr.set_source_rgb(fg.0, fg.1, fg.2);
    cr.select_font_face(
        "Sans",
        gtk4::cairo::FontSlant::Normal,
        gtk4::cairo::FontWeight::Normal,
    );
    cr.set_font_size(26.0);
    if let Ok(extents) = cr.text_extents(glyph) {
        cr.move_to(
            (f64::from(SIZE) - extents.width()) / 2.0,
            (f64::from(SIZE) - extents.height()) / 2.0,
        );
        let _ = cr.show_text(glyph);
    }
    let _ = cr.status();
    let (width, height, stride) = (surface.width(), surface.height(), surface.stride());
    let data = surface.take_data().ok()?;
    let bytes = gtk4::glib::Bytes::from(data.as_ref());
    drop(data);
    let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_bytes(
        &bytes,
        gtk4::gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        width,
        height,
        stride,
    );
    Some(pixbuf.upcast())
}

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
    // A GTK icon name: load it as a themed *paintable* so the tab bar keeps the
    // custom icon's own colours — a bare `ThemedIcon` can be painted symbolic
    // by the tab widget and read as the default icon. When the active theme
    // cannot resolve the name, fall back to the protocol's symbolic mark.
    if let Some(paintable) = themed_paintable(custom) {
        return paintable;
    }
    gtk4::gio::ThemedIcon::new(theme_icon_or(custom, protocol_icon)).upcast()
}

/// Resolves `name` in the active icon theme to a full-colour icon.
///
/// The tab bar paints `ThemedIcon`s as monochrome symbolic marks, which would
/// flatten a custom icon's own colours to the default grey; the resolved file
/// is loaded at the tab bar's size instead, so the custom icon keeps them.
/// `None` when no display or theme carries the name — the caller then offers
/// its fallback.
fn themed_paintable(name: &str) -> Option<gtk4::gio::Icon> {
    use gtk4::gio::prelude::FileExt;
    use gtk4::glib::object::Cast;

    let display = gtk4::gdk::Display::default()?;
    let theme = gtk4::IconTheme::for_display(&display);
    if !theme.has_icon(name) {
        return None;
    }
    // A custom connection icon may live in any context (and the nested
    // `scalable/` variants), so offer the usual set as lookup fallbacks and let
    // GTK pick the best match, its `NNxNN/` sizes and theme inheritance — the
    // resolution the tab bar could not rely on a bare `ThemedIcon` to do.
    let fallbacks: &[&str] = &[
        "apps",
        "status",
        "devices",
        "actions",
        "category",
        "places",
        "mimetypes",
        "any",
    ];
    let icon = theme.lookup_icon(
        name,
        fallbacks,
        24,
        1,
        gtk4::TextDirection::Ltr,
        gtk4::IconLookupFlags::empty(),
    );
    // `IconPaintable` is a `GdkPaintable`, not a `GIcon` — and the tab page
    // takes a `GIcon`. A full-colour icon's file loads straight into pixels;
    // a symbolic one hands back a `ThemedIcon` the tab bar can recolour.
    if let Some(file) = icon.file()
        && !icon.is_symbolic()
        && let Some(path) = file.path()
        && let Ok(pixbuf) = gtk4::gdk_pixbuf::Pixbuf::from_file_at_scale(&path, 24, 24, true)
    {
        return Some(gtk4::gdk::Texture::for_pixbuf(&pixbuf).upcast());
    }
    Some(gtk4::gio::ThemedIcon::new(name).upcast())
}

/// Draws a single glyph on a 32×32 ARgb32 surface for the tab bar.
///
/// Cairo's toy text API, the same route the status overlays use, so the glyph
/// picks up the system font the sidebar label would. `None` when a surface or
/// cairo context cannot be created (the caller then falls back to a themed
/// icon rather than an empty tab mark).
fn glyph_surface(glyph: &str) -> Option<gtk4::cairo::ImageSurface> {
    const SIZE: i32 = 32;
    let surface =
        gtk4::cairo::ImageSurface::create(gtk4::cairo::Format::ARgb32, SIZE, SIZE).ok()?;
    {
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
            // Centre on the ink, not the pen: account for the bearings so the
            // glyph does not sit too low (cairo's toy text API, the same route
            // the status overlays use, so the glyph picks up the system font the
            // sidebar label would).
            cr.move_to(
                (f64::from(SIZE) - extents.width()) / 2.0 - extents.x_bearing(),
                (f64::from(SIZE) - extents.height()) / 2.0 - extents.y_bearing(),
            );
            let _ = cr.show_text(glyph);
        }
        let _ = cr.status();
    }
    Some(surface)
}

/// Builds the tab icon's `GIcon` from a drawn surface.
///
/// `take_data` wants an exclusive handle, and the context above has dropped —
/// cairo returns `NonExclusive` (and a live `.ok()?` would fall through to the
/// protocol icon) while a context still references the surface. `take_data`
/// consumes the surface, so read its geometry first.
///
/// cairo ARGB32 on little-endian is premultiplied BGRA in memory, so a colour
/// glyph's red and blue must not be swapped into a plain RGBA texture — build
/// it with the matching memory format (the embedded sessions use the same one
/// for their frames).
fn surface_to_texture(surface: gtk4::cairo::ImageSurface) -> Option<gtk4::gio::Icon> {
    use gtk4::glib::object::Cast;

    let width = surface.width();
    let height = surface.height();
    let stride = surface.stride();
    let data = surface.take_data().ok()?;
    let bytes = gtk4::glib::Bytes::from(data.as_ref());
    let texture = gtk4::gdk::MemoryTexture::new(
        width,
        height,
        gtk4::gdk::MemoryFormat::B8g8r8a8Premultiplied,
        &bytes,
        stride as usize,
    );
    Some(texture.upcast())
}

/// Draws a single glyph as a 32×32 paintable for the tab bar.
fn glyph_paintable(glyph: &str) -> Option<gtk4::gio::Icon> {
    let surface = glyph_surface(glyph)?;
    surface_to_texture(surface)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk4::gdk::prelude::TextureExt;
    use gtk4::glib::object::Cast;

    // The glyph and themed-name paths draw through GTK and libadwaita (whose
    // `StyleManager` asserts a main thread), so those tests initialise GTK the
    // way the rest of the toolkit's do — a plain `#[test]` that skips when there
    // is no display. `#[gtk4::test]` would work too, but its process-wide init
    // holds the default main context and aborts the external-session timers that
    // run in the same suite, so these join the `#[ignore]`d on-device set. The
    // pure `ThemedIcon` branch and the cairo pixel checks need no display at all.

    #[test]
    fn tab_icon_empty_value_uses_the_protocol_icon() {
        let icon = tab_icon("", "network-workgroup");
        let themed = icon
            .downcast_ref::<gtk4::gio::ThemedIcon>()
            .expect("an empty custom icon keeps the protocol mark");
        assert!(
            themed
                .names()
                .iter()
                .any(|name| name == "network-workgroup")
        );
    }

    #[test]
    #[ignore = "initialises GTK: needs a display and its own process; run alone with `cargo test -p rustconn --bin rustconn -- --ignored --exact icon_render::tests::tab_icon_glyph_is_drawn_rather_than_themed`"]
    fn tab_icon_glyph_is_drawn_rather_than_themed() {
        if gtk4::init().is_err() {
            return;
        }
        // A glyph is painted as text, which a `ThemedIcon` cannot carry — the
        // tab mark is a texture, not the theme's lookup of the emoji.
        let icon = tab_icon("🌐", "network-workgroup");
        assert!(
            icon.downcast_ref::<gtk4::gio::ThemedIcon>().is_none(),
            "a glyph draws as text, not a themed lookup"
        );
    }

    #[test]
    #[ignore = "initialises GTK: needs a display and its own process; run alone with `cargo test -p rustconn --bin rustconn -- --ignored --exact icon_render::tests::tab_icon_missing_name_falls_back_to_themed`"]
    fn tab_icon_missing_name_falls_back_to_themed() {
        if gtk4::init().is_err() {
            return;
        }
        // No theme on this machine carries the name, so the tab bar keeps a
        // visible mark: the themed fallback, the custom name, or the protocol
        // icon — never an empty icon.
        let icon = tab_icon("rustconn-missing-icon-0123456789", "network-workgroup");
        let themed = icon
            .downcast_ref::<gtk4::gio::ThemedIcon>()
            .expect("an unresolvable icon name keeps a themed mark");
        assert!(
            themed
                .names()
                .iter()
                .any(|name| name == "rustconn-missing-icon-0123456789"
                    || name == "network-workgroup")
        );
    }

    #[test]
    #[ignore = "initialises GTK: needs a display and its own process; run alone with `cargo test -p rustconn --bin rustconn -- --ignored --exact icon_render::tests::glyph_surface_leaves_opaque_ink`"]
    fn glyph_surface_leaves_opaque_ink() {
        if gtk4::init().is_err() {
            return;
        }
        // A monochrome glyph the toy text API draws solidly (a colour emoji
        // may not), so the check is on the ink, not the glyph's own colours.
        let surface = glyph_surface("Ω").expect("the glyph surface is drawn");
        let data = match surface.take_data() {
            Ok(data) => data,
            Err(err) => panic!("an exclusive data handle: {err:?}"),
        };
        let bytes = data.as_ref();
        let ink = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] == 255 && (p[0] | p[1] | p[2]) != 0)
            .count();
        let total = bytes.len() / 4;
        assert!(
            ink > 0,
            "the glyph leaves fully-opaque, non-empty ink ({ink}/{total} pixels)"
        );
    }

    /// Draws a solid red 8×8 ARgb32 surface and returns it (context dropped).
    fn solid_red_surface() -> gtk4::cairo::ImageSurface {
        let surface = gtk4::cairo::ImageSurface::create(gtk4::cairo::Format::ARgb32, 8, 8).unwrap();
        let cr = gtk4::cairo::Context::new(&surface).unwrap();
        cr.set_source_rgb(1.0, 0.0, 0.0);
        let _ = cr.paint();
        drop(cr);
        surface
    }

    #[test]
    fn surface_to_texture_reads_cairo_argb32_as_bgra() {
        // Pin the memory order the `B8g8r8a8Premultiplied` format relies on:
        // cairo ARGB32 on little-endian is premultiplied BGRA, so a solid red
        // row reads B, G, R, A = 0, 0, 255, 255.
        let icon = surface_to_texture(solid_red_surface()).expect("a texture for the surface");
        let texture = icon
            .downcast::<gtk4::gdk::Texture>()
            .expect("the tab icon is a texture");
        assert_eq!(texture.width(), 8);
        assert_eq!(texture.height(), 8);
        assert_eq!(
            texture.format(),
            gtk4::gdk::MemoryFormat::B8g8r8a8Premultiplied
        );

        let data = solid_red_surface()
            .take_data()
            .expect("an exclusive data handle");
        let bytes = data.as_ref();
        let stride = 8 * 4;
        assert_eq!(bytes.len(), 8 * stride, "ARgb32 rows are 4 bytes per pixel");
        let offset = 4 * stride + 16;
        assert_eq!(&bytes[offset..offset + 4], &[0, 0, 255, 255]);
    }
}

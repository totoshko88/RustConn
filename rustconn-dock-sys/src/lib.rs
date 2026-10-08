//! Isolated FFI helper for the macOS Dock tile image.
//!
//! On macOS the Dock icon is not a property of the running program. It is read
//! by LaunchServices from the launched bundle's `Info.plist` (`CFBundleIconFile`
//! → `Contents/Resources/<name>.icns`). A process with no bundle behind it —
//! `rustconn` started from a shell, or a bundle whose `CFBundleExecutable` is a
//! wrapper that `exec`s a binary living outside the `.app` — has no icon to be
//! read, and the Dock falls back to the generic Unix-executable tile, which is
//! commonly mistaken for a terminal icon because that is roughly what it depicts.
//!
//! Nothing in GTK can change that. `gtk_window_set_icon_name` is an X11/Wayland
//! concept and is a no-op on the GDK macOS backend, and GDK exposes no Dock API
//! of its own. The one interface that works for a bundle-less process is
//! `-[NSApplication setApplicationIconImage:]`, which replaces the tile image of
//! the *running* application. That single call is what this crate wraps, so the
//! `unsafe` it requires does not have to be re-opened in `rustconn`, which keeps
//! a crate-local `forbid(unsafe_code)` over the workspace's `deny`.
//!
//! It exposes one operation, [`set_dock_icon_png`], and reports what happened as
//! a [`DockIconOutcome`] instead of an error type: replacing the Dock tile is
//! cosmetic and best-effort, there is no recovery for a caller to attempt, and a
//! refusal is something to log, not to propagate.
//!
//! # Scope, and what this deliberately does not do
//!
//! Only the Dock tile changes. The name under the tile, the Cmd-Tab entry and
//! the application menu still come from the bundle (or from the executable's
//! name when there is none) and are not reachable this way. A correctly built
//! `.app` remains the only way to get all of them, which is why the caller
//! [skips this crate entirely when it is already running inside one][caller] —
//! the bundle's `.icns` carries every representation from 16px to 1024px, and
//! also whatever custom icon a user may have pasted onto the app in Finder.
//! Overwriting that with a single embedded bitmap would be a regression, not a
//! fix.
//!
//! [caller]: https://github.com/totoshko88/RustConn/blob/main/rustconn/src/main.rs
//!
//! # The contract
//!
//! [`set_dock_icon_png`] may be called only:
//!
//! * from the main thread — AppKit's requirement for `NSApplication`, and the
//!   clause the `unsafe` block below actually rests on;
//! * after GTK has been initialised, so that the `NSApplication` singleton GDK
//!   sets up already exists. Calling earlier would create it here instead, which
//!   is not unsound but leaves GDK adopting an instance it did not configure.
//!
//! Both are checked or arranged rather than assumed: the main-thread clause is
//! enforced by `objc2::MainThreadMarker`, and a violation is reported as
//! [`DockIconOutcome::OffMainThread`] rather than panicking. A wrong Dock icon
//! does not justify taking the process down (M-PANIC-IS-STOP), which is the one
//! place this crate's contract differs in kind from [`rustconn-locale-sys`] and
//! [`rustconn-env-sys`], where a violation means memory unsoundness and does
//! panic.
//!
//! # What CI actually checks
//!
//! No CI job builds macOS, so the `unsafe` block below is compiled only on a
//! developer's Mac. Stating that plainly is better than implying otherwise: it
//! is why the crate is split into a platform-independent precondition layer
//! ([`png_signature_check`], exercised by the tests below on every platform) and
//! the few lines that talk to AppKit. The crate is nonetheless an unconditional
//! workspace member and an unconditional dependency of `rustconn` — the same
//! rule the other `-sys` crates follow — so its API surface, its outcome
//! reporting and its guard cannot rot unnoticed between two macOS builds.
//! `rustconn-pty-sys` has the identical property for its macOS
//! controlling-terminal path.
//!
//! [`rustconn-locale-sys`]: https://github.com/totoshko88/RustConn/tree/main/rustconn-locale-sys
//! [`rustconn-env-sys`]: https://github.com/totoshko88/RustConn/tree/main/rustconn-env-sys

// The one lint this crate re-opens out of the inherited workspace set, which is
// `deny` rather than `forbid` precisely so that this line is possible. `expect`
// rather than `allow`: if the `unsafe` call below ever goes away, the compiler
// says so instead of leaving a stale exemption behind. It is wrapped in
// `cfg_attr` because off macOS there is no `unsafe` in this crate at all, and a
// bare `expect` would then fire `unfulfilled_lint_expectations` on the Linux
// build — a warning, and the workspace gate is zero warnings.
#![cfg_attr(
    target_os = "macos",
    expect(
        unsafe_code,
        reason = "sanctioned FFI crate (M-UNSAFE); the guarded setApplicationIconImage call is its entire purpose"
    )
)]

// ============================================================================
// Outcome reporting
// ============================================================================

/// What became of a request to replace the Dock tile image.
///
/// Every variant other than [`Applied`](Self::Applied) means the Dock still
/// shows whatever it showed before. None of them is an error in the sense of
/// something a caller can retry or repair; they exist so a log line can say
/// which of them happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockIconOutcome {
    /// The Dock tile now shows the supplied image.
    Applied,
    /// The bytes are not a PNG, so `NSImage` was never asked to decode them.
    NotPng,
    /// AppKit decoded nothing usable out of a well-formed PNG signature.
    ImageRejected,
    /// The call came from a thread other than the main one, where AppKit forbids
    /// touching `NSApplication`.
    OffMainThread,
    /// The platform has no Dock. Every non-Apple target answers this.
    NoDock,
}

impl DockIconOutcome {
    /// Renders the outcome as a log-ready phrase naming the cause.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Applied => "the Dock tile now shows the supplied icon",
            Self::NotPng => {
                "the supplied bytes do not start with a PNG signature, so they were not decoded"
            }
            Self::ImageRejected => {
                "AppKit could not build an NSImage from the supplied PNG; the file is likely truncated"
            }
            Self::OffMainThread => {
                "called off the main thread, where AppKit forbids touching NSApplication; \
                 call this from the thread that ran main()"
            }
            Self::NoDock => "this platform has no Dock, so there is nothing to set",
        }
    }

    /// Whether the Dock tile actually changed.
    #[must_use]
    pub const fn is_applied(self) -> bool {
        matches!(self, Self::Applied)
    }
}

// ============================================================================
// Precondition guard
// ============================================================================

/// The eight bytes every PNG file starts with (PNG 1.2, §3.1).
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Rejects anything that is not a PNG before AppKit is involved.
///
/// `-[NSImage initWithData:]` accepts several formats and returns `nil` for the
/// rest, so this check is not what makes the call sound — it makes a mistake
/// legible. A caller that passes the wrong `include_bytes!` path gets
/// [`DockIconOutcome::NotPng`] naming the reason, instead of an
/// [`ImageRejected`](DockIconOutcome::ImageRejected) that could equally mean a
/// corrupt asset. Being platform-independent, it is also the part of this crate
/// that a Linux CI run can actually exercise.
fn png_signature_check(bytes: &[u8]) -> Result<(), DockIconOutcome> {
    if bytes.starts_with(&PNG_SIGNATURE) {
        Ok(())
    } else {
        Err(DockIconOutcome::NotPng)
    }
}

// ============================================================================
// Dock tile
// ============================================================================

/// Replaces the Dock tile of the running application with `png`.
///
/// Intended for launches that LaunchServices cannot find an icon for: a bare
/// binary started from a shell, or an `.app` whose `CFBundleExecutable` is a
/// wrapper that `exec`s a binary outside the bundle. Inside a correctly built
/// bundle this should not be called at all — see the crate-level "Scope".
///
/// `png` must be PNG-encoded; supply the largest representation available, since
/// the Dock draws tiles at up to 128pt, i.e. 256px on a Retina display. The
/// image is copied into AppKit's ownership, so nothing needs to be kept alive
/// afterwards.
///
/// Returns what happened; see [`DockIconOutcome`]. Never panics, and is a no-op
/// answering [`DockIconOutcome::NoDock`] off Apple platforms, so the caller does
/// not need a `cfg` of its own.
///
/// Must be called from the main thread, after GTK initialisation — the two
/// clauses of the contract described at the [crate] level.
pub fn set_dock_icon_png(png: &[u8]) -> DockIconOutcome {
    match png_signature_check(png) {
        Ok(()) => apply_dock_icon(png),
        Err(refusal) => refusal,
    }
}

#[cfg(target_os = "macos")]
fn apply_dock_icon(png: &[u8]) -> DockIconOutcome {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    // AppKit's own precondition, as a type: `NSApplication` may only be touched
    // from the main thread. `MainThreadMarker::new` asks the runtime rather than
    // trusting the caller, and there is no way to reach the call below without
    // one.
    let Some(main_thread) = MainThreadMarker::new() else {
        return DockIconOutcome::OffMainThread;
    };

    // Both of these are safe in objc2: `NSData::with_bytes` copies the slice,
    // and `initWithData:` is a plain initialiser returning `nil` on a format it
    // cannot decode — which is the `None` handled here.
    let data = NSData::with_bytes(png);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return DockIconOutcome::ImageRejected;
    };

    // Returns the singleton GDK already created during `gtk_init`; creates one
    // only if called earlier than the contract allows.
    let app = NSApplication::sharedApplication(main_thread);

    // SAFETY: objc2 marks this setter `unsafe` for exactly one reason, stated in
    // its own generated documentation: it cannot prove the argument accepts
    // `nil`. `Some(&image)` is passed unconditionally, so that question does not
    // arise — and `image` is a live, non-nil `NSImage` because `initWithData:`
    // returning `Some` is what got us here.
    //
    // The remaining requirement is AppKit's blanket main-thread rule for
    // `NSApplication`, which `main_thread` above establishes by asking the
    // runtime; `sharedApplication` cannot even be named without that proof.
    //
    // Ownership needs nothing further: `setApplicationIconImage:` retains the
    // image for as long as the Dock needs it, so dropping the local
    // `Retained<NSImage>` at the end of this function is correct.
    unsafe { app.setApplicationIconImage(Some(&image)) };

    DockIconOutcome::Applied
}

#[cfg(not(target_os = "macos"))]
fn apply_dock_icon(_png: &[u8]) -> DockIconOutcome {
    DockIconOutcome::NoDock
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal but real PNG: signature, IHDR for a 1×1 greyscale image, one
    /// IDAT and IEND. Written out rather than embedded from the asset tree so
    /// the guard's tests do not depend on a file the GUI crate owns.
    const ONE_PIXEL_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // signature
        0x00, 0x00, 0x00, 0x0D, b'I', b'H', b'D', b'R', // IHDR length + type
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1×1
        0x08, 0x00, 0x00, 0x00, 0x00, 0x3A, 0x7E, 0x9B, 0x55, // depth/colour + CRC
        0x00, 0x00, 0x00, 0x0A, b'I', b'D', b'A', b'T', // IDAT length + type
        0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, // zlib stream
        0x0D, 0x0A, 0x2D, 0xB4, // IDAT CRC
        0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', // IEND
        0xAE, 0x42, 0x60, 0x82, // IEND CRC
    ];

    /// The signature the guard looks for is the one from the PNG specification,
    /// not an approximation of it. Pinned byte by byte because a typo here would
    /// reject every valid icon while still passing a round-trip test written
    /// against the same constant.
    #[test]
    fn png_signature_is_the_specified_one() {
        assert_eq!(
            PNG_SIGNATURE,
            [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        );
    }

    /// A real PNG passes the guard.
    #[test]
    fn accepts_a_png() {
        assert_eq!(png_signature_check(ONE_PIXEL_PNG), Ok(()));
    }

    /// Trailing bytes are AppKit's problem, not the guard's: the guard only
    /// claims the file starts like a PNG, so a truncated one must still get
    /// through to be rejected as [`DockIconOutcome::ImageRejected`] with a
    /// message that says so.
    #[test]
    fn accepts_a_truncated_png_and_leaves_the_verdict_to_appkit() {
        assert_eq!(png_signature_check(&ONE_PIXEL_PNG[..12]), Ok(()));
    }

    /// The cases a wrong `include_bytes!` path actually produces: an SVG (the
    /// sibling asset in the same icon tree), a JPEG, and nothing at all.
    #[test]
    fn refuses_what_is_not_a_png() {
        for bytes in [
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".as_slice(),
            &[0xFF, 0xD8, 0xFF, 0xE0], // JPEG SOI + APP0
            b"".as_slice(),
        ] {
            assert_eq!(
                png_signature_check(bytes),
                Err(DockIconOutcome::NotPng),
                "{bytes:?} must not reach AppKit"
            );
        }
    }

    /// A signature-only prefix that is one byte short must not slip through.
    #[test]
    fn refuses_a_partial_signature() {
        assert_eq!(
            png_signature_check(&PNG_SIGNATURE[..7]),
            Err(DockIconOutcome::NotPng)
        );
    }

    /// The public entry point reports the guard's refusal rather than swallowing
    /// it, on every platform — this is the one behaviour of `set_dock_icon_png`
    /// that is observable without a Mac.
    #[test]
    fn the_entry_point_reports_a_refused_format() {
        assert_eq!(set_dock_icon_png(b"not an icon"), DockIconOutcome::NotPng);
    }

    /// Off Apple platforms a well-formed PNG is accepted by the guard and then
    /// answered by the no-op. Pinning it keeps the "no `cfg` needed at the call
    /// site" promise in the function's documentation honest.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_valid_png_is_a_no_op_where_there_is_no_dock() {
        let outcome = set_dock_icon_png(ONE_PIXEL_PNG);
        assert_eq!(outcome, DockIconOutcome::NoDock);
        assert!(!outcome.is_applied());
    }

    /// The main-thread guard refuses, rather than calling AppKit off the main
    /// thread or panicking.
    ///
    /// This is the one precondition that makes the `unsafe` call sound, and it had
    /// no test: the suite covered the PNG signature and the shape of every outcome,
    /// but never the guard itself. That is the same gap that let a
    /// `rustconn-pty-sys` contract test sit failing on macOS unnoticed until
    /// 0.20.11 — a guard nothing exercises is a guard nobody knows still works.
    /// The `macos-sys` CI job added in the same release is what now runs this on a
    /// machine other than the maintainer's.
    ///
    /// Asked from a spawned thread on purpose. A plain `#[test]` body is not a
    /// reliable negative here: libtest under `--test-threads=1`, and nextest with
    /// its process per test, both run the body *on* the main thread, where the
    /// guard would pass and the call would replace the developer's real Dock tile
    /// as a side effect of running the suite.
    #[cfg(target_os = "macos")]
    #[test]
    fn refuses_a_call_from_another_thread() {
        let outcome = std::thread::spawn(|| set_dock_icon_png(ONE_PIXEL_PNG))
            .join()
            .expect("the probe thread must not panic: the guard returns an outcome");
        assert_eq!(
            outcome,
            DockIconOutcome::OffMainThread,
            "a non-main thread must be refused before AppKit is reached"
        );
        assert!(!outcome.is_applied());
    }

    /// Only `Applied` counts as applied — a caller branching on `is_applied` must
    /// not treat a refusal as success.
    #[test]
    fn only_applied_is_applied() {
        assert!(DockIconOutcome::Applied.is_applied());
        for outcome in [
            DockIconOutcome::NotPng,
            DockIconOutcome::ImageRejected,
            DockIconOutcome::OffMainThread,
            DockIconOutcome::NoDock,
        ] {
            assert!(!outcome.is_applied(), "{outcome:?}");
        }
    }

    /// Every outcome explains itself, so a developer reading a log line does not
    /// have to read this crate to find out what happened.
    #[test]
    fn outcomes_explain_themselves() {
        for outcome in [
            DockIconOutcome::Applied,
            DockIconOutcome::NotPng,
            DockIconOutcome::ImageRejected,
            DockIconOutcome::OffMainThread,
            DockIconOutcome::NoDock,
        ] {
            let text = outcome.describe();
            assert!(text.len() > 20, "{outcome:?} says only {text:?}");
        }
    }
}

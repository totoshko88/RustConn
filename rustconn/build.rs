//! Build script for RustConn GUI crate.
//!
//! Compiles `.po` translation files into `.mo` files so that
//! `cargo run` picks up translations without a manual install step.
//! The compiled locale tree is placed under `OUT_DIR/locale/`.

use std::path::Path;
use std::process::Command;

fn main() {
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR must be set by Cargo");
    let locale_out = Path::new(&out_dir).join("locale");
    let po_dir = Path::new("../po");

    // Re-run if any .po file changes
    println!("cargo:rerun-if-changed=../po");

    // Check if msgfmt is available
    let has_msgfmt = Command::new("msgfmt").arg("--version").output().is_ok();
    if !has_msgfmt {
        println!(
            "cargo:warning=msgfmt not found — translations will not be compiled. \
             Install gettext: sudo apt install gettext"
        );
        // Still export an empty locale dir so env!() doesn't fail
        println!("cargo:rustc-env=RUSTCONN_LOCALE_DIR=");
        return;
    }

    let mut count = 0u32;
    if let Ok(entries) = std::fs::read_dir(po_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("po") {
                continue;
            }
            let lang = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if lang.is_empty() {
                continue;
            }

            let dest = locale_out.join(lang).join("LC_MESSAGES");
            std::fs::create_dir_all(&dest).ok();

            let mo_path = dest.join("rustconn.mo");
            let status = Command::new("msgfmt")
                .arg("-o")
                .arg(&mo_path)
                .arg(&path)
                .status();

            match status {
                Ok(s) if s.success() => count += 1,
                Ok(s) => {
                    println!(
                        "cargo:warning=msgfmt failed for {lang}.po (exit {})",
                        s.code().unwrap_or(-1)
                    );
                }
                Err(e) => {
                    println!("cargo:warning=msgfmt error for {lang}.po: {e}");
                }
            }
        }
    }

    if count > 0 {
        println!(
            "cargo:warning=Compiled {count} locale(s) into {}",
            locale_out.display()
        );
    }

    // Export the locale path so i18n.rs can find it at runtime
    println!(
        "cargo:rustc-env=RUSTCONN_LOCALE_DIR={}",
        locale_out.display()
    );
}

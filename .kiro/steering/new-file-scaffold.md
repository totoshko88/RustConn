---
inclusion: manual
description: "Boilerplate templates for creating new dialogs, protocols, and secret backends from scratch."
---

# New File Scaffolds

Use these minimal templates when creating a new component from scratch. They include
the required imports, trait impls, and registration steps. Adapt to your needs.

---

## New Dialog (rustconn/src/dialogs/)

```rust
use adw::prelude::*;
use gtk4::prelude::*;
use gettextrs::gettext as i18n;

use crate::i18n_f;

/// Short description of what this dialog does.
pub struct MyDialog {
    dialog: adw::Dialog,
}

impl MyDialog {
    pub fn new() -> Self {
        let dialog = adw::Dialog::builder()
            .title(&i18n("Dialog Title"))
            .content_width(480)
            .content_height(360)
            .build();

        // Build content here using adw:: widgets
        let content = adw::PreferencesPage::new();
        let group = adw::PreferencesGroup::builder()
            .title(&i18n("Section"))
            .build();
        content.add(&group);

        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .child(&content)
            .build();
        dialog.set_child(Some(&clamp));

        Self { dialog }
    }

    pub fn present(&self, parent: &impl IsA<gtk4::Widget>) {
        self.dialog.present(Some(parent));
    }
}
```

Registration: add `pub mod my_dialog;` in `rustconn/src/dialogs/mod.rs`.

---

## New Protocol (rustconn-core/src/protocol/)

```rust
use crate::protocol::{Protocol, ProtocolCapabilities, ProtocolType};

/// Short description of the protocol.
pub struct MyProtocol;

impl Protocol for MyProtocol {
    fn protocol_type(&self) -> ProtocolType {
        ProtocolType::MyProtocol
    }

    fn default_port(&self) -> u16 {
        22 // replace with actual default
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities {
            has_terminal: false,
            has_password: true,
            has_port: true,
            has_domain: false,
            ..Default::default()
        }
    }

    fn display_name(&self) -> &'static str {
        "My Protocol" // wrap in i18n() at call site in GUI
    }
}
```

Checklist:
1. Add variant to `ProtocolType` enum in `protocol/mod.rs`
2. Register in protocol factory / match arms
3. Add CLI flags in `rustconn-cli` (CRUD only; gate launch behind feature)
4. Add dialog tab in `rustconn/src/dialogs/connection/`

---

## New Secret Backend (rustconn-core/src/secret/)

```rust
use async_trait::async_trait;
use secrecy::SecretString;

use crate::secret::{SecretBackend, SecretError};

/// Short description of the backend.
pub struct MyBackend {
    // config fields
}

#[async_trait]
impl SecretBackend for MyBackend {
    fn name(&self) -> &str {
        "my-backend"
    }

    fn display_name(&self) -> &str {
        "My Backend" // wrap in i18n() at call site in GUI
    }

    async fn store(
        &self,
        key: &str,
        secret: &SecretString,
    ) -> Result<(), SecretError> {
        // Implementation — pass secret via stdin, never as arg
        todo!()
    }

    async fn retrieve(&self, key: &str) -> Result<SecretString, SecretError> {
        // Implementation — 10s timeout for vault ops
        todo!()
    }

    async fn delete(&self, key: &str) -> Result<(), SecretError> {
        todo!()
    }

    async fn has_secret(&self, key: &str) -> Result<bool, SecretError> {
        todo!()
    }
}
```

Registration: add to backend list in `secret/mod.rs`.

Security reminders (enforced by `security-review` hook):
- Never store/log SecretString values
- Intermediate strings → `Zeroizing::new()`
- External CLIs → stdin pipe, never `.arg(password)`

---

## New CLI Command (rustconn-cli/src/commands/)

```rust
use clap::Args;

use rustconn_core::error::CoreError;

/// Short description.
#[derive(Args, Debug)]
pub struct MyCommand {
    /// Argument description
    #[arg(long)]
    name: String,
}

impl MyCommand {
    pub fn run(&self) -> Result<(), CoreError> {
        // Implementation
        Ok(())
    }
}
```

Registration: add variant to `Commands` enum in `cli.rs`, wire `run()` in match.

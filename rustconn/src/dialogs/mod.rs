//! Dialog windows for `RustConn`

mod adw_dialogs;
mod backend_missing;
mod cluster;
mod command_palette;
mod connection;
pub mod connection_wizard;
mod document;
mod export;
mod flatpak_components;
mod history;
mod import;
pub mod keyboard;
mod log_viewer;
mod password;
mod password_generator;
mod progress;
pub mod recording;
pub(crate) mod settings;
mod shortcuts;
mod smart_folder;
mod snippet;
mod statistics;
mod template;
mod template_manager;
mod terminal_search;
pub mod tunnel;
pub mod tunnel_builder;
mod variable_setup;
mod variables;
pub mod widgets;
mod wol;
mod workspace;

use std::cell::RefCell;
use std::rc::Rc;

pub use adw_dialogs::*;
pub use backend_missing::{BackendMissingResponse, show_backend_missing_dialog};
pub use cluster::{ClusterCallback, ClusterDialog, ClusterListDialog};
pub use command_palette::{CommandPaletteDialog, OpenTabInfo};
pub use connection::ConnectionDialog;
pub use connection_wizard::{ConnectionWizard, PartialConnection, WizardResult};
pub use document::{
    CloseDocumentDialog, DocumentCallback, DocumentDialogResult, DocumentProtectionDialog,
    NewDocumentDialog, OpenDocumentDialog, SaveDocumentDialog,
};
pub use export::{ExportCallback, ExportDialog};
pub use flatpak_components::{FlatpakComponentsDialog, should_show_flatpak_components_menu};
pub use history::HistoryDialog;
pub use import::ImportDialog;
pub use log_viewer::LogViewerDialog;
pub use password::{PasswordDialog, PasswordDialogResult};
pub use password_generator::show_password_generator_dialog;
pub use progress::ProgressDialog;
pub use recording::RecordingsDialog;
use rustconn_core::config::AppSettings;
use rustconn_core::import::ImportResult;
use rustconn_core::models::{Connection, Snippet};
use rustconn_core::variables::Variable;
use secrecy::SecretString;
pub use settings::SettingsDialog;
pub use shortcuts::ShortcutsDialog;
pub use smart_folder::{SmartFolderCallback, SmartFolderDialog};
pub use snippet::SnippetDialog;
pub use statistics::{StatisticsDialog, empty_statistics};
pub use template::{TemplateCallback, TemplateDialog};
pub use template_manager::TemplateManagerDialog;
pub use terminal_search::TerminalSearchDialog;
pub use tunnel::TunnelManagerWindow;
pub use variable_setup::{VariableSetupResponse, show_variable_setup_dialog};
pub use variables::VariablesDialog;
pub use wol::WolDialog;
pub use workspace::WorkspaceManagerDialog;

/// Result from connection dialog containing connection and optional password
#[derive(Debug, Clone)]
pub struct ConnectionDialogResult {
    /// The connection configuration
    pub connection: Connection,
    /// Password value if user entered one (for KeePass/Keyring/Stored sources)
    pub password: Option<SecretString>,
}

/// Type alias for connection dialog callback
pub type ConnectionCallback = Rc<RefCell<Option<Box<dyn Fn(Option<ConnectionDialogResult>)>>>>;

/// Type alias for import dialog callback
pub type ImportCallback = Rc<RefCell<Option<Box<dyn Fn(Option<ImportResult>)>>>>;

/// Type alias for import dialog callback with source name
pub type ImportWithSourceCallback = Rc<RefCell<Option<Box<dyn Fn(Option<ImportResult>, String)>>>>;

/// Type alias for settings dialog callback
pub type SettingsCallback = Rc<RefCell<Option<Box<dyn Fn(Option<AppSettings>)>>>>;

/// Type alias for snippet dialog callback
pub type SnippetCallback = Rc<RefCell<Option<Box<dyn Fn(Option<Snippet>)>>>>;

/// Type alias for variables dialog callback
pub type VariablesCallback = Rc<RefCell<Option<Box<dyn Fn(Option<Vec<Variable>>)>>>>;

//! Dialog windows for `RustConn`

mod cluster;
mod connection;
mod document;
mod export;
mod history;
mod import;
mod log_viewer;
mod password;
mod password_generator;
mod progress;
mod settings;
mod snippet;
mod statistics;
mod template;
mod variables;

pub use cluster::{ClusterCallback, ClusterDialog, ClusterListDialog};
pub use connection::ConnectionDialog;
pub use document::{
    CloseDocumentDialog, DocumentCallback, DocumentDialogResult, DocumentProtectionDialog,
    NewDocumentDialog, OpenDocumentDialog, SaveDocumentDialog,
};
pub use export::{ExportCallback, ExportDialog};
pub use history::HistoryDialog;
pub use import::ImportDialog;
pub use log_viewer::LogViewerDialog;
pub use password::{PasswordDialog, PasswordDialogResult};
pub use password_generator::show_password_generator_dialog;
pub use progress::ProgressDialog;
pub use settings::SettingsDialog;
pub use snippet::SnippetDialog;
pub use statistics::{empty_statistics, StatisticsDialog};
pub use template::{TemplateCallback, TemplateDialog, TemplateManagerDialog};
pub use variables::VariablesDialog;

use rustconn_core::config::AppSettings;
use rustconn_core::import::ImportResult;
use rustconn_core::models::{Connection, Snippet};
use rustconn_core::variables::Variable;
use std::cell::RefCell;
use std::rc::Rc;

/// Type alias for connection dialog callback
pub type ConnectionCallback = Rc<RefCell<Option<Box<dyn Fn(Option<Connection>)>>>>;

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

//! Document system for `RustConn`
//!
//! This module provides a document-based organization system for connections,
//! allowing users to create portable documents that contain connections, groups,
//! variables, and templates. Documents can be encrypted with a password for
//! secure sharing.
//!
//! # Features
//!
//! - Create independent containers for connections and groups
//! - Password-based encryption for document protection
//! - Export/import for portable sharing
//! - Dirty state tracking for unsaved changes
//!
//! # Example
//!
//! ```rust,ignore
//! use rustconn_core::document::{Document, DocumentManager};
//!
//! let mut manager = DocumentManager::new();
//! let doc_id = manager.create("My Connections".to_string());
//!
//! // Add connections, groups, etc.
//! // ...
//!
//! // Save the document
//! manager.save(doc_id, Path::new("connections.rcdb"), None)?;
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

use crate::models::{Connection, ConnectionGroup, ConnectionTemplate};
use crate::variables::Variable;

/// Errors that can occur during document operations
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DocumentError {
    /// Document not found
    #[error("Document not found: {0}")]
    NotFound(Uuid),

    /// Failed to parse document
    #[error("Failed to parse document: {0}")]
    ParseError(String),

    /// Failed to serialize document
    #[error("Failed to serialize document: {0}")]
    SerializeError(String),

    /// I/O error during document operation
    #[error("I/O error: {0}")]
    IoError(String),

    /// Encryption/decryption error
    #[error("Encryption error: {0}")]
    EncryptionError(String),

    /// Invalid password provided
    #[error("Invalid password")]
    InvalidPassword,

    /// Document is encrypted but no password provided
    #[error("Document is encrypted, password required")]
    PasswordRequired,

    /// Invalid document format
    #[error("Invalid document format: {0}")]
    InvalidFormat(String),
}

/// Result type for document operations
pub type DocumentResult<T> = std::result::Result<T, DocumentError>;

/// Document format version for compatibility
pub const DOCUMENT_FORMAT_VERSION: u32 = 1;

/// Magic bytes for identifying encrypted documents
const ENCRYPTED_MAGIC: &[u8] = b"RCDB_ENC";

/// A portable document containing connections, groups, variables, and templates
///
/// Documents serve as independent containers that can be shared between users
/// or used to organize connections into separate environments (e.g., work, personal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// Unique identifier for the document
    pub id: Uuid,
    /// Human-readable name for the document
    pub name: String,
    /// Optional description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Connections contained in this document
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<Connection>,
    /// Groups for organizing connections
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<ConnectionGroup>,
    /// Document-level variables
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub variables: HashMap<String, Variable>,
    /// Connection templates
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<ConnectionTemplate>,
    /// Timestamp when the document was created
    pub created_at: DateTime<Utc>,
    /// Timestamp when the document was last modified
    pub modified_at: DateTime<Utc>,
    /// Document format version for compatibility
    #[serde(default = "default_format_version")]
    pub format_version: u32,
}

const fn default_format_version() -> u32 {
    DOCUMENT_FORMAT_VERSION
}

impl Document {
    /// Creates a new empty document with the given name
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: None,
            connections: Vec::new(),
            groups: Vec::new(),
            variables: HashMap::new(),
            templates: Vec::new(),
            created_at: now,
            modified_at: now,
            format_version: DOCUMENT_FORMAT_VERSION,
        }
    }

    /// Sets the description for this document
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Adds a connection to this document
    pub fn add_connection(&mut self, connection: Connection) {
        self.connections.push(connection);
        self.touch();
    }

    /// Removes a connection by ID
    ///
    /// Returns `true` if a connection was removed
    pub fn remove_connection(&mut self, id: Uuid) -> bool {
        let len_before = self.connections.len();
        self.connections.retain(|c| c.id != id);
        let removed = self.connections.len() < len_before;
        if removed {
            self.touch();
        }
        removed
    }

    /// Gets a connection by ID
    #[must_use]
    pub fn get_connection(&self, id: Uuid) -> Option<&Connection> {
        self.connections.iter().find(|c| c.id == id)
    }

    /// Gets a mutable reference to a connection by ID
    #[must_use]
    pub fn get_connection_mut(&mut self, id: Uuid) -> Option<&mut Connection> {
        self.connections.iter_mut().find(|c| c.id == id)
    }

    /// Adds a group to this document
    pub fn add_group(&mut self, group: ConnectionGroup) {
        self.groups.push(group);
        self.touch();
    }

    /// Removes a group by ID
    ///
    /// Returns `true` if a group was removed
    pub fn remove_group(&mut self, id: Uuid) -> bool {
        let len_before = self.groups.len();
        self.groups.retain(|g| g.id != id);
        let removed = self.groups.len() < len_before;
        if removed {
            self.touch();
        }
        removed
    }

    /// Gets a group by ID
    #[must_use]
    pub fn get_group(&self, id: Uuid) -> Option<&ConnectionGroup> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// Sets a variable in this document
    pub fn set_variable(&mut self, variable: Variable) {
        self.variables.insert(variable.name.clone(), variable);
        self.touch();
    }

    /// Removes a variable by name
    ///
    /// Returns the removed variable if it existed
    pub fn remove_variable(&mut self, name: &str) -> Option<Variable> {
        let removed = self.variables.remove(name);
        if removed.is_some() {
            self.touch();
        }
        removed
    }

    /// Gets a variable by name
    #[must_use]
    pub fn get_variable(&self, name: &str) -> Option<&Variable> {
        self.variables.get(name)
    }

    /// Adds a template to this document
    pub fn add_template(&mut self, template: ConnectionTemplate) {
        self.templates.push(template);
        self.touch();
    }

    /// Removes a template by ID
    ///
    /// Returns `true` if a template was removed
    pub fn remove_template(&mut self, id: Uuid) -> bool {
        let len_before = self.templates.len();
        self.templates.retain(|t| t.id != id);
        let removed = self.templates.len() < len_before;
        if removed {
            self.touch();
        }
        removed
    }

    /// Gets a template by ID
    #[must_use]
    pub fn get_template(&self, id: Uuid) -> Option<&ConnectionTemplate> {
        self.templates.iter().find(|t| t.id == id)
    }

    /// Updates the `modified_at` timestamp to now
    pub fn touch(&mut self) {
        self.modified_at = Utc::now();
    }

    /// Returns the number of connections in this document
    #[must_use]
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Returns the number of groups in this document
    #[must_use]
    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    /// Returns the number of variables in this document
    #[must_use]
    pub fn variable_count(&self) -> usize {
        self.variables.len()
    }

    /// Returns the number of templates in this document
    #[must_use]
    pub fn template_count(&self) -> usize {
        self.templates.len()
    }

    /// Serializes the document to JSON
    ///
    /// # Errors
    ///
    /// Returns `DocumentError::SerializeError` if serialization fails
    pub fn to_json(&self) -> DocumentResult<String> {
        serde_json::to_string_pretty(self).map_err(|e| DocumentError::SerializeError(e.to_string()))
    }

    /// Deserializes a document from JSON
    ///
    /// # Errors
    ///
    /// Returns `DocumentError::ParseError` if parsing fails
    pub fn from_json(json: &str) -> DocumentResult<Self> {
        serde_json::from_str(json).map_err(|e| DocumentError::ParseError(e.to_string()))
    }

    /// Serializes the document to YAML
    ///
    /// # Errors
    ///
    /// Returns `DocumentError::SerializeError` if serialization fails
    pub fn to_yaml(&self) -> DocumentResult<String> {
        serde_yaml::to_string(self).map_err(|e| DocumentError::SerializeError(e.to_string()))
    }

    /// Deserializes a document from YAML
    ///
    /// # Errors
    ///
    /// Returns `DocumentError::ParseError` if parsing fails
    pub fn from_yaml(yaml: &str) -> DocumentResult<Self> {
        serde_yaml::from_str(yaml).map_err(|e| DocumentError::ParseError(e.to_string()))
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new("Untitled")
    }
}

/// Manager for handling multiple documents with dirty state tracking
///
/// The `DocumentManager` provides CRUD operations for documents and tracks
/// which documents have unsaved changes.
#[derive(Debug, Default)]
pub struct DocumentManager {
    /// Loaded documents indexed by ID
    documents: HashMap<Uuid, Document>,
    /// Dirty flags for each document (true = has unsaved changes)
    dirty_flags: HashMap<Uuid, bool>,
    /// File paths for documents that have been saved
    file_paths: HashMap<Uuid, std::path::PathBuf>,
}

impl DocumentManager {
    /// Creates a new empty document manager
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new document with the given name
    ///
    /// Returns the ID of the newly created document
    pub fn create(&mut self, name: String) -> Uuid {
        let doc = Document::new(name);
        let id = doc.id;
        self.documents.insert(id, doc);
        self.dirty_flags.insert(id, true); // New documents are dirty
        id
    }

    /// Loads a document from a file
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the document file
    /// * `password` - Optional password for encrypted documents
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, parsed, or decrypted
    pub fn load(&mut self, path: &Path, password: Option<&str>) -> DocumentResult<Uuid> {
        let content = std::fs::read(path).map_err(|e| DocumentError::IoError(e.to_string()))?;

        let doc = if content.starts_with(ENCRYPTED_MAGIC) {
            // Document is encrypted
            let password = password.ok_or(DocumentError::PasswordRequired)?;
            decrypt_document(&content, password)?
        } else {
            // Try to parse as JSON first, then YAML
            let content_str =
                String::from_utf8(content).map_err(|e| DocumentError::ParseError(e.to_string()))?;

            Document::from_json(&content_str)
                .or_else(|_| Document::from_yaml(&content_str))
                .map_err(|e| DocumentError::ParseError(e.to_string()))?
        };

        let id = doc.id;
        self.documents.insert(id, doc);
        self.dirty_flags.insert(id, false); // Loaded documents are clean
        self.file_paths.insert(id, path.to_path_buf());
        Ok(id)
    }

    /// Saves a document to a file
    ///
    /// # Arguments
    ///
    /// * `id` - ID of the document to save
    /// * `path` - Path to save the document to
    /// * `password` - Optional password for encryption
    ///
    /// # Errors
    ///
    /// Returns an error if the document is not found or cannot be written
    pub fn save(&mut self, id: Uuid, path: &Path, password: Option<&str>) -> DocumentResult<()> {
        let doc = self.documents.get(&id).ok_or(DocumentError::NotFound(id))?;

        let content = if let Some(pwd) = password {
            encrypt_document(doc, pwd)?
        } else {
            doc.to_json()?.into_bytes()
        };

        std::fs::write(path, content).map_err(|e| DocumentError::IoError(e.to_string()))?;

        self.dirty_flags.insert(id, false);
        self.file_paths.insert(id, path.to_path_buf());
        Ok(())
    }

    /// Exports a document to a portable file format
    ///
    /// This creates a standalone file that can be imported elsewhere.
    ///
    /// # Errors
    ///
    /// Returns an error if the document is not found or cannot be written
    pub fn export(&self, id: Uuid, path: &Path) -> DocumentResult<()> {
        let doc = self.documents.get(&id).ok_or(DocumentError::NotFound(id))?;

        let json = doc.to_json()?;
        std::fs::write(path, json).map_err(|e| DocumentError::IoError(e.to_string()))
    }

    /// Imports a document from a file
    ///
    /// This is similar to `load` but always creates a new document ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed
    pub fn import(&mut self, path: &Path) -> DocumentResult<Uuid> {
        let content =
            std::fs::read_to_string(path).map_err(|e| DocumentError::IoError(e.to_string()))?;

        let mut doc = Document::from_json(&content)
            .or_else(|_| Document::from_yaml(&content))
            .map_err(|e| DocumentError::ParseError(e.to_string()))?;

        // Generate new ID for imported document
        doc.id = Uuid::new_v4();
        let id = doc.id;

        self.documents.insert(id, doc);
        self.dirty_flags.insert(id, true); // Imported documents are dirty
        Ok(id)
    }

    /// Gets a reference to a document by ID
    #[must_use]
    pub fn get(&self, id: Uuid) -> Option<&Document> {
        self.documents.get(&id)
    }

    /// Gets a mutable reference to a document by ID
    ///
    /// This automatically marks the document as dirty.
    pub fn get_mut(&mut self, id: Uuid) -> Option<&mut Document> {
        if self.documents.contains_key(&id) {
            self.dirty_flags.insert(id, true);
        }
        self.documents.get_mut(&id)
    }

    /// Removes a document from the manager
    ///
    /// Returns the removed document if it existed
    pub fn remove(&mut self, id: Uuid) -> Option<Document> {
        self.dirty_flags.remove(&id);
        self.file_paths.remove(&id);
        self.documents.remove(&id)
    }

    /// Returns true if the document has unsaved changes
    #[must_use]
    pub fn is_dirty(&self, id: Uuid) -> bool {
        self.dirty_flags.get(&id).copied().unwrap_or(false)
    }

    /// Marks a document as dirty (has unsaved changes)
    pub fn mark_dirty(&mut self, id: Uuid) {
        if self.documents.contains_key(&id) {
            self.dirty_flags.insert(id, true);
        }
    }

    /// Marks a document as clean (no unsaved changes)
    pub fn mark_clean(&mut self, id: Uuid) {
        if self.documents.contains_key(&id) {
            self.dirty_flags.insert(id, false);
        }
    }

    /// Returns the file path for a document if it has been saved
    #[must_use]
    pub fn get_path(&self, id: Uuid) -> Option<&Path> {
        self.file_paths.get(&id).map(std::path::PathBuf::as_path)
    }

    /// Returns all document IDs
    #[must_use]
    pub fn document_ids(&self) -> Vec<Uuid> {
        self.documents.keys().copied().collect()
    }

    /// Returns the number of loaded documents
    #[must_use]
    pub fn document_count(&self) -> usize {
        self.documents.len()
    }

    /// Returns true if any document has unsaved changes
    #[must_use]
    pub fn has_dirty_documents(&self) -> bool {
        self.dirty_flags.values().any(|&dirty| dirty)
    }

    /// Returns IDs of all dirty documents
    #[must_use]
    pub fn dirty_document_ids(&self) -> Vec<Uuid> {
        self.dirty_flags
            .iter()
            .filter_map(|(&id, &dirty)| if dirty { Some(id) } else { None })
            .collect()
    }

    /// Inserts a document directly into the manager
    ///
    /// This is primarily useful for testing. The document is marked as dirty.
    pub fn insert(&mut self, document: Document) -> Uuid {
        let id = document.id;
        self.documents.insert(id, document);
        self.dirty_flags.insert(id, true);
        id
    }
}

/// Encrypts a document using password-based encryption
///
/// Uses AES-256-GCM with Argon2id key derivation.
fn encrypt_document(doc: &Document, password: &str) -> DocumentResult<Vec<u8>> {
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
    use ring::rand::{SecureRandom, SystemRandom};

    // Serialize document to JSON
    let plaintext = doc.to_json()?;

    // Generate random salt and nonce
    let rng = SystemRandom::new();
    let mut salt = [0u8; 32];
    let mut nonce_bytes = [0u8; 12];
    rng.fill(&mut salt)
        .map_err(|_| DocumentError::EncryptionError("Failed to generate salt".to_string()))?;
    rng.fill(&mut nonce_bytes)
        .map_err(|_| DocumentError::EncryptionError("Failed to generate nonce".to_string()))?;

    // Derive key using Argon2id
    let key = derive_key(password, &salt)?;

    // Encrypt using AES-256-GCM
    let unbound_key = UnboundKey::new(&AES_256_GCM, &key)
        .map_err(|_| DocumentError::EncryptionError("Failed to create key".to_string()))?;
    let less_safe_key = LessSafeKey::new(unbound_key);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let mut ciphertext = plaintext.into_bytes();
    less_safe_key
        .seal_in_place_append_tag(nonce, Aad::empty(), &mut ciphertext)
        .map_err(|_| DocumentError::EncryptionError("Encryption failed".to_string()))?;

    // Build output: magic + salt + nonce + ciphertext
    let mut output = Vec::with_capacity(ENCRYPTED_MAGIC.len() + 32 + 12 + ciphertext.len());
    output.extend_from_slice(ENCRYPTED_MAGIC);
    output.extend_from_slice(&salt);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    Ok(output)
}

/// Decrypts an encrypted document
fn decrypt_document(data: &[u8], password: &str) -> DocumentResult<Document> {
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};

    // Verify magic bytes
    if !data.starts_with(ENCRYPTED_MAGIC) {
        return Err(DocumentError::InvalidFormat(
            "Not an encrypted document".to_string(),
        ));
    }

    let header_len = ENCRYPTED_MAGIC.len();
    let min_len = header_len + 32 + 12 + 16; // magic + salt + nonce + tag
    if data.len() < min_len {
        return Err(DocumentError::InvalidFormat(
            "Document too short".to_string(),
        ));
    }

    // Extract salt, nonce, and ciphertext
    let salt = &data[header_len..header_len + 32];
    let nonce_bytes = &data[header_len + 32..header_len + 32 + 12];
    let ciphertext = &data[header_len + 32 + 12..];

    // Derive key
    let key = derive_key(password, salt)?;

    // Decrypt using AES-256-GCM
    let unbound_key = UnboundKey::new(&AES_256_GCM, &key)
        .map_err(|_| DocumentError::EncryptionError("Failed to create key".to_string()))?;
    let less_safe_key = LessSafeKey::new(unbound_key);

    let mut nonce_array = [0u8; 12];
    nonce_array.copy_from_slice(nonce_bytes);
    let nonce = Nonce::assume_unique_for_key(nonce_array);

    let mut plaintext = ciphertext.to_vec();
    less_safe_key
        .open_in_place(nonce, Aad::empty(), &mut plaintext)
        .map_err(|_| DocumentError::InvalidPassword)?;

    // Remove the authentication tag
    let tag_len = AES_256_GCM.tag_len();
    plaintext.truncate(plaintext.len() - tag_len);

    // Parse JSON
    let json =
        String::from_utf8(plaintext).map_err(|e| DocumentError::ParseError(e.to_string()))?;
    Document::from_json(&json)
}

/// Derives an encryption key from a password using Argon2id
fn derive_key(password: &str, salt: &[u8]) -> DocumentResult<[u8; 32]> {
    use argon2::{Algorithm, Argon2, Params, Version};

    // Use lighter parameters for faster key derivation
    // In production, consider using higher values (e.g., m=65536, t=3, p=4)
    #[cfg(test)]
    let params = Params::new(4096, 2, 1, Some(32))
        .map_err(|e| DocumentError::EncryptionError(format!("Invalid Argon2 params: {e}")))?;

    #[cfg(not(test))]
    let params = Params::new(65536, 3, 4, Some(32))
        .map_err(|e| DocumentError::EncryptionError(format!("Invalid Argon2 params: {e}")))?;

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut key = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| DocumentError::EncryptionError(format!("Key derivation failed: {e}")))?;

    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_new() {
        let doc = Document::new("Test Document");
        assert_eq!(doc.name, "Test Document");
        assert!(doc.connections.is_empty());
        assert!(doc.groups.is_empty());
        assert!(doc.variables.is_empty());
        assert!(doc.templates.is_empty());
        assert_eq!(doc.format_version, DOCUMENT_FORMAT_VERSION);
    }

    #[test]
    fn test_document_with_description() {
        let doc = Document::new("Test").with_description("A test document");
        assert_eq!(doc.description, Some("A test document".to_string()));
    }

    #[test]
    fn test_document_add_remove_connection() {
        let mut doc = Document::new("Test");
        let conn = Connection::new_ssh("Server".to_string(), "host.com".to_string(), 22);
        let conn_id = conn.id;

        doc.add_connection(conn);
        assert_eq!(doc.connection_count(), 1);
        assert!(doc.get_connection(conn_id).is_some());

        assert!(doc.remove_connection(conn_id));
        assert_eq!(doc.connection_count(), 0);
        assert!(doc.get_connection(conn_id).is_none());
    }

    #[test]
    fn test_document_add_remove_group() {
        let mut doc = Document::new("Test");
        let group = ConnectionGroup::new("Servers".to_string());
        let group_id = group.id;

        doc.add_group(group);
        assert_eq!(doc.group_count(), 1);
        assert!(doc.get_group(group_id).is_some());

        assert!(doc.remove_group(group_id));
        assert_eq!(doc.group_count(), 0);
    }

    #[test]
    fn test_document_set_remove_variable() {
        let mut doc = Document::new("Test");
        let var = Variable::new("host", "example.com");

        doc.set_variable(var);
        assert_eq!(doc.variable_count(), 1);
        assert!(doc.get_variable("host").is_some());

        assert!(doc.remove_variable("host").is_some());
        assert_eq!(doc.variable_count(), 0);
    }

    #[test]
    fn test_document_add_remove_template() {
        let mut doc = Document::new("Test");
        let template = ConnectionTemplate::new_ssh("SSH Template".to_string());
        let template_id = template.id;

        doc.add_template(template);
        assert_eq!(doc.template_count(), 1);
        assert!(doc.get_template(template_id).is_some());

        assert!(doc.remove_template(template_id));
        assert_eq!(doc.template_count(), 0);
    }

    #[test]
    fn test_document_json_round_trip() {
        let mut doc = Document::new("Test Document");
        doc.add_connection(Connection::new_ssh(
            "Server".to_string(),
            "host.com".to_string(),
            22,
        ));
        doc.set_variable(Variable::new("user", "admin"));

        let json = doc.to_json().unwrap();
        let parsed = Document::from_json(&json).unwrap();

        assert_eq!(doc.id, parsed.id);
        assert_eq!(doc.name, parsed.name);
        assert_eq!(doc.connection_count(), parsed.connection_count());
        assert_eq!(doc.variable_count(), parsed.variable_count());
    }

    #[test]
    fn test_document_yaml_round_trip() {
        let doc = Document::new("Test Document");

        let yaml = doc.to_yaml().unwrap();
        let parsed = Document::from_yaml(&yaml).unwrap();

        assert_eq!(doc.id, parsed.id);
        assert_eq!(doc.name, parsed.name);
    }

    #[test]
    fn test_document_manager_create() {
        let mut manager = DocumentManager::new();
        let id = manager.create("New Document".to_string());

        assert!(manager.get(id).is_some());
        assert!(manager.is_dirty(id)); // New documents are dirty
        assert_eq!(manager.document_count(), 1);
    }

    #[test]
    fn test_document_manager_dirty_tracking() {
        let mut manager = DocumentManager::new();
        let id = manager.create("Test".to_string());

        assert!(manager.is_dirty(id));

        manager.mark_clean(id);
        assert!(!manager.is_dirty(id));

        manager.mark_dirty(id);
        assert!(manager.is_dirty(id));
    }

    #[test]
    fn test_document_manager_get_mut_marks_dirty() {
        let mut manager = DocumentManager::new();
        let id = manager.create("Test".to_string());

        manager.mark_clean(id);
        assert!(!manager.is_dirty(id));

        // Getting mutable reference should mark as dirty
        let _ = manager.get_mut(id);
        assert!(manager.is_dirty(id));
    }

    #[test]
    fn test_document_manager_remove() {
        let mut manager = DocumentManager::new();
        let id = manager.create("Test".to_string());

        assert!(manager.remove(id).is_some());
        assert!(manager.get(id).is_none());
        assert_eq!(manager.document_count(), 0);
    }

    #[test]
    fn test_document_manager_dirty_document_ids() {
        let mut manager = DocumentManager::new();
        let id1 = manager.create("Doc 1".to_string());
        let id2 = manager.create("Doc 2".to_string());

        manager.mark_clean(id1);

        let dirty_ids = manager.dirty_document_ids();
        assert_eq!(dirty_ids.len(), 1);
        assert!(dirty_ids.contains(&id2));
    }

    #[test]
    fn test_document_touch_updates_modified_at() {
        let mut doc = Document::new("Test");
        let initial = doc.modified_at;

        std::thread::sleep(std::time::Duration::from_millis(10));
        doc.touch();

        assert!(doc.modified_at > initial);
    }
}

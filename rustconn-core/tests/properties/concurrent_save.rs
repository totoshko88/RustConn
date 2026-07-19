//! Property-based tests for concurrent ConfigManager saves.
//!
//! **Validates: TEST-1 — two ConfigManager instances saving simultaneously must not corrupt data.**
//!
//! After ARCH-2 (file locking), concurrent saves should serialize via advisory lock.
//! The file must always contain valid TOML with a consistent set of connections.

use std::sync::Arc;
use std::thread;

use proptest::prelude::*;
use rustconn_core::config::ConfigManager;
use rustconn_core::models::{Connection, ProtocolConfig, SshConfig};
use tempfile::TempDir;

/// Strategy for generating a connection name.
fn name_strategy() -> impl Strategy<Value = String> {
    "[A-Z][a-z]{2,10} [A-Z][a-z]{2,8}"
}

/// Strategy for generating a hostname.
fn host_strategy() -> impl Strategy<Value = String> {
    "[a-z]{3,8}\\.[a-z]{2,5}\\.(com|net|org)"
}

/// Strategy for generating a valid port.
fn port_strategy() -> impl Strategy<Value = u16> {
    1u16..=65535
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20))]

    /// Two concurrent saves must not corrupt the config file.
    /// After both complete, the file must contain valid data (exactly 1 connection
    /// from one of the two writers — last-write-wins under exclusive lock).
    #[test]
    fn concurrent_save_no_corruption(
        name1 in name_strategy(),
        host1 in host_strategy(),
        port1 in port_strategy(),
        name2 in name_strategy(),
        host2 in host_strategy(),
        port2 in port_strategy(),
    ) {
        let temp_dir = TempDir::new().map_err(|e| TestCaseError::fail(format!("{e}")))?;
        let config_dir = temp_dir.path().to_path_buf();

        let manager1 = ConfigManager::with_config_dir(config_dir.clone());
        let manager2 = ConfigManager::with_config_dir(config_dir);

        manager1.ensure_config_dir().map_err(|e| TestCaseError::fail(format!("{e}")))?;

        let m1 = Arc::new(manager1);
        let m2 = Arc::new(manager2);

        let conn1 = Connection::new(
            name1.clone(),
            host1,
            port1,
            ProtocolConfig::Ssh(SshConfig::default()),
        );
        let conn2 = Connection::new(
            name2.clone(),
            host2,
            port2,
            ProtocolConfig::Ssh(SshConfig::default()),
        );

        let m1_clone = Arc::clone(&m1);
        let c1 = conn1.clone();
        let handle1 = thread::spawn(move || {
            m1_clone.save_connections(std::slice::from_ref(&c1))
        });

        let m2_clone = Arc::clone(&m2);
        let c2 = conn2.clone();
        let handle2 = thread::spawn(move || {
            m2_clone.save_connections(std::slice::from_ref(&c2))
        });

        let r1 = handle1.join().map_err(|_| TestCaseError::fail("thread 1 panicked"))?;
        let r2 = handle2.join().map_err(|_| TestCaseError::fail("thread 2 panicked"))?;

        // Both saves must succeed (lock serializes them)
        r1.map_err(|e| TestCaseError::fail(format!("save 1 failed: {e}")))?;
        r2.map_err(|e| TestCaseError::fail(format!("save 2 failed: {e}")))?;

        // File must be valid — load must succeed
        let loaded = m1.load_connections()
            .map_err(|e| TestCaseError::fail(format!("load failed: {e}")))?;

        // Exactly 1 connection (last writer wins)
        prop_assert_eq!(
            loaded.len(), 1,
            "Expected exactly 1 connection after concurrent save, got {}",
            loaded.len()
        );

        // The surviving connection must be one of the two we wrote
        prop_assert!(
            loaded[0].name == name1 || loaded[0].name == name2,
            "Loaded connection '{}' doesn't match either '{}' or '{}'",
            loaded[0].name, name1, name2
        );
    }

    /// Sequential saves must each persist correctly (no lock contention issues).
    #[test]
    fn sequential_saves_are_consistent(
        names in prop::collection::vec(name_strategy(), 1..5),
        hosts in prop::collection::vec(host_strategy(), 1..5),
        ports in prop::collection::vec(port_strategy(), 1..5),
    ) {
        let count = names.len().min(hosts.len()).min(ports.len());
        let temp_dir = TempDir::new().map_err(|e| TestCaseError::fail(format!("{e}")))?;
        let manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
        manager.ensure_config_dir().map_err(|e| TestCaseError::fail(format!("{e}")))?;

        let connections: Vec<Connection> = (0..count)
            .map(|i| {
                Connection::new(
                    names[i].clone(),
                    hosts[i].clone(),
                    ports[i],
                    ProtocolConfig::Ssh(SshConfig::default()),
                )
            })
            .collect();

        manager.save_connections(&connections)
            .map_err(|e| TestCaseError::fail(format!("save failed: {e}")))?;

        let loaded = manager.load_connections()
            .map_err(|e| TestCaseError::fail(format!("load failed: {e}")))?;

        prop_assert_eq!(loaded.len(), count);
        for (i, conn) in loaded.iter().enumerate() {
            prop_assert_eq!(&conn.name, &names[i]);
            prop_assert_eq!(conn.port, ports[i]);
        }
    }
}

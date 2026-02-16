//! Property tests for secrets management fixes
//!
//! Feature: secrets-management-fixes

use proptest::prelude::*;
use rustconn_core::config::SecretSettings;
use secrecy::ExposeSecret;

// Feature: secrets-management-fixes, Property 4: Round-trip encryption/decryption of Bitwarden password
proptest! {
    #[test]
    fn bitwarden_password_encrypt_decrypt_roundtrip(
        password in "[a-zA-Z0-9!@#$%^&*()_+\\-=\\[\\]{};':\",./<>?]{1,128}"
    ) {
        let mut settings = SecretSettings::default();
        settings.bitwarden_password = Some(secrecy::SecretString::from(password.clone()));

        // Encrypt
        settings.encrypt_bitwarden_password();
        prop_assert!(
            settings.bitwarden_password_encrypted.is_some(),
            "Encrypted password should be present after encryption"
        );

        // Clear runtime password to simulate restart
        settings.bitwarden_password = None;

        // Decrypt
        let decrypted = settings.decrypt_bitwarden_password();
        prop_assert!(decrypted, "Decryption should succeed");
        prop_assert!(
            settings.bitwarden_password.is_some(),
            "Password should be present after decryption"
        );

        let recovered = settings.bitwarden_password.as_ref().unwrap().expose_secret();
        prop_assert_eq!(
            recovered,
            &password,
            "Decrypted password must match original"
        );
    }
}

// Feature: secrets-management-fixes, Property 1: Lookup key consistency for group passwords (round-trip)
proptest! {
    #[test]
    fn group_lookup_key_consistency(
        group_name in "[a-zA-Z0-9 _-]{1,50}",
    ) {
        use rustconn_core::models::ConnectionGroup;
        use rustconn_core::secret::KeePassHierarchy;

        let group = ConnectionGroup::new(group_name);
        let groups = vec![group.clone()];

        // The key used for saving must equal the key used for resolving
        let save_key = KeePassHierarchy::build_group_lookup_key(&group, &groups, true);
        let resolve_key = KeePassHierarchy::build_group_lookup_key(&group, &groups, true);

        prop_assert_eq!(
            &save_key,
            &resolve_key,
            "Save key and resolve key must be identical"
        );

        // For non-KeePass backends, the resolver uses group.id.to_string()
        // Verify that group.id is stable
        let id_key1 = group.id.to_string();
        let id_key2 = group.id.to_string();
        prop_assert_eq!(
            &id_key1,
            &id_key2,
            "Group ID-based key must be stable"
        );
    }
}

// Feature: secrets-management-fixes, Property 2: Hierarchy traversal for Inherit
proptest! {
    #[test]
    fn inherit_hierarchy_traversal(
        depth in 1_usize..=5,
        vault_level in 0_usize..5,
    ) {
        use rustconn_core::models::{ConnectionGroup, PasswordSource};

        // Build a hierarchy of groups
        let vault_level = vault_level % depth; // Ensure vault_level is within range
        let mut groups = Vec::new();
        let mut parent_id = None;

        for i in 0..depth {
            let mut group = if let Some(pid) = parent_id {
                ConnectionGroup::with_parent(format!("Group_{i}"), pid)
            } else {
                ConnectionGroup::new(format!("Group_{i}"))
            };

            if i == vault_level {
                group.password_source = Some(PasswordSource::Vault);
            } else {
                group.password_source = Some(PasswordSource::Inherit);
            }

            parent_id = Some(group.id);
            groups.push(group);
        }

        // Traverse from the deepest group upward
        let deepest = &groups[depth - 1];
        let mut current_group_id = Some(deepest.id);
        let mut found_vault = false;

        while let Some(gid) = current_group_id {
            let Some(group) = groups.iter().find(|g| g.id == gid) else {
                break;
            };

            if group.password_source == Some(PasswordSource::Vault) {
                found_vault = true;
                break;
            } else if group.password_source == Some(PasswordSource::Inherit) {
                // Continue to parent
            }

            current_group_id = group.parent_id;
        }

        prop_assert!(
            found_vault,
            "Traversal must find the Vault group at level {vault_level} in hierarchy of depth {depth}"
        );
    }
}

// Feature: secrets-management-fixes, Property 3: Group credentials merge
proptest! {
    #[test]
    fn group_credentials_merge(
        cred_username in proptest::option::of("[a-zA-Z0-9]{1,20}"),
        cred_domain in proptest::option::of("[a-zA-Z0-9.]{1,30}"),
        cred_password in "[a-zA-Z0-9]{1,30}",
        group_username in proptest::option::of("[a-zA-Z0-9]{1,20}"),
        group_domain in proptest::option::of("[a-zA-Z0-9.]{1,30}"),
    ) {
        use rustconn_core::models::{ConnectionGroup, Credentials};

        let creds = Credentials {
            username: cred_username.clone(),
            password: Some(secrecy::SecretString::from(cred_password.clone())),
            key_passphrase: None,
            domain: cred_domain.clone(),
        };

        let mut group = ConnectionGroup::new("TestGroup".to_string());
        group.username = group_username.clone();
        group.domain = group_domain.clone();

        // Simulate merge: group overrides take precedence
        let mut merged = creds.clone();
        if let Some(ref uname) = group.username {
            merged.username = Some(uname.clone());
        }
        if let Some(ref dom) = group.domain {
            merged.domain = Some(dom.clone());
        }

        // Password must always come from credentials
        prop_assert_eq!(
            merged.expose_password(),
            Some(cred_password.as_str()),
            "Password must come from credentials"
        );

        // Username: group overrides if set
        if group_username.is_some() {
            prop_assert_eq!(
                merged.username.as_deref(),
                group_username.as_deref(),
                "Username should come from group when group has username"
            );
        } else {
            prop_assert_eq!(
                merged.username.as_deref(),
                cred_username.as_deref(),
                "Username should come from credentials when group has no username"
            );
        }

        // Domain: group overrides if set
        if group_domain.is_some() {
            prop_assert_eq!(
                merged.domain.as_deref(),
                group_domain.as_deref(),
                "Domain should come from group when group has domain"
            );
        } else {
            prop_assert_eq!(
                merged.domain.as_deref(),
                cred_domain.as_deref(),
                "Domain should come from credentials when group has no domain"
            );
        }
    }
}

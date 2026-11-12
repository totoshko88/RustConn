//! Property-based tests for jump-host secrets.
//!
//! A jump host's password belongs to that hop alone (issues #191, #203). Whatever
//! the chain length, the passwords present and the host-key policy, the route
//! built by `jump_route` hands each secret only to its own hop's askpass slot,
//! and the tunnel route (`tunnel_hop_args`) hands no hop past the tunnel's own
//! jump host any secret at all.

use proptest::prelude::*;
use rustconn_core::jump_route::{
    JumpHop, JumpRoute, JumpRouteRequest, TunnelHopsRequest, jump_route, tunnel_hop_args,
};
use rustconn_core::ssh_askpass::AskpassSlot;
use rustconn_core::ssh_tunnel::HopHostKeys;
use secrecy::{ExposeSecret, SecretString};

fn host_keys() -> impl Strategy<Value = HopHostKeys> {
    prop_oneof![Just(HopHostKeys::UserConfig), Just(HopHostKeys::KnownOnly)]
}

/// A chain of 1-5 plain hops, each with an optional password of its own.
fn chain() -> impl Strategy<Value = Vec<(String, Option<String>)>> {
    prop::collection::vec(
        (
            "[a-z]{1,8}\\.example",
            prop::option::of("[A-Za-z0-9]{8,16}"),
        ),
        1..=5,
    )
}

/// How many times `needle` occurs in `haystack`.
fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

proptest! {
    /// Each password goes to the slot of the hop it was given with, and a hop
    /// without a password gets no slot and no helper.
    #[test]
    fn a_secret_reaches_only_its_own_hop(
        hops in chain(),
        keys in host_keys(),
        target_pw in any::<bool>(),
    ) {
        let list: Vec<JumpHop> = hops
            .iter()
            .map(|(spec, password)| JumpHop {
                spec: spec.clone(),
                password: password.as_deref().map(SecretString::from),
            })
            .collect();
        let route = jump_route(&JumpRouteRequest {
            hops: &list,
            first_hop_identity: None,
            target_identity: None,
            deeper_hop_identity: None,
            first_hop_pkcs11: None,
            known_hosts: None,
            hop_host_keys: keys,
            target_askpass_allowed: target_pw,
        });
        let Some(JumpRoute::ProxyCommand { command, passwords }) = route else {
            // `-J` carries no secret: it is only chosen when no hop has one.
            prop_assert!(hops.iter().all(|(_, pw)| pw.is_none()));
            return Ok(());
        };

        let with_password: Vec<usize> = hops
            .iter()
            .enumerate()
            .filter_map(|(index, (_, pw))| pw.as_ref().map(|_| index))
            .collect();
        let mut slots: Vec<usize> = passwords
            .iter()
            .map(|(slot, _)| match slot {
                AskpassSlot::JumpHost(index) => *index,
                AskpassSlot::Target => usize::MAX,
            })
            .collect();
        slots.sort_unstable();
        prop_assert_eq!(&slots, &with_password);
        for (slot, secret) in &passwords {
            if let AskpassSlot::JumpHost(index) = slot {
                prop_assert_eq!(Some(secret.expose_secret()), hops[*index].1.as_deref());
            }
        }

        // No secret in argv; one helper per password hop.
        for (_, pw) in &hops {
            if let Some(pw) = pw {
                prop_assert!(!command.contains(pw.as_str()), "{}", command);
            }
        }
        prop_assert_eq!(count(&command, "SSH_ASKPASS=/"), with_password.len(), "{}", command);
        // Every hop's process starts behind an `env` that clears the target's file.
        prop_assert_eq!(
            count(&command, "env _RC_TGT_PW_FILE="),
            count(&command, " -W "),
            "{}",
            command
        );
    }

    /// Every hop past a tunnel's own jump host starts with the tunnel's helper
    /// and the target's file cleared, and gets no helper of its own.
    #[test]
    fn a_tunnel_gives_no_hop_past_its_jump_host_a_secret(
        hops in chain(),
        keys in host_keys(),
    ) {
        let specs: Vec<String> = hops.into_iter().map(|(spec, _)| spec).collect();
        let args = tunnel_hop_args(&TunnelHopsRequest {
            hops: &specs,
            first_hop_identity: None,
            deeper_hop_identity: None,
            first_hop_pkcs11: None,
            known_hosts: None,
            hop_host_keys: keys,
        });
        let args = args.expect("plain hops give a route");
        prop_assert_eq!(args.len(), 2);
        let command = &args[1];
        prop_assert_eq!(count(command, "SSH_ASKPASS=/"), 0, "{}", command);
        prop_assert_eq!(count(command, "SSH_ASKPASS_REQUIRE=force"), 0, "{}", command);
        prop_assert_eq!(count(command, " -W "), specs.len(), "{}", command);
        prop_assert_eq!(count(command, "env _RC_TGT_PW_FILE="), specs.len(), "{}", command);
    }
}

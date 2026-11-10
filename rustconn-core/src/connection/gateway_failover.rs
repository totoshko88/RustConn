//! Ordered failover between saved jump hosts.
//!
//! A connection (or a group) can list fallback jump hosts behind its primary.
//! Just before a launch, the gateways are probed in order and the first one that
//! answers is used for that launch only; the stored configuration is not
//! changed, so the primary is tried again on the next connection.
//!
//! The launchers need no knowledge of this. [`use_gateway`] writes the choice to
//! the launch's private copy of the connection, and a connection's own
//! `jump_host_id` outranks every inherited tier.
//!
//! A failover never connects directly: when no gateway answers, the first one is
//! returned anyway so the launch fails with the normal SSH error.

use std::collections::HashSet;

use uuid::Uuid;

use super::jump_chain::{MAX_HOPS, hop_spec, proxy_jump_of, routes_through_jump_host};
use super::port_check::check_ssh_banner;
use super::ssh_inheritance::resolve_ssh_jump_host_candidates;
use crate::config::NetworkSettings;
use crate::models::{Connection, ConnectionGroup, ProtocolConfig};

/// One gateway to try, in failover order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayProbe {
    /// The saved connection acting as this gateway.
    pub gateway_id: Uuid,
    /// Its display name, for telling the user which gateway was used.
    pub gateway_name: String,
    /// The `(host, port)` the client dials first to use this gateway, or `None`
    /// when a free-text `ProxyJump` stands in front of it and that first hop
    /// cannot be known. An untestable gateway is treated as usable.
    pub entry_point: Option<(String, u16)>,
    /// Whether this is the configured primary rather than a fallback.
    pub is_primary: bool,
}

/// How a gateway came to be chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayOutcome {
    /// The primary answered.
    Primary,
    /// The primary was unreachable or missing, and a fallback answered.
    FailedOver,
    /// Nothing answered; the first gateway is used so SSH reports the failure.
    NoneReachable,
}

/// The gateway to use for one launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewaySelection {
    /// The chosen gateway.
    pub gateway_id: Uuid,
    /// Its display name.
    pub gateway_name: String,
    /// How it was chosen.
    pub outcome: GatewayOutcome,
}

/// Returns `true` when the launch path for `connection` consults saved gateways.
///
/// A custom `ProxyCommand` route replaces RustConn-managed hops entirely.
fn consults_gateways(connection: &Connection) -> bool {
    let unmanaged = matches!(
        &connection.protocol_config,
        ProtocolConfig::Ssh(cfg) | ProtocolConfig::Sftp(cfg)
            if crate::ssh_tunnel::has_unmanaged_proxy_route(cfg)
    );
    routes_through_jump_host(connection) && !unmanaged
}

/// Returns the first hop the client dials to use `gateway`, walking outward
/// through its own jump hosts.
///
/// `None` when a hop on the way carries a free-text `ProxyJump`, which has no
/// host this function can address.
fn entry_point<'a>(
    gateway: &'a Connection,
    target_id: Uuid,
    lookup: &impl Fn(Uuid) -> Option<&'a Connection>,
) -> Option<(String, u16)> {
    let mut visited = HashSet::from([target_id, gateway.id]);
    let mut current = gateway;
    for _ in 0..MAX_HOPS {
        if proxy_jump_of(current).is_some_and(|p| !p.trim().is_empty()) {
            return None;
        }
        let Some(next) = current
            .jump_host_id()
            .filter(|id| visited.insert(*id))
            .and_then(lookup)
        else {
            break;
        };
        current = next;
    }
    Some((current.host.clone(), current.port))
}

/// Returns the gateways to try for `connection`, in failover order.
///
/// Empty when failover does not apply: fewer than two gateways are configured,
/// none of them exists any more, or the protocol does not route through one. A
/// configured gateway that no longer exists is left out, so a deleted primary
/// moves straight on to its fallbacks.
#[must_use]
pub fn gateway_probes_by<'a>(
    connection: &Connection,
    lookup: impl Fn(Uuid) -> Option<&'a Connection>,
    groups: &[ConnectionGroup],
    network: &NetworkSettings,
) -> Vec<GatewayProbe> {
    if !consults_gateways(connection) {
        return Vec::new();
    }
    let candidates = resolve_ssh_jump_host_candidates(connection, groups, network);
    if candidates.len() < 2 {
        return Vec::new();
    }
    candidates
        .iter()
        .enumerate()
        .filter_map(|(index, id)| {
            let gateway = lookup(*id)?;
            Some(GatewayProbe {
                gateway_id: *id,
                gateway_name: gateway.name.clone(),
                entry_point: entry_point(gateway, connection.id, &lookup),
                is_primary: index == 0,
            })
        })
        .collect()
}

/// Picks the first usable gateway, asking `reachable` about each in turn.
///
/// Returns `None` only for an empty list. When nothing is reachable the first
/// gateway is returned, never "no gateway": an empty answer would let the launch
/// fall through to a direct connection.
pub fn select_gateway(
    probes: &[GatewayProbe],
    mut reachable: impl FnMut(&str, u16) -> bool,
) -> Option<GatewaySelection> {
    let answered = probes.iter().find(|probe| {
        probe
            .entry_point
            .as_ref()
            .is_none_or(|(host, port)| reachable(host, *port))
    });
    let (probe, outcome) = match answered {
        Some(probe) if probe.is_primary => (probe, GatewayOutcome::Primary),
        Some(probe) => (probe, GatewayOutcome::FailedOver),
        None => (probes.first()?, GatewayOutcome::NoneReachable),
    };
    Some(GatewaySelection {
        gateway_id: probe.gateway_id,
        gateway_name: probe.gateway_name.clone(),
        outcome,
    })
}

/// Probes `probes` over the network. Blocks for up to about twice
/// `timeout_secs` per unreachable gateway, so keep it off the GTK main thread.
#[must_use]
pub fn probe_gateways(probes: &[GatewayProbe], timeout_secs: u32) -> Option<GatewaySelection> {
    select_gateway(probes, |host, port| {
        check_ssh_banner(host, port, timeout_secs)
    })
}

/// Routes this copy of `connection` through `gateway_id`.
///
/// Call on a launch's private clone: the stored connection must keep its
/// configured primary. Protocols without a jump host are left alone.
pub fn use_gateway(connection: &mut Connection, gateway_id: Uuid) {
    connection.set_jump_host_id(gateway_id);
}

/// Describes every configured gateway route, for detecting a re-pointed
/// connection.
///
/// Covers the whole ordered list, including each gateway's own onward hops, so
/// that editing a fallback is noticed. It reads configuration, not the gateway
/// chosen for a launch, so an ordinary failover is not reported as a change. A
/// reference that no longer resolves is described by its id.
#[must_use]
pub fn gateway_route_by<'a>(
    connection: &Connection,
    lookup: impl Fn(Uuid) -> Option<&'a Connection>,
    groups: &[ConnectionGroup],
    network: &NetworkSettings,
) -> String {
    // An empty connection list resolves only the free-text tier, in `-J` form.
    let free_text = super::jump_chain::resolve_proxy_jump_value(connection, &[], groups, network);
    let mut parts: Vec<String> = free_text.into_iter().collect();
    if consults_gateways(connection) {
        for id in resolve_ssh_jump_host_candidates(connection, groups, network) {
            parts.push(describe_gateway_chain(id, connection.id, &lookup));
        }
    }
    parts.join(" | ")
}

/// Renders one gateway and its onward hops as `gateway > next > …`.
fn describe_gateway_chain<'a>(
    gateway_id: Uuid,
    target_id: Uuid,
    lookup: &impl Fn(Uuid) -> Option<&'a Connection>,
) -> String {
    let mut visited = HashSet::from([target_id]);
    let mut hops = Vec::new();
    let mut current = Some(gateway_id);
    for _ in 0..MAX_HOPS {
        let Some(id) = current.filter(|id| visited.insert(*id)) else {
            break;
        };
        let Some(hop) = lookup(id) else {
            hops.push(id.to_string());
            break;
        };
        hops.push(hop_spec(hop));
        current = hop.jump_host_id();
    }
    hops.join(" > ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::resolve_jump_chain;
    use crate::connection::ssh_inheritance::resolve_ssh_jump_host_id;
    use crate::models::NetworkMode;

    fn ssh(name: &str) -> Connection {
        Connection::new_ssh(name.to_string(), format!("{name}.example.com"), 22)
    }

    fn set_jump(conn: &mut Connection, id: Uuid) {
        use_gateway(conn, id);
    }

    fn target_with(gateways: &[&Connection]) -> Connection {
        let mut target = ssh("target");
        set_jump(&mut target, gateways[0].id);
        target.fallback_jump_host_ids = gateways[1..].iter().map(|g| g.id).collect();
        target
    }

    fn probes(target: &Connection, all: &[Connection]) -> Vec<GatewayProbe> {
        gateway_probes_by(
            target,
            |id| all.iter().find(|c| c.id == id),
            &[],
            &NetworkSettings::default(),
        )
    }

    fn outcome_when(probes: &[GatewayProbe], up: &[&str]) -> (String, GatewayOutcome) {
        let selection = select_gateway(probes, |host, _| {
            up.iter().any(|name| host.starts_with(name))
        })
        .expect("a non-empty list always selects");
        (selection.gateway_name, selection.outcome)
    }

    #[test]
    fn a_single_gateway_is_not_probed() {
        let gateway = ssh("a");
        let target = target_with(&[&gateway]);
        assert!(probes(&target, &[gateway]).is_empty());
    }

    #[test]
    fn the_first_reachable_gateway_wins_in_order() {
        let (a, b, c) = (ssh("a"), ssh("b"), ssh("c"));
        let target = target_with(&[&a, &b, &c]);
        let list = probes(&target, &[a, b, c]);

        assert_eq!(
            outcome_when(&list, &["a", "b", "c"]),
            ("a".to_string(), GatewayOutcome::Primary)
        );
        assert_eq!(
            outcome_when(&list, &["b", "c"]),
            ("b".to_string(), GatewayOutcome::FailedOver)
        );
        assert_eq!(
            outcome_when(&list, &["c"]),
            ("c".to_string(), GatewayOutcome::FailedOver)
        );
    }

    #[test]
    fn with_nothing_reachable_the_primary_is_still_used_never_direct() {
        let (a, b) = (ssh("a"), ssh("b"));
        let target = target_with(&[&a, &b]);
        let list = probes(&target, &[a, b]);
        assert_eq!(
            outcome_when(&list, &[]),
            ("a".to_string(), GatewayOutcome::NoneReachable)
        );
    }

    #[test]
    fn a_deleted_primary_moves_to_its_fallback() {
        let (a, b) = (ssh("a"), ssh("b"));
        let target = target_with(&[&a, &b]);
        let list = probes(&target, std::slice::from_ref(&b));
        assert_eq!(list.len(), 1);
        assert_eq!(
            outcome_when(&list, &["b"]),
            ("b".to_string(), GatewayOutcome::FailedOver)
        );
        // Even when down: the launch must keep a gateway, not lose the route.
        assert_eq!(
            outcome_when(&list, &[]),
            ("b".to_string(), GatewayOutcome::NoneReachable)
        );
    }

    #[test]
    fn the_probe_targets_the_hop_the_client_dials_first() {
        let (mut a, b, outer) = (ssh("a"), ssh("b"), ssh("outer"));
        set_jump(&mut a, outer.id);
        let target = target_with(&[&a, &b]);
        let list = probes(&target, &[a, b, outer]);
        assert_eq!(
            list[0].entry_point,
            Some(("outer.example.com".to_string(), 22))
        );
    }

    #[test]
    fn a_gateway_behind_free_text_proxy_is_treated_as_usable() {
        let (mut a, b) = (ssh("a"), ssh("b"));
        if let ProtocolConfig::Ssh(cfg) = &mut a.protocol_config {
            cfg.proxy_jump = Some("opaque.example.com".to_string());
        }
        let target = target_with(&[&a, &b]);
        let list = probes(&target, &[a, b]);
        assert_eq!(list[0].entry_point, None);
        assert_eq!(
            outcome_when(&list, &[]),
            ("a".to_string(), GatewayOutcome::Primary)
        );
    }

    #[test]
    fn group_fallbacks_are_inherited_with_the_group_primary() {
        let (a, b) = (ssh("a"), ssh("b"));
        let mut group = ConnectionGroup::new("g".to_string());
        group.ssh_jump_host_id = Some(a.id);
        group.ssh_fallback_jump_host_ids = vec![b.id];
        let mut target = ssh("target");
        target.group_id = Some(group.id);

        let all = [a.clone(), b.clone()];
        let list = gateway_probes_by(
            &target,
            |id| all.iter().find(|c| c.id == id),
            &[group],
            &NetworkSettings::default(),
        );
        let ids: Vec<Uuid> = list.iter().map(|p| p.gateway_id).collect();
        assert_eq!(ids, vec![a.id, b.id]);
    }

    #[test]
    fn a_connection_primary_does_not_pick_up_group_fallbacks() {
        let (own, group_primary, group_fallback) = (ssh("own"), ssh("gp"), ssh("gf"));
        let mut group = ConnectionGroup::new("g".to_string());
        group.ssh_jump_host_id = Some(group_primary.id);
        group.ssh_fallback_jump_host_ids = vec![group_fallback.id];
        let mut target = ssh("target");
        target.group_id = Some(group.id);
        set_jump(&mut target, own.id);

        assert_eq!(
            resolve_ssh_jump_host_candidates(&target, &[group], &NetworkSettings::default()),
            vec![own.id]
        );
    }

    #[test]
    fn candidates_start_with_the_resolved_primary_and_drop_duplicates_and_self() {
        let (a, b) = (ssh("a"), ssh("b"));
        let mut target = target_with(&[&a, &b]);
        target.fallback_jump_host_ids = vec![b.id, a.id, b.id, target.id];
        let network = NetworkSettings::default();

        let candidates = resolve_ssh_jump_host_candidates(&target, &[], &network);
        assert_eq!(candidates, vec![a.id, b.id]);
        assert_eq!(
            candidates.first().copied(),
            resolve_ssh_jump_host_id(&target, &[], &network)
        );
    }

    #[test]
    fn fallbacks_without_a_primary_of_their_own_are_ignored() {
        let mut target = ssh("target");
        target.fallback_jump_host_ids = vec![Uuid::new_v4()];
        assert!(
            resolve_ssh_jump_host_candidates(&target, &[], &NetworkSettings::default()).is_empty()
        );
    }

    #[test]
    fn direct_mode_refuses_inherited_gateways() {
        let (a, b) = (ssh("a"), ssh("b"));
        let mut group = ConnectionGroup::new("g".to_string());
        group.ssh_jump_host_id = Some(a.id);
        group.ssh_fallback_jump_host_ids = vec![b.id];
        let mut target = ssh("target");
        target.group_id = Some(group.id);
        target.network_mode = NetworkMode::Direct;

        assert!(
            resolve_ssh_jump_host_candidates(&target, &[group], &NetworkSettings::default())
                .is_empty()
        );
    }

    #[test]
    fn the_chosen_gateway_outranks_every_inherited_tier() {
        let (a, b) = (ssh("a"), ssh("b"));
        let mut group = ConnectionGroup::new("g".to_string());
        group.ssh_jump_host_id = Some(a.id);
        group.ssh_fallback_jump_host_ids = vec![b.id];
        let mut launch = ssh("target");
        launch.group_id = Some(group.id);

        use_gateway(&mut launch, b.id);

        let chain = resolve_jump_chain(
            &launch,
            &[a, b.clone()],
            &[group],
            &NetworkSettings::default(),
        );
        assert_eq!(chain.hop_ids, vec![Some(b.id)]);
    }

    #[test]
    fn a_custom_proxy_command_disables_failover() {
        let (a, b) = (ssh("a"), ssh("b"));
        let mut target = target_with(&[&a, &b]);
        if let ProtocolConfig::Ssh(cfg) = &mut target.protocol_config {
            cfg.proxy_command = Some("nc %h %p".to_string());
        }
        assert!(probes(&target, &[a, b]).is_empty());
    }

    fn route(target: &Connection, all: &[Connection]) -> String {
        gateway_route_by(
            target,
            |id| all.iter().find(|c| c.id == id),
            &[],
            &NetworkSettings::default(),
        )
    }

    #[test]
    fn the_route_names_picker_selected_gateways() {
        let (a, b) = (ssh("a"), ssh("b"));
        let target = target_with(&[&a, &b]);
        assert_eq!(
            route(&target, &[a, b]),
            "a.example.com | b.example.com".to_string()
        );
    }

    #[test]
    fn editing_or_reordering_fallbacks_changes_the_route() {
        let (a, b, c) = (ssh("a"), ssh("b"), ssh("c"));
        let all = [a.clone(), b.clone(), c.clone()];
        let original = route(&target_with(&[&a, &b, &c]), &all);
        assert_ne!(original, route(&target_with(&[&a, &c, &b]), &all));
        assert_ne!(original, route(&target_with(&[&a, &b]), &all));
    }

    #[test]
    fn repointing_a_gateway_host_changes_the_route() {
        let (a, b) = (ssh("a"), ssh("b"));
        let target = target_with(&[&a, &b]);
        let before = route(&target, &[a.clone(), b.clone()]);
        let mut moved = b;
        moved.host = "attacker.example.com".to_string();
        assert_ne!(before, route(&target, &[a, moved]));
    }

    #[test]
    fn a_free_text_route_keeps_its_remembered_form() {
        let mut target = ssh("target");
        if let ProtocolConfig::Ssh(cfg) = &mut target.protocol_config {
            cfg.proxy_jump = Some("bastion.example.com".to_string());
        }
        assert_eq!(route(&target, &[]), "bastion.example.com".to_string());
    }

    #[test]
    fn a_missing_primary_with_a_live_fallback_is_covered_by_failover() {
        use crate::connection::jump_chain::{
            failover_covers_missing_primary, find_dangling_bastions,
        };

        let (a, b) = (ssh("a"), ssh("b"));
        let target = target_with(&[&a, &b]);
        let network = NetworkSettings::default();
        let live = std::slice::from_ref(&b);
        // A path without failover still skips the primary, so it is reported.
        let dangling = find_dangling_bastions(&target, live, &[], &network);
        assert_eq!(dangling.len(), 1);
        assert_eq!(dangling[0].referenced_id, a.id);
        // A path that fails over goes through `b` instead.
        assert!(failover_covers_missing_primary(
            &target,
            find_in(live),
            &[],
            &network
        ));
    }

    fn find_in<'a>(list: &'a [Connection]) -> impl Fn(Uuid) -> Option<&'a Connection> {
        move |id| list.iter().find(|c| c.id == id)
    }

    #[test]
    fn failover_does_not_cover_a_live_primary_or_dead_fallbacks() {
        use crate::connection::jump_chain::failover_covers_missing_primary;

        let (a, b) = (ssh("a"), ssh("b"));
        let target = target_with(&[&a, &b]);
        let network = NetworkSettings::default();
        for live in [vec![a.clone(), b], Vec::new(), vec![a]] {
            assert!(!failover_covers_missing_primary(
                &target,
                find_in(&live),
                &[],
                &network
            ));
        }
    }
}

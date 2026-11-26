//! Which probe a connection test runs, and how its outcome reads.
//!
//! [`plan_probe`] decides from the connection alone (plus the connection list,
//! groups and network settings that resolve its jump hosts) and touches no
//! network, so the decision is unit-tested. The tester in the parent module
//! runs the plan.
//!
//! A connection behind a jump host is probed *through* it, with
//! `ssh -W host:port` to the jump host nearest the target: a direct dial to a
//! private address times out, and `127.0.0.1` would probe this computer. The
//! probe never sends a password. It runs with `BatchMode=yes`, so a jump host
//! that needs a password, an unknown host key or a PIN reports the test as
//! skipped rather than failed.

use std::path::PathBuf;

use crate::config::NetworkSettings;
use crate::connection::jump_chain::{resolve_jump_chain, routes_through_jump_host};
use crate::connection::ssh_inheritance::resolve_ssh_key_path;
use crate::jump_route::{
    JumpHop, JumpRoute, JumpRouteRequest, hops_are_plain, jump_route, plain_path,
};
use crate::models::{
    Connection, ConnectionGroup, KubernetesConfig, ProtocolConfig, ProtocolType,
    ZeroTrustProviderConfig,
};
use crate::ssh_tunnel::{HopHostKeys, has_unmanaged_proxy_route};

/// What resolves a connection's jump hosts: the other connections, the groups
/// and the global network settings.
#[derive(Debug, Clone, Copy)]
pub struct TestContext<'a> {
    /// Every saved connection, so a `jump_host_id` can be looked up.
    pub connections: &'a [Connection],
    /// The groups a jump host can be inherited from.
    pub groups: &'a [ConnectionGroup],
    /// The global jump host and ProxyJump.
    pub network: &'a NetworkSettings,
}

/// The probe [`plan_probe`] picked for a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbePlan {
    /// Dial `host:port` from this computer; for SSH, also read the banner.
    Tcp {
        /// Host name or address.
        host: String,
        /// TCP port.
        port: u16,
        /// Whether to check for an SSH banner after connecting.
        ssh_banner: bool,
    },
    /// Open `target` from the jump host nearest it, with `ssh <args>`.
    ThroughJump(JumpProbe),
    /// Run `kubectl <args>`; exit status 0 is a pass.
    Kubectl {
        /// The arguments, without the program.
        args: Vec<String>,
    },
    /// Run `aws <args>`, which prints the instance's SSM `PingStatus`.
    AwsSsm {
        /// The arguments, without the program.
        args: Vec<String>,
    },
    /// Nothing to probe; the reason says why.
    Skip(String),
    /// The configuration cannot be probed as it stands.
    Invalid(String),
}

/// An `ssh -W` probe through a jump host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpProbe {
    /// The jump host the target is opened from, as the user knows it: its
    /// connection name, or its `[user@]host[:port]`.
    pub jump_host: String,
    /// `host:port` of the target, as the jump host dials it.
    pub target: String,
    /// The `ssh` arguments, without the program.
    pub args: Vec<String>,
}

/// Decides how to test `connection`.
///
/// `timeout_secs` becomes `ConnectTimeout` of a jump probe and the request
/// timeout of `kubectl`.
#[must_use]
pub fn plan_probe(connection: &Connection, ctx: &TestContext<'_>, timeout_secs: u64) -> ProbePlan {
    match &connection.protocol_config {
        ProtocolConfig::Serial(_) => {
            return ProbePlan::Skip("serial device, no network address to probe".to_string());
        }
        ProtocolConfig::ZeroTrust(config) => {
            return match &config.provider_config {
                ZeroTrustProviderConfig::AwsSsm(ssm) if !ssm.target.trim().is_empty() => {
                    ProbePlan::AwsSsm {
                        args: aws_ssm_args(&ssm.target, &ssm.profile, ssm.region.as_deref()),
                    }
                }
                _ => ProbePlan::Skip(format!(
                    "{} target is a resource ID, not a network address; \
                     this provider has no reachability check",
                    config.provider.display_name()
                )),
            };
        }
        ProtocolConfig::Kubernetes(config) => return kubectl_plan(config, timeout_secs),
        ProtocolConfig::Web(config) => {
            if config.tunnel_via.is_some() {
                return ProbePlan::Skip(
                    "browses through an SSH tunnel (Tunnel via), which the test does not open"
                        .to_string(),
                );
            }
            if let Some(proxy) = config
                .socks_proxy
                .as_deref()
                .filter(|p| !p.trim().is_empty())
            {
                return ProbePlan::Skip(format!(
                    "browses through the SOCKS proxy {}, which the test does not use",
                    without_userinfo(proxy)
                ));
            }
            return match web_endpoint(&connection.host, connection.port) {
                Ok((host, port)) => ProbePlan::Tcp {
                    host,
                    port,
                    ssh_banner: false,
                },
                Err(reason) => ProbePlan::Invalid(reason),
            };
        }
        ProtocolConfig::Ssh(config) | ProtocolConfig::Sftp(config)
            if has_unmanaged_proxy_route(config) =>
        {
            return ProbePlan::Skip(
                "routed by a custom ProxyCommand or ProxyJump option, which the test does not run"
                    .to_string(),
            );
        }
        _ => {}
    }

    if routes_through_jump_host(connection) {
        let chain = resolve_jump_chain(connection, ctx.connections, ctx.groups, ctx.network);
        if !chain.is_empty() {
            if !hops_are_plain(&chain.hops) {
                return ProbePlan::Skip(format!(
                    "jump host route {} is not a plain [user@]host[:port] list the test can follow",
                    chain.hops.join(",")
                ));
            }
            let find = |id: Option<uuid::Uuid>| {
                id.and_then(|id| ctx.connections.iter().find(|c| c.id == id))
            };
            let hops: Vec<HopSpec<'_>> = chain
                .hops
                .iter()
                .zip(&chain.hop_ids)
                .map(|(spec, id)| HopSpec {
                    spec,
                    connection: find(*id),
                })
                .collect();
            return ProbePlan::ThroughJump(jump_probe(
                &connection.host,
                connection.port,
                &hops,
                ctx.groups,
                timeout_secs,
            ));
        }
    }

    ProbePlan::Tcp {
        host: connection.host.clone(),
        port: connection.port,
        ssh_banner: connection.protocol == ProtocolType::Ssh,
    }
}

/// `aws ssm describe-instance-information` for one instance, printing only its
/// `PingStatus` (`None` when SSM does not know the instance).
fn aws_ssm_args(target: &str, profile: &str, region: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = [
        "ssm",
        "describe-instance-information",
        "--filters",
        &format!("Key=InstanceIds,Values={}", target.trim()),
        "--query",
        "InstanceInformationList[0].PingStatus",
        "--output",
        "text",
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    if profile != "default" && !profile.is_empty() {
        args.push("--profile".to_string());
        args.push(profile.to_string());
    }
    if let Some(region) = region.filter(|r| !r.is_empty()) {
        args.push("--region".to_string());
        args.push(region.to_string());
    }
    args
}

/// `kubectl get pod <pod>` for an exec connection, `kubectl auth can-i create
/// pods` for one that starts a temporary busybox pod.
fn kubectl_plan(config: &KubernetesConfig, timeout_secs: u64) -> ProbePlan {
    let mut args = Vec::new();
    if let Some(kubeconfig) = &config.kubeconfig {
        args.push("--kubeconfig".to_string());
        args.push(kubeconfig.display().to_string());
    }
    if let Some(context) = config.context.as_deref().filter(|c| !c.is_empty()) {
        args.push("--context".to_string());
        args.push(context.to_string());
    }
    if let Some(namespace) = config.namespace.as_deref().filter(|n| !n.is_empty()) {
        args.push("--namespace".to_string());
        args.push(namespace.to_string());
    }
    args.push(format!("--request-timeout={timeout_secs}s"));
    if config.use_busybox {
        args.extend(["auth", "can-i", "create", "pods"].map(String::from));
    } else {
        let Some(pod) = config
            .pod
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        else {
            return ProbePlan::Skip("no pod configured".to_string());
        };
        // `--` keeps a pod name starting with `-` from being read as an option.
        args.extend(["get", "pod", "-o", "name", "--", pod].map(String::from));
    }
    ProbePlan::Kubectl { args }
}

/// The host and port a Web connection's URL points at.
///
/// The scheme gives the default port (`http` 80, `https` 443); without a
/// scheme or for another one, `fallback_port` is used.
///
/// # Errors
///
/// Returns the reason when the URL has no host or an invalid port.
pub fn web_endpoint(url: &str, fallback_port: u16) -> Result<(String, u16), String> {
    let raw = url.trim();
    // Error text is printed by the CLI, so it never carries `user:password@`.
    let url = without_userinfo(raw);
    let url = url.as_str();
    let (default_port, rest) = match raw.split_once("://") {
        Some((scheme, rest)) => {
            let port = match scheme.to_ascii_lowercase().as_str() {
                "http" => 80,
                "https" => 443,
                _ => fallback_port,
            };
            (port, rest)
        }
        None => (fallback_port, raw),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = if let Some(bracketed) = host_port.strip_prefix('[') {
        let Some((host, after)) = bracketed.split_once(']') else {
            return Err(format!("unclosed IPv6 address in URL: {url}"));
        };
        (host, after.strip_prefix(':'))
    } else {
        match host_port.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_port, None),
        }
    };
    if host.is_empty() {
        return Err(format!("no host in URL: {url}"));
    }
    let port = match port {
        Some(port) => port
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or_else(|| format!("invalid port in URL: {url}"))?,
        None => default_port,
    };
    Ok((host.to_string(), port))
}

/// `url` with any `user[:password]@` in its authority replaced by `***@`.
///
/// For text a user sees (skip reasons, errors): a Web URL or SOCKS proxy may
/// carry credentials, and those must not reach the terminal or a log.
fn without_userinfo(url: &str) -> String {
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, url),
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(authority_end);
    let authority = match authority.rsplit_once('@') {
        Some((_, host)) => format!("***@{host}"),
        None => authority.to_string(),
    };
    match scheme {
        Some(scheme) => format!("{scheme}://{authority}{tail}"),
        None => format!("{authority}{tail}"),
    }
}

/// One hop of a resolved chain and the connection behind it, if any.
struct HopSpec<'a> {
    spec: &'a str,
    connection: Option<&'a Connection>,
}

/// The identity file of a hop's connection, as a path `ssh` can open.
fn hop_identity(hop: Option<&Connection>, groups: &[ConnectionGroup]) -> Option<PathBuf> {
    hop.and_then(|hop| resolve_ssh_key_path(hop, groups))
        .and_then(|path| crate::resolve_key_path(&path))
}

/// The PKCS#11 provider of a hop's connection.
fn hop_pkcs11(hop: Option<&Connection>) -> Option<String> {
    hop.and_then(|hop| match &hop.protocol_config {
        ProtocolConfig::Ssh(c) => c.pkcs11_provider.clone(),
        _ => None,
    })
    .filter(|p| !p.trim().is_empty())
}

/// Splits a `[user@]host[:port]` hop into its parts.
fn split_hop(spec: &str) -> (Option<&str>, &str, Option<&str>) {
    let (user, host_port) = match spec.rsplit_once('@') {
        Some((user, rest)) => (Some(user), rest),
        None => (None, spec),
    };
    if let Some(bracketed) = host_port.strip_prefix('[')
        && let Some((host, after)) = bracketed.split_once(']')
    {
        return (user, host, after.strip_prefix(':'));
    }
    match host_port.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
            (user, host, Some(port))
        }
        _ => (user, host_port, None),
    }
}

/// Builds `ssh … -W target host0`, where `host0` is `hops[0]`, the jump host
/// nearest the target, and the hops past it are reached the way
/// `rustconn-cli run-script` reaches them: through [`jump_route`] with
/// [`HopHostKeys::KnownOnly`] and no passwords.
fn jump_probe(
    target_host: &str,
    target_port: u16,
    hops: &[HopSpec<'_>],
    groups: &[ConnectionGroup],
    timeout_secs: u64,
) -> JumpProbe {
    let nearest = &hops[0];
    let known_hosts = crate::get_flatpak_known_hosts_path();
    let mut args: Vec<String> = [
        "-o",
        "BatchMode=yes",
        "-o",
        &format!("ConnectTimeout={}", timeout_secs.max(1)),
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "ExitOnForwardFailure=yes",
        // `Authenticated to …` at this level tells a slow target from a slow
        // jump host when the probe times out.
        "-o",
        "LogLevel=VERBOSE",
        // A ControlMaster from the user's config would leave a master behind.
        "-o",
        "ControlMaster=no",
        "-o",
        "ControlPath=none",
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    if let Some(path) = &known_hosts {
        args.push("-o".to_string());
        args.push(format!("UserKnownHostsFile={}", path.display()));
    }
    let nearest_identity = hop_identity(nearest.connection, groups);
    if let Some(key) = &nearest_identity {
        args.push("-i".to_string());
        args.push(key.display().to_string());
        args.push("-o".to_string());
        args.push("IdentitiesOnly=yes".to_string());
    }
    if let Some(provider) = hop_pkcs11(nearest.connection) {
        args.push("-o".to_string());
        args.push(format!("PKCS11Provider={provider}"));
    }

    args.extend(farther_route_args(
        hops,
        groups,
        nearest_identity.as_deref(),
        known_hosts.as_deref(),
    ));

    let target = if target_host.contains(':') {
        format!("[{target_host}]:{target_port}")
    } else {
        format!("{target_host}:{target_port}")
    };
    args.push("-W".to_string());
    args.push(target.clone());

    let (user, host, port) = if let Some(hop) = nearest.connection {
        (
            hop.username.clone().filter(|u| !u.is_empty()),
            hop.host.clone(),
            Some(hop.port.to_string()),
        )
    } else {
        let (user, host, port) = split_hop(nearest.spec);
        (
            user.map(str::to_string),
            host.to_string(),
            port.map(str::to_string),
        )
    };
    if let Some(port) = port.filter(|p| p != "22") {
        args.push("-p".to_string());
        args.push(port);
    }
    if let Some(user) = user {
        args.push("-l".to_string());
        args.push(user);
    }
    // `hops_are_plain` already refused a hop starting with `-`.
    args.push(host);

    JumpProbe {
        jump_host: nearest
            .connection
            .map_or_else(|| nearest.spec.to_string(), |hop| hop.name.clone()),
        target,
        args,
    }
}

/// The `-o ProxyCommand=…` or `-J …` that takes `ssh` to `hops[0]` through
/// the hops past it, the way `run-script` routes: [`jump_route`] with
/// [`HopHostKeys::KnownOnly`] and no passwords. Empty for a single hop.
fn farther_route_args(
    hops: &[HopSpec<'_>],
    groups: &[ConnectionGroup],
    nearest_identity: Option<&std::path::Path>,
    known_hosts: Option<&std::path::Path>,
) -> Vec<String> {
    if hops.len() < 2 {
        return Vec::new();
    }
    let farther: Vec<JumpHop> = hops[1..]
        .iter()
        .map(|hop| JumpHop {
            spec: hop.spec.to_string(),
            password: None,
        })
        .collect();
    let next = hops[1].connection;
    let next_identity = hop_identity(next, groups).map(|p| p.display().to_string());
    let next_pkcs11 = hop_pkcs11(next);
    let nearest_identity = nearest_identity.map(|p| p.display().to_string());
    let route = jump_route(&JumpRouteRequest {
        hops: &farther,
        first_hop_identity: next_identity.as_deref().and_then(plain_path),
        target_identity: nearest_identity.as_deref().and_then(plain_path),
        deeper_hop_identity: None,
        first_hop_pkcs11: next_pkcs11.as_deref().and_then(plain_path),
        known_hosts,
        hop_host_keys: HopHostKeys::KnownOnly,
        target_askpass_allowed: false,
    });
    match route {
        Some(JumpRoute::ProxyCommand { command, .. }) => {
            vec!["-o".to_string(), format!("ProxyCommand={command}")]
        }
        Some(JumpRoute::ProxyJump { value, .. }) => vec!["-J".to_string(), value],
        None => Vec::new(),
    }
}

/// How an `ssh -W` jump probe ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JumpOutcome {
    /// The jump host opened the target.
    Reachable,
    /// The jump host answered but could not open the target.
    TargetFailed(String),
    /// The jump host itself could not be reached.
    JumpFailed(String),
    /// The jump host needs something `BatchMode` refuses: a password, a host
    /// key the user has not accepted, a passphrase or PIN.
    JumpNeedsInteraction(String),
    /// Nothing finished in time; `authenticated` tells whether the jump host
    /// had already let the probe in.
    TimedOut {
        /// Whether the jump host login had succeeded.
        authenticated: bool,
    },
}

/// Reads the outcome of a jump probe from its exit code (`None` when it was
/// stopped at the timeout) and its standard error.
#[must_use]
pub fn classify_jump_probe(exit_code: Option<i32>, stderr: &str) -> JumpOutcome {
    let line_with = |needle: &str| {
        stderr
            .lines()
            .find(|line| line.contains(needle))
            .map(|line| line.trim().to_string())
    };
    let Some(code) = exit_code else {
        return JumpOutcome::TimedOut {
            authenticated: stderr.contains("Authenticated to"),
        };
    };
    if let Some(line) = line_with("open failed") {
        // `channel 0: open failed: connect failed: Connection refused`
        let reason = line
            .split_once("open failed:")
            .map_or(line.as_str(), |(_, reason)| reason)
            .trim()
            .to_string();
        // `AllowTcpForwarding no` on the jump host: the target was never tried.
        if reason.starts_with("administratively prohibited") {
            return JumpOutcome::JumpFailed(format!("the jump host refuses forwarding ({reason})"));
        }
        return JumpOutcome::TargetFailed(reason);
    }
    if line_with("stdio forwarding failed").is_some() {
        return JumpOutcome::TargetFailed("the jump host could not open the target".to_string());
    }
    if code == 0 {
        return JumpOutcome::Reachable;
    }
    // A changed key cannot be accepted interactively and may mean the jump
    // host is not the configured machine: a failure, never a skip.
    if line_with("REMOTE HOST IDENTIFICATION HAS CHANGED").is_some() {
        return JumpOutcome::JumpFailed(
            "the jump host key has changed and does not match known_hosts".to_string(),
        );
    }
    for needle in [
        "Permission denied",
        "Host key verification failed",
        "host key is known",
        "passphrase",
        "Enter PIN",
    ] {
        if let Some(line) = line_with(needle) {
            return JumpOutcome::JumpNeedsInteraction(line);
        }
    }
    // The last line that is not a `LogLevel=VERBOSE` progress note.
    let reason = stderr
        .lines()
        .map(str::trim)
        .rfind(|line| {
            !line.is_empty()
                && !line.starts_with("Authenticated to")
                && !line.starts_with("Connection established")
                && !line.starts_with("Transferred:")
                && !line.starts_with("Bytes per second")
        })
        .map_or_else(|| format!("ssh exited with status {code}"), str::to_string);
    JumpOutcome::JumpFailed(reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{SshConfig, WebConfig};

    fn ctx<'a>(
        connections: &'a [Connection],
        groups: &'a [ConnectionGroup],
        network: &'a NetworkSettings,
    ) -> TestContext<'a> {
        TestContext {
            connections,
            groups,
            network,
        }
    }

    fn bastion() -> Connection {
        let mut b = Connection::new_ssh("Linux SSH (key) bastion".into(), "203.0.113.7".into(), 22);
        b.username = Some("ubuntu".into());
        b
    }

    #[test]
    fn a_direct_ssh_connection_is_dialled_and_its_banner_read() {
        let network = NetworkSettings::default();
        let ssh = Connection::new_ssh("s".into(), "host.lan".into(), 2222);
        let plan = plan_probe(&ssh, &ctx(&[], &[], &network), 10);
        assert_eq!(
            plan,
            ProbePlan::Tcp {
                host: "host.lan".into(),
                port: 2222,
                ssh_banner: true
            }
        );
    }

    #[test]
    fn rdp_and_vnc_behind_their_own_jump_host_go_through_it() {
        let network = NetworkSettings::default();
        let jump = bastion();
        for mut target in [
            Connection::new_rdp("rdp".into(), "10.0.1.5".into(), 3389),
            Connection::new_vnc("vnc".into(), "127.0.0.1".into(), 5901),
        ] {
            target.set_jump_host_id(jump.id);
            let connections = [jump.clone(), target.clone()];
            let ProbePlan::ThroughJump(probe) =
                plan_probe(&target, &ctx(&connections, &[], &network), 7)
            else {
                panic!("expected a jump probe");
            };
            assert_eq!(probe.jump_host, "Linux SSH (key) bastion");
            assert_eq!(probe.target, format!("{}:{}", target.host, target.port));
            let args = probe.args.join(" ");
            assert!(args.contains("BatchMode=yes"), "{args}");
            assert!(args.contains("ConnectTimeout=7"), "{args}");
            assert!(args.contains("StrictHostKeyChecking=yes"), "{args}");
            assert!(args.ends_with(&format!("-W {} -l ubuntu 203.0.113.7", probe.target)));
            assert!(!args.contains("-J"), "single hop needs no route: {args}");
        }
    }

    #[test]
    fn a_jump_host_inherited_from_the_group_is_used() {
        let network = NetworkSettings::default();
        let jump = bastion();
        let mut group = ConnectionGroup::new("AWS".into());
        group.ssh_jump_host_id = Some(jump.id);
        let mut target = Connection::new_ssh("private".into(), "10.0.1.9".into(), 22);
        target.group_id = Some(group.id);
        let connections = [jump, target.clone()];
        let groups = [group];
        let plan = plan_probe(&target, &ctx(&connections, &groups, &network), 10);
        assert!(matches!(plan, ProbePlan::ThroughJump(p) if p.target == "10.0.1.9:22"));
    }

    #[test]
    fn hops_past_the_nearest_jump_host_get_a_known_hosts_only_route() {
        let network = NetworkSettings::default();
        let far = Connection::new_ssh("far".into(), "far.example".into(), 22);
        let mut near = Connection::new_ssh("near".into(), "near.lan".into(), 2200);
        near.set_jump_host_id(far.id);
        let mut target = Connection::new_rdp("win".into(), "10.0.0.4".into(), 3389);
        target.set_jump_host_id(near.id);
        let connections = [far, near, target.clone()];
        let ProbePlan::ThroughJump(probe) =
            plan_probe(&target, &ctx(&connections, &[], &network), 10)
        else {
            panic!("expected a jump probe");
        };
        let args = probe.args.join(" ");
        assert!(args.contains("far.example"), "{args}");
        assert!(
            args.ends_with("-W 10.0.0.4:3389 -p 2200 near.lan"),
            "{args}"
        );
        assert!(!args.contains("password"), "{args}");
    }

    #[test]
    fn a_free_text_chain_that_is_not_plain_is_skipped() {
        let network = NetworkSettings::default();
        let mut target = Connection::new_ssh("t".into(), "10.0.0.2".into(), 22);
        if let ProtocolConfig::Ssh(c) = &mut target.protocol_config {
            c.proxy_jump = Some("a.example,b.example".into());
        }
        let plan = plan_probe(&target, &ctx(&[], &[], &network), 10);
        assert!(matches!(plan, ProbePlan::Skip(r) if r.contains("not a plain")));
    }

    #[test]
    fn a_free_text_proxy_jump_becomes_the_jump_destination() {
        let network = NetworkSettings::default();
        let mut target = Connection::new_ssh("t".into(), "10.0.0.2".into(), 22);
        if let ProtocolConfig::Ssh(c) = &mut target.protocol_config {
            c.proxy_jump = Some("ops@gw.example:2022".into());
        }
        let ProbePlan::ThroughJump(probe) = plan_probe(&target, &ctx(&[], &[], &network), 10)
        else {
            panic!("expected a jump probe");
        };
        assert!(probe.args.join(" ").ends_with("-p 2022 -l ops gw.example"));
    }

    #[test]
    fn a_custom_proxy_command_is_skipped_not_dialled() {
        let network = NetworkSettings::default();
        let config = SshConfig {
            proxy_command: Some("nc -x proxy %h %p".into()),
            ..SshConfig::default()
        };
        let conn = Connection::new("t".into(), "h".into(), 22, ProtocolConfig::Ssh(config));
        let plan = plan_probe(&conn, &ctx(&[], &[], &network), 10);
        assert!(matches!(plan, ProbePlan::Skip(_)));
    }

    #[test]
    fn web_tests_the_host_and_port_of_its_url() {
        let network = NetworkSettings::default();
        let web = Connection::new(
            "cockpit".into(),
            "https://203.0.113.10:9090/system".into(),
            443,
            ProtocolConfig::Web(WebConfig::default()),
        );
        let plan = plan_probe(&web, &ctx(&[], &[], &network), 10);
        assert_eq!(
            plan,
            ProbePlan::Tcp {
                host: "203.0.113.10".into(),
                port: 9090,
                ssh_banner: false
            }
        );
    }

    #[test]
    fn web_through_a_tunnel_or_socks_proxy_is_skipped() {
        let network = NetworkSettings::default();
        let tunnelled = WebConfig {
            tunnel_via: Some(uuid::Uuid::new_v4()),
            ..WebConfig::default()
        };
        let socks = WebConfig {
            socks_proxy: Some("socks5://127.0.0.1:1080".into()),
            ..WebConfig::default()
        };
        for config in [tunnelled, socks] {
            let web = Connection::new(
                "w".into(),
                "https://10.0.0.1".into(),
                443,
                ProtocolConfig::Web(config),
            );
            let plan = plan_probe(&web, &ctx(&[], &[], &network), 10);
            assert!(matches!(plan, ProbePlan::Skip(_)), "{plan:?}");
        }
    }

    #[test]
    fn url_credentials_never_reach_messages() {
        let secret = "hunter2-probe-leak";
        let bad_port = format!("https://admin:{secret}@host:99999/x");
        let Err(reason) = web_endpoint(&bad_port, 443) else {
            panic!("an invalid port must be rejected");
        };
        assert!(!reason.contains(secret), "{reason}");
        assert!(reason.contains("***@host:99999"), "{reason}");

        let network = NetworkSettings::default();
        let socks = WebConfig {
            socks_proxy: Some(format!("socks5://user:{secret}@127.0.0.1:1080")),
            ..WebConfig::default()
        };
        let web = Connection::new(
            "w".into(),
            "https://10.0.0.1".into(),
            443,
            ProtocolConfig::Web(socks),
        );
        let ProbePlan::Skip(reason) = plan_probe(&web, &ctx(&[], &[], &network), 10) else {
            panic!("a SOCKS-proxied Web connection is skipped");
        };
        assert!(!reason.contains(secret), "{reason}");

        assert_eq!(without_userinfo("https://h/p@q"), "https://h/p@q");
        assert_eq!(without_userinfo("u:p@h:1080"), "***@h:1080");
    }

    #[test]
    fn web_urls_are_parsed() {
        assert_eq!(
            web_endpoint("https://h.example", 1),
            Ok(("h.example".into(), 443))
        );
        assert_eq!(
            web_endpoint("http://h.example/a:b", 1),
            Ok(("h.example".into(), 80))
        );
        assert_eq!(
            web_endpoint("h.example:8080", 1),
            Ok(("h.example".into(), 8080))
        );
        assert_eq!(
            web_endpoint("h.example", 8443),
            Ok(("h.example".into(), 8443))
        );
        assert_eq!(
            web_endpoint("https://user:pw@[2001:db8::1]:8443/x?y#z", 1),
            Ok(("2001:db8::1".into(), 8443))
        );
        assert!(web_endpoint("https://", 443).is_err());
        assert!(web_endpoint("https://h:99999", 443).is_err());
        assert!(web_endpoint("https://[::1", 443).is_err());
    }

    #[test]
    fn kubernetes_gets_kubectl_with_its_context() {
        let network = NetworkSettings::default();
        let config = KubernetesConfig {
            kubeconfig: Some("/home/u/.kube/test.yaml".into()),
            context: Some("kind".into()),
            namespace: Some("apps".into()),
            pod: Some("rc-demo".into()),
            ..KubernetesConfig::default()
        };
        let conn = Connection::new(
            "k".into(),
            String::new(),
            0,
            ProtocolConfig::Kubernetes(config),
        );
        let ProbePlan::Kubectl { args } = plan_probe(&conn, &ctx(&[], &[], &network), 5) else {
            panic!("expected kubectl");
        };
        assert_eq!(
            args.join(" "),
            "--kubeconfig /home/u/.kube/test.yaml --context kind --namespace apps \
             --request-timeout=5s get pod -o name -- rc-demo"
        );
    }

    #[test]
    fn kubernetes_busybox_checks_pod_creation_and_no_pod_is_skipped() {
        let network = NetworkSettings::default();
        let busybox = KubernetesConfig {
            use_busybox: true,
            ..KubernetesConfig::default()
        };
        let conn = Connection::new(
            "k".into(),
            String::new(),
            0,
            ProtocolConfig::Kubernetes(busybox),
        );
        assert!(matches!(
            plan_probe(&conn, &ctx(&[], &[], &network), 5),
            ProbePlan::Kubectl { args } if args.ends_with(&["auth", "can-i", "create", "pods"].map(String::from))
        ));
        let no_pod = Connection::new(
            "k".into(),
            String::new(),
            0,
            ProtocolConfig::Kubernetes(KubernetesConfig::default()),
        );
        assert_eq!(
            plan_probe(&no_pod, &ctx(&[], &[], &network), 5),
            ProbePlan::Skip("no pod configured".into())
        );
    }

    #[test]
    fn aws_ssm_asks_for_the_ping_status_and_other_providers_are_skipped() {
        use crate::models::{AwsSsmConfig, ZeroTrustConfig, ZeroTrustProvider};
        let network = NetworkSettings::default();
        let ssm = ZeroTrustConfig {
            provider: ZeroTrustProvider::AwsSsm,
            provider_config: ZeroTrustProviderConfig::AwsSsm(AwsSsmConfig {
                target: "i-0123456789abcdef0".into(),
                profile: "lab".into(),
                region: Some("eu-west-1".into()),
            }),
            custom_args: Vec::new(),
        };
        let conn = Connection::new(
            "ssm".into(),
            "i-0123456789abcdef0".into(),
            22,
            ProtocolConfig::ZeroTrust(ssm),
        );
        let ProbePlan::AwsSsm { args } = plan_probe(&conn, &ctx(&[], &[], &network), 5) else {
            panic!("expected aws");
        };
        assert_eq!(
            args.join(" "),
            "ssm describe-instance-information --filters Key=InstanceIds,Values=i-0123456789abcdef0 \
             --query InstanceInformationList[0].PingStatus --output text --profile lab --region eu-west-1"
        );
        let other = ZeroTrustConfig {
            provider: ZeroTrustProvider::GcpIap,
            provider_config: ZeroTrustProviderConfig::GcpIap(crate::models::GcpIapConfig::default()),
            custom_args: Vec::new(),
        };
        let conn = Connection::new(
            "gcp".into(),
            "vm".into(),
            22,
            ProtocolConfig::ZeroTrust(other),
        );
        assert!(matches!(
            plan_probe(&conn, &ctx(&[], &[], &network), 5),
            ProbePlan::Skip(_)
        ));
    }

    #[test]
    fn jump_probe_outcomes_are_told_apart() {
        assert_eq!(
            classify_jump_probe(Some(0), "Authenticated to b\n"),
            JumpOutcome::Reachable
        );
        assert_eq!(
            classify_jump_probe(
                Some(255),
                "Authenticated to b\nchannel 0: open failed: connect failed: Connection refused\n\
                 stdio forwarding failed\n"
            ),
            JumpOutcome::TargetFailed("connect failed: Connection refused".into())
        );
        assert!(matches!(
            classify_jump_probe(Some(255), "ubuntu@b: Permission denied (publickey).\n"),
            JumpOutcome::JumpNeedsInteraction(_)
        ));
        assert!(matches!(
            classify_jump_probe(
                Some(255),
                "No ED25519 host key is known for b and you have requested strict checking.\nHost key verification failed.\n"
            ),
            JumpOutcome::JumpNeedsInteraction(_)
        ));
        assert_eq!(
            classify_jump_probe(
                Some(255),
                "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
                 @    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n\
                 @@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
                 Offending ED25519 key in /home/u/.ssh/known_hosts:3\n\
                 Host key verification failed.\n"
            ),
            JumpOutcome::JumpFailed(
                "the jump host key has changed and does not match known_hosts".into()
            )
        );
        assert_eq!(
            classify_jump_probe(
                Some(255),
                "Authenticated to b\nchannel 0: open failed: administratively prohibited: open failed\n\
                 stdio forwarding failed\n"
            ),
            JumpOutcome::JumpFailed(
                "the jump host refuses forwarding (administratively prohibited: open failed)"
                    .into()
            )
        );
        assert_eq!(
            classify_jump_probe(
                Some(255),
                "ssh: connect to host b port 22: Connection refused\n"
            ),
            JumpOutcome::JumpFailed("ssh: connect to host b port 22: Connection refused".into())
        );
        assert_eq!(
            classify_jump_probe(
                None,
                "Authenticated to b ([1.2.3.4]:22) using \"publickey\".\n"
            ),
            JumpOutcome::TimedOut {
                authenticated: true
            }
        );
        assert_eq!(
            classify_jump_probe(None, ""),
            JumpOutcome::TimedOut {
                authenticated: false
            }
        );
    }
}

//! F-01: the Settings agent socket reaches every launch that goes through the
//! shared resolution in `rustconn_core::sftp`.
//!
//! The Settings socket is process-wide, so this file is its own test binary and
//! its one test runs the steps in order.

use std::ffi::OsStr;

use rustconn_core::sftp::{
    agent_env_vars, apply_agent_env, apply_agent_env_async, apply_agent_env_with_overrides,
    custom_agent_socket, default_agent_socket, resolve_ssh_agent_socket, set_global_agent_socket,
};
use rustconn_core::ssh_agent::SshAgentManager;

fn std_env(cmd: &std::process::Command, name: &str) -> Option<String> {
    cmd.get_envs()
        .find(|(k, _)| *k == OsStr::new(name))
        .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
}

#[test]
fn the_settings_socket_reaches_every_launch_and_a_connection_socket_beats_it() {
    // Nothing set: the launch inherits whatever the environment holds, so no
    // custom socket is reported.
    set_global_agent_socket(None);
    assert_eq!(custom_agent_socket(None), None);

    // Unset, empty and blank Settings values clear it.
    for cleared in [None, Some(""), Some("   ")] {
        set_global_agent_socket(Some("/run/rc-first.sock"));
        set_global_agent_socket(cleared);
        assert_eq!(custom_agent_socket(None), None, "{cleared:?}");
    }

    // A path typed with `~` and padding is stored expanded and trimmed.
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        set_global_agent_socket(Some("  ~/agent.sock "));
        assert_eq!(
            custom_agent_socket(None),
            Some(format!("{}/agent.sock", home.trim_end_matches('/')))
        );
    }

    // A relative path is kept as typed so the caller can report it.
    set_global_agent_socket(Some("rel/agent.sock"));
    assert_eq!(custom_agent_socket(None), Some("rel/agent.sock".to_owned()));

    let global = "/run/rc-settings.sock";
    set_global_agent_socket(Some(global));

    // std Command: terminal-less launches (file manager, mc, sftp, ssh-add).
    let mut cmd = std::process::Command::new("sftp");
    apply_agent_env(&mut cmd);
    assert_eq!(std_env(&cmd, "SSH_AUTH_SOCK").as_deref(), Some(global));

    // Connection socket first.
    let mut cmd = std::process::Command::new("ssh");
    apply_agent_env_with_overrides(&mut cmd, Some("/run/rc-conn.sock"), None);
    assert_eq!(
        std_env(&cmd, "SSH_AUTH_SOCK").as_deref(),
        Some("/run/rc-conn.sock")
    );

    // An empty connection socket falls through to Settings.
    let mut cmd = std::process::Command::new("ssh");
    apply_agent_env_with_overrides(&mut cmd, Some(""), None);
    assert_eq!(std_env(&cmd, "SSH_AUTH_SOCK").as_deref(), Some(global));

    // tokio Command: tunnels, monitoring, remote edit.
    let mut cmd = tokio::process::Command::new("ssh");
    apply_agent_env_async(&mut cmd, None);
    assert_eq!(
        std_env(cmd.as_std(), "SSH_AUTH_SOCK").as_deref(),
        Some(global)
    );
    let mut cmd = tokio::process::Command::new("ssh");
    apply_agent_env_async(&mut cmd, Some("/run/rc-conn.sock"));
    assert_eq!(
        std_env(cmd.as_std(), "SSH_AUTH_SOCK").as_deref(),
        Some("/run/rc-conn.sock")
    );

    // VTE environment vector source (terminal SSH, mc, telnet tabs).
    assert!(agent_env_vars(None).contains(&("SSH_AUTH_SOCK", global.to_owned())));

    // ssh-add and the agent status page ask the same function.
    assert_eq!(default_agent_socket().as_deref(), Some(global));
    assert_eq!(
        resolve_ssh_agent_socket(None, None).as_deref(),
        Some(global)
    );
    assert_eq!(
        SshAgentManager::from_env().socket_path(),
        Some(global),
        "the agent page and ssh-add use the Settings socket"
    );

    // Clearing the setting returns every launch to the environment.
    set_global_agent_socket(None);
    assert_ne!(default_agent_socket().as_deref(), Some(global));
}

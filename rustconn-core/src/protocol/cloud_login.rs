//! Browser sign-in offered when a Zero Trust CLI reports expired credentials.
//!
//! A cloud CLI whose cached token has run out fails the session with a message
//! such as `aws: [ERROR]: Your session has expired. Please reauthenticate using
//! 'aws login'.` Reconnecting cannot help until the user signs in again, so the
//! GUI offers the provider's own login command next to "Reconnect". This module
//! is the headless half: it recognises the message in the session's last output
//! lines and builds the login command from the connection's configuration.

use crate::models::{ZeroTrustConfig, ZeroTrustProviderConfig};
use crate::ssh_tunnel::format_argv_for_display;

/// Number of trailing terminal lines inspected for an expiry message.
///
/// The CLI prints the error as its last words before exiting, so a short tail
/// is enough, and it keeps a hint printed at the start of a long session from
/// producing a login button hours later.
const TAIL_LINES: usize = 15;

/// AWS CLI messages for an expired SSO, `aws login` or STS session.
const AWS_MARKERS: &[&str] = &[
    "session has expired",
    "token has expired",
    "token is expired",
    "expiredtoken",
    "security token included in the request is expired",
    "error loading sso token",
    "reauthenticate",
    "aws sso login",
    "aws login",
];

/// AWS markers that mean the profile is an IAM Identity Center (SSO) profile.
const AWS_SSO_MARKERS: &[&str] = &["aws sso login", "sso token", "sso session", "from sso"];

/// gcloud messages for a refresh token that is no longer accepted.
const GCLOUD_MARKERS: &[&str] = &[
    "reauthentication required",
    "reauthentication failed",
    "problem refreshing your current auth tokens",
    "gcloud auth login",
    "invalid_grant",
];

/// Azure CLI messages; the `AADSTS` codes are Entra ID token-expiry errors.
const AZURE_MARKERS: &[&str] = &[
    "az login",
    "aadsts700082",
    "aadsts70043",
    "aadsts50173",
    "aadsts50078",
    "aadsts50076",
    "refresh token has expired",
    "interactive authentication is needed",
];

/// OCI CLI messages for an expired session token profile.
const OCI_MARKERS: &[&str] = &["oci session authenticate", "oci session refresh"];

/// cloudflared messages for an expired Access application token.
const CLOUDFLARE_MARKERS: &[&str] = &["cloudflared access login", "token has expired"];

/// tsh messages for an expired user certificate.
const TELEPORT_MARKERS: &[&str] = &[
    "tsh login",
    "cert has expired",
    "certificate has expired",
    "credentials have expired",
    "not logged in",
];

/// Boundary messages for an expired or missing auth token.
const BOUNDARY_MARKERS: &[&str] = &[
    "boundary authenticate",
    "token is expired",
    "unauthenticated",
];

/// hoop messages for an expired gateway token.
const HOOP_MARKERS: &[&str] = &["hoop login", "token expired", "token is expired"];

/// A provider login command that renews the credentials in a browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudLogin {
    provider_name: &'static str,
    program: &'static str,
    args: Vec<String>,
}

impl CloudLogin {
    /// Returns the short, untranslated provider name, e.g. `"AWS"`.
    #[must_use]
    pub const fn provider_name(&self) -> &'static str {
        self.provider_name
    }

    /// Returns the CLI program, e.g. `"aws"`.
    #[must_use]
    pub const fn program(&self) -> &'static str {
        self.program
    }

    /// Returns the arguments passed to [`Self::program`].
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Returns the command line for display, e.g. `"aws sso login --profile dev"`.
    ///
    /// Arguments a shell would split or interpret are single-quoted, so the
    /// echoed line shows the argv that actually runs.
    #[must_use]
    pub fn command_line(&self) -> String {
        let argv: Vec<String> = std::iter::once(self.program.to_string())
            .chain(self.args.iter().cloned())
            .collect();
        format_argv_for_display(&argv)
    }

    /// Returns an argv that runs the login through `/bin/sh` without shell parsing.
    ///
    /// The shell only resolves the program on the child's `PATH` (which carries
    /// the sandboxed CLI directories); the program and every argument are passed
    /// as positional parameters, so a profile or cluster name taken from an
    /// imported connection can never be interpreted as shell syntax.
    #[must_use]
    pub fn spawn_argv(&self) -> Vec<String> {
        let mut argv = vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "exec \"$0\" \"$@\"".to_string(),
            self.program.to_string(),
        ];
        argv.extend(self.args.iter().cloned());
        argv
    }
}

/// Returns the login command when the output shows the provider's credentials expired.
///
/// Only the last few lines of `output` are inspected. Returns `None` when no
/// expiry message is recognised, and for providers that have no separate login
/// step: Tailscale SSH runs its own browser check, and a Generic command's CLI
/// is unknown.
#[must_use]
pub fn expired_credentials_login(config: &ZeroTrustConfig, output: &str) -> Option<CloudLogin> {
    let tail = output_tail(output);
    let seen = |markers: &[&str]| markers.iter().any(|m| tail.contains(m));

    match &config.provider_config {
        ZeroTrustProviderConfig::AwsSsm(cfg) => {
            if !seen(AWS_MARKERS) {
                return None;
            }
            let mut args = if seen(AWS_SSO_MARKERS) {
                vec!["sso".to_string(), "login".to_string()]
            } else {
                vec!["login".to_string()]
            };
            let profile = match cfg.profile.trim() {
                "" | "default" => custom_profile(&config.custom_args),
                profile => Some(profile),
            };
            if let Some(profile) = profile {
                args.push("--profile".to_string());
                args.push(profile.to_string());
            }
            Some(login("AWS", "aws", args))
        }
        ZeroTrustProviderConfig::GcpIap(_) => seen(GCLOUD_MARKERS).then(|| {
            login(
                "Google Cloud",
                "gcloud",
                vec!["auth".into(), "login".into()],
            )
        }),
        ZeroTrustProviderConfig::AzureBastion(_) | ZeroTrustProviderConfig::AzureSsh(_) => {
            seen(AZURE_MARKERS).then(|| login("Azure", "az", vec!["login".into()]))
        }
        ZeroTrustProviderConfig::OciBastion(_) => seen(OCI_MARKERS)
            .then(|| login("OCI", "oci", vec!["session".into(), "authenticate".into()])),
        ZeroTrustProviderConfig::CloudflareAccess(cfg) => seen(CLOUDFLARE_MARKERS).then(|| {
            let url = if cfg.hostname.contains("://") {
                cfg.hostname.clone()
            } else {
                format!("https://{}", cfg.hostname)
            };
            login(
                "Cloudflare",
                "cloudflared",
                vec!["access".into(), "login".into(), url],
            )
        }),
        ZeroTrustProviderConfig::Teleport(cfg) => seen(TELEPORT_MARKERS).then(|| {
            let mut args = vec!["login".to_string()];
            // `tsh login [<cluster>]`: the proxy comes from the current tsh profile.
            // A value starting with `-` would be read as a tsh flag, not a cluster.
            if let Some(cluster) = cfg
                .cluster
                .as_deref()
                .filter(|c| !c.is_empty() && !c.starts_with('-'))
            {
                args.push(cluster.to_string());
            }
            login("Teleport", "tsh", args)
        }),
        ZeroTrustProviderConfig::Boundary(cfg) => seen(BOUNDARY_MARKERS).then(|| {
            let mut args = vec!["authenticate".to_string()];
            if let Some(addr) = cfg.addr.as_deref().filter(|a| !a.is_empty()) {
                args.push("-addr".to_string());
                args.push(addr.to_string());
            }
            login("Boundary", "boundary", args)
        }),
        ZeroTrustProviderConfig::HoopDev(_) => {
            seen(HOOP_MARKERS).then(|| login("Hoop.dev", "hoop", vec!["login".into()]))
        }
        // ponytail: a Generic command may wrap any CLI, and under Flatpak it runs
        // on the host, where a login would have to run too. Infer the CLI from
        // the template if users ask for it.
        ZeroTrustProviderConfig::TailscaleSsh(_) | ZeroTrustProviderConfig::Generic(_) => None,
    }
}

fn login(provider_name: &'static str, program: &'static str, args: Vec<String>) -> CloudLogin {
    CloudLogin {
        provider_name,
        program,
        args,
    }
}

/// Lowercased last [`TAIL_LINES`] non-empty lines of `output`.
pub(super) fn output_tail(output: &str) -> String {
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    lines[start..].join("\n").to_lowercase()
}

/// The `--profile` value among the connection's custom arguments, if any.
fn custom_profile(custom_args: &[String]) -> Option<&str> {
    let mut iter = custom_args.iter();
    while let Some(arg) = iter.next() {
        if let Some(value) = arg.strip_prefix("--profile=") {
            return Some(value).filter(|v| !v.is_empty());
        }
        if arg == "--profile" {
            return iter.next().map(String::as_str).filter(|v| !v.is_empty());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        AwsSsmConfig, BoundaryConfig, CloudflareAccessConfig, GcpIapConfig, GenericZeroTrustConfig,
        TailscaleSshConfig, TeleportConfig, ZeroTrustProvider,
    };

    fn config(
        provider: ZeroTrustProvider,
        provider_config: ZeroTrustProviderConfig,
    ) -> ZeroTrustConfig {
        ZeroTrustConfig {
            provider,
            provider_config,
            custom_args: Vec::new(),
        }
    }

    fn aws(profile: &str) -> ZeroTrustConfig {
        config(
            ZeroTrustProvider::AwsSsm,
            ZeroTrustProviderConfig::AwsSsm(AwsSsmConfig {
                target: "i-0b5e04edcf7719d27".into(),
                profile: profile.into(),
                region: Some("eu-west-1".into()),
            }),
        )
    }

    const AWS_LOGIN_EXPIRED: &str = "🔗 Connecting via AWS Session Manager to i-0b5e04edcf7719d27...\n\
        ⚡ Executing: aws ssm start-session --target i-0b5e04edcf7719d27 --region eu-west-1\n\n\n\
        aws: [ERROR]: Your session has expired. Please reauthenticate using 'aws login'.\n";

    #[test]
    fn aws_login_session_expired_offers_aws_login() {
        let login = expired_credentials_login(&aws("default"), AWS_LOGIN_EXPIRED).unwrap();
        assert_eq!(login.provider_name(), "AWS");
        assert_eq!(login.command_line(), "aws login");
    }

    #[test]
    fn aws_sso_token_expired_offers_sso_login_with_profile() {
        let output = "Error when retrieving token from sso: Token has expired and refresh failed";
        let login = expired_credentials_login(&aws("dev"), output).unwrap();
        assert_eq!(login.command_line(), "aws sso login --profile dev");
    }

    #[test]
    fn aws_profile_is_taken_from_custom_args() {
        let mut cfg = aws("default");
        cfg.custom_args = vec!["--profile".into(), "prod".into()];
        let login = expired_credentials_login(&cfg, AWS_LOGIN_EXPIRED).unwrap();
        assert_eq!(login.command_line(), "aws login --profile prod");

        cfg.custom_args = vec!["--profile=stage".into()];
        let login = expired_credentials_login(&cfg, AWS_LOGIN_EXPIRED).unwrap();
        assert_eq!(login.command_line(), "aws login --profile stage");
    }

    #[test]
    fn unrelated_failure_offers_nothing() {
        let output =
            "An error occurred (TargetNotConnected) when calling the StartSession operation";
        assert_eq!(expired_credentials_login(&aws("default"), output), None);
    }

    #[test]
    fn marker_outside_tail_is_ignored() {
        let filler: Vec<String> = (0..TAIL_LINES).map(|i| format!("line {i}")).collect();
        let output = format!("hint: run aws login if needed\n{}\n", filler.join("\n"));
        assert_eq!(expired_credentials_login(&aws("default"), &output), None);
    }

    #[test]
    fn gcloud_reauth_offers_auth_login() {
        let cfg = config(
            ZeroTrustProvider::GcpIap,
            ZeroTrustProviderConfig::GcpIap(GcpIapConfig::default()),
        );
        let output = "ERROR: (gcloud.compute.ssh) There was a problem refreshing your current \
                      auth tokens: Reauthentication required.";
        let login = expired_credentials_login(&cfg, output).unwrap();
        assert_eq!(login.command_line(), "gcloud auth login");
    }

    #[test]
    fn azure_expired_refresh_token_offers_az_login() {
        let cfg = config(
            ZeroTrustProvider::AzureSsh,
            ZeroTrustProviderConfig::AzureSsh(crate::models::AzureSshConfig::default()),
        );
        let output = "AADSTS700082: The refresh token has expired due to inactivity.\n\
                      Please run 'az login' to setup account.";
        let login = expired_credentials_login(&cfg, output).unwrap();
        assert_eq!(login.command_line(), "az login");
    }

    #[test]
    fn teleport_login_includes_cluster() {
        let cfg = config(
            ZeroTrustProvider::Teleport,
            ZeroTrustProviderConfig::Teleport(TeleportConfig {
                host: "node".into(),
                username: None,
                cluster: Some("leaf".into()),
            }),
        );
        let output = "ERROR: ssh: cert has expired";
        let login = expired_credentials_login(&cfg, output).unwrap();
        assert_eq!(login.command_line(), "tsh login leaf");
    }

    #[test]
    fn boundary_login_includes_addr() {
        let cfg = config(
            ZeroTrustProvider::Boundary,
            ZeroTrustProviderConfig::Boundary(BoundaryConfig {
                target: "ttcp_1".into(),
                addr: Some("https://boundary.example.com".into()),
            }),
        );
        let output = "Error from controller when performing authorize-session: Unauthenticated";
        let login = expired_credentials_login(&cfg, output).unwrap();
        assert_eq!(
            login.command_line(),
            "boundary authenticate -addr https://boundary.example.com"
        );
    }

    #[test]
    fn cloudflare_login_targets_application_url() {
        let cfg = config(
            ZeroTrustProvider::CloudflareAccess,
            ZeroTrustProviderConfig::CloudflareAccess(CloudflareAccessConfig {
                hostname: "ssh.example.com".into(),
                username: None,
            }),
        );
        let login = expired_credentials_login(&cfg, "token has expired").unwrap();
        assert_eq!(
            login.command_line(),
            "cloudflared access login https://ssh.example.com"
        );
    }

    #[test]
    fn providers_without_login_step_offer_nothing() {
        let tailscale = config(
            ZeroTrustProvider::TailscaleSsh,
            ZeroTrustProviderConfig::TailscaleSsh(TailscaleSshConfig::default()),
        );
        let generic = config(
            ZeroTrustProvider::Generic,
            ZeroTrustProviderConfig::Generic(GenericZeroTrustConfig {
                command_template: "aws ssm start-session --target i-1".into(),
            }),
        );
        assert_eq!(
            expired_credentials_login(&tailscale, AWS_LOGIN_EXPIRED),
            None
        );
        assert_eq!(expired_credentials_login(&generic, AWS_LOGIN_EXPIRED), None);
    }

    #[test]
    fn an_empty_aws_profile_is_not_passed_as_an_empty_value() {
        let login = expired_credentials_login(&aws(""), AWS_LOGIN_EXPIRED).unwrap();
        assert_eq!(login.command_line(), "aws login");
        assert!(!login.args().iter().any(|a| a == "--profile"));
    }

    #[test]
    fn oci_expired_session_offers_session_authenticate() {
        let cfg = config(
            ZeroTrustProvider::OciBastion,
            ZeroTrustProviderConfig::OciBastion(crate::models::OciBastionConfig::default()),
        );
        let output = "ERROR: This CLI session has expired, so it cannot currently be used to \
                      run commands. Refresh with: oci session refresh";
        let login = expired_credentials_login(&cfg, output).unwrap();
        assert_eq!(login.command_line(), "oci session authenticate");
    }

    #[test]
    fn hoop_expired_token_offers_hoop_login() {
        let cfg = config(
            ZeroTrustProvider::HoopDev,
            ZeroTrustProviderConfig::HoopDev(crate::models::HoopDevConfig {
                connection_name: "db".into(),
                gateway_url: None,
                grpc_url: None,
            }),
        );
        let login =
            expired_credentials_login(&cfg, "error: token expired, run hoop login").unwrap();
        assert_eq!(login.command_line(), "hoop login");
    }

    #[test]
    fn a_teleport_cluster_that_looks_like_a_flag_is_left_out() {
        let cfg = config(
            ZeroTrustProvider::Teleport,
            ZeroTrustProviderConfig::Teleport(TeleportConfig {
                host: "node".into(),
                username: None,
                cluster: Some("--proxy=evil.example.com".into()),
            }),
        );
        let login = expired_credentials_login(&cfg, "ERROR: ssh: cert has expired").unwrap();
        assert_eq!(login.command_line(), "tsh login");
    }

    #[test]
    fn the_displayed_command_line_quotes_what_a_shell_would_split() {
        let login = expired_credentials_login(&aws("x; rm -rf ~"), AWS_LOGIN_EXPIRED).unwrap();
        assert_eq!(login.command_line(), "aws login --profile 'x; rm -rf ~'");
    }

    #[test]
    fn spawn_argv_passes_values_as_positional_parameters() {
        let login = expired_credentials_login(&aws("x; rm -rf ~"), AWS_LOGIN_EXPIRED).unwrap();
        assert_eq!(
            login.spawn_argv(),
            vec![
                "/bin/sh",
                "-c",
                "exec \"$0\" \"$@\"",
                "aws",
                "login",
                "--profile",
                "x; rm -rf ~",
            ]
        );
    }
}

//! Unit tests for `ssh_key_setup`: everything here runs without a network.
//! Tests that need `ssh-keygen` skip themselves when it is not installed.

#![cfg(unix)]

use super::*;
use proptest::prelude::*;
use std::os::unix::fs::PermissionsExt;

const PROBE: &str = "hunter2-leak-probe";

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn have(binary: &str) -> bool {
    crate::which::find_in_path(binary).is_some()
}

fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// True when file modes do not bind this process (running as root).
fn modes_are_ignored() -> bool {
    let dir = tempfile::tempdir().unwrap();
    let locked = dir.path().join("locked");
    fs::create_dir(&locked).unwrap();
    set_mode(&locked, 0o500);
    let writable = fs::File::create(locked.join("probe")).is_ok();
    set_mode(&locked, 0o700);
    writable
}

fn fake_blob(key_type: &str, body_len: usize) -> Vec<u8> {
    let mut blob = u32::try_from(key_type.len())
        .unwrap()
        .to_be_bytes()
        .to_vec();
    blob.extend(key_type.as_bytes());
    blob.extend(vec![7u8; body_len]);
    blob
}

fn fake_key(algo: KeyAlgorithm, comment: &str) -> String {
    let body = data_encoding::BASE64.encode(&fake_blob(algo.ssh_type(), 32));
    format!("{} {body} {comment}", algo.ssh_type())
}

fn parsed_key(comment: &str) -> PublicKeyLine {
    PublicKeyLine::parse(&fake_key(KeyAlgorithm::Ed25519, comment)).unwrap()
}

fn id() -> Uuid {
    Uuid::parse_str("5fb5186b-da31-4901-94a0-2187851c87a9").unwrap()
}

fn target() -> SshTarget {
    SshTarget {
        host: "example.com".to_owned(),
        port: 2222,
        username: Some("user".to_owned()),
        session_args: Vec::new(),
        known_hosts_file: Some(PathBuf::from("/kh")),
    }
}

fn keygen(path: &Path) {
    let status = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "test", "-f"])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

/// Values of every `-o` in `args` (`-o K=V` and `-oK=V`).
fn option_values(args: &[String]) -> Vec<String> {
    let mut values = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-o" {
            if let Some(value) = args.get(i + 1) {
                values.push(value.clone());
            }
            i += 2;
        } else {
            if let Some(rest) = args[i].strip_prefix("-o").filter(|r| !r.is_empty()) {
                values.push(rest.to_owned());
            }
            i += 1;
        }
    }
    values
}

/// Every value given to option `key`, case-insensitively.
fn values_of(args: &[String], key: &str) -> Vec<String> {
    option_values(args)
        .into_iter()
        .filter(|v| option_key(v).eq_ignore_ascii_case(key))
        .collect()
}

fn expected_prefix() -> Vec<String> {
    OVERRIDE_OPTIONS
        .iter()
        .flat_map(|o| ["-o".to_owned(), (*o).to_owned()])
        .collect()
}

fn invalid_key(text: &str) -> Option<PublicKeyError> {
    match PublicKeyLine::parse(text) {
        Err(KeySetupError::InvalidPublicKey(reason)) => Some(reason),
        _ => None,
    }
}

// ---- A. naming, directory, public-key line ---------------------------------

#[test]
fn slug_lowercases_and_collapses_separators() {
    assert_eq!(connection_slug("Prod DB #1"), "prod-db-1");
    assert_eq!(connection_slug("  --a__b  "), "a-b");
}

#[test]
fn slug_drops_non_ascii() {
    assert_eq!(connection_slug("Київ сервер"), "connection");
    assert_eq!(connection_slug("Web Київ 2"), "web-2");
}

#[test]
fn slug_is_capped_at_32_without_trailing_dash() {
    let long = connection_slug(&"a".repeat(80));
    assert_eq!(long.len(), SLUG_MAX_LEN);
    // The cut lands on a separator: it must not leave a dangling dash.
    let cut = connection_slug(&format!("{} tail", "a".repeat(31)));
    assert_eq!(cut, "a".repeat(31));
}

#[test]
fn slug_empty_and_punctuation_only_fall_back() {
    assert_eq!(connection_slug(""), "connection");
    assert_eq!(connection_slug("!!! ---"), "connection");
}

#[test]
fn stem_has_slug_id8_and_type() {
    assert_eq!(
        key_file_stem("Prod DB #1 (Київ)", id(), KeyAlgorithm::Ed25519),
        "prod-db-1-5fb5186b_ed25519"
    );
    assert_eq!(
        key_file_stem("x", id(), KeyAlgorithm::Rsa4096),
        "x-5fb5186b_rsa"
    );
}

#[test]
fn comment_has_no_connection_name_and_iso_date() {
    let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
    assert_eq!(
        key_comment("my-laptop", date),
        "rustconn@my-laptop 2026-10-27"
    );
    assert_eq!(key_comment("a b;c", date), "rustconn@a-b-c 2026-10-27");
    assert_eq!(key_comment("", date), "rustconn@localhost 2026-10-27");
    assert_eq!(key_comment("///", date), "rustconn@localhost 2026-10-27");
    let long = key_comment(&"h".repeat(200), date);
    assert_eq!(long.len(), "rustconn@".len() + 64 + " 2026-10-27".len());
    assert!(!default_key_comment().contains(['\n', '\'', '"']));
}

#[test]
fn key_setup_dir_native_is_ssh_dir_rustconn() {
    assert_eq!(
        key_setup_dir(Path::new("/home/u/.ssh"), None),
        PathBuf::from("/home/u/.ssh/rustconn")
    );
}

#[test]
fn key_setup_dir_snap_uses_current_not_revision() {
    assert_eq!(
        key_setup_dir(
            Path::new("/ignored"),
            Some(Path::new("/home/u/snap/rustconn/123"))
        ),
        PathBuf::from("/home/u/snap/rustconn/current/.ssh/rustconn")
    );
}

#[test]
fn ensure_key_setup_dir_creates_0700_and_tightens_existing() {
    let tmp = tempfile::tempdir().unwrap();
    let fresh = tmp.path().join("a").join("rustconn");
    ensure_key_setup_dir(&fresh).unwrap();
    assert_eq!(mode(&fresh), 0o700);
    assert_eq!(mode(fresh.parent().unwrap()), 0o700);

    let loose = tmp.path().join("loose");
    fs::create_dir(&loose).unwrap();
    set_mode(&loose, 0o755);
    ensure_key_setup_dir(&loose).unwrap();
    assert_eq!(mode(&loose), 0o700);
    // The write probe leaves nothing behind.
    assert_eq!(fs::read_dir(&loose).unwrap().count(), 0);
}

#[test]
fn ensure_key_setup_dir_rejects_symlink_and_file() {
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("real");
    fs::create_dir(&real).unwrap();
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(matches!(
        ensure_key_setup_dir(&link),
        Err(KeySetupError::KeyDir { .. })
    ));
    let file = tmp.path().join("file");
    fs::write(&file, b"x").unwrap();
    assert!(matches!(
        ensure_key_setup_dir(&file),
        Err(KeySetupError::KeyDir { .. })
    ));
}

#[test]
fn ensure_key_setup_dir_reports_read_only_parent() {
    if modes_are_ignored() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().join("ro");
    fs::create_dir(&parent).unwrap();
    set_mode(&parent, 0o500);
    let result = ensure_key_setup_dir(&parent.join("rustconn"));
    set_mode(&parent, 0o700);
    assert!(matches!(result, Err(KeySetupError::KeyDir { .. })));
}

#[test]
fn resolve_slot_fresh_when_nothing_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let slot = resolve_key_slot(tmp.path(), "Prod", id(), KeyAlgorithm::Ed25519).unwrap();
    assert_eq!(
        slot,
        KeySlot::Fresh(tmp.path().join("prod-5fb5186b_ed25519"))
    );
}

#[test]
fn resolve_slot_suffixes_when_only_one_file_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let stem = tmp.path().join("prod-5fb5186b_ed25519");
    fs::write(public_path(&stem), b"x").unwrap();
    let slot = resolve_key_slot(tmp.path(), "Prod", id(), KeyAlgorithm::Ed25519).unwrap();
    assert_eq!(
        slot,
        KeySlot::Fresh(tmp.path().join("prod-5fb5186b_ed25519-2"))
    );

    fs::write(tmp.path().join("prod-5fb5186b_ed25519-2"), b"x").unwrap();
    let slot = resolve_key_slot(tmp.path(), "Prod", id(), KeyAlgorithm::Ed25519).unwrap();
    assert_eq!(
        slot,
        KeySlot::Fresh(tmp.path().join("prod-5fb5186b_ed25519-3"))
    );
}

#[test]
fn resolve_slot_reports_a_directory_with_no_free_name() {
    let tmp = tempfile::tempdir().unwrap();
    let base = key_file_stem("p", id(), KeyAlgorithm::Ed25519);
    fs::write(tmp.path().join(&base), b"x").unwrap();
    for n in 2..=MAX_SLOT_ATTEMPTS {
        fs::write(tmp.path().join(format!("{base}-{n}")), b"x").unwrap();
    }
    assert!(matches!(
        resolve_key_slot(tmp.path(), "p", id(), KeyAlgorithm::Ed25519),
        Err(KeySetupError::NoFreeKeyName { .. })
    ));
}

#[test]
fn resolve_slot_reuses_a_consistent_pair() {
    if !have("ssh-keygen") {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let private = tmp.path().join("prod-5fb5186b_ed25519");
    keygen(&private);
    let slot = resolve_key_slot(tmp.path(), "Prod", id(), KeyAlgorithm::Ed25519).unwrap();
    let (private_key_path, public_key) = match slot {
        KeySlot::Reusable {
            private_key_path,
            public_key,
        } => (private_key_path, public_key),
        other @ KeySlot::Fresh(_) => panic!("a consistent pair must be reused, got {other:?}"),
    };
    assert_eq!(private_key_path, private);
    assert_eq!(public_key.algorithm(), KeyAlgorithm::Ed25519);
}

#[test]
fn resolve_slot_suffixes_on_mismatched_pair() {
    if !have("ssh-keygen") {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let private = tmp.path().join("prod-5fb5186b_ed25519");
    let other = tmp.path().join("other");
    keygen(&private);
    keygen(&other);
    fs::copy(public_path(&other), public_path(&private)).unwrap();
    let slot = resolve_key_slot(tmp.path(), "Prod", id(), KeyAlgorithm::Ed25519).unwrap();
    assert_eq!(
        slot,
        KeySlot::Fresh(tmp.path().join("prod-5fb5186b_ed25519-2"))
    );
}

#[test]
fn public_key_parse_accepts_ed25519_and_rsa() {
    for algo in [KeyAlgorithm::Ed25519, KeyAlgorithm::Rsa4096] {
        let text = fake_key(algo, "rustconn@host 2026-10-27");
        let key = PublicKeyLine::parse(&format!("{text}\n")).unwrap();
        assert_eq!(key.as_str(), text);
        assert_eq!(key.algorithm(), algo);
        assert_eq!(
            key.fingerprint(),
            PublicKeyInfo::from_pub_line(&text).unwrap().fingerprint
        );
    }
    // A key without a comment is valid.
    let bare = fake_key(KeyAlgorithm::Ed25519, "");
    assert!(PublicKeyLine::parse(bare.trim_end()).is_ok());
}

#[test]
fn public_key_fingerprint_matches_ssh_keygen() {
    if !have("ssh-keygen") {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let private = tmp.path().join("k");
    keygen(&private);
    let key = PublicKeyLine::read(&public_path(&private)).unwrap();
    let listing = Command::new("ssh-keygen")
        .arg("-lf")
        .arg(public_path(&private))
        .output()
        .unwrap();
    let listing = String::from_utf8_lossy(&listing.stdout).into_owned();
    assert!(
        listing.split_whitespace().any(|w| w == key.fingerprint()),
        "{listing:?} vs {}",
        key.fingerprint()
    );
}

#[test]
fn public_key_parse_rejects() {
    let ed = fake_key(KeyAlgorithm::Ed25519, "c");
    let body = ed.split(' ').nth(1).unwrap().to_owned();
    let table: Vec<(String, PublicKeyError)> = vec![
        (
            format!("command=\"x\" {ed}"),
            PublicKeyError::UnsupportedType,
        ),
        (format!("{ed}\n{ed}"), PublicKeyError::MultipleLines),
        (format!("{ed}\r\n"), PublicKeyError::ControlCharacter),
        (
            fake_key(KeyAlgorithm::Ed25519, "a\0b"),
            PublicKeyError::ControlCharacter,
        ),
        (
            fake_key(KeyAlgorithm::Ed25519, "a\tb"),
            PublicKeyError::ControlCharacter,
        ),
        (
            fake_key(KeyAlgorithm::Ed25519, "Київ"),
            PublicKeyError::BadComment,
        ),
        (
            format!("ecdsa-sha2-nistp256 {body} c"),
            PublicKeyError::UnsupportedType,
        ),
        ("ssh-ed25519 !!!! c".to_owned(), PublicKeyError::BadBase64),
        (format!("ssh-rsa {body} c"), PublicKeyError::TypeMismatch),
        (
            format!("ssh-ed25519 {}", "A".repeat(MAX_PUBLIC_KEY_LEN + 1)),
            PublicKeyError::TooLong,
        ),
        (String::new(), PublicKeyError::MissingBody),
        ("ssh-ed25519".to_owned(), PublicKeyError::MissingBody),
        ("ssh-ed25519 ".to_owned(), PublicKeyError::MissingBody),
    ];
    for (text, expected) in table {
        assert_eq!(invalid_key(&text), Some(expected), "for {text:?}");
    }
}

#[test]
fn public_key_comment_may_hold_quotes_and_shell_characters() {
    // Printable ASCII is allowed: the line never reaches a shell, it goes in on stdin.
    let text = fake_key(KeyAlgorithm::Ed25519, "it's \"x\" $(y) `z` ; !");
    assert!(PublicKeyLine::parse(&text).is_ok());
}

#[test]
fn delete_generated_key_refuses_foreign_names() {
    let tmp = tempfile::tempdir().unwrap();
    let ssh = tmp.path().join(".ssh");
    let dir = tmp.path().join(KEY_SETUP_SUBDIR);
    fs::create_dir_all(&ssh).unwrap();
    fs::create_dir_all(&dir).unwrap();

    let foreign = ssh.join("id_ed25519");
    fs::write(&foreign, b"mine").unwrap();
    let lookalike_in_wrong_dir = ssh.join("x-12345678_ed25519");
    fs::write(&lookalike_in_wrong_dir, b"mine").unwrap();
    let wrong_name_in_right_dir = dir.join("id_ed25519");
    fs::write(&wrong_name_in_right_dir, b"mine").unwrap();
    for path in [&foreign, &lookalike_in_wrong_dir, &wrong_name_in_right_dir] {
        assert!(matches!(
            delete_generated_key(path),
            Err(KeySetupError::RefusedToDelete { .. })
        ));
        assert_eq!(fs::read(path).unwrap(), b"mine");
    }

    let ours = dir.join("x-12345678_ed25519");
    fs::write(&ours, b"k").unwrap();
    fs::write(public_path(&ours), b"p").unwrap();
    delete_generated_key(&ours).unwrap();
    assert!(!ours.exists() && !public_path(&ours).exists());

    // The `.pub` may already be gone, and so may both; a `-2` suffix is ours too.
    let retry = dir.join("x-12345678_rsa-2");
    fs::write(&retry, b"k").unwrap();
    delete_generated_key(&retry).unwrap();
    delete_generated_key(&retry).unwrap();
    assert!(!retry.exists());
}

// ---- B. DEPLOY_SCRIPT, run locally -----------------------------------------

fn run_sh(home: &Path, script: &str, stdin: &str) -> std::process::Output {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .current_dir(home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn deploy_locally(home: &Path, key: &str) -> std::process::Output {
    run_sh(home, DEPLOY_SCRIPT, &format!("{key}\n"))
}

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[test]
fn deploy_script_is_single_line_without_quote_or_bang() {
    assert!(!DEPLOY_SCRIPT.contains(['\'', '\n', '!']));
    assert!(DEPLOY_SCRIPT.contains(SENTINEL_INSTALLED));
    assert!(DEPLOY_SCRIPT.contains(SENTINEL_PRESENT));
    assert_eq!(
        deploy_remote_command(),
        format!("exec sh -c '{DEPLOY_SCRIPT}'")
    );
}

#[test]
fn deploy_script_creates_dir_0700_and_file_0600() {
    let home = tempfile::tempdir().unwrap();
    let key = fake_key(KeyAlgorithm::Ed25519, "c");
    let out = deploy_locally(home.path(), &key);
    assert!(out.status.success());
    assert_eq!(stdout_of(&out), SENTINEL_INSTALLED);
    let ssh = home.path().join(".ssh");
    assert_eq!(mode(&ssh), 0o700);
    assert_eq!(mode(&ssh.join("authorized_keys")), 0o600);
    assert_eq!(
        fs::read_to_string(ssh.join("authorized_keys")).unwrap(),
        format!("{key}\n")
    );
}

#[test]
fn deploy_script_second_run_reports_present_without_duplicate() {
    let home = tempfile::tempdir().unwrap();
    let key = fake_key(KeyAlgorithm::Ed25519, "c");
    deploy_locally(home.path(), &key);
    let again = deploy_locally(home.path(), &key);
    assert!(again.status.success());
    assert_eq!(stdout_of(&again), SENTINEL_PRESENT);
    let content = fs::read_to_string(home.path().join(".ssh/authorized_keys")).unwrap();
    assert_eq!(content, format!("{key}\n"));
}

#[test]
fn deploy_script_appends_newline_to_file_without_one() {
    let home = tempfile::tempdir().unwrap();
    let ssh = home.path().join(".ssh");
    fs::create_dir(&ssh).unwrap();
    fs::write(ssh.join("authorized_keys"), "other-key").unwrap();
    let key = fake_key(KeyAlgorithm::Ed25519, "c");
    let out = deploy_locally(home.path(), &key);
    assert_eq!(stdout_of(&out), SENTINEL_INSTALLED);
    assert_eq!(
        fs::read_to_string(ssh.join("authorized_keys")).unwrap(),
        format!("other-key\n{key}\n")
    );
}

#[test]
fn deploy_script_keeps_existing_lines_and_modes() {
    let home = tempfile::tempdir().unwrap();
    let ssh = home.path().join(".ssh");
    fs::create_dir(&ssh).unwrap();
    set_mode(&ssh, 0o755);
    let file = ssh.join("authorized_keys");
    fs::write(&file, "first\nsecond\n").unwrap();
    set_mode(&file, 0o644);
    let key = fake_key(KeyAlgorithm::Ed25519, "c");
    assert_eq!(
        stdout_of(&deploy_locally(home.path(), &key)),
        SENTINEL_INSTALLED
    );
    assert_eq!(mode(&ssh), 0o755);
    assert_eq!(mode(&file), 0o644);
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        format!("first\nsecond\n{key}\n")
    );
}

#[test]
fn deploy_script_exit_codes() {
    if modes_are_ignored() {
        return;
    }
    let key = fake_key(KeyAlgorithm::Ed25519, "c");

    let home = tempfile::tempdir().unwrap();
    set_mode(home.path(), 0o500);
    let code = deploy_locally(home.path(), &key).status.code();
    set_mode(home.path(), 0o700);
    assert_eq!(code, Some(10));

    let home = tempfile::tempdir().unwrap();
    let ssh = home.path().join(".ssh");
    fs::create_dir(&ssh).unwrap();
    set_mode(&ssh, 0o500);
    let code = deploy_locally(home.path(), &key).status.code();
    set_mode(&ssh, 0o700);
    assert_eq!(code, Some(11));

    let home = tempfile::tempdir().unwrap();
    assert_eq!(
        run_sh(home.path(), DEPLOY_SCRIPT, "").status.code(),
        Some(12)
    );

    let home = tempfile::tempdir().unwrap();
    let ssh = home.path().join(".ssh");
    fs::create_dir(&ssh).unwrap();
    let file = ssh.join("authorized_keys");
    fs::write(&file, "other\n").unwrap();
    set_mode(&file, 0o400);
    assert_eq!(deploy_locally(home.path(), &key).status.code(), Some(13));
}

#[test]
fn deploy_script_survives_the_exec_wrapper() {
    let home = tempfile::tempdir().unwrap();
    let key = fake_key(KeyAlgorithm::Ed25519, "c");
    let out = run_sh(home.path(), &deploy_remote_command(), &format!("{key}\n"));
    assert!(out.status.success());
    assert_eq!(stdout_of(&out), SENTINEL_INSTALLED);
    assert_eq!(
        fs::read_to_string(home.path().join(".ssh/authorized_keys")).unwrap(),
        format!("{key}\n")
    );
}

#[test]
fn deploy_script_reads_a_key_without_trailing_newline() {
    let home = tempfile::tempdir().unwrap();
    let key = fake_key(KeyAlgorithm::Ed25519, "c");
    let out = run_sh(home.path(), DEPLOY_SCRIPT, &key);
    assert_eq!(stdout_of(&out), SENTINEL_INSTALLED);
    assert_eq!(
        fs::read_to_string(home.path().join(".ssh/authorized_keys")).unwrap(),
        format!("{key}\n")
    );
}

#[test]
fn deploy_script_treats_a_hostile_comment_as_data() {
    let home = tempfile::tempdir().unwrap();
    let key = fake_key(
        KeyAlgorithm::Ed25519,
        "x $(touch PWNED1) `touch PWNED2` ' \" ; touch PWNED3 #",
    );
    assert!(PublicKeyLine::parse(&key).is_ok());
    let out = run_sh(home.path(), &deploy_remote_command(), &format!("{key}\n"));
    assert_eq!(stdout_of(&out), SENTINEL_INSTALLED);
    for name in ["PWNED1", "PWNED2", "PWNED3"] {
        assert!(!home.path().join(name).exists(), "{name} was created");
    }
    assert_eq!(
        fs::read_to_string(home.path().join(".ssh/authorized_keys")).unwrap(),
        format!("{key}\n")
    );
}

// ---- C. argument vectors ---------------------------------------------------

#[test]
fn override_prefix_comes_first() {
    let args = control_channel_args(&[]);
    assert_eq!(args.len(), 20);
    assert_eq!(args, expected_prefix());
    assert_eq!(args[..2], ["-o", "StrictHostKeyChecking=yes"]);
}

#[test]
fn strict_host_key_checking_yes_beats_custom_options() {
    let session = strs(&[
        "-o",
        "StrictHostKeyChecking=no",
        "-oStrictHostKeyChecking=accept-new",
        "-o",
        "StrictHostKeyChecking no",
        "-o",
        "strictHOSTkeychecking = no",
    ]);
    let mut all = vec![control_channel_args(&session)];
    all.push(
        deploy_args(
            &SshTarget {
                session_args: session.clone(),
                ..target()
            },
            true,
        )
        .unwrap(),
    );
    all.push(
        verify_args(
            &SshTarget {
                session_args: session,
                ..target()
            },
            Path::new("/k/new"),
            false,
        )
        .unwrap(),
    );
    for args in all {
        assert_eq!(
            values_of(&args, "StrictHostKeyChecking"),
            ["StrictHostKeyChecking=yes"],
            "in {args:?}"
        );
        assert_eq!(args[..2], ["-o", "StrictHostKeyChecking=yes"]);
    }
}

#[test]
fn strips_forwarding_and_multiplexing() {
    let session = strs(&[
        "-A",
        "-X",
        "-Y",
        "-C",
        "-t",
        "-tt",
        "-N",
        "-f",
        "-L",
        "8080:h:80",
        "-L8080:h:80",
        "-R",
        "8080:h:80",
        "-D",
        "1080",
        "-W",
        "h:22",
        "-S",
        "/sock",
        "-O",
        "exit",
        "-o",
        "ControlMaster=auto",
        "-o",
        "ControlPath=/x",
        "-o",
        "ControlPersist=60",
        "-o",
        "RemoteCommand=top",
        "-o",
        "RequestTTY=force",
        "-o",
        "ForwardAgent=yes",
        "-o",
        "SessionType=none",
        "-oLocalForward=1 h:2",
        "-v",
    ]);
    let args = control_channel_args(&session);
    assert_eq!(args[20..], ["-v"], "full: {args:?}");
}

#[test]
fn keeps_jump_proxy_and_custom_options() {
    let session = strs(&[
        "-J",
        "a@b:22",
        "-o",
        "ProxyCommand=ssh -W %h:%p bastion",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ConnectTimeout=15",
        "-F",
        "cfg",
        "-v",
    ]);
    let args = control_channel_args(&session);
    assert_eq!(args[20..], session[..]);
}

#[test]
fn deploy_args_tail_and_batchmode() {
    let mut t = target();
    t.session_args = strs(&["-p", "22", "-l", "other", "-o", "Port=99", "-o", "User=zed"]);
    let args = deploy_args(&t, false).unwrap();
    assert_eq!(values_of(&args, "BatchMode"), ["BatchMode=yes"]);
    assert_eq!(args.iter().filter(|a| *a == "-p").count(), 1);
    assert!(!args.contains(&"-l".to_owned()));
    assert!(values_of(&args, "Port").is_empty() && values_of(&args, "User").is_empty());
    let mut expected_tail = strs(&[
        "-p",
        "2222",
        "-o",
        "UserKnownHostsFile=/kh",
        "user@example.com",
    ]);
    expected_tail.push(deploy_remote_command());
    assert_eq!(args[args.len() - 6..], expected_tail[..]);

    let with_password = deploy_args(&t, true).unwrap();
    assert!(values_of(&with_password, "BatchMode").is_empty());
}

#[test]
fn deploy_args_keep_a_known_hosts_option_the_session_already_has() {
    let mut t = target();
    t.session_args = strs(&["-o", "UserKnownHostsFile=/x"]);
    let args = deploy_args(&t, true).unwrap();
    assert_eq!(
        values_of(&args, "UserKnownHostsFile"),
        ["UserKnownHostsFile=/x"]
    );

    t.session_args = Vec::new();
    t.known_hosts_file = Some(PathBuf::from("/a b/kh"));
    let args = deploy_args(&t, true).unwrap();
    assert_eq!(
        values_of(&args, "UserKnownHostsFile"),
        ["UserKnownHostsFile=\"/a b/kh\""]
    );
    t.known_hosts_file = None;
    let args = deploy_args(&t, true).unwrap();
    assert!(values_of(&args, "UserKnownHostsFile").is_empty());
}

#[test]
fn verify_args_are_publickey_only_with_new_identity() {
    let mut t = target();
    t.session_args = strs(&[
        "-i",
        "old",
        "-o",
        "IdentityFile=old",
        "-o",
        "CertificateFile=c",
        "-o",
        "IdentityAgent=ssh-agent",
        "-o",
        "PasswordAuthentication=yes",
        "-oPubkeyAuthentication=no",
        "-J",
        "j",
    ]);
    let args = verify_args(&t, Path::new("/k/new"), false).unwrap();
    for (key, value) in [
        ("StrictHostKeyChecking", "yes"),
        ("ControlMaster", "no"),
        ("ControlPath", "none"),
        ("IdentityAgent", "none"),
        ("IdentitiesOnly", "yes"),
        ("PreferredAuthentications", "publickey"),
        ("PasswordAuthentication", "no"),
        ("KbdInteractiveAuthentication", "no"),
        ("PubkeyAuthentication", "yes"),
        ("BatchMode", "yes"),
    ] {
        assert_eq!(values_of(&args, key), [format!("{key}={value}")], "{key}");
    }
    assert!(values_of(&args, "IdentityFile").is_empty());
    assert!(values_of(&args, "CertificateFile").is_empty());
    let identities: Vec<_> = args
        .iter()
        .enumerate()
        .filter(|(_, a)| *a == "-i")
        .collect();
    assert_eq!(identities.len(), 1);
    assert_eq!(args[identities[0].0 + 1], "/k/new");
    assert!(args.contains(&"-v".to_owned()));
    assert!(args.windows(2).any(|w| w == ["-J", "j"]));
    assert_eq!(args.last().map(String::as_str), Some("exit 0"));

    let with_passphrase = verify_args(&t, Path::new("/k/new"), true).unwrap();
    assert!(values_of(&with_passphrase, "BatchMode").is_empty());
}

#[test]
fn invalid_target_rejected() {
    let bad_hosts = [
        "-oProxyCommand=x",
        "",
        "a b",
        "a\nb",
        "u@h",
        "h;reboot",
        "h$(id)",
        "h`id`",
        "h|x",
        "h&x",
        "h'x",
        "h\"x",
        "h\\x",
        "h>x",
    ];
    for host in bad_hosts {
        let t = SshTarget {
            host: host.to_owned(),
            ..target()
        };
        assert!(
            matches!(deploy_args(&t, true), Err(KeySetupError::InvalidTarget)),
            "host {host:?}"
        );
    }
    for user in ["-x", "", "a@b", "a b"] {
        let t = SshTarget {
            username: Some(user.to_owned()),
            ..target()
        };
        assert!(
            matches!(
                verify_args(&t, Path::new("/k"), false),
                Err(KeySetupError::InvalidTarget)
            ),
            "user {user:?}"
        );
    }
    let t = SshTarget {
        port: 0,
        ..target()
    };
    assert!(matches!(
        deploy_args(&t, true),
        Err(KeySetupError::InvalidTarget)
    ));
    let t = SshTarget {
        username: None,
        ..target()
    };
    assert_eq!(
        deploy_args(&t, true)
            .unwrap()
            .iter()
            .filter(|a| *a == "example.com")
            .count(),
        1
    );
}

#[test]
fn keygen_args_never_carry_a_passphrase() {
    let path = Path::new("/d/rustconn/x_ed25519");
    let to_strings = |args: Vec<OsString>| -> Vec<String> {
        args.into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    };
    let without = to_strings(keygen_args(
        path,
        KeyAlgorithm::Ed25519,
        "rustconn@h 2026-01-01",
        false,
    ));
    assert_eq!(
        without,
        strs(&[
            "-q",
            "-t",
            "ed25519",
            "-C",
            "rustconn@h 2026-01-01",
            "-f",
            "/d/rustconn/x_ed25519",
            "-N",
            ""
        ])
    );
    let with = to_strings(keygen_args(path, KeyAlgorithm::Rsa4096, "c", true));
    assert!(!with.contains(&"-N".to_owned()));
    assert_eq!(with[..5], ["-q", "-t", "rsa", "-b", "4096"]);
}

fn chunk() -> impl Strategy<Value = Vec<String>> {
    let keys = prop::sample::select(vec![
        "StrictHostKeyChecking",
        "strictHostKeyChecking",
        "ControlMaster",
        "ControlPath",
        "ClearAllForwardings",
        "ForwardAgent",
        "ForwardX11",
        "RemoteCommand",
        "RequestTTY",
        "PermitLocalCommand",
        "NumberOfPasswordPrompts",
        "BatchMode",
        "ServerAliveInterval",
    ]);
    prop_oneof![
        (keys.clone(), "[a-z]{0,6}").prop_map(|(k, v)| vec!["-o".to_owned(), format!("{k}={v}")]),
        (keys.clone(), "[a-z]{0,6}").prop_map(|(k, v)| vec![format!("-o{k}={v}")]),
        (keys.clone(), "[a-z]{0,6}").prop_map(|(k, v)| vec!["-vo".to_owned(), format!("{k}={v}")]),
        (keys, "[a-z]{0,6}").prop_map(|(k, v)| vec!["-o".to_owned(), format!("{k} {v}")]),
        Just(vec!["-A".to_owned()]),
        Just(vec!["-L".to_owned(), "x".to_owned()]),
        Just(vec!["-J".to_owned(), "y".to_owned()]),
        Just(vec!["-v".to_owned()]),
        Just(vec!["-vA".to_owned()]),
    ]
}

proptest! {
    #[test]
    fn prefix_always_wins(chunks in prop::collection::vec(chunk(), 0..8)) {
        let session: Vec<String> = chunks.concat();
        for args in [
            control_channel_args(&session),
            deploy_args(&SshTarget { session_args: session.clone(), ..target() }, true).unwrap(),
        ] {
            prop_assert_eq!(&args[..20], &expected_prefix()[..]);
            for value in option_values(&args[20..]) {
                let key = option_key(&value);
                prop_assert!(
                    !OVERRIDE_OPTIONS
                        .iter()
                        .any(|o| o.split('=').next().is_some_and(|k| k.eq_ignore_ascii_case(key))),
                    "override key {} repeated in {:?}",
                    key,
                    args
                );
            }
        }
    }
}

// ---- D. classification on captured samples ---------------------------------

const HOST_KEY_UNKNOWN: &str = "No ED25519 host key is known for [127.0.0.1]:2222 and you have requested strict checking.\r\nHost key verification failed.\r\n";
const HOST_KEY_CHANGED: &str = "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\r\n@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\r\n@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\r\nIT IS POSSIBLE THAT SOMEONE IS DOING SOMETHING NASTY!\r\nHost key for [127.0.0.1]:2222 has changed and you have requested strict checking.\r\nHost key verification failed.\r\n";

/// Captured stderr and the name of the error it must map to.
const SHARED_SAMPLES: [(&str, &str); 9] = [
    ("Host key verification failed.\r\n", "HostKeyUnknown"),
    (HOST_KEY_UNKNOWN, "HostKeyUnknown"),
    (HOST_KEY_CHANGED, "HostKeyChanged"),
    (
        "ssh: connect to host h port 22: Connection refused",
        "Refused",
    ),
    (
        "ssh: connect to host h port 22: Connection timed out",
        "TimedOut",
    ),
    (
        "ssh: connect to host h port 22: No route to host",
        "Unreachable",
    ),
    (
        "ssh: Could not resolve hostname x: Name or service not known",
        "UnknownHost",
    ),
    (
        "kex_exchange_identification: Connection closed by remote host",
        "ClosedByRemote",
    ),
    (
        "You are required to change your password immediately (administrator enforced)",
        "PasswordExpired",
    ),
];

fn label(error: &KeySetupError) -> String {
    match error {
        KeySetupError::Network(kind) => format!("{kind:?}"),
        other => format!("{other:?}"),
    }
}

fn methods_of(error: KeySetupError) -> Vec<String> {
    match error {
        KeySetupError::AuthFailed { server_methods } => server_methods,
        other => panic!("expected AuthFailed, got {other:?}"),
    }
}

#[test]
fn classify_deploy_success_and_script_failures() {
    assert!(matches!(
        classify_deploy(Some(0), "RUSTCONN-KEY-INSTALLED\n", ""),
        Ok(DeployOutcome::Installed)
    ));
    assert!(matches!(
        classify_deploy(Some(0), "Welcome\r\nRUSTCONN-KEY-PRESENT\r\n", ""),
        Ok(DeployOutcome::AlreadyPresent)
    ));
    assert!(matches!(
        classify_deploy(Some(0), "RUSTCONN_FORCECOMMAND_SENTINEL\n", ""),
        Err(KeySetupError::NoSentinel { code: Some(0) })
    ));
    // A line that merely contains the sentinel is not the sentinel.
    assert!(matches!(
        classify_deploy(Some(0), "echo x RUSTCONN-KEY-INSTALLED\n", ""),
        Err(KeySetupError::NoSentinel { code: Some(0) })
    ));
    for (code, step) in [
        (10, DeployStep::CreateSshDir),
        (11, DeployStep::CreateAuthorizedKeys),
        (12, DeployStep::ReadKey),
        (13, DeployStep::AppendKey),
    ] {
        assert!(matches!(
            classify_deploy(Some(code), "", ""),
            Err(KeySetupError::DeployScriptFailed(s)) if s == step
        ));
    }
    assert!(matches!(
        classify_deploy(
            Some(1),
            "",
            "'exec' is not recognized as an internal or external command,\r\noperable program or batch file.\r\n"
        ),
        Err(KeySetupError::NoPosixShell)
    ));
    assert!(matches!(
        classify_deploy(Some(7), "", "boom"),
        Err(KeySetupError::NoSentinel { code: Some(7) })
    ));
    assert!(matches!(
        classify_deploy(None, "", ""),
        Err(KeySetupError::SshFailed { code: None })
    ));
    assert!(matches!(
        classify_deploy(Some(255), "", "something new"),
        Err(KeySetupError::SshFailed { code: Some(255) })
    ));
}

#[test]
fn classify_shared_ssh_failures_for_deploy_and_verify() {
    for (stderr, expected) in SHARED_SAMPLES {
        let deploy = classify_deploy(Some(255), "", stderr).unwrap_err();
        assert_eq!(label(&deploy), expected, "deploy: {stderr:?}");
        let verify = classify_verify(Some(255), stderr, "SHA256:abc").unwrap_err();
        assert_eq!(label(&verify), expected, "verify: {stderr:?}");
    }
}

#[test]
fn classify_permission_denied_reports_the_server_methods() {
    let both = classify_deploy(
        Some(255),
        "",
        "user@h: Permission denied (publickey,password).\r\n",
    )
    .unwrap_err();
    assert_eq!(methods_of(both), ["publickey", "password"]);
    let otp = classify_deploy(
        Some(255),
        "",
        "Permission denied, please try again.\r\nuser@h: Permission denied (publickey,keyboard-interactive).\r\n",
    )
    .unwrap_err();
    assert_eq!(methods_of(otp), ["publickey", "keyboard-interactive"]);
}

const ACCEPTS: &str = "debug1: Offering public key: /k/x ED25519 SHA256:abc explicit\ndebug1: Server accepts key: /k/x ED25519 SHA256:abc explicit\n";

#[test]
fn classify_verify_needs_the_new_fingerprint() {
    let ok = format!("{ACCEPTS}Authenticated to h ([1.2.3.4]:22) using \"publickey\".\n");
    assert!(matches!(
        classify_verify(Some(0), &ok, "SHA256:abc"),
        Ok(VerifyOutcome::Accepted)
    ));
    // Another key's fingerprint, a prefix of ours, and no accept line at all.
    assert!(matches!(
        classify_verify(Some(0), &ok, "SHA256:zzz"),
        Err(KeySetupError::KeyNotAccepted)
    ));
    assert!(matches!(
        classify_verify(Some(0), &ok, "SHA256:ab"),
        Err(KeySetupError::KeyNotAccepted)
    ));
    assert!(matches!(
        classify_verify(
            Some(0),
            "debug1: Authentication succeeded (publickey).\n",
            "SHA256:abc"
        ),
        Err(KeySetupError::KeyNotAccepted)
    ));
    assert!(matches!(
        classify_verify(None, &ok, "SHA256:abc"),
        Err(KeySetupError::SshFailed { code: None })
    ));
}

#[test]
fn classify_verify_failure_modes() {
    let partial = format!(
        "{ACCEPTS}Authenticated using \"publickey\" with partial success.\nuser@h: Permission denied (keyboard-interactive).\n"
    );
    assert!(matches!(
        classify_verify(Some(255), &partial, "SHA256:abc"),
        Ok(VerifyOutcome::AcceptedSecondFactorRequired)
    ));
    assert!(matches!(
        classify_verify(
            Some(255),
            "user@h: Permission denied (publickey).\n",
            "SHA256:abc"
        ),
        Err(KeySetupError::KeyNotAccepted)
    ));
    let wrong = "Load key \"/k/x\": incorrect passphrase supplied to decrypt private key\nuser@h: Permission denied (publickey).\n";
    assert!(matches!(
        classify_verify(Some(255), wrong, "SHA256:abc"),
        Err(KeySetupError::WrongPassphrase)
    ));
    // OpenSSH 10 at `-v`: the key is accepted, the passphrase is asked for
    // once, and nothing else is printed about it (captured from a live server).
    let live_wrong = format!(
        "{ACCEPTS}debug1: read_passphrase: requested to askpass\ndebug1: No more authentication methods to try.\nuser@h: Permission denied (publickey,password,keyboard-interactive).\n"
    );
    assert!(matches!(
        classify_verify(Some(255), &live_wrong, "SHA256:abc"),
        Err(KeySetupError::WrongPassphrase)
    ));
    // The server accepted the key, but signing failed: still not a success.
    let accepted_then_denied = format!("{ACCEPTS}user@h: Permission denied (publickey).\n");
    assert!(matches!(
        classify_verify(Some(255), &accepted_then_denied, "SHA256:abc"),
        Err(KeySetupError::KeyNotAccepted)
    ));
}

#[test]
fn classify_verify_ignores_narration_in_debug_lines() {
    // A dual-stack host: the first address refuses, the second works, auth fails.
    let stderr = "debug1: connect to address ::1 port 22: Connection refused\ndebug1: Connection established\nuser@h: Permission denied (publickey).\n";
    assert!(matches!(
        classify_verify(Some(255), stderr, "SHA256:abc"),
        Err(KeySetupError::KeyNotAccepted)
    ));
}

/// What `-J bastion` adds to the stream: the bastion's own `-v` narration,
/// ahead of the target's `Offering public key` / `Server accepts key` lines.
const BASTION: &str = "debug1: Authenticated to bastion ([10.0.0.1]:22) using \"publickey\".\ndebug1: Authenticated using \"publickey\" with partial success.\n";

#[test]
fn classify_verify_ignores_jump_host_markers_before_the_accept_line() {
    // (a) Wrong passphrase behind a bastion: the bastion's `Authenticated to`
    // must not hide the askpass request that follows the target's accept line.
    let wrong = format!(
        "{BASTION}{ACCEPTS}debug1: read_passphrase: requested to askpass\nuser@h: Permission denied (publickey).\n"
    );
    assert!(matches!(
        classify_verify(Some(255), &wrong, "SHA256:abc"),
        Err(KeySetupError::WrongPassphrase)
    ));
    // (b) A bastion's `partial success` must not turn a plain success into a
    // second-factor verdict.
    let ok = format!(
        "{BASTION}{ACCEPTS}debug1: Authenticated to h ([1.2.3.4]:22) using \"publickey\".\n"
    );
    assert!(matches!(
        classify_verify(Some(0), &ok, "SHA256:abc"),
        Ok(VerifyOutcome::Accepted)
    ));
    // (c) Regression: `partial success` after the accept line still counts.
    let partial = format!(
        "{BASTION}{ACCEPTS}Authenticated using \"publickey\" with partial success.\nuser@h: Permission denied (keyboard-interactive).\n"
    );
    assert!(matches!(
        classify_verify(Some(255), &partial, "SHA256:abc"),
        Ok(VerifyOutcome::AcceptedSecondFactorRequired)
    ));
    // A bastion line alone, with no accept line from the target, proves nothing.
    assert!(matches!(
        classify_verify(Some(0), BASTION, "SHA256:abc"),
        Err(KeySetupError::KeyNotAccepted)
    ));
}

// ---- E. processes and secrets ----------------------------------------------

/// First argument that makes a stub exit at once, for the readiness check.
const STUB_PROBE: &str = "__rustconn_stub_probe__";

fn write_stub(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(
        &path,
        format!("#!/bin/sh\n[ \"$1\" = {STUB_PROBE} ] && exit 0\n{body}"),
    )
    .unwrap();
    set_mode(&path, 0o755);
    // A test on another thread forking while this file was open for writing
    // keeps it open in the child until that child execs, and exec fails with
    // ETXTBSY until then. Wait it out here, where it cannot look like the
    // behaviour under test.
    for _ in 0..500 {
        let started = std::process::Command::new(&path)
            .arg(STUB_PROBE)
            .stdin(std::process::Stdio::null())
            .status();
        if !matches!(&started, Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    path
}

fn keygen_request(dir: &Path, algo: KeyAlgorithm, passphrase: Option<&str>) -> KeygenRequest {
    KeygenRequest {
        private_key_path: dir.join(format!("x-12345678_{}", algo.file_suffix())),
        algorithm: algo,
        comment: "rustconn@test 2026-01-01".to_owned(),
        passphrase: passphrase.map(SecretString::from),
    }
}

#[test]
fn generate_key_ed25519_without_passphrase() {
    if !have("ssh-keygen") {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(KEY_SETUP_SUBDIR);
    ensure_key_setup_dir(&dir).unwrap();
    let request = keygen_request(&dir, KeyAlgorithm::Ed25519, None);
    let key = generate_key(&request, &RunControl::default()).unwrap();

    assert_eq!(mode(&key.private_key_path), 0o600);
    assert_eq!(mode(&key.public_key_path), 0o644);
    assert!(
        key.public_key
            .as_str()
            .ends_with("rustconn@test 2026-01-01")
    );
    let contents = fs::read_to_string(&key.private_key_path).unwrap();
    let info = crate::ssh_agent::inspect_key_file(&contents).unwrap();
    assert_eq!(
        info.encryption,
        crate::ssh_agent::KeyEncryption::Unencrypted
    );
    assert_eq!(
        PublicKeyInfo::from_private_key_file(&contents)
            .unwrap()
            .fingerprint,
        key.public_key.fingerprint()
    );
}

#[test]
fn generate_key_rsa4096() {
    if !have("ssh-keygen") {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let request = keygen_request(tmp.path(), KeyAlgorithm::Rsa4096, None);
    let key = generate_key(&request, &RunControl::default()).unwrap();
    assert_eq!(key.public_key.algorithm(), KeyAlgorithm::Rsa4096);
    let body = key.public_key.as_str().split(' ').nth(1).unwrap();
    assert!(data_encoding::BASE64.decode(body.as_bytes()).unwrap().len() >= 512);
}

#[test]
fn generate_key_with_passphrase_via_askpass() {
    if !have("ssh-keygen") {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let request = keygen_request(
        tmp.path(),
        KeyAlgorithm::Ed25519,
        Some("correct horse battery"),
    );
    let key = generate_key(&request, &RunControl::default()).unwrap();
    let contents = fs::read_to_string(&key.private_key_path).unwrap();
    let info = crate::ssh_agent::inspect_key_file(&contents).unwrap();
    assert_eq!(info.encryption, crate::ssh_agent::KeyEncryption::Encrypted);
    assert_eq!(
        PublicKeyInfo::from_private_key_file(&contents)
            .unwrap()
            .fingerprint,
        key.public_key.fingerprint()
    );
}

#[test]
fn generate_key_refuses_existing_private_or_pub() {
    if !have("ssh-keygen") {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let request = keygen_request(tmp.path(), KeyAlgorithm::Ed25519, None);
    let public = public_path(&request.private_key_path);

    fs::write(&public, b"keep").unwrap();
    assert!(matches!(
        generate_key(&request, &RunControl::default()),
        Err(KeySetupError::KeyFileExists { .. })
    ));
    assert_eq!(fs::read(&public).unwrap(), b"keep");
    fs::remove_file(&public).unwrap();

    fs::write(&request.private_key_path, b"keep").unwrap();
    assert!(matches!(
        generate_key(&request, &RunControl::default()),
        Err(KeySetupError::KeyFileExists { .. })
    ));
    assert_eq!(fs::read(&request.private_key_path).unwrap(), b"keep");
}

#[test]
fn generate_key_failure_leaves_no_files_behind() {
    let tmp = tempfile::tempdir().unwrap();
    let stub = write_stub(tmp.path(), "fake-keygen", "exit 3\n");
    let request = keygen_request(tmp.path(), KeyAlgorithm::Ed25519, None);
    assert!(matches!(
        generate_key_with(&stub, &request, &RunControl::default()),
        Err(KeySetupError::KeygenFailed { code: Some(3) })
    ));
    assert!(!request.private_key_path.exists());
}

#[test]
fn generate_key_reports_missing_binary() {
    let tmp = tempfile::tempdir().unwrap();
    let request = keygen_request(tmp.path(), KeyAlgorithm::Ed25519, None);
    assert!(matches!(
        generate_key_with(
            Path::new("/nonexistent/ssh-keygen"),
            &request,
            &RunControl::default()
        ),
        Err(KeySetupError::KeygenMissing)
    ));
}

/// A stub `ssh` that records argv, stdin and environment under `dir`, then
/// prints `tail`.
fn recording_stub(dir: &Path, tail: &str) -> PathBuf {
    let body = format!(
        "D='{d}'\n\
         for a in \"$@\"; do printf '%s\\n' \"$a\"; done > \"$D/argv\"\n\
         cat > \"$D/stdin\"\n\
         env > \"$D/env\"\n\
         for v in _RC_TGT_PW_FILE _RC_TUN_PW_FILE; do\n\
         eval \"f=\\${{$v}}\"\n\
         [ -n \"$f\" ] || continue\n\
         printf '%s' \"$f\" > \"$D/$v.path\"\n\
         (stat -c %a \"$f\" 2>/dev/null || stat -f %Lp \"$f\") > \"$D/$v.mode\"\n\
         cat \"$f\" > \"$D/$v.content\"\n\
         done\n\
         printf '%s' \"$SSH_ASKPASS\" > \"$D/askpass.path\"\n\
         {tail}",
        d = dir.display()
    );
    write_stub(dir, "ssh", &body)
}

#[test]
fn run_deploy_uses_stub_ssh_and_delivers_the_password_by_file() {
    let tmp = tempfile::tempdir().unwrap();
    let stub = recording_stub(tmp.path(), "echo RUSTCONN-KEY-INSTALLED\n");
    let key = parsed_key("rustconn@t 2026-01-01");
    let credentials = DeployCredentials {
        password: Some(SecretString::from(PROBE)),
        jump_host_passwords: Vec::new(),
    };
    let outcome = run_deploy_with(&stub, &target(), &key, &credentials, &RunControl::default());
    assert!(
        matches!(outcome, Ok(DeployOutcome::Installed)),
        "{outcome:?}"
    );

    let read = |name: &str| fs::read_to_string(tmp.path().join(name)).unwrap();
    assert_eq!(read("stdin"), format!("{}\n", key.as_str()));
    let expected_args = deploy_args(&target(), true).unwrap();
    assert_eq!(read("argv").lines().collect::<Vec<_>>(), expected_args);

    let env = read("env");
    assert!(env.contains("SSH_ASKPASS_REQUIRE=force"));
    assert!(env.contains("_RC_TGT_PW_FILE="));
    assert!(!env.contains(PROBE) && !read("argv").contains(PROBE));
    // During the run the secret sat in an owner-only file...
    assert_eq!(read("_RC_TGT_PW_FILE.mode").trim(), "600");
    assert_eq!(read("_RC_TGT_PW_FILE.content"), PROBE);
    // ...and nothing is left once the call returns.
    assert!(!Path::new(&read("_RC_TGT_PW_FILE.path")).exists());
}

#[test]
fn run_deploy_without_a_password_is_batch_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let stub = recording_stub(tmp.path(), "echo RUSTCONN-KEY-PRESENT\n");
    let key = parsed_key("c");
    let outcome = run_deploy_with(
        &stub,
        &target(),
        &key,
        &DeployCredentials::default(),
        &RunControl::default(),
    );
    assert!(matches!(outcome, Ok(DeployOutcome::AlreadyPresent)));
    let argv = fs::read_to_string(tmp.path().join("argv")).unwrap();
    assert!(argv.lines().any(|l| l == "BatchMode=yes"));
}

#[test]
fn run_verify_with_passphrase_uses_one_shot_helper() {
    let tmp = tempfile::tempdir().unwrap();
    let key = parsed_key("c");
    let tail = format!(
        "echo 'debug1: Server accepts key: /k ED25519 {} explicit' >&2\n",
        key.fingerprint()
    );
    let stub = recording_stub(tmp.path(), &tail);
    let passphrase = SecretString::from(PROBE);
    let outcome = run_verify_with(
        &stub,
        &target(),
        Path::new("/k/new"),
        &key,
        Some(&passphrase),
        &[],
        &RunControl::default(),
    );
    assert!(
        matches!(outcome, Ok(VerifyOutcome::Accepted)),
        "{outcome:?}"
    );

    let read = |name: &str| fs::read_to_string(tmp.path().join(name)).unwrap();
    let env = read("env");
    assert!(env.contains("_RC_TUN_PW_FILE=") && env.contains("SSH_ASKPASS_REQUIRE=force"));
    assert!(!env.contains(PROBE) && !read("argv").contains(PROBE));
    assert!(!read("argv").lines().any(|l| l == "BatchMode=yes"));
    assert_eq!(read("_RC_TUN_PW_FILE.mode").trim(), "600");
    assert_eq!(read("_RC_TUN_PW_FILE.content"), PROBE);
    assert!(!Path::new(&read("_RC_TUN_PW_FILE.path")).exists());
    assert!(!Path::new(&read("askpass.path")).exists());
}

#[test]
fn run_verify_without_a_passphrase_is_batch_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let key = parsed_key("c");
    let tail = format!(
        "echo 'debug1: Server accepts key: /k ED25519 {} explicit' >&2\n",
        key.fingerprint()
    );
    let stub = recording_stub(tmp.path(), &tail);
    let outcome = run_verify_with(
        &stub,
        &target(),
        Path::new("/k/new"),
        &key,
        None,
        &[],
        &RunControl::default(),
    );
    assert!(matches!(outcome, Ok(VerifyOutcome::Accepted)));
    let argv = fs::read_to_string(tmp.path().join("argv")).unwrap();
    assert!(argv.lines().any(|l| l == "BatchMode=yes"));
}

#[test]
fn cancel_kills_the_child_and_timeout_is_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let stub = write_stub(tmp.path(), "ssh", "exec sleep 30\n");
    let key = parsed_key("c");
    let run = |control: &RunControl| {
        run_verify_with(
            &stub,
            &target(),
            Path::new("/k/new"),
            &key,
            None,
            &[],
            control,
        )
    };

    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    let raiser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        flag.store(true, Ordering::Relaxed);
    });
    let started = std::time::Instant::now();
    let result = run(&RunControl {
        timeout: Duration::from_secs(30),
        cancel,
    });
    raiser.join().unwrap();
    assert!(
        matches!(result, Err(KeySetupError::Cancelled)),
        "{result:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));

    let result = run(&RunControl {
        timeout: Duration::from_millis(150),
        cancel: Arc::new(AtomicBool::new(false)),
    });
    assert!(matches!(result, Err(KeySetupError::Timeout)), "{result:?}");
}

#[test]
fn a_missing_stub_is_a_spawn_error_not_a_panic() {
    let key = parsed_key("c");
    let result = run_deploy_with(
        Path::new("/nonexistent/ssh"),
        &target(),
        &key,
        &DeployCredentials::default(),
        &RunControl::default(),
    );
    assert!(matches!(
        result,
        Err(KeySetupError::Spawn { program: "ssh", .. })
    ));
}

#[test]
fn debug_does_not_leak() {
    let request = KeygenRequest {
        private_key_path: PathBuf::from("/k"),
        algorithm: KeyAlgorithm::Ed25519,
        comment: "c".to_owned(),
        passphrase: Some(SecretString::from(PROBE)),
    };
    let credentials = DeployCredentials {
        password: Some(SecretString::from(PROBE)),
        jump_host_passwords: vec![(AskpassSlot::JumpHost(0), SecretString::from(PROBE))],
    };
    for rendered in [format!("{request:?}"), format!("{credentials:?}")] {
        assert!(!rendered.contains(PROBE), "{rendered}");
        assert!(rendered.contains("[REDACTED]") || rendered.contains("jump_host_passwords: 1"));
    }
    let errors = [
        KeySetupError::KeyFileExists {
            path: PathBuf::from("/k"),
        },
        KeySetupError::AuthFailed {
            server_methods: vec!["password".to_owned()],
        },
        KeySetupError::KeygenFailed { code: Some(1) },
        KeySetupError::WrongPassphrase,
        KeySetupError::InvalidPublicKey(PublicKeyError::BadComment),
    ];
    for error in errors {
        assert!(!format!("{error:?}{error}").contains(PROBE));
    }
}

// ---- review fixes: passphrase helper, clusters, classification ---------------

/// Runs the verify passphrase helper with `prompt`; returns (stdout, success).
fn run_passphrase_helper(
    dir: &Path,
    key_path: &str,
    prompt: &str,
    secret_file: &Path,
) -> (String, bool) {
    let script = create_key_passphrase_askpass_script().unwrap();
    let output = Command::new("sh")
        .arg(&script)
        .arg(prompt)
        .env(KEY_PATH_ENV_VAR, key_path)
        .env(ssh_tunnel::TUNNEL_ASKPASS_ENV_VAR, secret_file)
        .current_dir(dir)
        .output()
        .unwrap();
    ssh_tunnel::cleanup_askpass_script(&script);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.success(),
    )
}

#[test]
fn passphrase_helper_answers_only_the_new_keys_passphrase_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let key = "/home/u/.ssh/rustconn/web-1234abcd_ed25519";
    let fresh_secret = || {
        let path = tmp.path().join("secret");
        fs::write(&path, PROBE).unwrap();
        path
    };

    // Answered: the OpenSSH 9/10 and the older prompt forms.
    for prompt in [
        format!("Enter passphrase for key '{key}': "),
        format!("Enter passphrase for {key}: "),
    ] {
        let secret = fresh_secret();
        assert_eq!(
            run_passphrase_helper(tmp.path(), key, &prompt, &secret),
            (PROBE.to_owned(), true),
            "{prompt}"
        );
        // One shot: the file is emptied, a second ask gets nothing.
        assert_eq!(
            run_passphrase_helper(tmp.path(), key, &prompt, &secret),
            (String::new(), false)
        );
    }

    // Refused: a bastion's password prompt (the leak the review found), another
    // key's passphrase, OTP, host-key confirmation, an empty prompt.
    for prompt in [
        "admin@bastion's password: ",
        "Password: ",
        "(root@bastion) Password: ",
        "Enter passphrase for key '/home/u/.ssh/id_ed25519': ",
        "Enter passphrase for key '/home/u/.ssh/rustconn/other-1234abcd_ed25519': ",
        "Verification code: ",
        "Are you sure you want to continue connecting (yes/no/[fingerprint])? ",
        "",
    ] {
        let secret = fresh_secret();
        assert_eq!(
            run_passphrase_helper(tmp.path(), key, prompt, &secret),
            (String::new(), false),
            "{prompt:?}"
        );
        assert_eq!(fs::read_to_string(&secret).unwrap(), PROBE, "{prompt:?}");
    }

    // Glob characters in the key path stay literal.
    let secret = fresh_secret();
    assert_eq!(
        run_passphrase_helper(
            tmp.path(),
            "/k/*",
            "Enter passphrase for key '/k/other': ",
            &secret
        ),
        (String::new(), false)
    );
    // No key path in the environment: nothing is released.
    assert_eq!(
        run_passphrase_helper(tmp.path(), "", "Enter passphrase for key '': ", &secret),
        (String::new(), false)
    );
}

#[test]
fn verify_hands_the_helper_the_key_path_and_not_the_passphrase() {
    let tmp = tempfile::tempdir().unwrap();
    let key = parsed_key("c");
    let tail = format!(
        "echo 'debug1: Server accepts key: /k ED25519 {} explicit' >&2\n\
         \"$SSH_ASKPASS\" \"admin@bastion's password: \" > \"{d}/bastion.answer\"\n\
         \"$SSH_ASKPASS\" \"Enter passphrase for key '/k/new': \" > \"{d}/key.answer\"\n",
        key.fingerprint(),
        d = tmp.path().display()
    );
    let stub = recording_stub(tmp.path(), &tail);
    let passphrase = SecretString::from(PROBE);
    let outcome = run_verify_with(
        &stub,
        &target(),
        Path::new("/k/new"),
        &key,
        Some(&passphrase),
        &[],
        &RunControl::default(),
    );
    assert!(
        matches!(outcome, Ok(VerifyOutcome::Accepted)),
        "{outcome:?}"
    );
    let read = |name: &str| fs::read_to_string(tmp.path().join(name)).unwrap();
    assert!(read("env").contains("_RC_KEY_PASSPHRASE_FOR=/k/new"));
    assert!(!read("env").contains(PROBE));
    assert_eq!(read("bastion.answer"), "");
    assert_eq!(read("key.answer"), PROBE);
}

#[test]
fn generate_key_keeps_the_passphrase_out_of_argv_and_env() {
    let tmp = tempfile::tempdir().unwrap();
    let body = format!(
        "D='{d}'\n\
         for a in \"$@\"; do printf '%s\\n' \"$a\"; done > \"$D/argv\"\n\
         env > \"$D/env\"\n\
         cat \"$_RC_TUN_PW_FILE\" > \"$D/secret.content\"\n\
         printf '%s' \"$_RC_TUN_PW_FILE\" > \"$D/secret.path\"\n\
         exit 3\n",
        d = tmp.path().display()
    );
    let stub = write_stub(tmp.path(), "fake-keygen", &body);
    let request = keygen_request(tmp.path(), KeyAlgorithm::Ed25519, Some(PROBE));
    assert!(matches!(
        generate_key_with(&stub, &request, &RunControl::default()),
        Err(KeySetupError::KeygenFailed { code: Some(3) })
    ));
    let read = |name: &str| fs::read_to_string(tmp.path().join(name)).unwrap();
    assert!(!read("argv").contains(PROBE));
    assert!(!read("argv").lines().any(|l| l == "-N"));
    assert!(!read("env").contains(PROBE));
    assert!(read("env").contains("SSH_ASKPASS_REQUIRE=force"));
    assert_eq!(read("secret.content"), PROBE);
    assert!(!Path::new(&read("secret.path")).exists());
}

#[test]
fn clustered_short_flags_cannot_bypass_the_prefix() {
    for (session, expect) in [
        (&["-vA"][..], &["-v"][..]),
        (&["-vX"], &["-v"]),
        (&["-vt"], &["-v"]),
        (&["-vAX"], &["-v"]),
        (&["-Av"], &["-v"]),
        (&["-AX"], &[]),
        (&["-vq"], &["-v", "-q"]),
        (&["-vS", "/sock"], &["-v"]),
        (&["-vM"], &["-v"]),
        (&["-vp22"], &["-v"]),
        (&["-vi", "/id", "-q"], &["-v", "-i", "/id", "-q"]),
        (&["-vL80:h:1", "-q"], &["-v", "-q"]),
        (&["-vJ", "bastion"], &["-v", "-J", "bastion"]),
        (&["-vo", "StrictHostKeyChecking=no", "-q"], &["-v", "-q"]),
        (&["-voForwardAgent=yes"], &["-v"]),
        (
            &["-vo", "ServerAliveInterval=15"],
            &["-v", "-o", "ServerAliveInterval=15"],
        ),
    ] {
        let args = control_channel_args(&strs(session));
        assert_eq!(args[20..], strs(expect)[..], "session {session:?}");
    }
    // A flag-looking value of a value flag is a value, not a cluster.
    let args = control_channel_args(&strs(&["-F", "-vA"]));
    assert_eq!(args[20..], strs(&["-F", "-vA"])[..]);
}

#[test]
fn a_post_auth_255_is_not_a_wrong_passphrase() {
    let stderr = format!(
        "{ACCEPTS}debug1: read_passphrase: requested to askpass\nAuthenticated to h ([1.2.3.4]:22) using \"publickey\".\nConnection to h closed by remote host.\n"
    );
    assert!(!matches!(
        classify_verify(Some(255), &stderr, "SHA256:abc"),
        Err(KeySetupError::WrongPassphrase)
    ));
}

#[test]
fn acceptance_must_be_a_debug_line() {
    let fake = "Server accepts key: /k ED25519 SHA256:abc explicit\n";
    assert!(matches!(
        classify_verify(Some(0), fake, "SHA256:abc"),
        Err(KeySetupError::KeyNotAccepted)
    ));
    let indented = " debug1: Server accepts key: /k ED25519 SHA256:abc explicit\n";
    assert!(matches!(
        classify_verify(Some(0), indented, "SHA256:abc"),
        Err(KeySetupError::KeyNotAccepted)
    ));
}

#[test]
fn too_many_authentication_failures_is_an_auth_failure() {
    let stderr = "Received disconnect from 10.0.0.5 port 22:2: Too many authentication failures\r\nDisconnected from 10.0.0.5 port 22\r\n";
    let error = classify_deploy(Some(255), "", stderr).unwrap_err();
    assert!(methods_of(error).is_empty());
    // A `Permission denied` line still wins and carries its methods.
    let both = format!("{stderr}u@h: Permission denied (publickey,password).\n");
    let error = classify_deploy(Some(255), "", &both).unwrap_err();
    assert_eq!(methods_of(error), ["publickey", "password"]);
}

// ---- K: existing_key (K5: "Use an Existing Key") ---------------------------

/// A deterministic ed25519 public blob, built the way `fake_key` builds its
/// `.pub` lines, so a private-key fixture and a `.pub` file can be made to
/// agree (or disagree) on purpose.
fn ed25519_blob() -> Vec<u8> {
    fake_blob(KeyAlgorithm::Ed25519.ssh_type(), 32)
}

/// An OpenSSH private-key fixture (optionally "encrypted") whose public blob
/// is `ed25519_blob()`, plus the public-key line it derives to.
fn openssh_fixture(ciphername: &str) -> (String, String) {
    let blob = ed25519_blob();
    let pem = crate::ssh_agent::openssh_key_fixture(ciphername, &blob);
    let line = format!(
        "{} {}",
        KeyAlgorithm::Ed25519.ssh_type(),
        data_encoding::BASE64.encode(&blob)
    );
    (pem, line)
}

fn write_private(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).unwrap();
    path
}

#[test]
fn existing_key_unencrypted_without_pub_derives_the_line() {
    let tmp = tempfile::tempdir().unwrap();
    let (pem, line) = openssh_fixture("none");
    let path = write_private(tmp.path(), "id_ed25519", &pem);

    let key = existing_key(&path).unwrap();
    assert_eq!(key.encryption, KeyEncryption::Unencrypted);
    assert_eq!(key.public_key.as_str(), line);
    assert_eq!(key.private_key_path, path);
}

#[test]
fn existing_key_encrypted_without_pub_still_derives_the_line() {
    let tmp = tempfile::tempdir().unwrap();
    let (pem, line) = openssh_fixture("aes256-ctr");
    let path = write_private(tmp.path(), "id_ed25519", &pem);

    let key = existing_key(&path).unwrap();
    assert_eq!(key.encryption, KeyEncryption::Encrypted);
    assert_eq!(key.public_key.as_str(), line);
}

#[test]
fn existing_key_real_encrypted_key_from_ssh_keygen() {
    if !have("ssh-keygen") {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let private = tmp.path().join("k");
    let status = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "secret", "-C", "test", "-f"])
        .arg(&private)
        .status()
        .unwrap();
    assert!(status.success());
    fs::remove_file(public_path(&private)).unwrap();

    let key = existing_key(&private).unwrap();
    assert_eq!(key.encryption, KeyEncryption::Encrypted);
    assert_eq!(key.public_key.algorithm(), KeyAlgorithm::Ed25519);
}

#[test]
fn existing_key_prefers_a_matching_pub_file_for_its_comment() {
    let tmp = tempfile::tempdir().unwrap();
    let (pem, _) = openssh_fixture("none");
    let path = write_private(tmp.path(), "id_ed25519", &pem);
    let pub_text = format!(
        "{} {} tester@host\n",
        KeyAlgorithm::Ed25519.ssh_type(),
        data_encoding::BASE64.encode(&ed25519_blob())
    );
    fs::write(public_path(&path), &pub_text).unwrap();

    let key = existing_key(&path).unwrap();
    assert!(key.public_key.as_str().ends_with("tester@host"));
}

#[test]
fn existing_key_ignores_a_mismatched_pub_file() {
    let tmp = tempfile::tempdir().unwrap();
    let (pem, line) = openssh_fixture("none");
    let path = write_private(tmp.path(), "id_ed25519", &pem);
    // A different key entirely.
    let other_blob = fake_blob(KeyAlgorithm::Ed25519.ssh_type(), 16);
    fs::write(
        public_path(&path),
        format!(
            "{} {} someone-else@host\n",
            KeyAlgorithm::Ed25519.ssh_type(),
            data_encoding::BASE64.encode(&other_blob)
        ),
    )
    .unwrap();

    let key = existing_key(&path).unwrap();
    assert_eq!(key.public_key.as_str(), line);
}

#[test]
fn existing_key_ignores_a_pub_file_with_a_bad_comment() {
    let tmp = tempfile::tempdir().unwrap();
    let (pem, line) = openssh_fixture("none");
    let path = write_private(tmp.path(), "id_ed25519", &pem);
    fs::write(
        public_path(&path),
        format!(
            "{} {} Київ\n",
            KeyAlgorithm::Ed25519.ssh_type(),
            data_encoding::BASE64.encode(&ed25519_blob())
        ),
    )
    .unwrap();

    let key = existing_key(&path).unwrap();
    assert_eq!(key.public_key.as_str(), line);
}

#[test]
fn existing_key_refuses_a_pub_file_random_text_or_an_oversized_file() {
    let tmp = tempfile::tempdir().unwrap();
    let as_pub = write_private(
        tmp.path(),
        "id_ed25519.pub",
        &format!(
            "{} {} tester@host\n",
            KeyAlgorithm::Ed25519.ssh_type(),
            data_encoding::BASE64.encode(&ed25519_blob())
        ),
    );
    assert!(matches!(
        existing_key(&as_pub),
        Err(KeySetupError::NotAPrivateKey { .. })
    ));

    let random = write_private(tmp.path(), "random.txt", "not a key at all\n");
    assert!(matches!(
        existing_key(&random),
        Err(KeySetupError::NotAPrivateKey { .. })
    ));

    let big = tmp.path().join("big");
    fs::write(&big, vec![b'a'; 64 * 1024 + 1]).unwrap();
    assert!(matches!(
        existing_key(&big),
        Err(KeySetupError::NotAPrivateKey { .. })
    ));
}

#[test]
fn existing_key_legacy_pem_needs_a_pub_file() {
    let tmp = tempfile::tempdir().unwrap();
    let pem = "-----BEGIN RSA PRIVATE KEY-----\nMII\n-----END RSA PRIVATE KEY-----\n";
    let path = write_private(tmp.path(), "legacy", pem);
    assert!(matches!(
        existing_key(&path),
        Err(KeySetupError::NoPublicKey { .. })
    ));

    fs::write(
        public_path(&path),
        format!(
            "{} {} legacy@host\n",
            KeyAlgorithm::Rsa4096.ssh_type(),
            data_encoding::BASE64.encode(&fake_blob(KeyAlgorithm::Rsa4096.ssh_type(), 256))
        ),
    )
    .unwrap();
    let key = existing_key(&path).unwrap();
    assert_eq!(key.encryption, KeyEncryption::Unknown);
    assert!(key.public_key.as_str().ends_with("legacy@host"));
}

#[test]
fn existing_key_unsupported_key_type_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let blob = fake_blob("ecdsa-sha2-nistp256", 64);
    let pem = crate::ssh_agent::openssh_key_fixture("none", &blob);
    let path = write_private(tmp.path(), "id_ecdsa", &pem);

    assert!(matches!(
        existing_key(&path),
        Err(KeySetupError::InvalidPublicKey(
            PublicKeyError::UnsupportedType
        ))
    ));
}

#[test]
fn existing_key_missing_file_is_a_key_file_error() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("nope");
    assert!(matches!(
        existing_key(&missing),
        Err(KeySetupError::KeyFile { .. })
    ));
}

#[test]
fn existing_key_candidates_in_dedupes_and_skips_missing_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let dir_a = tmp.path().join("a");
    let dir_b = dir_a.join("rustconn");
    fs::create_dir_all(&dir_b).unwrap();
    let (pem, _) = openssh_fixture("none");
    write_private(&dir_a, "id_ed25519", &pem);
    write_private(&dir_b, "no_id_pattern_but_a_key", &pem);
    let missing = tmp.path().join("does-not-exist");

    let candidates = existing_key_candidates_in(&[
        dir_a.clone(),
        dir_b.clone(),
        missing,
        dir_a.clone(), // repeated directory: no duplicate entries.
    ]);

    assert_eq!(candidates.len(), 2);
    assert!(candidates.contains(&dir_a.join("id_ed25519")));
    assert!(candidates.contains(&dir_b.join("no_id_pattern_but_a_key")));
}

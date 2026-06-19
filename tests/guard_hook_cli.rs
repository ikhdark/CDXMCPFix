use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn run_cdxcore(args: &[&str], stdin: &str) -> (i32, String, String) {
    run_cdxcore_with_env(args, stdin, &[])
}

fn run_cdxcore_with_env(
    args: &[&str],
    stdin: &str,
    envs: &[(&str, &str)],
) -> (i32, String, String) {
    run_cdxcore_with_env_and_removed(args, stdin, envs, &[])
}

fn run_cdxcore_with_env_and_removed(
    args: &[&str],
    stdin: &str,
    envs: &[(&str, &str)],
    removed_envs: &[&str],
) -> (i32, String, String) {
    run_cdxcore_with_env_removed_and_cwd(args, stdin, envs, removed_envs, None)
}

fn run_cdxcore_with_env_removed_and_cwd(
    args: &[&str],
    stdin: &str,
    envs: &[(&str, &str)],
    removed_envs: &[&str],
    current_dir: Option<&std::path::Path>,
) -> (i32, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cdxcore"));
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }
    for key in removed_envs {
        command.env_remove(key);
    }
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("spawn cdxcore");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait cdxcore");
    (
        output.status.code().unwrap_or_default(),
        String::from_utf8(output.stdout).expect("stdout utf8"),
        String::from_utf8(output.stderr).expect("stderr utf8"),
    )
}

#[test]
fn guard_hook_json_flag_does_not_change_contract_output() {
    let payload = r#"{"tool_name":"Bash","tool_input":{"command":"rm -rf target"}}"#;
    let (plain_code, plain_stdout, plain_stderr) =
        run_cdxcore(&["guard-hook", "pre-tool-use"], payload);
    let (json_code, json_stdout, json_stderr) =
        run_cdxcore(&["--json", "guard-hook", "pre-tool-use"], payload);

    assert_eq!(plain_code, 0);
    assert_eq!(json_code, 0);
    assert_eq!(plain_stderr, "");
    assert_eq!(json_stderr, "");
    assert_eq!(plain_stdout, json_stdout);
    assert!(plain_stdout.contains("\"hookSpecificOutput\""));
    assert!(!plain_stdout.contains("permissionDecision"));
}

#[test]
fn guard_hook_malformed_input_is_silent_success() {
    let (code, stdout, stderr) = run_cdxcore(&["guard-hook", "pre-tool-use"], "{");

    assert_eq!(code, 0);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
}

#[test]
fn guard_hook_debug_env_does_not_write_stderr() {
    let (code, stdout, stderr) = run_cdxcore_with_env(
        &["guard-hook", "pre-tool-use"],
        "{",
        &[("CDXCORE_DEBUG", "1")],
    );

    assert_eq!(code, 0);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
}

#[test]
fn guard_hook_oversized_input_is_silent_success() {
    let oversized = format!(
        "{{\"tool_name\":\"Bash\",\"tool_input\":{{\"command\":\"rm -rf target\"}},\"padding\":\"{}\"}}",
        "x".repeat(140 * 1024)
    );
    let (code, stdout, stderr) = run_cdxcore(&["guard-hook", "pre-tool-use"], &oversized);

    assert_eq!(code, 0);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
}

#[test]
fn guard_hook_default_does_not_touch_ledger_files() {
    let home = tempdir().unwrap();
    let home_text = home.path().to_string_lossy().to_string();
    let payload = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"C:\\Users\\demo\\project","tool_input":{"command":"rm -rf target"}}"#;

    let (code, stdout, stderr) = run_cdxcore_with_env(
        &["guard-hook", "pre-tool-use"],
        payload,
        &[("CODEX_HOME", &home_text)],
    );

    assert_eq!(code, 0);
    assert_eq!(stderr, "");
    assert!(stdout.contains("destructive-looking"));
    assert!(!home.path().join("cdxcore").exists());
}

#[test]
fn guard_hook_ledger_repeats_risky_shapes_without_raw_storage() {
    let home = tempdir().unwrap();
    let home_text = home.path().to_string_lossy().to_string();
    let command = "TOKEN=not-a-real-secret git reset --hard";
    let cwd = "C:\\Users\\Alice\\Project With Secret Name";
    let payload = format!(
        r#"{{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"{}","tool_input":{{"command":"{}"}}}}"#,
        cwd.replace('\\', "\\\\"),
        command
    );

    let first = run_cdxcore_with_env(
        &["guard-hook", "pre-tool-use", "--ledger"],
        &payload,
        &[("CODEX_HOME", &home_text)],
    );
    assert_eq!(first.0, 0);
    assert_eq!(first.2, "");
    assert!(first.1.contains("destructive-looking"));
    assert!(!first.1.contains("seen repeatedly"));

    let second = run_cdxcore_with_env(
        &["guard-hook", "pre-tool-use", "--ledger"],
        &payload,
        &[("CODEX_HOME", &home_text)],
    );
    assert_eq!(second.0, 0);
    assert_eq!(second.2, "");
    assert!(second.1.contains("seen repeatedly"));
    assert!(!second.1.contains("failed"));

    let third = run_cdxcore_with_env(
        &["guard-hook", "pre-tool-use", "--ledger"],
        &payload,
        &[("CODEX_HOME", &home_text)],
    );
    assert_eq!(third.0, 0);
    assert_eq!(third.2, "");
    assert!(third.1.contains("retry loop"));
    assert!(!third.1.contains("failed"));

    let ledger = fs::read_to_string(home.path().join("cdxcore").join("guard-ledger-v1.jsonl"))
        .expect("ledger");
    assert!(ledger.contains("\"schema_version\":1"));
    assert!(!ledger.contains(command));
    assert!(!ledger.contains("TOKEN"));
    assert!(!ledger.contains("Alice"));
    assert!(!ledger.contains("Project With Secret Name"));
    assert!(!ledger.contains("git reset --hard"));
}

#[test]
fn guard_hook_ledger_env_precedence_and_non_risky_laziness() {
    let off_home = tempdir().unwrap();
    let off_home_text = off_home.path().to_string_lossy().to_string();
    let risky = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git clean -fd"}}"#;

    let off = run_cdxcore_with_env(
        &["guard-hook", "pre-tool-use", "--ledger"],
        risky,
        &[
            ("CODEX_HOME", &off_home_text),
            ("CDXCORE_GUARD_LEDGER", "off"),
        ],
    );
    assert_eq!(off.0, 0);
    assert_eq!(off.2, "");
    assert!(off.1.contains("destructive-looking"));
    assert!(!off_home.path().join("cdxcore").exists());

    let on_home = tempdir().unwrap();
    let on_home_text = on_home.path().to_string_lossy().to_string();
    let on = run_cdxcore_with_env(
        &["guard-hook", "pre-tool-use"],
        risky,
        &[
            ("CODEX_HOME", &on_home_text),
            ("CDXCORE_GUARD_LEDGER", "on"),
        ],
    );
    assert_eq!(on.0, 0);
    assert_eq!(on.2, "");
    assert!(on_home
        .path()
        .join("cdxcore")
        .join("guard-ledger-v1.jsonl")
        .exists());

    let quiet_home = tempdir().unwrap();
    let quiet_home_text = quiet_home.path().to_string_lossy().to_string();
    let quiet = run_cdxcore_with_env(
        &["guard-hook", "pre-tool-use", "--ledger"],
        r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"Write-Host ok"}}"#,
        &[("CODEX_HOME", &quiet_home_text)],
    );
    assert_eq!(quiet.0, 0);
    assert_eq!(quiet.1, "");
    assert_eq!(quiet.2, "");
    assert!(!quiet_home.path().join("cdxcore").exists());
}

#[test]
fn guard_hook_ledger_home_fallback_stays_out_of_process_cwd() {
    let fallback_home = tempdir().unwrap();
    let process_cwd = tempdir().unwrap();
    let home_text = fallback_home.path().to_string_lossy().to_string();
    let cwd_text = process_cwd.path().to_string_lossy().to_string();
    let risky = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git reset --hard"}}"#;
    let (code, _stdout, stderr) = run_cdxcore_with_env_removed_and_cwd(
        &["guard-hook", "pre-tool-use", "--ledger"],
        risky,
        &[
            ("HOME", &home_text),
            ("USERPROFILE", &home_text),
            ("PWD", &cwd_text),
        ],
        &["CODEX_HOME", "HOMEDRIVE", "HOMEPATH"],
        Some(process_cwd.path()),
    );

    assert_eq!(code, 0);
    assert_eq!(stderr, "");
    assert!(fallback_home
        .path()
        .join(".codex")
        .join("cdxcore")
        .join("guard-ledger-v1.jsonl")
        .exists());
    assert!(!process_cwd.path().join("cdxcore").exists());
}

#[test]
fn guard_hook_ledger_failures_silently_fall_back() {
    let malformed_home = tempdir().unwrap();
    let malformed_guard_dir = malformed_home.path().join("cdxcore");
    fs::create_dir_all(&malformed_guard_dir).unwrap();
    fs::write(
        malformed_guard_dir.join("guard-ledger-key"),
        "not-a-valid-key",
    )
    .unwrap();
    let malformed_home_text = malformed_home.path().to_string_lossy().to_string();
    let risky = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"rm -rf target"}}"#;

    let malformed = run_cdxcore_with_env(
        &["guard-hook", "pre-tool-use", "--ledger"],
        risky,
        &[("CODEX_HOME", &malformed_home_text)],
    );
    assert_eq!(malformed.0, 0);
    assert_eq!(malformed.2, "");
    assert!(malformed.1.contains("destructive-looking"));
    assert_eq!(
        fs::read_to_string(malformed_guard_dir.join("guard-ledger-key")).unwrap(),
        "not-a-valid-key"
    );
    assert!(!malformed_guard_dir.join("guard-ledger-v1.jsonl").exists());

    let locked_home = tempdir().unwrap();
    let locked_guard_dir = locked_home.path().join("cdxcore");
    fs::create_dir_all(&locked_guard_dir).unwrap();
    fs::write(locked_guard_dir.join("guard-ledger.lock"), "stale").unwrap();
    let locked_home_text = locked_home.path().to_string_lossy().to_string();
    let locked = run_cdxcore_with_env(
        &["guard-hook", "pre-tool-use", "--ledger"],
        risky,
        &[("CODEX_HOME", &locked_home_text)],
    );
    assert_eq!(locked.0, 0);
    assert_eq!(locked.2, "");
    assert!(locked.1.contains("destructive-looking"));
    assert!(locked_guard_dir.join("guard-ledger.lock").exists());
    assert!(!locked_guard_dir.join("guard-ledger-v1.jsonl").exists());
}

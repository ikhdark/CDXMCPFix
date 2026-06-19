use serde_json::Value as JsonValue;
use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

fn cdxcore_output(args: &[&str]) -> String {
    let output = cdxcore_command(args).output().expect("run cdxcore");
    assert!(output.status.success());
    String::from_utf8(output.stdout).expect("stdout utf8")
}

fn cdxcore_command(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cdxcore"));
    command.args(args);
    command
}

#[test]
fn top_level_help_mentions_setup_and_optional_command_guard() {
    let stdout = cdxcore_output(&["--help"]);

    assert!(stdout.contains("setup"));
    assert!(stdout.contains("guard-hook"));
    assert!(stdout.contains("Optional command guard"));
    assert!(stdout.contains("--enable-command-guard"));
}

#[test]
fn guard_hook_help_marks_hooks_optional_and_feedback_only() {
    let stdout = cdxcore_output(&["guard-hook", "--help"]);

    assert!(stdout.contains("optional"));
    assert!(stdout.contains("feedback-only"));
    assert!(stdout.contains("pre-tool-use"));
    assert!(stdout.contains("post-tool-use"));
}

#[test]
fn setup_codex_help_mentions_mcp_default_and_guard_opt_in() {
    let stdout = cdxcore_output(&["setup", "codex", "--help"]);

    assert!(stdout.contains("cdxcore serve"));
    assert!(stdout.contains("--enable-command-guard"));
    assert!(stdout.contains("--enable-retry-ledger"));
    assert!(stdout.contains("Default setup installs only the CDXCore MCP server"));
    assert!(stdout.contains("feedback-only PreToolUse command guard hook"));
}

#[test]
fn setup_codex_enable_command_guard_writes_isolated_pre_tool_use_hook() {
    let (command, pre_hook) = run_setup_codex(&["setup", "codex", "--enable-command-guard"]);
    assert!(command.status.success());
    assert_eq!(
        pre_hook.get("command").and_then(JsonValue::as_str),
        Some("cdxcore guard-hook pre-tool-use")
    );
    assert_eq!(pre_hook.get("timeout").and_then(JsonValue::as_u64), Some(3));
    assert!(!Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("hooks")
        .join("hooks.json")
        .exists());
}

#[test]
fn setup_codex_enable_retry_ledger_writes_pre_tool_use_ledger_hook() {
    let (command, pre_hook) = run_setup_codex(&[
        "setup",
        "codex",
        "--enable-command-guard",
        "--enable-retry-ledger",
    ]);
    assert!(command.status.success());
    assert_eq!(
        pre_hook.get("command").and_then(JsonValue::as_str),
        Some("cdxcore guard-hook pre-tool-use --ledger")
    );
    assert_eq!(pre_hook.get("timeout").and_then(JsonValue::as_u64), Some(3));
}

fn run_setup_codex(args: &[&str]) -> (std::process::Output, JsonValue) {
    let fake_bin = tempdir().unwrap();
    let codex_home = tempdir().unwrap();
    write_fake_codex(fake_bin.path());

    let mut paths = vec![fake_bin.path().to_path_buf()];
    if let Some(existing_path) = env::var_os("PATH") {
        paths.extend(env::split_paths(&existing_path));
    }
    let joined_path = env::join_paths(paths).unwrap();

    let output = cdxcore_command(args)
        .env("CODEX_HOME", codex_home.path())
        .env("PATH", joined_path)
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
        .output()
        .expect("run setup codex");

    assert!(
        output.status.success(),
        "setup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let hooks_path = codex_home.path().join("hooks.json");
    let text = fs::read_to_string(&hooks_path).expect("hooks.json written");
    let value: JsonValue = serde_json::from_str(&text).expect("valid hooks json");
    let hooks = value.get("hooks").and_then(JsonValue::as_object).unwrap();
    assert!(hooks.get("PostToolUse").is_none());

    let pre_hook = hooks
        .get("PreToolUse")
        .and_then(JsonValue::as_array)
        .and_then(|groups| groups.first())
        .and_then(|group| group.get("hooks"))
        .and_then(JsonValue::as_array)
        .and_then(|handlers| handlers.first())
        .unwrap()
        .clone();
    (output, pre_hook)
}

#[cfg(windows)]
fn write_fake_codex(dir: &Path) {
    let cmd = env::var_os("COMSPEC").unwrap_or_else(|| "C:\\Windows\\System32\\cmd.exe".into());
    fs::copy(cmd, dir.join("codex.exe")).unwrap();
}

#[cfg(unix)]
fn write_fake_codex(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("codex");
    fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

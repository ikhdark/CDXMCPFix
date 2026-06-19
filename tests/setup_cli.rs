use std::env;
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
fn top_level_help_mentions_setup_and_mcp_server() {
    let stdout = cdxcore_output(&["--help"]);

    assert!(stdout.contains("setup"));
    assert!(stdout.contains("mcp-server"));
    assert!(!stdout.contains("guard-hook"));
}

#[test]
fn setup_codex_help_mentions_mcp_default_only() {
    let stdout = cdxcore_output(&["setup", "codex", "--help"]);

    assert!(stdout.contains("cdxcore mcp-server"));
    assert!(stdout.contains("Default setup installs only the CDXCore MCP server"));
    assert!(!stdout.contains("--enable-command-guard"));
    assert!(!stdout.contains("--enable-retry-ledger"));
    assert!(!stdout.contains("--enable-command-repair"));
}

#[test]
fn setup_codex_configures_mcp_server_without_hooks() {
    let fake_bin = tempdir().unwrap();
    let codex_home = tempdir().unwrap();
    write_fake_codex(fake_bin.path());

    let mut paths = vec![fake_bin.path().to_path_buf()];
    if let Some(existing_path) = env::var_os("PATH") {
        paths.extend(env::split_paths(&existing_path));
    }
    let joined_path = env::join_paths(paths).unwrap();

    let output = cdxcore_command(&["setup", "codex"])
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
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Configured Codex MCP server `cdxcore`"));
    assert!(!codex_home.path().join("hooks.json").exists());
}

#[cfg(windows)]
fn write_fake_codex(dir: &std::path::Path) {
    let cmd = env::var_os("COMSPEC").unwrap_or_else(|| "C:\\Windows\\System32\\cmd.exe".into());
    std::fs::copy(cmd, dir.join("codex.exe")).unwrap();
}

#[cfg(unix)]
fn write_fake_codex(dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("codex");
    std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

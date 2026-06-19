use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout, Command};
use tokio::time::{timeout, Duration};

pub const SCHEMA_VERSION: &str = "cdxcore.diagnostics.v1";
const INIT_TIMEOUT: Duration = Duration::from_millis(3_000);
const TOOLS_TIMEOUT: Duration = Duration::from_millis(3_000);
const MAX_TOOL_PAGES: usize = 8;
const SECRET_TERMS: &[&str] = &[
    "token",
    "key",
    "secret",
    "password",
    "bearer",
    "auth",
    "credential",
    "cookie",
    "session",
    "api",
    "oauth",
];

#[derive(Parser, Debug)]
#[command(
    name = "cdxcore",
    version,
    about = "Read-only MCP diagnostics for Codex"
)]
pub struct Cli {
    #[arg(long, global = true, help = "Emit stable JSON output")]
    pub json: bool,
    #[command(subcommand)]
    pub command: CliCommand,
}

#[derive(Subcommand, Debug)]
pub enum CliCommand {
    InspectConfig,
    Profile,
    Validate { server: String },
    DiagnoseRuntime { server: String },
    SuggestFixes,
    Serve,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Warn,
    Fail,
}

impl Status {
    fn merge(self, other: Status) -> Status {
        match (self, other) {
            (Status::Fail, _) | (_, Status::Fail) => Status::Fail,
            (Status::Warn, _) | (_, Status::Warn) => Status::Warn,
            _ => Status::Pass,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TimingReport {
    pub process_spawn_ms: Option<u128>,
    pub first_stdout_ms: Option<u128>,
    pub first_stderr_ms: Option<u128>,
    pub initialize_ms: Option<u128>,
    pub tools_list_ms: Option<u128>,
    pub total_profile_ms: Option<u128>,
    pub timeout_stage: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerReport {
    pub name: String,
    pub status: Status,
    pub config_source: String,
    pub source_kind: String,
    pub command: Option<String>,
    pub args_redacted: Vec<String>,
    pub cwd: Option<String>,
    pub env_keys_only: Vec<String>,
    pub timings: TimingReport,
    pub evidence: Vec<String>,
    pub probable_cause: Option<String>,
    pub suggested_fix: Option<String>,
    pub safe_config_snippet: Option<String>,
    pub risk: Option<String>,
    pub managed: bool,
    pub effective: bool,
    pub overwritten_by: Option<String>,
    pub fingerprint: String,
}

impl ServerReport {
    fn warn(&mut self, evidence: impl Into<String>) {
        self.status = self.status.merge(Status::Warn);
        self.evidence.push(evidence.into());
    }

    fn fail(&mut self, evidence: impl Into<String>) {
        self.status = self.status.merge(Status::Fail);
        self.evidence.push(evidence.into());
    }

    fn set_cause_if_empty(&mut self, cause: impl Into<String>) {
        if self.probable_cause.is_none() {
            self.probable_cause = Some(cause.into());
        }
    }

    fn set_fix_if_empty(&mut self, fix: impl Into<String>) {
        if self.suggested_fix.is_none() {
            self.suggested_fix = Some(fix.into());
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiagnosticEnvelope {
    pub schema_version: String,
    pub status: Status,
    pub config_error_blocks_enumeration: bool,
    pub incomplete_effective_surface: bool,
    pub notices: Vec<String>,
    pub servers: Vec<ServerReport>,
}

impl DiagnosticEnvelope {
    fn new() -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            status: Status::Pass,
            config_error_blocks_enumeration: false,
            incomplete_effective_surface: false,
            notices: Vec::new(),
            servers: Vec::new(),
        }
    }

    fn recompute_status(&mut self) {
        self.status = self
            .servers
            .iter()
            .fold(Status::Pass, |status, report| status.merge(report.status));
        if self.status == Status::Pass && !self.notices.is_empty() {
            self.status = Status::Warn;
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TransportKind {
    Stdio,
    Http,
    PolicyOverride,
    Unknown,
}

#[derive(Clone, Debug)]
struct ServerConfig {
    name: String,
    source: PathBuf,
    source_kind: String,
    base_dir: PathBuf,
    transport: TransportKind,
    command: Option<String>,
    args: Vec<String>,
    cwd: Option<String>,
    env_literals: BTreeMap<String, String>,
    env_vars: Vec<String>,
    http_headers: BTreeMap<String, String>,
    env_http_headers: BTreeMap<String, String>,
    token_env_vars: Vec<String>,
    url: Option<String>,
    sensitive_literals: BTreeMap<String, String>,
    policy_plugin: Option<String>,
    policy_server: Option<String>,
    plugin_aliases: Vec<String>,
    enabled: bool,
    managed: bool,
    effective: bool,
    overwritten_by: Option<String>,
}

impl ServerConfig {
    fn env_keys(&self) -> Vec<String> {
        let mut keys = BTreeSet::new();
        for key in self.env_literals.keys() {
            keys.insert(key.clone());
        }
        for key in &self.env_vars {
            keys.insert(key.clone());
        }
        for key in self.http_headers.keys() {
            keys.insert(key.clone());
        }
        for key in self.env_http_headers.values() {
            keys.insert(key.clone());
        }
        for key in &self.token_env_vars {
            keys.insert(key.clone());
        }
        keys.into_iter().collect()
    }

    fn secret_values(&self) -> Vec<String> {
        self.env_literals
            .iter()
            .chain(self.http_headers.iter())
            .chain(self.sensitive_literals.iter())
            .map(|(_key, value)| value.clone())
            .collect()
    }
}

#[derive(Clone, Debug, Default)]
struct Discovery {
    servers: Vec<ServerConfig>,
    notices: Vec<String>,
    fatal_config_error: bool,
    incomplete_effective_surface: bool,
}

#[derive(Clone, Debug)]
enum RunMode {
    StaticAll,
    ProfileAll,
    StaticOne(String),
    ProfileOne(String),
}

pub async fn run_cli(cli: Cli) -> Result<i32> {
    match cli.command {
        CliCommand::Serve => {
            run_mcp_stdio_server().await?;
            Ok(0)
        }
        CliCommand::InspectConfig => {
            let envelope = build_diagnostics(RunMode::StaticAll).await?;
            write_output(&envelope, cli.json)?;
            Ok(exit_for(&envelope))
        }
        CliCommand::Profile => {
            let envelope = build_diagnostics(RunMode::ProfileAll).await?;
            write_output(&envelope, cli.json)?;
            Ok(exit_for(&envelope))
        }
        CliCommand::Validate { server } => {
            let envelope = build_diagnostics(RunMode::ProfileOne(server)).await?;
            write_output(&envelope, cli.json)?;
            Ok(exit_for(&envelope))
        }
        CliCommand::DiagnoseRuntime { server } => {
            let envelope = build_diagnostics(RunMode::StaticOne(server)).await?;
            write_output(&envelope, cli.json)?;
            Ok(exit_for(&envelope))
        }
        CliCommand::SuggestFixes => {
            let envelope = build_diagnostics(RunMode::StaticAll).await?;
            write_output(&envelope, cli.json)?;
            Ok(exit_for(&envelope))
        }
    }
}

async fn build_diagnostics(mode: RunMode) -> Result<DiagnosticEnvelope> {
    let discovery = discover_codex_surface();
    let mut envelope = DiagnosticEnvelope::new();
    envelope.notices = discovery.notices;
    envelope.incomplete_effective_surface = discovery.incomplete_effective_surface;
    envelope.config_error_blocks_enumeration = discovery.fatal_config_error;

    let mut servers = discovery.servers;
    let target = match &mode {
        RunMode::StaticOne(name) | RunMode::ProfileOne(name) => Some(name.as_str()),
        _ => None,
    };
    if let Some(name) = target {
        servers.retain(|server| server.name == name);
        if servers.is_empty() {
            envelope.servers.push(missing_server_report(name));
            envelope.recompute_status();
            return Ok(envelope);
        }
    }

    envelope.servers = servers.iter().map(static_report_for).collect();
    apply_duplicate_diagnostics(&mut envelope.servers);
    for (report, server) in envelope.servers.iter_mut().zip(servers.iter()) {
        apply_static_diagnostics(report, server);
    }

    match mode {
        RunMode::ProfileAll | RunMode::ProfileOne(_) => {
            for (report, server) in envelope.servers.iter_mut().zip(servers.iter()) {
                profile_server(report, server).await;
            }
        }
        RunMode::StaticAll | RunMode::StaticOne(_) => {}
    }

    if discovery.fatal_config_error {
        envelope.notices.push(
            "primary Codex config could not be parsed; enumeration is incomplete".to_string(),
        );
    }
    envelope.recompute_status();
    Ok(envelope)
}

fn exit_for(envelope: &DiagnosticEnvelope) -> i32 {
    if envelope.config_error_blocks_enumeration {
        return 3;
    }
    if envelope
        .servers
        .iter()
        .any(|server| server.status == Status::Fail)
    {
        1
    } else {
        0
    }
}

fn write_output(envelope: &DiagnosticEnvelope, json_output: bool) -> Result<()> {
    if json_output {
        println!("{}", serde_json::to_string_pretty(envelope)?);
        return Ok(());
    }

    if envelope.servers.is_empty() {
        println!("CDXCore: no MCP servers discovered");
    }
    for notice in &envelope.notices {
        println!("Notice: {notice}");
    }
    if envelope.incomplete_effective_surface {
        println!("Notice: effective Codex MCP surface is best-effort and incomplete");
    }
    for report in &envelope.servers {
        println!("Server: {}", report.name);
        println!("Status: {}", status_text(report.status));
        if let Some(cause) = &report.probable_cause {
            println!("Cause: {cause}");
        }
        if let Some(evidence) = report.evidence.first() {
            println!("Evidence: {evidence}");
        }
        if let Some(fix) = &report.suggested_fix {
            println!("Suggested fix: {fix}");
        }
        println!("Config source: {}", report.config_source);
        println!("Secrets: redacted");
        println!();
    }
    Ok(())
}

fn status_text(status: Status) -> &'static str {
    match status {
        Status::Pass => "pass",
        Status::Warn => "warn",
        Status::Fail => "fail",
    }
}

fn discover_codex_surface() -> Discovery {
    let mut discovery = Discovery::default();
    let user_config = codex_user_config_path();
    if user_config.exists() {
        match parse_codex_toml(&user_config, "codex_user_config") {
            Ok(mut servers) => discovery.servers.append(&mut servers),
            Err(err) => {
                discovery.fatal_config_error = true;
                discovery.incomplete_effective_surface = true;
                discovery.notices.push(format!(
                    "invalid Codex config {}: {err}",
                    display_path(&user_config),
                    err = redact_error(&err)
                ));
            }
        }
    } else {
        discovery.notices.push(format!(
            "missing Codex config file: {}",
            display_path(&user_config)
        ));
    }

    let project_config = env::current_dir()
        .ok()
        .map(|cwd| cwd.join(".codex").join("config.toml"));
    if let Some(path) = project_config {
        if path.exists() {
            match parse_codex_toml(&path, "codex_project_config") {
                Ok(mut servers) => discovery.servers.append(&mut servers),
                Err(err) => {
                    discovery.incomplete_effective_surface = true;
                    discovery.notices.push(format!(
                        "invalid project Codex config {}: {err}",
                        display_path(&path),
                        err = redact_error(&err)
                    ));
                }
            }
        }
    }

    let plugin_result = discover_plugin_servers();
    discovery.servers.extend(plugin_result.servers);
    discovery.notices.extend(plugin_result.notices);
    discovery.incomplete_effective_surface |= plugin_result.incomplete_effective_surface;
    apply_plugin_policy_overrides(&mut discovery.servers);
    discovery
        .notices
        .push("effective surface reconstruction is best-effort in v1; unproven plugin cache or bundled/injected provenance is marked incomplete instead of guessed".to_string());
    discovery.incomplete_effective_surface = true;
    discovery
}

fn apply_plugin_policy_overrides(servers: &mut [ServerConfig]) {
    let policies: Vec<(String, String, bool, String)> = servers
        .iter()
        .filter(|server| server.transport == TransportKind::PolicyOverride)
        .filter_map(|server| {
            Some((
                server.policy_plugin.clone()?,
                server.policy_server.clone()?,
                server.enabled,
                display_path(&server.source),
            ))
        })
        .collect();

    for server in servers {
        if server.transport == TransportKind::PolicyOverride {
            server.effective = false;
            continue;
        }
        if server.source_kind != "codex_plugin" {
            continue;
        }
        for (plugin, target_server, enabled, source) in &policies {
            if server.name == *target_server && plugin_matches(server, plugin) {
                server.enabled = *enabled;
                server.effective = *enabled;
                server.overwritten_by = Some(source.clone());
            }
        }
    }
}

fn plugin_matches(server: &ServerConfig, plugin: &str) -> bool {
    server.policy_plugin.as_deref() == Some(plugin)
        || server.plugin_aliases.iter().any(|alias| alias == plugin)
}

fn codex_user_config_path() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
        .join("config.toml")
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(PathBuf::from))
}

fn parse_codex_toml(path: &Path, source_kind: &str) -> Result<Vec<ServerConfig>> {
    let text = fs::read_to_string(path).with_context(|| format!("read {}", display_path(path)))?;
    let value: toml::Value = toml::from_str(&text).context("parse TOML")?;
    let base_dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut out = Vec::new();
    if let Some(raw_servers) = value.get("mcp_servers") {
        let servers = raw_servers
            .as_table()
            .ok_or_else(|| anyhow!("mcp_servers must be a table"))?;
        for (name, value) in servers {
            let table = value
                .as_table()
                .ok_or_else(|| anyhow!("mcp_servers.{name} must be a table"))?;
            out.push(server_from_toml_table(
                name,
                table,
                path,
                source_kind,
                &base_dir,
            ));
        }
    }
    out.extend(plugin_policy_overrides_from_toml(&value, path, &base_dir));
    Ok(out)
}

fn server_from_toml_table(
    name: &str,
    table: &toml::map::Map<String, toml::Value>,
    source: &Path,
    source_kind: &str,
    base_dir: &Path,
) -> ServerConfig {
    let command = table
        .get("command")
        .and_then(toml::Value::as_str)
        .map(str::to_string);
    let args = string_array_from_toml(table.get("args"));
    let cwd = table
        .get("cwd")
        .and_then(toml::Value::as_str)
        .map(str::to_string);
    let url = table
        .get("url")
        .and_then(toml::Value::as_str)
        .map(str::to_string);
    let transport = transport_from(
        table.get("type").and_then(toml::Value::as_str),
        command.as_deref(),
        url.as_deref(),
    );
    let env_literals = string_map_from_toml(table.get("env"));
    let env_vars = env_var_names_from_toml(table.get("env_vars"));
    let http_headers = string_map_from_toml(table.get("http_headers"));
    let env_http_headers = string_map_from_toml(table.get("env_http_headers"));
    let token_env_vars = table
        .get("bearer_token_env_var")
        .and_then(toml::Value::as_str)
        .map(|name| vec![name.to_string()])
        .unwrap_or_default();
    let enabled = table
        .get("enabled")
        .and_then(toml::Value::as_bool)
        .unwrap_or(true);
    ServerConfig {
        name: name.to_string(),
        source: source.to_path_buf(),
        source_kind: source_kind.to_string(),
        base_dir: base_dir.to_path_buf(),
        transport,
        command,
        args,
        cwd,
        env_literals,
        env_vars,
        http_headers,
        env_http_headers,
        token_env_vars,
        url,
        sensitive_literals: sensitive_literals_from_toml(table),
        policy_plugin: None,
        policy_server: None,
        plugin_aliases: Vec::new(),
        enabled,
        managed: is_known_managed_name(name),
        effective: enabled,
        overwritten_by: None,
    }
}

fn plugin_policy_overrides_from_toml(
    value: &toml::Value,
    source: &Path,
    base_dir: &Path,
) -> Vec<ServerConfig> {
    let Some(plugins) = value.get("plugins").and_then(toml::Value::as_table) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (plugin, plugin_value) in plugins {
        let Some(mcp_servers) = plugin_value
            .get("mcp_servers")
            .and_then(toml::Value::as_table)
        else {
            continue;
        };
        for (server, policy) in mcp_servers {
            let enabled = policy
                .as_table()
                .and_then(|table| table.get("enabled"))
                .and_then(toml::Value::as_bool)
                .unwrap_or(true);
            out.push(ServerConfig {
                name: format!("{plugin}/{server}"),
                source: source.to_path_buf(),
                source_kind: "codex_plugin_policy_override".to_string(),
                base_dir: base_dir.to_path_buf(),
                transport: TransportKind::PolicyOverride,
                command: None,
                args: Vec::new(),
                cwd: None,
                env_literals: BTreeMap::new(),
                env_vars: Vec::new(),
                http_headers: BTreeMap::new(),
                env_http_headers: BTreeMap::new(),
                token_env_vars: Vec::new(),
                url: None,
                sensitive_literals: BTreeMap::new(),
                policy_plugin: Some(plugin.clone()),
                policy_server: Some(server.clone()),
                plugin_aliases: Vec::new(),
                enabled,
                managed: false,
                effective: false,
                overwritten_by: None,
            });
        }
    }
    out
}

fn string_array_from_toml(value: Option<&toml::Value>) -> Vec<String> {
    value
        .and_then(toml::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(toml::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn env_var_names_from_toml(value: Option<&toml::Value>) -> Vec<String> {
    value
        .and_then(toml::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.as_str().map(str::to_string).or_else(|| {
                        item.as_table()
                            .and_then(|table| table.get("name"))
                            .and_then(toml::Value::as_str)
                            .map(str::to_string)
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn string_map_from_toml(value: Option<&toml::Value>) -> BTreeMap<String, String> {
    value
        .and_then(toml::Value::as_table)
        .map(|table| {
            table
                .iter()
                .map(|(key, value)| {
                    let value = value
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| value.to_string());
                    (key.clone(), value)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn sensitive_literals_from_toml(
    table: &toml::map::Map<String, toml::Value>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(value) = table.get("bearer_token").and_then(toml::Value::as_str) {
        out.insert("bearer_token".to_string(), value.to_string());
    }
    if let Some(value) = table.get("oauth_resource").and_then(toml::Value::as_str) {
        out.insert("oauth_resource".to_string(), value.to_string());
    }
    if let Some(oauth) = table.get("oauth").and_then(toml::Value::as_table) {
        for (key, value) in oauth {
            if let Some(value) = value.as_str() {
                out.insert(format!("oauth.{key}"), value.to_string());
            }
        }
    }
    out
}

fn discover_plugin_servers() -> Discovery {
    let mut discovery = Discovery::default();
    let Some(home) = home_dir() else {
        discovery.incomplete_effective_surface = true;
        discovery
            .notices
            .push("could not locate home directory for plugin cache discovery".to_string());
        return discovery;
    };
    let roots = [
        home.join(".codex").join("plugins").join("cache"),
        home.join(".agents").join("plugins"),
    ];
    for root in roots {
        if !root.exists() {
            discovery.incomplete_effective_surface = true;
            discovery.notices.push(format!(
                "plugin cache root not found: {}",
                display_path(&root)
            ));
            continue;
        }
        let manifests = find_plugin_manifests(&root, 7, 2_000);
        for manifest in manifests {
            match plugin_mcp_path_from_manifest(&manifest) {
                Ok(Some(mcp_path)) => match parse_plugin_mcp_json(&mcp_path, &manifest) {
                    Ok(mut servers) => discovery.servers.append(&mut servers),
                    Err(err) => {
                        discovery.incomplete_effective_surface = true;
                        discovery.notices.push(format!(
                            "invalid plugin MCP config {}: {err}",
                            display_path(&mcp_path),
                            err = redact_error(&err)
                        ));
                    }
                },
                Ok(None) => {}
                Err(err) => {
                    discovery.incomplete_effective_surface = true;
                    discovery.notices.push(format!(
                        "invalid plugin manifest {}: {err}",
                        display_path(&manifest),
                        err = redact_error(&err)
                    ));
                }
            }
        }
    }
    discovery
}

fn find_plugin_manifests(root: &Path, max_depth: usize, max_entries: usize) -> Vec<PathBuf> {
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut manifests = Vec::new();
    let mut seen = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        if depth > max_depth || seen >= max_entries {
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            let path = entry.path();
            if path.ends_with(Path::new(".codex-plugin").join("plugin.json")) {
                manifests.push(path);
                continue;
            }
            if is_real_dir(&path) && depth < max_depth {
                stack.push((path, depth + 1));
            }
            if seen >= max_entries {
                break;
            }
        }
    }
    manifests
}

fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| {
            let file_type = metadata.file_type();
            file_type.is_dir() && !file_type.is_symlink()
        })
        .unwrap_or(false)
}

fn plugin_mcp_path_from_manifest(manifest_path: &Path) -> Result<Option<PathBuf>> {
    let text = fs::read_to_string(manifest_path)?;
    let value: JsonValue = serde_json::from_str(&text)?;
    let plugin_root = manifest_path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| anyhow!("manifest is not under .codex-plugin"))?;
    if let Some(mcp) = value.get("mcpServers").and_then(JsonValue::as_str) {
        let mcp_path = resolve_manifest_path(plugin_root, mcp);
        if mcp_path.exists() {
            return Ok(Some(mcp_path));
        }
        return Ok(None);
    }
    let default = plugin_root.join(".mcp.json");
    if default.exists() {
        Ok(Some(default))
    } else {
        Ok(None)
    }
}

fn resolve_manifest_path(plugin_root: &Path, raw: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        plugin_root.join(path)
    }
}

fn parse_plugin_mcp_json(path: &Path, manifest_path: &Path) -> Result<Vec<ServerConfig>> {
    let text = fs::read_to_string(path)?;
    let value: JsonValue = serde_json::from_str(&text)?;
    let servers = plugin_servers_from_json(&value)?;
    let plugin_root = manifest_path
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")).to_path_buf());
    let plugin_aliases = plugin_aliases_from_manifest(manifest_path, &plugin_root);
    let plugin_id = plugin_aliases.first().cloned();
    let mut out = Vec::new();
    for (name, value) in servers {
        let Some(obj) = value.as_object() else {
            return Err(anyhow!("plugin MCP server {name} must be an object"));
        };
        out.push(server_from_json_object(
            name,
            obj,
            path,
            "codex_plugin",
            &plugin_root,
            plugin_id.clone(),
            plugin_aliases.clone(),
        ));
    }
    Ok(out)
}

fn plugin_servers_from_json(value: &JsonValue) -> Result<&serde_json::Map<String, JsonValue>> {
    if let Some(mcp_servers) = value.get("mcpServers") {
        return mcp_servers
            .as_object()
            .ok_or_else(|| anyhow!("mcpServers must be an object"));
    }
    if let Some(mcp_servers) = value.get("mcp_servers") {
        return mcp_servers
            .as_object()
            .ok_or_else(|| anyhow!("mcp_servers must be an object"));
    }
    value
        .as_object()
        .ok_or_else(|| anyhow!("expected object, mcpServers object, or mcp_servers object"))
}

fn plugin_aliases_from_manifest(manifest_path: &Path, plugin_root: &Path) -> Vec<String> {
    let mut aliases = Vec::new();
    if let Ok(text) = fs::read_to_string(manifest_path) {
        if let Ok(value) = serde_json::from_str::<JsonValue>(&text) {
            if let Some(name) = value.get("name").and_then(JsonValue::as_str) {
                push_unique_alias(&mut aliases, name);
            }
        }
    }
    if let Some(name) = plugin_root.file_name().and_then(OsStr::to_str) {
        push_unique_alias(&mut aliases, name);
    }
    let parent = plugin_root
        .parent()
        .and_then(Path::file_name)
        .and_then(OsStr::to_str);
    let grandparent = plugin_root
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(OsStr::to_str);
    if let (Some(parent), Some(name)) = (parent, plugin_root.file_name().and_then(OsStr::to_str)) {
        push_unique_alias(&mut aliases, &format!("{parent}/{name}"));
    }
    if let (Some(grandparent), Some(parent)) = (grandparent, parent) {
        push_unique_alias(&mut aliases, &format!("{grandparent}/{parent}"));
    }
    aliases
}

fn push_unique_alias(aliases: &mut Vec<String>, alias: &str) {
    if !alias.is_empty() && !aliases.iter().any(|existing| existing == alias) {
        aliases.push(alias.to_string());
    }
}

fn server_from_json_object(
    name: &str,
    obj: &serde_json::Map<String, JsonValue>,
    source: &Path,
    source_kind: &str,
    base_dir: &Path,
    plugin_id: Option<String>,
    plugin_aliases: Vec<String>,
) -> ServerConfig {
    let command = obj
        .get("command")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let args = string_array_from_json(obj.get("args"));
    let cwd = obj
        .get("cwd")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let url = obj
        .get("url")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let transport = transport_from(
        obj.get("type").and_then(JsonValue::as_str),
        command.as_deref(),
        url.as_deref(),
    );
    let enabled = obj
        .get("enabled")
        .and_then(JsonValue::as_bool)
        .unwrap_or(true);
    ServerConfig {
        name: name.to_string(),
        source: source.to_path_buf(),
        source_kind: source_kind.to_string(),
        base_dir: base_dir.to_path_buf(),
        transport,
        command,
        args,
        cwd,
        env_literals: string_map_from_json(obj.get("env")),
        env_vars: env_var_names_from_json(obj.get("env_vars")),
        http_headers: string_map_from_json(obj.get("http_headers")),
        env_http_headers: string_map_from_json(obj.get("env_http_headers")),
        token_env_vars: obj
            .get("bearer_token_env_var")
            .and_then(JsonValue::as_str)
            .map(|name| vec![name.to_string()])
            .unwrap_or_default(),
        url,
        sensitive_literals: sensitive_literals_from_json(obj),
        policy_plugin: plugin_id,
        policy_server: None,
        plugin_aliases,
        enabled,
        managed: is_known_managed_name(name),
        effective: enabled,
        overwritten_by: None,
    }
}

fn string_array_from_json(value: Option<&JsonValue>) -> Vec<String> {
    value
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(JsonValue::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn env_var_names_from_json(value: Option<&JsonValue>) -> Vec<String> {
    value
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.as_str().map(str::to_string).or_else(|| {
                        item.as_object()
                            .and_then(|obj| obj.get("name"))
                            .and_then(JsonValue::as_str)
                            .map(str::to_string)
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn string_map_from_json(value: Option<&JsonValue>) -> BTreeMap<String, String> {
    value
        .and_then(JsonValue::as_object)
        .map(|obj| {
            obj.iter()
                .map(|(key, value)| {
                    let value = value
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| value.to_string());
                    (key.clone(), value)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn sensitive_literals_from_json(
    obj: &serde_json::Map<String, JsonValue>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for key in [
        "bearer_token",
        "bearerToken",
        "oauth_resource",
        "oauthResource",
    ] {
        if let Some(value) = obj.get(key).and_then(JsonValue::as_str) {
            out.insert(key.to_string(), value.to_string());
        }
    }
    if let Some(oauth) = obj.get("oauth").and_then(JsonValue::as_object) {
        for (key, value) in oauth {
            if let Some(value) = value.as_str() {
                out.insert(format!("oauth.{key}"), value.to_string());
            }
        }
    }
    out
}

fn transport_from(raw: Option<&str>, command: Option<&str>, url: Option<&str>) -> TransportKind {
    match raw.unwrap_or_default() {
        "http" | "streamable_http" | "streamable-http" => TransportKind::Http,
        "stdio" => TransportKind::Stdio,
        "" if command.is_some() => TransportKind::Stdio,
        "" if url.is_some() => TransportKind::Http,
        _ => TransportKind::Unknown,
    }
}

fn static_report_for(server: &ServerConfig) -> ServerReport {
    let args_redacted = redact_args(&server.args);
    let env_keys_only = server.env_keys();
    let fingerprint = fingerprint_for(server, &args_redacted, &env_keys_only);
    let command = match server.transport {
        TransportKind::Http => server.url.as_deref().map(redact_url_for_report),
        _ => server.command.clone(),
    };
    ServerReport {
        name: server.name.clone(),
        status: Status::Pass,
        config_source: display_path(&server.source),
        source_kind: server.source_kind.clone(),
        command,
        args_redacted,
        cwd: server.cwd.clone(),
        env_keys_only,
        timings: TimingReport::default(),
        evidence: Vec::new(),
        probable_cause: None,
        suggested_fix: None,
        safe_config_snippet: None,
        risk: None,
        managed: server.managed,
        effective: server.effective,
        overwritten_by: server.overwritten_by.clone(),
        fingerprint,
    }
}

fn missing_server_report(name: &str) -> ServerReport {
    ServerReport {
        name: name.to_string(),
        status: Status::Fail,
        config_source: "<not found>".to_string(),
        source_kind: "missing".to_string(),
        command: None,
        args_redacted: Vec::new(),
        cwd: None,
        env_keys_only: Vec::new(),
        timings: TimingReport::default(),
        evidence: vec!["server name was not found in the discovered Codex MCP surface".to_string()],
        probable_cause: Some(
            "the requested MCP server is not configured or the effective surface is incomplete"
                .to_string(),
        ),
        suggested_fix: Some(
            "run cdxcore inspect-config --json and verify the server name and source provenance"
                .to_string(),
        ),
        safe_config_snippet: None,
        risk: Some(
            "validation could not run because there is no server config to profile".to_string(),
        ),
        managed: false,
        effective: false,
        overwritten_by: None,
        fingerprint: "missing".to_string(),
    }
}

fn apply_static_diagnostics(report: &mut ServerReport, server: &ServerConfig) {
    if server.transport == TransportKind::PolicyOverride {
        report.evidence.push(
            "plugin MCP policy override; launch transport remains owned by plugin .mcp.json"
                .to_string(),
        );
        return;
    }
    if !server.enabled {
        report.evidence.push(
            "server is disabled by Codex config or plugin policy; profiling skipped".to_string(),
        );
        report.set_cause_if_empty("server disabled");
        return;
    }
    if server.transport == TransportKind::Http {
        report.warn("HTTP/streamable HTTP server received static validation only in v1");
        report.set_cause_if_empty("v1 profiles stdio MCP servers only");
        report
            .set_fix_if_empty("verify HTTP reachability with the client that owns this MCP config");
    }
    if server.transport == TransportKind::Unknown {
        report.fail("server transport could not be inferred from command, url, or type");
        report.set_cause_if_empty("missing or unsupported MCP transport fields");
        report.set_fix_if_empty("set type = \"stdio\" with command/args or configure an HTTP url");
    }

    if let Some(cwd) = &server.cwd {
        let resolved = resolve_cwd(server, cwd);
        if !resolved.is_dir() {
            report.fail(format!(
                "cwd does not exist or is not a directory: {}",
                display_path(&resolved)
            ));
            report.set_cause_if_empty("bad working directory");
            report.set_fix_if_empty(
                "create the directory or update cwd to an existing absolute path",
            );
            report.safe_config_snippet = Some(safe_cwd_snippet(&server.name));
        }
    }

    if let Some(command) = &server.command {
        let cwd = server.cwd.as_ref().map(|cwd| resolve_cwd(server, cwd));
        let client_path = env::var_os("PATH").unwrap_or_default();
        let pathext = env::var_os("PATHEXT").unwrap_or_default();
        let client_resolution = resolve_command(command, cwd.as_deref(), &client_path, &pathext);
        if client_resolution.is_none() {
            if let Some(system_probe) = system_shell_probe() {
                let system_resolution = resolve_command(
                    command,
                    cwd.as_deref(),
                    &system_probe.path,
                    &system_probe.pathext,
                );
                if system_resolution.is_some() {
                    report.fail(format!(
                        "{command} resolves outside client PATH but not in CDXCore client_path"
                    ));
                    report.set_cause_if_empty(format!("{command} not found from GUI/client PATH"));
                    report.set_fix_if_empty(
                        "use an absolute executable path or add PATH in the MCP env block",
                    );
                    report.safe_config_snippet = Some(safe_command_snippet(&server.name));
                } else {
                    report.fail(format!("{command} does not resolve from client PATH"));
                    report.set_cause_if_empty("command does not exist or is not on PATH");
                    report.set_fix_if_empty(
                        "install the command or configure an absolute executable path",
                    );
                    report.safe_config_snippet = Some(safe_command_snippet(&server.name));
                }
            } else {
                report.fail(format!("{command} does not resolve from client PATH"));
                report.set_cause_if_empty("command does not exist or is not on PATH");
                report.set_fix_if_empty(
                    "install the command or configure an absolute executable path",
                );
                report.safe_config_snippet = Some(safe_command_snippet(&server.name));
            }
        } else if is_standalone_terminal_run() {
            report.warn("PATH check is based on this process; standalone terminal runs may not match Codex GUI PATH");
            report.set_cause_if_empty("PATH provenance caveat");
        }
    } else if server.transport == TransportKind::Stdio {
        report.fail("stdio server is missing command");
        report.set_cause_if_empty("missing MCP command");
        report.set_fix_if_empty("add a command field for this stdio MCP server");
        report.safe_config_snippet = Some(safe_command_snippet(&server.name));
    }

    for key in env_var_references(server) {
        if env::var_os(key).is_none() {
            report.warn(format!(
                "env var {key} is referenced by config but missing from this process"
            ));
            report.set_cause_if_empty("missing environment variable in client process");
            report.set_fix_if_empty("set the variable in the launching client environment or put a placeholder-backed value in the MCP env block");
            report.safe_config_snippet = Some(safe_env_snippet(&server.name, key));
        }
    }

    for (key, value) in &server.env_literals {
        if value.trim().is_empty() {
            report.warn(format!("env key {key} has an empty literal value"));
            report.set_cause_if_empty("suspicious empty environment value");
        } else if is_secretish(key) || looks_secretish_value(value) {
            report.warn(format!(
                "env key {key} contains a literal value; value redacted"
            ));
            report.set_cause_if_empty("literal secret-like value in MCP config");
            report.set_fix_if_empty("move secrets to a dedicated environment variable and reference that name from config");
            report.safe_config_snippet = Some(safe_env_snippet(&server.name, key));
        }
    }

    for (key, value) in &server.sensitive_literals {
        if is_secretish(key) || looks_secretish_value(value) {
            report.warn(format!("{key} contains a literal value; value redacted"));
            report.set_cause_if_empty("literal secret-like value in MCP config");
            report.set_fix_if_empty(
                "move secret-like OAuth or bearer values to a dedicated environment variable",
            );
        }
    }

    for (key, value) in &server.http_headers {
        if is_secretish(key) || looks_secretish_value(value) {
            report.warn(format!(
                "HTTP header {key} contains a literal value; value redacted"
            ));
            report.set_cause_if_empty("literal secret-like HTTP header in MCP config");
            report.set_fix_if_empty("move header secrets into env_http_headers or a client-managed environment variable");
        }
    }

    if is_heavyweight_server(&server.name, server.command.as_deref()) {
        report.warn("server appears to be an eager heavyweight MCP server");
        report.risk = Some(
            "browser/node_repl/computer-use style servers can slow sessions that do not need them"
                .to_string(),
        );
    }

    if is_known_managed_name(&server.name) && server.source_kind.contains("config") {
        report.warn("server name resembles a bundled or injected Codex MCP server");
        report.set_cause_if_empty(
            "managed/bundled MCP settings may appear editable but be overwritten by the client",
        );
        report.set_fix_if_empty(
            "prefer plugin/client settings for managed servers instead of editing generated config",
        );
    }
}

fn env_var_references(server: &ServerConfig) -> Vec<&str> {
    server
        .env_vars
        .iter()
        .map(String::as_str)
        .chain(server.env_http_headers.values().map(String::as_str))
        .chain(server.token_env_vars.iter().map(String::as_str))
        .collect()
}

fn apply_duplicate_diagnostics(reports: &mut [ServerReport]) {
    let mut names: HashMap<String, Vec<usize>> = HashMap::new();
    let mut fingerprints: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, report) in reports.iter().enumerate() {
        names.entry(report.name.clone()).or_default().push(idx);
        fingerprints
            .entry(report.fingerprint.clone())
            .or_default()
            .push(idx);
    }
    for indexes in names.values().filter(|indexes| indexes.len() > 1) {
        let sources: Vec<String> = indexes
            .iter()
            .map(|idx| reports[*idx].source_kind.clone())
            .collect();
        for idx in indexes {
            let report = &mut reports[*idx];
            report.warn(format!(
                "server name appears in multiple discovered sources: {}",
                sources.join(", ")
            ));
            report.set_cause_if_empty("source/name collision in effective MCP surface");
            report
                .set_fix_if_empty("keep one owner for this server name or rename one config entry");
        }
    }
    for indexes in fingerprints.values().filter(|indexes| indexes.len() > 1) {
        for idx in indexes {
            let report = &mut reports[*idx];
            report.warn("another server has the same runtime fingerprint");
            report.set_cause_if_empty("duplicate runtime MCP server identity");
            report.set_fix_if_empty(
                "remove or rename duplicate server definitions that launch the same command/url",
            );
        }
    }
}

async fn profile_server(report: &mut ServerReport, server: &ServerConfig) {
    if !server.enabled {
        return;
    }
    if report.status == Status::Fail && server.command.is_none() {
        return;
    }
    if server.transport != TransportKind::Stdio {
        return;
    }
    let Some(command) = &server.command else {
        return;
    };
    if resolves_to_self(command, server).unwrap_or(false) {
        report.warn("self-profiling recursion guard skipped launching CDXCore itself");
        report.set_cause_if_empty("configured server resolves to the cdxcore binary");
        report.set_fix_if_empty(
            "do not profile CDXCore from CDXCore; validate the plugin entry from the Codex client",
        );
        return;
    }

    let start = Instant::now();
    let mut cmd = Command::new(command);
    cmd.args(&server.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = &server.cwd {
        cmd.current_dir(resolve_cwd(server, cwd));
    }
    for (key, value) in &server.env_literals {
        cmd.env(key, value);
    }

    let spawn_started = Instant::now();
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            report.fail(format!("process failed before MCP initialize: {err}"));
            report.set_cause_if_empty("server process could not spawn");
            report.set_fix_if_empty("fix the command, cwd, or PATH used by the launching client");
            report.timings.total_profile_ms = Some(start.elapsed().as_millis());
            return;
        }
    };
    report.timings.process_spawn_ms = Some(spawn_started.elapsed().as_millis());

    let mut stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            report.fail("child stdin was unavailable");
            kill_child(&mut child).await;
            return;
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            report.fail("child stdout was unavailable");
            kill_child(&mut child).await;
            return;
        }
    };
    let stderr = child.stderr.take();
    let secret_values = server.secret_values();
    let stderr_secret_values = secret_values.clone();
    let stderr_task = stderr.map(|stderr| {
        let stderr_start = start;
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            match lines.next_line().await {
                Ok(Some(line)) => Some((
                    stderr_start.elapsed().as_millis(),
                    redact_text(&line, &stderr_secret_values),
                )),
                _ => None,
            }
        })
    });

    let mut stdout_lines = BufReader::new(stdout).lines();
    let initialize_started = Instant::now();
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {
                "name": "cdxcore",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    });
    if write_json_line(&mut stdin, &initialize).await.is_err() {
        report.fail("server stdin closed before MCP initialize");
        report.set_cause_if_empty("server exited before MCP handshake");
        kill_child(&mut child).await;
        return;
    }
    match timeout(
        INIT_TIMEOUT,
        read_response(
            &mut stdout_lines,
            1,
            start,
            &mut report.timings.first_stdout_ms,
            &secret_values,
        ),
    )
    .await
    {
        Ok(Ok(_)) => {
            report.timings.initialize_ms = Some(initialize_started.elapsed().as_millis());
        }
        Ok(Err(err)) => {
            report.fail(err);
            report.set_cause_if_empty("server exited before MCP handshake");
            report.set_fix_if_empty("inspect server stderr and verify it speaks MCP over stdio");
            report.timings.total_profile_ms = Some(start.elapsed().as_millis());
            finish_stderr(report, stderr_task).await;
            kill_child(&mut child).await;
            return;
        }
        Err(_) => {
            report.fail("initialize timed out");
            report.set_cause_if_empty("initialize timeout");
            report.set_fix_if_empty(
                "reduce server startup work or raise the client startup timeout intentionally",
            );
            report.timings.timeout_stage = Some("initialize".to_string());
            report.timings.total_profile_ms = Some(start.elapsed().as_millis());
            finish_stderr(report, stderr_task).await;
            kill_child(&mut child).await;
            return;
        }
    }

    let _ = write_json_line(
        &mut stdin,
        &json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}),
    )
    .await;

    let tools_started = Instant::now();
    let mut cursor: Option<String> = None;
    let mut seen_cursors = HashSet::new();
    let mut request_id = 2u64;
    for _ in 0..MAX_TOOL_PAGES {
        let params = cursor
            .as_ref()
            .map(|cursor| json!({"cursor": cursor}))
            .unwrap_or_else(|| json!({}));
        let request = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "tools/list",
            "params": params
        });
        if write_json_line(&mut stdin, &request).await.is_err() {
            report.fail("server stdin closed before tools/list completed");
            report.set_cause_if_empty("server exited before tools/list");
            break;
        }
        let response = match timeout(
            TOOLS_TIMEOUT,
            read_response(
                &mut stdout_lines,
                request_id,
                start,
                &mut report.timings.first_stdout_ms,
                &secret_values,
            ),
        )
        .await
        {
            Ok(Ok(value)) => value,
            Ok(Err(err)) => {
                report.fail(err);
                report.set_cause_if_empty("tools/list failed");
                break;
            }
            Err(_) => {
                report.fail("tools/list timed out");
                report.set_cause_if_empty("tools/list timeout");
                report.set_fix_if_empty(
                    "defer heavyweight startup work and return tools/list quickly",
                );
                report.timings.timeout_stage = Some("tools_list".to_string());
                break;
            }
        };
        if let Some(tools) = response
            .get("result")
            .and_then(|result| result.get("tools"))
            .and_then(JsonValue::as_array)
        {
            for tool in tools {
                if tool.get("inputSchema").is_none() {
                    let name = tool
                        .get("name")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("<unnamed>");
                    report.fail(format!("tool {name} is missing inputSchema"));
                    report.set_cause_if_empty("tools/list returned invalid tool metadata");
                    report.set_fix_if_empty("add inputSchema to every tool returned by tools/list");
                }
            }
        }
        let next_cursor = response
            .get("result")
            .and_then(|result| result.get("nextCursor"))
            .and_then(JsonValue::as_str)
            .map(str::to_string);
        match next_cursor {
            Some(next) if seen_cursors.contains(&next) => {
                report.fail("tools/list pagination cursor loop detected");
                report.set_cause_if_empty("tools/list returned a repeated nextCursor");
                break;
            }
            Some(next) => {
                seen_cursors.insert(next.clone());
                cursor = Some(next);
                request_id += 1;
            }
            None => break,
        }
    }
    if cursor.is_some() && seen_cursors.len() >= MAX_TOOL_PAGES {
        report.fail("tools/list pagination exceeded CDXCore page limit");
        report.set_cause_if_empty(
            "tools/list pagination did not terminate within the bounded profiler",
        );
    }
    report.timings.tools_list_ms = Some(tools_started.elapsed().as_millis());
    report.timings.total_profile_ms = Some(start.elapsed().as_millis());
    if report.status == Status::Pass {
        report
            .evidence
            .push("MCP initialize and bounded tools/list completed".to_string());
    }
    finish_stderr(report, stderr_task).await;
    kill_child(&mut child).await;
}

async fn finish_stderr(
    report: &mut ServerReport,
    stderr_task: Option<tokio::task::JoinHandle<Option<(u128, String)>>>,
) {
    let Some(task) = stderr_task else {
        return;
    };
    if let Ok(Ok(Some((ms, line)))) = timeout(Duration::from_millis(50), task).await {
        report.timings.first_stderr_ms = Some(ms);
        if !line.trim().is_empty() {
            report.evidence.push(format!("early stderr: {line}"));
        }
    }
}

async fn kill_child(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn write_json_line(
    stdin: &mut tokio::process::ChildStdin,
    value: &JsonValue,
) -> io::Result<()> {
    stdin.write_all(value.to_string().as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await
}

async fn read_response(
    lines: &mut Lines<BufReader<ChildStdout>>,
    id: u64,
    start: Instant,
    first_stdout_ms: &mut Option<u128>,
    known_secret_values: &[String],
) -> std::result::Result<JsonValue, String> {
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|err| format!("failed reading stdout: {err}"))?
    {
        if first_stdout_ms.is_none() {
            *first_stdout_ms = Some(start.elapsed().as_millis());
        }
        let value: JsonValue = serde_json::from_str(&line)
            .map_err(|_| "non-JSON stdout before MCP response".to_string())?;
        if value.get("id").and_then(JsonValue::as_u64) == Some(id) {
            if let Some(error) = value.get("error") {
                return Err(format!(
                    "MCP request {id} returned error: {}",
                    redact_text(&error.to_string(), known_secret_values)
                ));
            }
            return Ok(value);
        }
    }
    Err("server exited before MCP response".to_string())
}

fn resolve_cwd(server: &ServerConfig, cwd: &str) -> PathBuf {
    let path = Path::new(cwd);
    if path.is_absolute() {
        path.to_path_buf()
    } else if server.source_kind == "codex_plugin" {
        server.base_dir.join(path)
    } else {
        env::current_dir()
            .unwrap_or_else(|_| server.base_dir.clone())
            .join(path)
    }
}

fn resolve_command(
    command: &str,
    cwd: Option<&Path>,
    path_env: &OsStr,
    pathext: &OsStr,
) -> Option<PathBuf> {
    let command_path = Path::new(command);
    if has_path_separator(command) {
        let base = if command_path.is_absolute() {
            command_path.to_path_buf()
        } else {
            cwd.unwrap_or_else(|| Path::new(".")).join(command_path)
        };
        return executable_candidates(&base, pathext)
            .into_iter()
            .find(|path| path.is_file());
    }
    for dir in env::split_paths(path_env) {
        for candidate in executable_candidates(&dir.join(command), pathext) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn executable_candidates(path: &Path, pathext: &OsStr) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        if path.extension().is_some() {
            return vec![path.to_path_buf()];
        }
        let raw = pathext.to_string_lossy();
        let mut out = Vec::new();
        for ext in raw.split(';').filter(|ext| !ext.is_empty()) {
            let ext = ext.trim_start_matches('.');
            out.push(path.with_extension(ext));
        }
        if out.is_empty() {
            out.push(path.with_extension("exe"));
            out.push(path.with_extension("cmd"));
        }
        out
    }
    #[cfg(not(windows))]
    {
        let _ = pathext;
        vec![path.to_path_buf()]
    }
}

fn has_path_separator(command: &str) -> bool {
    command.contains('/') || command.contains('\\')
}

struct SystemShellProbe {
    path: OsString,
    pathext: OsString,
}

fn system_shell_probe() -> Option<SystemShellProbe> {
    #[cfg(windows)]
    {
        let machine_path = registry_env_value(
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
            "Path",
        );
        let user_path = registry_env_value(r"HKCU\Environment", "Path");
        let joined_path = [machine_path, user_path]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(";");
        if joined_path.is_empty() {
            None
        } else {
            let machine_pathext = registry_env_value(
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
                "PATHEXT",
            );
            let user_pathext = registry_env_value(r"HKCU\Environment", "PATHEXT");
            let joined_pathext = [machine_pathext, user_pathext]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(";");
            let pathext = if joined_pathext.is_empty() {
                env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(".COM;.EXE;.BAT;.CMD"))
            } else {
                OsString::from(expand_windows_env_vars(&joined_pathext))
            };
            Some(SystemShellProbe {
                path: OsString::from(expand_windows_env_vars(&joined_path)),
                pathext,
            })
        }
    }
    #[cfg(not(windows))]
    {
        let output = std::process::Command::new("sh")
            .args(["-lc", "printf %s \"$PATH\""])
            .output()
            .ok()?;
        Some(SystemShellProbe {
            path: OsString::from(String::from_utf8_lossy(&output.stdout).to_string()),
            pathext: OsString::new(),
        })
    }
}

#[cfg(windows)]
fn registry_env_value(key: &str, value_name: &str) -> Option<String> {
    let output = std::process::Command::new("reg")
        .args(["query", key, "/v", value_name])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if !line.trim_start().starts_with(value_name) {
            continue;
        }
        if let Some(idx) = line.find("REG_") {
            let rest = &line[idx..];
            let mut parts = rest.splitn(2, char::is_whitespace);
            let _kind = parts.next();
            return parts
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
    }
    None
}

#[cfg(windows)]
fn expand_windows_env_vars(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        let mut name = String::new();
        while let Some(&next) = chars.peek() {
            chars.next();
            if next == '%' {
                break;
            }
            name.push(next);
        }
        if name.is_empty() {
            out.push('%');
        } else if let Ok(value) = env::var(&name) {
            out.push_str(&value);
        } else {
            out.push('%');
            out.push_str(&name);
            out.push('%');
        }
    }
    out
}

fn resolves_to_self(command: &str, server: &ServerConfig) -> Result<bool> {
    let cwd = server.cwd.as_ref().map(|cwd| resolve_cwd(server, cwd));
    let path = resolve_command(
        command,
        cwd.as_deref(),
        &env::var_os("PATH").unwrap_or_default(),
        &env::var_os("PATHEXT").unwrap_or_default(),
    );
    let Some(path) = path else {
        return Ok(false);
    };
    let current = env::current_exe()?;
    Ok(canonicalize_lossy(&path) == canonicalize_lossy(&current))
}

fn canonicalize_lossy(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn is_standalone_terminal_run() -> bool {
    env::var_os("CDXCORE_UNDER_CODEX_MCP").is_none()
}

fn fingerprint_for(server: &ServerConfig, args_redacted: &[String], env_keys: &[String]) -> String {
    let target = match server.transport {
        TransportKind::Stdio => server
            .command
            .clone()
            .unwrap_or_else(|| "<missing>".to_string()),
        TransportKind::Http => server
            .url
            .as_deref()
            .map(redact_url_for_report)
            .unwrap_or_else(|| "<missing>".to_string()),
        TransportKind::PolicyOverride => server.name.clone(),
        TransportKind::Unknown => "<unknown>".to_string(),
    };
    format!(
        "{:?}|{}|{}|{}|{}",
        server.transport,
        target,
        args_redacted.join("\u{1f}"),
        server.cwd.as_deref().unwrap_or(""),
        env_keys.join(",")
    )
}

fn is_heavyweight_server(name: &str, command: Option<&str>) -> bool {
    let haystack = format!("{} {}", name, command.unwrap_or_default()).to_ascii_lowercase();
    [
        "browser",
        "node_repl",
        "node-repl",
        "computer-use",
        "computer_use",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
}

fn is_known_managed_name(name: &str) -> bool {
    matches!(
        name,
        "browser" | "node_repl" | "node-repl" | "computer-use" | "computer_use"
    )
}

fn safe_command_snippet(name: &str) -> String {
    format!(
        "[mcp_servers.{name}]\ncommand = \"<absolute path>\"\nargs = []",
        name = toml_key(name)
    )
}

fn safe_cwd_snippet(name: &str) -> String {
    format!(
        "[mcp_servers.{name}]\ncwd = \"<absolute path>\"",
        name = toml_key(name)
    )
}

fn safe_env_snippet(name: &str, key: &str) -> String {
    let placeholder = if is_secretish(key) {
        "${TOKEN_ENV_VAR}"
    } else {
        "<safe value>"
    };
    format!(
        "[mcp_servers.{name}.env]\n{key} = \"{placeholder}\"",
        name = toml_key(name),
        key = toml_key(key)
    )
}

fn toml_key(key: &str) -> String {
    if key
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        key.to_string()
    } else {
        format!("{key:?}")
    }
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn redact_url_for_report(url: &str) -> String {
    let without_userinfo = redact_url_userinfo(url);
    let (without_fragment, fragment) = match without_userinfo.split_once('#') {
        Some((before, fragment)) => (before, Some(fragment)),
        None => (without_userinfo.as_str(), None),
    };
    let (base, query) = match without_fragment.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (without_fragment, None),
    };

    let mut redacted = redact_url_path(base);
    if let Some(query) = query {
        redacted.push('?');
        redacted.push_str(&redact_query_like(query));
    }
    if let Some(fragment) = fragment {
        redacted.push('#');
        redacted.push_str(&redact_url_fragment(fragment));
    }
    redacted
}

fn redact_url_userinfo(url: &str) -> String {
    let Some(scheme_idx) = url.find("://") else {
        return url.to_string();
    };
    let authority_start = scheme_idx + 3;
    let rest = &url[authority_start..];
    let authority_end = rest
        .find(|ch| ['/', '?', '#'].contains(&ch))
        .unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let Some(at_idx) = authority.rfind('@') else {
        return url.to_string();
    };
    format!(
        "{}<redacted>@{}",
        &url[..authority_start],
        &url[authority_start + at_idx + 1..]
    )
}

fn redact_url_path(base: &str) -> String {
    let Some(scheme_idx) = base.find("://") else {
        return redact_path_segments(base);
    };
    let authority_start = scheme_idx + 3;
    let rest = &base[authority_start..];
    let Some(path_idx) = rest.find('/') else {
        return base.to_string();
    };
    let path_start = authority_start + path_idx;
    format!(
        "{}{}",
        &base[..path_start],
        redact_path_segments(&base[path_start..])
    )
}

fn redact_path_segments(path: &str) -> String {
    let mut redact_next = false;
    let mut out = Vec::new();
    for segment in path.split('/') {
        if segment.is_empty() {
            out.push(segment.to_string());
            continue;
        }
        let decoded = percent_decode_lossy(segment);
        if redact_next || looks_secretish_value(segment) || looks_secretish_value(&decoded) {
            out.push("<redacted>".to_string());
            redact_next = false;
            continue;
        }
        if segment.contains('=') {
            out.push(redact_query_pair(segment));
            continue;
        }
        if should_redact_following_word(&decoded) {
            out.push(segment.to_string());
            redact_next = true;
            continue;
        }
        out.push(segment.to_string());
    }
    out.join("/")
}

fn redact_url_fragment(fragment: &str) -> String {
    if fragment.contains('=') || fragment.contains('&') {
        redact_query_like(fragment)
    } else {
        redact_path_segments(fragment)
    }
}

fn redact_query_like(query: &str) -> String {
    query
        .split('&')
        .map(redact_query_pair)
        .collect::<Vec<_>>()
        .join("&")
}

fn redact_query_pair(pair: &str) -> String {
    if let Some((key, value)) = pair.split_once('=') {
        let decoded_key = percent_decode_lossy(key);
        let decoded_value = percent_decode_lossy(value);
        if is_secretish(&decoded_key)
            || looks_secretish_value(value)
            || looks_secretish_value(&decoded_value)
        {
            return format!("{key}=<redacted>");
        }
        pair.to_string()
    } else if looks_secretish_value(pair) || looks_secretish_value(&percent_decode_lossy(pair)) {
        "<redacted>".to_string()
    } else {
        pair.to_string()
    }
}

fn percent_decode_lossy(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0;
    while idx < bytes.len() {
        if bytes[idx] == b'%' && idx + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_value(bytes[idx + 1]), hex_value(bytes[idx + 2]))
            {
                out.push((high << 4) | low);
                idx += 3;
                continue;
            }
        }
        out.push(bytes[idx]);
        idx += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn redact_args(args: &[String]) -> Vec<String> {
    let mut redact_next = false;
    let mut out = Vec::with_capacity(args.len());
    for arg in args {
        if redact_next {
            out.push("<redacted>".to_string());
            redact_next = false;
            continue;
        }
        if let Some((key, _value)) = arg.split_once('=') {
            if is_secretish(key) {
                out.push(format!("{key}=<redacted>"));
                continue;
            }
        }
        if is_secretish(arg) {
            if arg.starts_with('-') && !arg.contains('=') {
                out.push(arg.clone());
                redact_next = true;
            } else {
                out.push("<redacted>".to_string());
            }
        } else {
            out.push(arg.clone());
        }
    }
    out
}

fn redact_text(text: &str, known_secret_values: &[String]) -> String {
    let mut redacted = text.to_string();
    for value in known_secret_values {
        if value.len() > 2 {
            redacted = redacted.replace(value, "<redacted>");
        }
    }
    let mut words = Vec::new();
    let mut redact_next = false;
    let mut pending_secret_key = false;
    for word in redacted.split_whitespace() {
        if redact_next {
            words.push(redact_value_word(word));
            redact_next = should_redact_following_word(word);
            continue;
        }
        if pending_secret_key {
            pending_secret_key = false;
            if is_assignment_separator_word(word) {
                words.push(word.to_string());
                redact_next = true;
            } else {
                words.push(redact_value_word(word));
                if should_redact_following_word(word) {
                    redact_next = true;
                }
            }
            continue;
        }
        if let Some((key, _value)) = word.split_once('=') {
            if is_secretish(key) {
                if word.ends_with('=') {
                    words.push(word.to_string());
                    redact_next = true;
                } else {
                    words.push(format!("{key}=<redacted>"));
                }
                continue;
            }
        }
        if let Some((key, _value)) = word.split_once(':') {
            if is_secretish(key) {
                if word.ends_with(':') {
                    words.push(word.to_string());
                    redact_next = true;
                } else {
                    words.push(format!("{key}:<redacted>"));
                }
                continue;
            }
        }
        if should_redact_following_word(word) {
            words.push(word.to_string());
            pending_secret_key = true;
            continue;
        }
        if looks_secretish_value(normalized_value_word(word)) {
            words.push(redact_value_word(word));
            continue;
        }
        words.push(word.to_string());
    }
    words.join(" ")
}

fn redact_error(err: &anyhow::Error) -> String {
    redact_text(&format!("{err:#}"), &[])
}

fn is_assignment_separator_word(word: &str) -> bool {
    let trimmed = word.trim_matches(|ch: char| {
        ch.is_ascii_whitespace()
            || matches!(
                ch,
                '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
            )
    });
    matches!(trimmed, "=" | ":")
}

fn should_redact_following_word(word: &str) -> bool {
    let normalized = normalized_key_word(word);
    if normalized.is_empty() || normalized == "api" {
        return false;
    }
    is_secretish(normalized)
}

fn normalized_key_word(word: &str) -> &str {
    word.trim_matches(|ch: char| ch.is_ascii_punctuation() && ch != '-' && ch != '_' && ch != '.')
}

fn normalized_value_word(word: &str) -> &str {
    word.trim_matches(|ch: char| ch.is_ascii_punctuation() && ch != '-' && ch != '_' && ch != '.')
}

fn redact_value_word(word: &str) -> String {
    let leading = word
        .find(|ch: char| !is_wrapping_punctuation(ch))
        .unwrap_or(word.len());
    let trailing = word
        .rfind(|ch: char| !is_wrapping_punctuation(ch))
        .map(|idx| idx + 1)
        .unwrap_or(leading);
    format!("{}<redacted>{}", &word[..leading], &word[trailing..])
}

fn is_wrapping_punctuation(ch: char) -> bool {
    ch.is_ascii_punctuation() && ch != '-' && ch != '_' && ch != '.'
}

fn is_secretish(input: &str) -> bool {
    let tokens = identifier_tokens(input);
    if tokens.is_empty() {
        return false;
    }

    let compact = tokens.join("");
    if matches!(
        compact.as_str(),
        "authorization" | "apikey" | "bearertoken" | "clientsecret" | "sessionid"
    ) || compact.ends_with("token")
        || compact.ends_with("secret")
        || compact.ends_with("password")
        || compact.ends_with("credential")
        || compact.ends_with("apikey")
    {
        return true;
    }

    let has_secret_token = tokens.iter().any(|token| {
        (token.as_str() != "api" && SECRET_TERMS.contains(&token.as_str()))
            || token.starts_with("oauth")
    });
    if has_secret_token {
        return true;
    }

    tokens.len() == 1 && tokens[0] == "api"
}

fn identifier_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut prev_lower_or_digit = false;
    for ch in input.trim_start_matches('-').chars() {
        if !ch.is_ascii_alphanumeric() {
            push_token(&mut tokens, &mut current);
            prev_lower_or_digit = false;
            continue;
        }
        if ch.is_ascii_uppercase() && !current.is_empty() && prev_lower_or_digit {
            push_token(&mut tokens, &mut current);
        }
        current.push(ch.to_ascii_lowercase());
        prev_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
    }
    push_token(&mut tokens, &mut current);
    tokens
}

fn push_token(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(std::mem::take(current));
    }
}

fn looks_secretish_value(input: &str) -> bool {
    let trimmed = input.trim();
    let lower = trimmed.to_ascii_lowercase();
    if [
        "sk-",
        "ghp_",
        "gho_",
        "ghs_",
        "github_pat_",
        "glpat-",
        "xoxb-",
        "xoxp-",
        "hf_",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
    {
        return true;
    }
    trimmed.len() >= 24
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
}

async fn run_mcp_stdio_server() -> Result<()> {
    env::set_var("CDXCORE_UNDER_CODEX_MCP", "1");
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut lines = BufReader::new(stdin).lines();
    while let Some(line) = lines.next_line().await? {
        let Ok(request) = serde_json::from_str::<JsonValue>(&line) else {
            continue;
        };
        let Some(method) = request.get("method").and_then(JsonValue::as_str) else {
            continue;
        };
        let id = request.get("id").cloned();
        if id.is_none() {
            continue;
        }
        let response = match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": mcp_capabilities_json(),
                    "serverInfo": {
                        "name": "CDXCore",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "tools": mcp_tool_definitions() }
            }),
            "tools/call" => handle_mcp_tool_call(id, request.get("params")).await,
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "method not found" }
            }),
        };
        stdout.write_all(response.to_string().as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }
    Ok(())
}

fn mcp_tool_definitions() -> JsonValue {
    json!([
        {
            "name": "inspect_mcp_config",
            "description": "Read Codex MCP config and report static diagnostics without launching servers.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "profile_mcp_startup",
            "description": "Launch configured stdio MCP servers with short timeouts and report startup/handshake diagnostics.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "validate_mcp_server",
            "description": "Profile one configured stdio MCP server by name.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"],
                "additionalProperties": false
            }
        },
        {
            "name": "diagnose_runtime",
            "description": "Run static runtime diagnostics for one configured MCP server without launching it.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"],
                "additionalProperties": false
            }
        },
        {
            "name": "suggest_config_fixes",
            "description": "Return safe, redacted config fix suggestions without editing files.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }
    ])
}

fn mcp_capabilities_json() -> JsonValue {
    let _rmcp_capabilities = rmcp::model::ServerCapabilities::builder()
        .enable_tools()
        .build();
    json!({ "tools": {} })
}

async fn handle_mcp_tool_call(id: Option<JsonValue>, params: Option<&JsonValue>) -> JsonValue {
    let Some(name) = params
        .and_then(|params| params.get("name"))
        .and_then(JsonValue::as_str)
    else {
        return mcp_error(id, -32602, "missing tool name");
    };
    let args = params
        .and_then(|params| params.get("arguments"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let result = match name {
        "inspect_mcp_config" => build_diagnostics(RunMode::StaticAll).await,
        "profile_mcp_startup" => build_diagnostics(RunMode::ProfileAll).await,
        "validate_mcp_server" => {
            let Some(server) = args.get("name").and_then(JsonValue::as_str) else {
                return mcp_error(id, -32602, "missing name");
            };
            build_diagnostics(RunMode::ProfileOne(server.to_string())).await
        }
        "diagnose_runtime" => {
            let Some(server) = args.get("name").and_then(JsonValue::as_str) else {
                return mcp_error(id, -32602, "missing name");
            };
            build_diagnostics(RunMode::StaticOne(server.to_string())).await
        }
        "suggest_config_fixes" => build_diagnostics(RunMode::StaticAll).await,
        _ => return mcp_error(id, -32602, "unknown tool"),
    };
    match result {
        Ok(envelope) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&envelope).unwrap_or_else(|_| "{}".to_string())
                }],
                "isError": false
            }
        }),
        Err(err) => mcp_error(id, -32603, &format!("{err:#}")),
    }
}

fn mcp_error(id: Option<JsonValue>, code: i64, message: &str) -> JsonValue {
    let message = redact_text(message, &[]);
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn duplicate_toml_table_key_is_invalid_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
[mcp_servers.demo]
command = "one"

[mcp_servers.demo]
command = "two"
"#,
        )
        .unwrap();
        let err = parse_codex_toml(&path, "test").unwrap_err().to_string();
        assert!(err.contains("parse TOML"));
    }

    #[test]
    fn invalid_codex_mcp_server_shape_is_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "mcp_servers = []").unwrap();
        let err = parse_codex_toml(&path, "test").unwrap_err().to_string();
        assert!(err.contains("mcp_servers must be a table"));

        fs::write(&path, "[mcp_servers]\ndemo = \"bad\"").unwrap();
        let err = parse_codex_toml(&path, "test").unwrap_err().to_string();
        assert!(err.contains("mcp_servers.demo must be a table"));
    }

    #[test]
    fn parses_plugin_policy_overrides_from_codex_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
[plugins."sample@test".mcp_servers.sample]
enabled = false
"#,
        )
        .unwrap();
        let servers = parse_codex_toml(&path, "test").unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "sample@test/sample");
        assert_eq!(servers[0].source_kind, "codex_plugin_policy_override");
        assert_eq!(servers[0].transport, TransportKind::PolicyOverride);
    }

    #[test]
    fn parses_plugin_mcp_json_snake_case_wrapper() {
        let dir = tempdir().unwrap();
        let plugin_root = dir.path().join("plugin");
        let manifest_dir = plugin_root.join(".codex-plugin");
        fs::create_dir_all(&manifest_dir).unwrap();
        let manifest = manifest_dir.join("plugin.json");
        let mcp = plugin_root.join(".mcp.json");
        fs::write(&manifest, "{}").unwrap();
        fs::write(
            &mcp,
            r#"{"mcp_servers":{"demo":{"command":"demo-mcp","args":["--safe"]}}}"#,
        )
        .unwrap();
        let servers = parse_plugin_mcp_json(&mcp, &manifest).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "demo");
        assert_eq!(servers[0].command.as_deref(), Some("demo-mcp"));
    }

    #[test]
    fn redacts_args_and_inline_secret_values() {
        let args = vec![
            "--api-key".to_string(),
            "sk-test-1234567890".to_string(),
            "token=abc123".to_string(),
            "--capital-city".to_string(),
            "Austin".to_string(),
            "--api-base".to_string(),
            "https://example.test".to_string(),
            "--keyboard-layout".to_string(),
            "us".to_string(),
            "--safe".to_string(),
        ];
        assert_eq!(
            redact_args(&args),
            vec![
                "--api-key",
                "<redacted>",
                "token=<redacted>",
                "--capital-city",
                "Austin",
                "--api-base",
                "https://example.test",
                "--keyboard-layout",
                "us",
                "--safe"
            ]
        );
        let text = redact_text(
            "failed auth token=abc123 Bearer xyz token sk-STDERRLEAK-1234567890abcdef ghp_1234567890abcdef1234567890abcdef1234 capital=city keyboard:us",
            &["abc123".to_string()],
        );
        assert!(!text.contains("abc123"));
        assert!(!text.contains("xyz"));
        assert!(!text.contains("sk-STDERRLEAK-1234567890abcdef"));
        assert!(!text.contains("ghp_1234567890abcdef1234567890abcdef1234"));
        assert!(text.contains("capital=city"));
        assert!(text.contains("keyboard:us"));
    }

    #[test]
    fn redacts_separated_secret_assignments() {
        let marker_one = "plainliteral123";
        let marker_two = "otherliteral456";
        let marker_three = "thirdliteral789";
        let text = redact_text(
            &format!(
                "parse error token = \"{marker_one}\" and \"api_key\": \"{marker_two}\" Authorization: Bearer {marker_three} safe=value"
            ),
            &[],
        );

        assert!(!text.contains(marker_one));
        assert!(!text.contains(marker_two));
        assert!(!text.contains(marker_three));
        assert!(text.contains("safe=value"));
    }

    #[test]
    fn invalid_config_error_messages_are_redacted() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let marker = "plainliteral123";
        fs::write(
            &path,
            format!(
                r#"
[mcp_servers.demo]
env = {{ TOKEN = "{marker}" 
"#
            ),
        )
        .unwrap();
        let err = parse_codex_toml(&path, "test").unwrap_err();
        let redacted = redact_error(&err);

        assert!(!redacted.contains(marker));
        assert!(redacted.contains("<redacted>"));
    }

    #[test]
    fn env_literal_secret_never_reaches_report_fields() {
        let mut env_literals = BTreeMap::new();
        env_literals.insert(
            "NOTION_TOKEN".to_string(),
            "super-secret-token-value".to_string(),
        );
        let server = ServerConfig {
            name: "notion".to_string(),
            source: PathBuf::from("config.toml"),
            source_kind: "codex_user_config".to_string(),
            base_dir: PathBuf::from("."),
            transport: TransportKind::Stdio,
            command: Some("missing-cmd".to_string()),
            args: vec![
                "--token".to_string(),
                "super-secret-token-value".to_string(),
            ],
            cwd: None,
            env_literals,
            env_vars: Vec::new(),
            http_headers: BTreeMap::new(),
            env_http_headers: BTreeMap::new(),
            token_env_vars: Vec::new(),
            url: None,
            sensitive_literals: BTreeMap::new(),
            policy_plugin: None,
            policy_server: None,
            plugin_aliases: Vec::new(),
            enabled: true,
            managed: false,
            effective: true,
            overwritten_by: None,
        };
        let mut report = static_report_for(&server);
        apply_static_diagnostics(&mut report, &server);
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("super-secret-token-value"));
        assert!(json.contains("NOTION_TOKEN"));
        assert!(report
            .safe_config_snippet
            .as_deref()
            .unwrap_or_default()
            .contains("${TOKEN_ENV_VAR}"));
    }

    #[test]
    fn detects_bad_cwd() {
        let server = ServerConfig {
            name: "badcwd".to_string(),
            source: PathBuf::from("config.toml"),
            source_kind: "codex_user_config".to_string(),
            base_dir: PathBuf::from("."),
            transport: TransportKind::Stdio,
            command: Some("missing-cmd".to_string()),
            args: Vec::new(),
            cwd: Some("definitely-missing-cdxcore-dir".to_string()),
            env_literals: BTreeMap::new(),
            env_vars: Vec::new(),
            http_headers: BTreeMap::new(),
            env_http_headers: BTreeMap::new(),
            token_env_vars: Vec::new(),
            url: None,
            sensitive_literals: BTreeMap::new(),
            policy_plugin: None,
            policy_server: None,
            plugin_aliases: Vec::new(),
            enabled: true,
            managed: false,
            effective: true,
            overwritten_by: None,
        };
        let mut report = static_report_for(&server);
        apply_static_diagnostics(&mut report, &server);
        assert_eq!(report.status, Status::Fail);
        assert!(report
            .evidence
            .iter()
            .any(|item| item.contains("cwd does not exist")));
    }

    #[test]
    fn detects_runtime_fingerprint_duplicates() {
        let source = PathBuf::from("config.toml");
        let server = |name: &str| ServerConfig {
            name: name.to_string(),
            source: source.clone(),
            source_kind: "codex_user_config".to_string(),
            base_dir: PathBuf::from("."),
            transport: TransportKind::Stdio,
            command: Some("npx".to_string()),
            args: vec!["server".to_string()],
            cwd: None,
            env_literals: BTreeMap::new(),
            env_vars: Vec::new(),
            http_headers: BTreeMap::new(),
            env_http_headers: BTreeMap::new(),
            token_env_vars: Vec::new(),
            url: None,
            sensitive_literals: BTreeMap::new(),
            policy_plugin: None,
            policy_server: None,
            plugin_aliases: Vec::new(),
            enabled: true,
            managed: is_known_managed_name(name),
            effective: true,
            overwritten_by: None,
        };
        let mut reports = vec![
            static_report_for(&server("one")),
            static_report_for(&server("two")),
        ];
        apply_duplicate_diagnostics(&mut reports);
        assert!(reports.iter().all(|report| report.status == Status::Warn));
    }

    #[test]
    fn path_resolution_uses_pathext() {
        let dir = tempdir().unwrap();
        let exe = dir.path().join("npx.cmd");
        fs::write(&exe, "@echo off").unwrap();
        let found = resolve_command(
            "npx",
            None,
            OsStr::new(dir.path().as_os_str()),
            OsStr::new(".COM;.EXE;.BAT;.CMD"),
        );
        if cfg!(windows) {
            assert_eq!(
                found
                    .as_ref()
                    .map(|path| path.display().to_string().to_ascii_lowercase()),
                Some(exe.display().to_string().to_ascii_lowercase())
            );
        }
    }

    #[tokio::test]
    async fn profiler_redacts_leaky_stderr() {
        let dir = tempdir().unwrap();
        let script = dir.path().join("leaky_mcp.py");
        let leaked = ["sk", "-", "STDERRLEAK", "-", "1234567890abcdef"].join("");
        let bare = ["ghp", "_", "1234567890abcdef1234567890abcdef1234"].join("");
        fs::write(
            &script,
            format!(
                r#"
import json
import sys

sys.stderr.write("FATAL token {leaked} failed bare {bare}\n")
sys.stderr.flush()
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "initialize":
        print(json.dumps({{
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": {{
                "protocolVersion": "2025-06-18",
                "capabilities": {{"tools": {{}}}},
                "serverInfo": {{"name": "leaky", "version": "0.0.0"}}
            }}
        }}), flush=True)
    elif method == "tools/list":
        print(json.dumps({{
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": {{
                "tools": [{{
                    "name": "ok",
                    "description": "ok",
                    "inputSchema": {{"type": "object", "properties": {{}}}}
                }}]
            }}
        }}), flush=True)
"#
            ),
        )
        .unwrap();
        let python = if resolve_command(
            "python",
            None,
            &env::var_os("PATH").unwrap_or_default(),
            &env::var_os("PATHEXT").unwrap_or_default(),
        )
        .is_some()
        {
            "python"
        } else {
            "python3"
        };
        let server = ServerConfig {
            name: "leaky".to_string(),
            source: PathBuf::from("config.toml"),
            source_kind: "codex_user_config".to_string(),
            base_dir: PathBuf::from("."),
            transport: TransportKind::Stdio,
            command: Some(python.to_string()),
            args: vec![script.display().to_string()],
            cwd: None,
            env_literals: BTreeMap::new(),
            env_vars: Vec::new(),
            http_headers: BTreeMap::new(),
            env_http_headers: BTreeMap::new(),
            token_env_vars: Vec::new(),
            url: None,
            sensitive_literals: BTreeMap::new(),
            policy_plugin: None,
            policy_server: None,
            plugin_aliases: Vec::new(),
            enabled: true,
            managed: false,
            effective: true,
            overwritten_by: None,
        };
        let mut report = static_report_for(&server);
        profile_server(&mut report, &server).await;
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains(&leaked));
        assert!(!json.contains(&bare));
        assert!(json.contains("<redacted>"));
    }

    #[tokio::test]
    async fn profiler_reports_initialize_timeout() {
        let dir = tempdir().unwrap();
        let script = dir.path().join("hang_initialize.py");
        fs::write(
            &script,
            r#"
import time
time.sleep(60)
"#,
        )
        .unwrap();
        let mut server = minimal_test_server("hang-init", TransportKind::Stdio);
        server.command = Some(python_command().to_string());
        server.args = vec![script.display().to_string()];

        let started = Instant::now();
        let mut report = static_report_for(&server);
        profile_server(&mut report, &server).await;

        assert_eq!(report.status, Status::Fail);
        assert_eq!(report.timings.timeout_stage.as_deref(), Some("initialize"));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn profiler_reports_tools_list_timeout() {
        let dir = tempdir().unwrap();
        let script = dir.path().join("hang_tools.py");
        fs::write(
            &script,
            r#"
import json
import sys
import time

for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "initialize":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": {
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "slow-tools", "version": "0.0.0"}
            }
        }), flush=True)
    elif method == "tools/list":
        time.sleep(60)
"#,
        )
        .unwrap();
        let mut server = minimal_test_server("hang-tools", TransportKind::Stdio);
        server.command = Some(python_command().to_string());
        server.args = vec![script.display().to_string()];

        let started = Instant::now();
        let mut report = static_report_for(&server);
        profile_server(&mut report, &server).await;

        assert_eq!(report.status, Status::Fail);
        assert_eq!(report.timings.timeout_stage.as_deref(), Some("tools_list"));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn profiler_reports_missing_input_schema_and_pagination_loop() {
        let dir = tempdir().unwrap();
        let script = dir.path().join("bad_tools.py");
        fs::write(
            &script,
            r#"
import json
import sys

for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "initialize":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": {
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "bad-tools", "version": "0.0.0"}
            }
        }), flush=True)
    elif method == "tools/list":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": {
                "tools": [{"name": "bad"}],
                "nextCursor": "loop"
            }
        }), flush=True)
"#,
        )
        .unwrap();
        let mut server = minimal_test_server("bad-tools", TransportKind::Stdio);
        server.command = Some(python_command().to_string());
        server.args = vec![script.display().to_string()];

        let mut report = static_report_for(&server);
        profile_server(&mut report, &server).await;
        let evidence = report.evidence.join(" | ");

        assert_eq!(report.status, Status::Fail);
        assert!(evidence.contains("missing inputSchema"));
        assert!(evidence.contains("pagination cursor loop"));
    }

    fn minimal_test_server(name: &str, transport: TransportKind) -> ServerConfig {
        ServerConfig {
            name: name.to_string(),
            source: PathBuf::from("config.toml"),
            source_kind: "codex_user_config".to_string(),
            base_dir: PathBuf::from("."),
            transport,
            command: None,
            args: Vec::new(),
            cwd: None,
            env_literals: BTreeMap::new(),
            env_vars: Vec::new(),
            http_headers: BTreeMap::new(),
            env_http_headers: BTreeMap::new(),
            token_env_vars: Vec::new(),
            url: None,
            sensitive_literals: BTreeMap::new(),
            policy_plugin: None,
            policy_server: None,
            plugin_aliases: Vec::new(),
            enabled: true,
            managed: is_known_managed_name(name),
            effective: true,
            overwritten_by: None,
        }
    }

    fn python_command() -> &'static str {
        if resolve_command(
            "python",
            None,
            &env::var_os("PATH").unwrap_or_default(),
            &env::var_os("PATHEXT").unwrap_or_default(),
        )
        .is_some()
        {
            "python"
        } else {
            "python3"
        }
    }

    #[test]
    fn http_url_secrets_are_redacted_in_report_and_fingerprint() {
        let marker = "do-not-echo-this-value";
        let mut server = minimal_test_server("http", TransportKind::Http);
        server.url = Some(format!(
            "https://user:pass@example.test/mcp?api_key={marker}&safe=1#frag"
        ));

        let report = static_report_for(&server);
        let json = serde_json::to_string(&report).unwrap();

        assert!(!json.contains(marker));
        assert!(!json.contains("user:pass"));
        assert!(json.contains("api_key=<redacted>"));
        assert!(!report.fingerprint.contains(marker));
    }

    #[test]
    fn url_path_and_fragment_secrets_are_redacted() {
        let path_value = "sk%2DPATHVALUELEAK1234567890abcdef";
        let fragment_value = "plainliteral123";
        let redacted = redact_url_for_report(&format!(
            "https://example.test/token/{path_value}/mcp#access%5Ftoken={fragment_value}&safe=value"
        ));

        assert!(!redacted.contains(path_value));
        assert!(!redacted.contains(fragment_value));
        assert!(redacted.contains("/token/<redacted>/mcp"));
        assert!(redacted.contains("safe=value"));
    }

    #[test]
    fn encoded_secret_url_query_keys_are_redacted() {
        let marker = "plainliteral123";
        for url in [
            format!("https://example.test/mcp?api%5Fkey={marker}"),
            format!("https://example.test/mcp?access%5Ftoken={marker}&safe=value"),
            format!("https://example.test/mcp?oauth%5Ftoken={marker}"),
            format!("https://example.test/mcp?bearer%5Ftoken={marker}"),
        ] {
            let redacted = redact_url_for_report(&url);
            assert!(!redacted.contains(marker));
            assert!(redacted.contains("<redacted>"));
        }
        let mixed = redact_url_for_report(&format!(
            "https://example.test/mcp?access%5Ftoken={marker}&safe=value"
        ));
        assert!(mixed.contains("safe=value"));
    }

    #[test]
    fn url_query_redaction_preserves_safe_params_and_handles_malformed_encoding() {
        let safe = redact_url_for_report("https://example.test/mcp?safe=value&city=Austin");
        assert!(safe.contains("safe=value"));
        assert!(safe.contains("city=Austin"));

        let malformed =
            redact_url_for_report("https://example.test/mcp?api%ZZkey=plainliteral123&safe=value");
        assert!(malformed.contains("safe=value"));
    }

    #[test]
    fn encoded_secretish_query_values_are_redacted() {
        let encoded = "sk%2DURLVALUELEAK1234567890abcdef";
        let decoded = "sk-URLVALUELEAK1234567890abcdef";
        let redacted = redact_url_for_report(&format!("https://example.test/mcp?safe={encoded}"));

        assert!(!redacted.contains(encoded));
        assert!(!redacted.contains(decoded));
        assert!(redacted.contains("safe=<redacted>"));
    }

    #[test]
    fn json_output_conforms_to_public_schema() {
        let mut server = minimal_test_server("schema", TransportKind::Stdio);
        server.command = Some("schema-mcp".to_string());
        let mut envelope = DiagnosticEnvelope::new();
        envelope.servers.push(static_report_for(&server));
        envelope.recompute_status();

        let schema: JsonValue = serde_json::from_str(include_str!(
            "../schemas/cdxcore.diagnostics.v1.schema.json"
        ))
        .unwrap();
        let output = serde_json::to_value(&envelope).unwrap();

        validate_schema_subset(&output, &schema, &schema).unwrap();
    }

    fn validate_schema_subset(
        instance: &JsonValue,
        schema: &JsonValue,
        root: &JsonValue,
    ) -> std::result::Result<(), String> {
        if let Some(reference) = schema.get("$ref").and_then(JsonValue::as_str) {
            let pointer = reference
                .strip_prefix('#')
                .ok_or_else(|| format!("unsupported schema ref {reference}"))?;
            let target = root
                .pointer(pointer)
                .ok_or_else(|| format!("schema ref not found {reference}"))?;
            return validate_schema_subset(instance, target, root);
        }
        if let Some(options) = schema.get("anyOf").and_then(JsonValue::as_array) {
            if options
                .iter()
                .any(|option| validate_schema_subset(instance, option, root).is_ok())
            {
                return Ok(());
            }
            return Err(format!("value {instance} did not match anyOf"));
        }
        if let Some(expected) = schema.get("const") {
            if instance != expected {
                return Err(format!("expected const {expected}, got {instance}"));
            }
        }
        if let Some(values) = schema.get("enum").and_then(JsonValue::as_array) {
            if !values.iter().any(|value| value == instance) {
                return Err(format!("value {instance} not in enum"));
            }
        }
        if let Some(kind) = schema.get("type").and_then(JsonValue::as_str) {
            validate_json_type(instance, kind)?;
        }
        if schema.get("minimum").is_some() {
            let value = instance
                .as_i64()
                .or_else(|| {
                    instance
                        .as_u64()
                        .and_then(|value| i64::try_from(value).ok())
                })
                .ok_or_else(|| format!("expected integer for minimum check, got {instance}"))?;
            let minimum = schema
                .get("minimum")
                .and_then(JsonValue::as_i64)
                .ok_or_else(|| "minimum must be an integer".to_string())?;
            if value < minimum {
                return Err(format!("value {value} below minimum {minimum}"));
            }
        }
        if let Some(properties) = schema.get("properties").and_then(JsonValue::as_object) {
            let object = instance
                .as_object()
                .ok_or_else(|| format!("expected object, got {instance}"))?;
            if let Some(required) = schema.get("required").and_then(JsonValue::as_array) {
                for key in required.iter().filter_map(JsonValue::as_str) {
                    if !object.contains_key(key) {
                        return Err(format!("missing required key {key}"));
                    }
                }
            }
            if schema.get("additionalProperties") == Some(&JsonValue::Bool(false)) {
                for key in object.keys() {
                    if !properties.contains_key(key) {
                        return Err(format!("additional property {key}"));
                    }
                }
            }
            for (key, value) in object {
                if let Some(property_schema) = properties.get(key) {
                    validate_schema_subset(value, property_schema, root)
                        .map_err(|err| format!("{key}: {err}"))?;
                }
            }
        }
        if let Some(item_schema) = schema.get("items") {
            let items = instance
                .as_array()
                .ok_or_else(|| format!("expected array, got {instance}"))?;
            for (idx, item) in items.iter().enumerate() {
                validate_schema_subset(item, item_schema, root)
                    .map_err(|err| format!("item {idx}: {err}"))?;
            }
        }
        Ok(())
    }

    fn validate_json_type(instance: &JsonValue, kind: &str) -> std::result::Result<(), String> {
        let valid = match kind {
            "array" => instance.is_array(),
            "boolean" => instance.is_boolean(),
            "integer" => instance.as_i64().is_some() || instance.as_u64().is_some(),
            "null" => instance.is_null(),
            "object" => instance.is_object(),
            "string" => instance.is_string(),
            other => return Err(format!("unsupported schema type {other}")),
        };
        if valid {
            Ok(())
        } else {
            Err(format!("expected {kind}, got {instance}"))
        }
    }

    #[tokio::test]
    async fn disabled_server_skips_static_failures_and_profile() {
        let mut server = minimal_test_server("disabled", TransportKind::Stdio);
        server.command = Some("missing-cdxcore-fixture-command".to_string());
        server.enabled = false;
        server.effective = false;

        let mut report = static_report_for(&server);
        apply_static_diagnostics(&mut report, &server);
        profile_server(&mut report, &server).await;

        assert_eq!(report.status, Status::Pass);
        assert_eq!(report.timings.total_profile_ms, None);
        assert!(report.evidence.iter().any(|item| item.contains("disabled")));
    }

    #[test]
    fn plugin_policy_override_disables_matching_plugin_server() {
        let mut plugin_server = minimal_test_server("sample", TransportKind::Stdio);
        plugin_server.source_kind = "codex_plugin".to_string();
        plugin_server.policy_plugin = Some("sample@test".to_string());
        plugin_server.plugin_aliases = vec!["sample@test".to_string()];
        plugin_server.command = Some("sample-mcp".to_string());

        let mut policy = minimal_test_server("sample@test/sample", TransportKind::PolicyOverride);
        policy.source = PathBuf::from("policy.toml");
        policy.source_kind = "codex_plugin_policy_override".to_string();
        policy.policy_plugin = Some("sample@test".to_string());
        policy.policy_server = Some("sample".to_string());
        policy.enabled = false;

        let mut servers = vec![plugin_server, policy];
        apply_plugin_policy_overrides(&mut servers);

        assert!(!servers[0].enabled);
        assert!(!servers[0].effective);
        assert!(!servers[1].effective);
        assert_eq!(servers[0].overwritten_by.as_deref(), Some("policy.toml"));
    }

    #[test]
    fn invalid_plugin_mcp_shapes_are_rejected() {
        let dir = tempdir().unwrap();
        let plugin_root = dir.path().join("plugin");
        let manifest_dir = plugin_root.join(".codex-plugin");
        fs::create_dir_all(&manifest_dir).unwrap();
        let manifest = manifest_dir.join("plugin.json");
        let mcp = plugin_root.join(".mcp.json");
        fs::write(&manifest, r#"{"name":"demo"}"#).unwrap();

        fs::write(&mcp, r#"{"mcpServers":[]}"#).unwrap();
        assert!(parse_plugin_mcp_json(&mcp, &manifest).is_err());

        fs::write(&mcp, r#"{"demo":[]}"#).unwrap();
        assert!(parse_plugin_mcp_json(&mcp, &manifest).is_err());
    }

    #[test]
    fn structured_env_vars_and_oauth_literals_are_parsed_safely() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
[mcp_servers.docs]
url = "https://example.test/mcp"
env_vars = [{ name = "DOCS_TOKEN_ENV", source = "local" }]
oauth_resource = "https://example.test"

[mcp_servers.docs.oauth]
client_id = "client-id-literal"
"#,
        )
        .unwrap();

        let servers = parse_codex_toml(&path, "codex_user_config").unwrap();
        assert_eq!(servers[0].env_vars, vec!["DOCS_TOKEN_ENV"]);
        assert!(servers[0]
            .sensitive_literals
            .contains_key("oauth.client_id"));

        let report = static_report_for(&servers[0]);
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("client-id-literal"));
    }

    #[test]
    fn http_static_validation_includes_env_and_header_checks() {
        let mut server = minimal_test_server("web", TransportKind::Http);
        server.url = Some("https://example.test/mcp".to_string());
        server.token_env_vars = vec!["MISSING_CDXCORE_REVIEW_TOKEN".to_string()];
        server.http_headers.insert(
            "Authorization".to_string(),
            "Bearer literal-token-fixture".to_string(),
        );

        let mut report = static_report_for(&server);
        apply_static_diagnostics(&mut report, &server);
        let evidence = report.evidence.join(" | ");

        assert_eq!(report.status, Status::Warn);
        assert!(evidence.contains("static validation only"));
        assert!(evidence.contains("MISSING_CDXCORE_REVIEW_TOKEN"));
        assert!(evidence.contains("HTTP header Authorization"));
    }

    #[test]
    fn known_managed_names_set_managed_provenance_field() {
        let server = minimal_test_server("browser", TransportKind::Stdio);
        let report = static_report_for(&server);

        assert!(report.managed);
    }

    #[tokio::test]
    async fn json_rpc_error_redacts_known_config_secret_values() {
        let dir = tempdir().unwrap();
        let script = dir.path().join("error_mcp.py");
        let marker = "plainliteral123";
        fs::write(
            &script,
            format!(
                r#"
import json
import sys

for line in sys.stdin:
    request = json.loads(line)
    print(json.dumps({{
        "jsonrpc": "2.0",
        "id": request["id"],
        "error": {{"code": -32000, "message": "bad value {marker}"}}
    }}), flush=True)
"#
            ),
        )
        .unwrap();
        let python = if resolve_command(
            "python",
            None,
            &env::var_os("PATH").unwrap_or_default(),
            &env::var_os("PATHEXT").unwrap_or_default(),
        )
        .is_some()
        {
            "python"
        } else {
            "python3"
        };
        let mut env_literals = BTreeMap::new();
        env_literals.insert("PLAIN_VALUE".to_string(), marker.to_string());
        let mut server = minimal_test_server("leaky-error", TransportKind::Stdio);
        server.command = Some(python.to_string());
        server.args = vec![script.display().to_string()];
        server.env_literals = env_literals;

        let mut report = static_report_for(&server);
        profile_server(&mut report, &server).await;
        let json = serde_json::to_string(&report).unwrap();

        assert!(!json.contains(marker));
        assert!(json.contains("<redacted>"));
    }
}

# CDXCore

CDXCore is a Codex MCP diagnostics and startup profiler.

It gives Codex a local support layer for MCP startup problems, confusing client config, and PATH differences. If an MCP server
works in your terminal but Codex cannot start it, CDXCore helps answer the
boring but important questions:

- Which Codex config did this server come from?
- Is the command actually on the PATH Codex sees?
- Is `node`, `npx`, `uv`, `python`, `pnpm`, or `bun` missing?
- Is the working directory wrong?
- Did the server exit before the MCP `initialize` handshake?
- Did `tools/list` hang, return bad tools, or miss `inputSchema`?
- Are plugin, bundled, or managed MCP entries confusing the picture?

Command Guard moved to the standalone CDXCoreGuard tool. CDXCore stays focused on the v1 read-only MCP profiler.

Do not install CDXCore through an MCP marketplace for normal use. Install the
`cdxcore` CLI, then let Codex launch `cdxcore mcp-server`.

## Quick Install

Windows:

```powershell
irm https://github.com/ikhdark/CDXCore/releases/latest/download/install.ps1 | iex
```

macOS/Linux:

```sh
curl -fsSL https://github.com/ikhdark/CDXCore/releases/latest/download/install.sh | sh
```

Then check it:

```powershell
cdxcore --version
```

The installer puts `cdxcore` on PATH and runs:

```powershell
cdxcore setup codex
```

That registers this MCP entry with Codex:

```toml
[mcp_servers.cdxcore]
command = "cdxcore"
args = ["mcp-server"]
```

Restart Codex after install so the app sees the updated PATH.

## What You Run

Most of the time, start here:

```powershell
cdxcore doctor
```

That inspects your Codex MCP config and briefly starts each configured stdio MCP
server to time startup and the MCP handshake. It kills child processes after
profiling.

Example report:

```text
Server: notion
Status: fail (completely unwired)
Meaning: CDXCore could not prove this server is wired and working.
What to do: Treat the server as unavailable, fix the reported command/cwd/PATH/env/transport/handshake issue, then rerun check or doctor.
Cause: npx not found from Codex PATH
Evidence: command failed before MCP initialize
Suggested fix: use an absolute Node/npm path or add PATH in the MCP env block
Config source: C:\Users\<you>\.codex\config.toml
Secrets: redacted
```

Suggested command order:

```powershell
cdxcore scan
cdxcore doctor
cdxcore explain <server>
cdxcore fixes
cdxcore check <server>
cdxcore mcp-server
```

Older names remain available as aliases: `inspect-config`, `profile`,
`diagnose-runtime`, `suggest-fixes`, `validate`, and `serve`.

Add `--json` when you want machine-readable output:

```powershell
cdxcore doctor --json
```

The JSON schema is `cdxcore.diagnostics.v1` and lives at
`schemas/cdxcore.diagnostics.v1.schema.json`.

Diagnostic exit codes:

- `0`: successfully working; diagnostics completed with `pass`
- `1`: working but needs review; diagnostics completed with `warn`
- `2`: completely unwired; diagnostics completed with `fail`, or config could
  not be read or parsed enough to enumerate servers

CLI parser errors and unexpected internal CDXCore errors are not health results.

Health meanings:

- `pass` / exit `0` means CDXCore completed the requested check and found no
  diagnostic concerns. No action is required.
- `warn` / exit `1` means the server or config appears reachable enough to
  inspect, but CDXCore found something that needs review. Read `Cause`,
  `Evidence`, and `Suggested fix`; common actions are verifying the result from
  the Codex client that owns the config, moving literal secrets into environment
  variables, or confirming that a v1 limitation such as HTTP static-only checks
  is acceptable.
- `fail` / exit `2` means CDXCore could not prove the server is wired and
  working. Treat the server as unavailable until the reported command, cwd,
  PATH, env, transport, or MCP handshake problem is fixed, then rerun
  `cdxcore check <server>` or `cdxcore doctor`.
- Config blocked / exit `2` means CDXCore could not read or parse enough config
  to enumerate servers. Fix the reported TOML/JSON/path problem first, then run
  `cdxcore scan` before profiling individual servers.

## What `cdxcore mcp-server` Is

`cdxcore mcp-server` starts CDXCore as an MCP server for Codex.

It exposes these tools to Codex:

- `inspect_mcp_config`
- `profile_mcp_startup`
- `validate_mcp_server`
- `diagnose_runtime`
- `suggest_config_fixes`

They do the same kind of checks as the CLI, but from inside a Codex session.

## What CDXCore Checks

CDXCore can report:

- missing config files
- invalid TOML or JSON
- duplicate names across discovered surfaces
- duplicate runtime fingerprints
- missing executables
- bad working directories
- PATH mismatch between a terminal shell and the Codex process environment
- missing or suspicious env/header/OAuth values
- stdio servers that exit before `initialize`
- slow or timed-out `initialize`
- slow or timed-out `tools/list`
- missing tool `inputSchema`
- bounded `tools/list` pagination problems
- plugin `.mcp.json` servers
- plugin MCP policy overrides
- bundled or managed entries when provenance is locally discoverable

HTTP and streamable HTTP servers are static-checked only in this release. CDXCore
does not profile HTTP MCP servers yet.

Effective surface reconstruction is best-effort. If CDXCore cannot prove where a
plugin cache entry, bundled server, or injected server came from, it marks the
report incomplete instead of guessing.

## Secrets

CDXCore is built to be safe to paste back into an AI session.

It does not print raw env values by default. It redacts args, headers, OAuth
fields, URL userinfo/query values, stdout/stderr evidence, safe snippets, and
JSON/TOML values under secret-looking keys.

Secret-looking terms include:

```text
token, key, secret, password, bearer, auth, credential, cookie, session, api, oauth
```

Suggested config snippets use placeholders such as `${TOKEN_ENV_VAR}` or
`<absolute path>`. They should not echo discovered secret values.

## Read-Only Promise

The diagnostic commands and MCP tools do not edit your MCP configs, delete
state, reset state, or call arbitrary MCP tools.

The only command that writes Codex config is:

```powershell
cdxcore setup codex
```

That setup command is explicit. It installs only the CDXCore MCP entry.

Profiling launches configured stdio MCP servers only when you run `doctor`,
`check <server>`, or the matching MCP diagnostic tool.

## If PATH Is Weird

GUI apps often have a different PATH than your terminal.

If `cdxcore --version` works in a terminal but Codex cannot start CDXCore, use an
absolute path in `~/.codex/config.toml` or `$CODEX_HOME/config.toml`:

```toml
[mcp_servers.cdxcore]
startup_timeout_sec = 15
command = "C:\\Users\\you\\AppData\\Local\\CDXCore\\bin\\cdxcore.exe"
args = ["mcp-server"]
```

For local developer testing, point at your local build:

```toml
[mcp_servers.cdxcore]
startup_timeout_sec = 15
command = "C:\\Users\\kuh\\Desktop\\CDXCore\\target\\release\\cdxcore.exe"
args = ["mcp-server"]
```

## Plugin Files In This Repo

This repo includes:

- `.codex-plugin/plugin.json`
- `.mcp.json`

Those are for plugin-wrapper and local marketplace testing. Normal users should
install the CLI instead.

The plugin wrapper runs:

```json
{
  "command": "cdxcore",
  "args": ["mcp-server"]
}
```

If Codex cannot resolve `cdxcore` from its GUI/client PATH, use an absolute path
to the binary in `.mcp.json`.

## Updating

Update through the same release channel you used to install. If release notes say
the Codex MCP entry changed, rerun:

```powershell
cdxcore setup codex
```

## Uninstalling

Remove the `cdxcore` binary using the same method you used to install it.

If you also want to remove the Codex MCP entry, delete this block from
`~/.codex/config.toml` or `$CODEX_HOME/config.toml`:

```toml
[mcp_servers.cdxcore]
```


## For Contributors

You only need this section if you are working on CDXCore itself. Normal users
should use the installers above.

Local checks:

```powershell
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings
sh -n scripts/install.sh
bash -n scripts/install.sh
```

Release downloads:

- Windows x64: `cdxcore-v0.1.5-x86_64-pc-windows-msvc.zip`
- Linux x64: `cdxcore-v0.1.5-x86_64-unknown-linux-gnu.tar.gz`
- macOS Apple Silicon: `cdxcore-v0.1.5-aarch64-apple-darwin.tar.gz`
- Installers: `install.ps1`, `install.sh`

Latest release:

```text
https://github.com/ikhdark/CDXCore/releases/latest
```

Versioned release:

```text
https://github.com/ikhdark/CDXCore/releases/tag/v0.1.5
```

Verify downloads against `SHA256SUMS.txt`. CDXCore binaries are not signed yet.

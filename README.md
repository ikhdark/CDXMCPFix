# CDXCore

CDXCore is a CLI and MCP server for diagnosing Codex MCP startup and
configuration problems.

The v1 target is Codex MCP configuration. Claude Desktop, Cursor, Windsurf, and VS Code adapters are intentionally left for later.

Important: do not install CDXCore through an MCP or plugin marketplace for normal
use. Install the `cdxcore` CLI, then run `cdxcore setup codex`.

## Features

- Read-only Codex MCP config inspection and startup profiling.
- Install the `cdxcore` CLI, then let Codex launch
  `cdxcore serve`.
- Optional v2a command guard hooks are visible but inactive by default; they are
  feedback-only and never block or rewrite commands.

## Quick Start

1. Install the prebuilt `cdxcore` CLI from the current release. The first
   public artifact is `cdxcore-v0.1.0-x86_64-pc-windows-msvc.zip` for Windows
   x64.
2. Verify that the command is available:

   ```powershell
   cdxcore --version
   ```

3. Configure Codex:

   ```powershell
   cdxcore setup codex
   ```

4. Start a new Codex session. Codex launches `cdxcore serve` as the MCP server.

## Prerequisites

CDXCore should be installed as a normal CLI package or prebuilt binary. For
normal use, Rust and this source repository are not prerequisites.

The only path requirement is that `cdxcore` must be available to the app that
launches Codex. GUI-launched clients can have a different PATH than an
interactive terminal.

## Installing and Initialising CDXCore

Install a prebuilt `cdxcore` CLI for your platform from the package or release
channel published for the current CDXCore release.

Current prebuilt artifact:

- Windows x64: `cdxcore-v0.1.0-x86_64-pc-windows-msvc.zip`

Download the ZIP from
`https://github.com/ikhdark/CDXCore/releases/tag/v0.1.0`, extract it, and put
the extracted directory on the PATH used by Codex.

Upon completion, the command `cdxcore` should be available:

```powershell
cdxcore --version
```

To initialise CDXCore for Codex, run:

```powershell
cdxcore setup codex
```

This configures Codex to launch:

```toml
[mcp_servers.cdxcore]
command = "cdxcore"
args = ["serve"]
```

The manual fallback below includes `startup_timeout_sec = 15`. The default
setup path uses Codex's MCP CLI, which writes the launch command and args.

The setup command prefers:

```powershell
codex mcp add cdxcore -- cdxcore serve
```

If the Codex CLI is unavailable, use the manual fallback below.

## Updating CDXCore

Update CDXCore with the same package manager or release channel used for the
original CLI install, then rerun setup if the release notes say the Codex MCP
entry changed:

```powershell
cdxcore setup codex
```

## Uninstalling CDXCore

Uninstall CDXCore with the same package manager or release channel used for the
original CLI install. If you also want to remove the Codex MCP entry, delete the
`[mcp_servers.cdxcore]` block from `~/.codex/config.toml`.

Normal users do not need to clone this repository, install Rust, or build from
source.

## Manual fallback

Add this to `~/.codex/config.toml` or to `$CODEX_HOME/config.toml` when
`CODEX_HOME` is set:

```toml
[mcp_servers.cdxcore]
startup_timeout_sec = 15
command = "cdxcore"
args = ["serve"]
```

For local developer testing only, an absolute path is acceptable:

```toml
[mcp_servers.cdxcore]
startup_timeout_sec = 15
command = "C:\\Users\\kuh\\Desktop\\CDXCore\\target\\release\\cdxcore.exe"
args = ["serve"]
```

## Commands

```powershell
cdxcore setup codex
cdxcore inspect-config
cdxcore profile
cdxcore validate <server>
cdxcore diagnose-runtime <server>
cdxcore suggest-fixes
cdxcore serve
```

Add `--json` for the stable `cdxcore.diagnostics.v1` JSON schema. The schema is committed at `schemas/cdxcore.diagnostics.v1.schema.json`.

Optional command-guard hook entrypoints:

```powershell
cdxcore setup codex --enable-command-guard
cdxcore guard-hook pre-tool-use
cdxcore guard-hook post-tool-use
```

These are not MCP tools and are not wired into the default plugin manifest.
`setup codex --enable-command-guard` writes the requested Codex hook config as
an explicit opt-in. The hook entrypoint commands themselves are feedback-only:
they do not write files, execute submitted commands, block, or rewrite input.
Hook stdout is contract-only: empty for no feedback or one
`hookSpecificOutput.additionalContext` JSON object. `--json` does not change
hook output.

Exit codes:

- `0`: completed with no failing diagnostics
- `1`: completed with at least one failing diagnostic
- warnings alone exit `0`
- `2`: CLI usage or input error
- `3`: config read or parse failure prevents enumeration
- `4`: unexpected CDXCore internal or profiler error

## Read-only contract

CDXCore diagnostic commands and MCP tools do not edit config files, delete state,
reset state, print configured MCP env/header secret values, or call arbitrary MCP
tools. The `setup` command is the explicit exception: it installs the requested
Codex MCP entry, and `setup codex --enable-command-guard` also opts into
feedback-only hook configuration.

Profiling launches configured stdio servers only when the user explicitly runs
`profile`, `validate <server>`, or the equivalent MCP diagnostic tool. Child
processes are killed after profiling.

`safe_config_snippet` values use placeholders such as `${TOKEN_ENV_VAR}` or
`<absolute path>` and never echo discovered secret values.

## Advanced developer plugin testing

This repo includes:

- `.codex-plugin/plugin.json`
- `.mcp.json`

These files are for plugin-wrapper and local marketplace testing only. They are
not the normal user install path. The plugin manifest references `.mcp.json`,
whose default MCP entry runs:

```json
{
  "command": "cdxcore",
  "args": ["serve"]
}
```

If Codex cannot resolve `cdxcore` from its GUI/client PATH, use the absolute path to the binary in `.mcp.json`.

## Optional v2a command guard

The command guard is not active by default. To opt in, run:

```powershell
cdxcore setup codex --enable-command-guard
```

That command writes `$CODEX_HOME/hooks.json` or `~/.codex/hooks.json` and
installs only the PreToolUse guard hook. The `post-tool-use` entrypoint is kept
for future success-path rules and custom manual configs, but v2a does not install
it by default.

The standalone hook example lives at
`docs/examples/codex-command-guard-hooks.json`. Keep an explicit low timeout so a
feedback-only hook cannot stall tool execution. If the hook environment cannot
resolve `cdxcore`, replace the command with an absolute executable path. On
Windows, quote paths with spaces in the hook command itself.

Depending on Codex's current hook trust settings, Codex may ask you to review or
trust the installed hook before it runs.

## Developer local build/testing

Developer builds are for CDXCore contributors and local validation only:

```powershell
cargo build --release
```

When testing that local binary with Codex, use an absolute MCP command path as
shown in the manual fallback section. Do not use local builds as the normal user
install flow.

## Effective surface caveat

Effective Codex MCP surface reconstruction is best-effort in v1. CDXCore inspects raw user/project config and locally discoverable plugin cache MCP config. If it cannot prove plugin cache or bundled/injected server provenance offline, it marks the report incomplete instead of guessing.

When `cdxcore serve` is launched by Codex, `client_path` is authoritative for that CDXCore MCP process, but not necessarily every other Codex-managed child process. Standalone terminal runs include a PATH caveat.

## Secrets

Secret redaction applies to args, env literals, headers, OAuth fields,
stderr/stdout evidence, safe snippets, and parsed JSON/TOML values under
secret-like keys. CDXCore redacts secret-like keys, header names, env names, and
argument names/flags containing:

`token`, `key`, `secret`, `password`, `bearer`, `auth`, `credential`, `cookie`, `session`, `api`, `oauth`.

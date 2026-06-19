# CDXCore

CDXCore is a CLI-first, Codex-setup assisted, MCP-backed diagnostic tool for
Codex MCP startup and configuration problems.

The v1 target is Codex MCP configuration. Claude Desktop, Cursor, Windsurf, and VS Code adapters are intentionally left for later.

Important: do not install CDXCore through an MCP or plugin marketplace for normal
use. Install the `cdxcore` CLI, then run `cdxcore setup codex`.

## Features

- Read-only Codex MCP config inspection and startup profiling.
- Install the `cdxcore` CLI, then let Codex launch
  `cdxcore serve`.
- Optional command guard hooks are shipped but inactive by default. v2a is
  stateless feedback-only; v2b adds an explicit opt-in retry-shape ledger.

## Quick Start

1. Install and configure CDXCore for Codex with the one-command installer for
   your platform.

   Windows x64:

   ```powershell
   irm https://github.com/ikhdark/CDXCore/releases/latest/download/install.ps1 | iex
   ```

   macOS/Linux:

   ```sh
   curl -fsSL https://github.com/ikhdark/CDXCore/releases/latest/download/install.sh | sh
   ```

2. Verify that the command is available:

   ```powershell
   cdxcore --version
   ```

3. Start a new Codex session. Codex launches `cdxcore serve` as the MCP server.

## Prerequisites

CDXCore should be installed as a normal CLI package or prebuilt binary. For
normal use, Rust and this source repository are not prerequisites.

The only path requirement is that `cdxcore` must be available to the app that
launches Codex. GUI-launched clients can have a different PATH than an
interactive terminal.

## Installing and Initializing CDXCore

Install CDXCore with the one-command installer for your platform.

Windows x64:

```powershell
irm https://github.com/ikhdark/CDXCore/releases/latest/download/install.ps1 | iex
```

macOS/Linux:

```sh
curl -fsSL https://github.com/ikhdark/CDXCore/releases/latest/download/install.sh | sh
```

The installer downloads the current release ZIP, verifies `SHA256SUMS.txt`,
installs `cdxcore.exe` to `%LOCALAPPDATA%\CDXCore\bin`, updates the user PATH,
and runs `cdxcore setup codex`. On macOS/Linux, the installer downloads the
current release tarball, verifies `SHA256SUMS.txt`, installs `cdxcore` to
`$HOME/.local/bin`, adds that directory to the current PATH and shell profile
when needed, and runs `cdxcore setup codex`.

Current prebuilt artifacts:

- Windows x64: `cdxcore-v0.1.3-x86_64-pc-windows-msvc.zip`
- Linux x64: `cdxcore-v0.1.3-x86_64-unknown-linux-gnu.tar.gz`
- macOS Apple Silicon: `cdxcore-v0.1.3-aarch64-apple-darwin.tar.gz`
- Installers: `install.ps1`, `install.sh`

Manual archive install remains available from
`https://github.com/ikhdark/CDXCore/releases/tag/v0.1.3`.

For manual installs, download the Windows ZIP or macOS/Linux tarball from the
release page and verify its SHA256 checksum against `SHA256SUMS.txt` before
extracting it. CDXCore does not sign binaries yet.

Upon completion, the command `cdxcore` should be available:

```powershell
cdxcore --version
```

To initialize CDXCore for Codex, run:

```powershell
cdxcore setup codex
```

The installer runs this setup command automatically unless invoked with
`-SkipCodexSetup`.

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

To opt into the feedback-only command guard during install, download the
installer first and run it with `-EnableCommandGuard`. Add
`-EnableRetryLedger` only if you also want the explicit v2b ledger:

```powershell
$installer = "$env:TEMP\install-cdxcore.ps1"
irm https://github.com/ikhdark/CDXCore/releases/latest/download/install.ps1 -OutFile $installer
powershell -NoProfile -ExecutionPolicy Bypass -File $installer -EnableCommandGuard
# or:
powershell -NoProfile -ExecutionPolicy Bypass -File $installer -EnableRetryLedger
```

On macOS/Linux:

```sh
curl -fsSL https://github.com/ikhdark/CDXCore/releases/latest/download/install.sh | sh -s -- --enable-command-guard
# or:
curl -fsSL https://github.com/ikhdark/CDXCore/releases/latest/download/install.sh | sh -s -- --enable-retry-ledger
```

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

## Troubleshooting PATH

If `cdxcore --version` works in your terminal but Codex cannot start the MCP
server, Codex may be running with a different GUI PATH. Use an absolute path in
the MCP config.

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

## MCP tools

`cdxcore serve` exposes these MCP tools:

- `inspect_mcp_config`
- `profile_mcp_startup`
- `validate_mcp_server`
- `diagnose_runtime`
- `suggest_config_fixes`

Optional command-guard hook entrypoints:

```powershell
cdxcore setup codex --enable-command-guard
cdxcore guard-hook pre-tool-use
cdxcore guard-hook pre-tool-use --ledger
cdxcore guard-hook post-tool-use
```

These are not MCP tools and are not wired into the default plugin manifest.
`setup codex --enable-command-guard` writes the requested Codex hook config as
an explicit opt-in. Default command guard behavior remains stateless v2a.
`setup codex --enable-command-guard --enable-retry-ledger` installs the explicit
v2b retry-shape ledger by adding `--ledger` to the PreToolUse hook command. The
hook entrypoint commands themselves are feedback-only: they do not execute
submitted commands, block, rewrite input, read `transcript_path`, or call MCP
tools. Hook stdout is contract-only: empty for no feedback or one
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
Codex MCP entry, and `setup codex --enable-command-guard` opts into hook
configuration.

Default command guard remains stateless v2a. If retry ledger support is
explicitly enabled with `--ledger`, `CDXCORE_GUARD_LEDGER=on`, or
`setup codex --enable-command-guard --enable-retry-ledger`, the hook may write
hash-only observations under `$CODEX_HOME/cdxcore` or `~/.codex/cdxcore`. Hook
entrypoints never write Codex config; only setup does.

Profiling launches configured stdio servers only when the user explicitly runs
`profile`, `validate <server>`, or the equivalent MCP diagnostic tool. Child
processes are killed after profiling.

`safe_config_snippet` values use placeholders such as `${TOKEN_ENV_VAR}` or
`<absolute path>` and never echo discovered secret values.

## What CDXCore does not do

CDXCore v1 does not edit your existing MCP server configs, call arbitrary MCP
tools, profile HTTP servers, repair Codex state, or install hooks unless you
explicitly run the hook setup command.

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

## Optional v2 command guard

The command guard is not active by default. To opt into stateless v2a feedback,
run:

```powershell
cdxcore setup codex --enable-command-guard
```

To opt into v2b repeated risky shape feedback, run:

```powershell
cdxcore setup codex --enable-command-guard --enable-retry-ledger
```

That command writes `$CODEX_HOME/hooks.json` or `~/.codex/hooks.json` and
installs only the PreToolUse guard hook. v2a installs `cdxcore guard-hook
pre-tool-use`; v2b installs `cdxcore guard-hook pre-tool-use --ledger`. The
`post-tool-use` entrypoint is kept for future success-path rules and custom
manual configs, but setup does not install it by default.

The v2b ledger files are:

- `$CODEX_HOME/cdxcore/guard-ledger-v1.jsonl`
- `$CODEX_HOME/cdxcore/guard-ledger-key`
- `$CODEX_HOME/cdxcore/guard-ledger.lock`

If `CODEX_HOME` is not set, CDXCore uses `~/.codex/cdxcore`. The ledger stores
only keyed hashes of repeated risky command shapes, not raw commands or cwd
values. Entries older than 24 hours are ignored, lines over 8 KiB are ignored,
and compaction is best-effort after 256 KiB. A stale `guard-ledger.lock` can be
deleted safely. Set `CDXCORE_GUARD_LEDGER=off` to force v2a behavior, or
`CDXCORE_GUARD_LEDGER=on` to enable the ledger for a hook command.

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

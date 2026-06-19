# CDXCore

CDXCore is a workflow helper for Codex.

It gives Codex a local support layer for the stuff that slows agent work down:
MCP startup problems, confusing client config, PATH differences, repeated risky
shell-command shapes, and a tiny set of safe PowerShell command repairs.

The first shipping workflow is MCP startup/config diagnostics. If an MCP server
works in your terminal but Codex cannot start it, CDXCore helps answer the
boring but important questions:

- Which Codex config did this server come from?
- Is the command actually on the PATH Codex sees?
- Is `node`, `npx`, `uv`, `python`, `pnpm`, or `bun` missing?
- Is the working directory wrong?
- Did the server exit before the MCP `initialize` handshake?
- Did `tools/list` hang, return bad tools, or miss `inputSchema`?
- Are plugin, bundled, or managed MCP entries confusing the picture?

MCP diagnostics are only one part of CDXCore. Command Guard adds workflow help
before Codex runs shell commands, so Codex can catch command-shape problems
earlier. Claude Desktop, Cursor, Windsurf, and VS Code readers are planned
later.

Do not install CDXCore through an MCP marketplace for normal use. Install the
`cdxcore` CLI, then let Codex launch `cdxcore serve`.

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
args = ["serve"]
```

Restart Codex after install so the app sees the updated PATH.

## What You Run

Most of the time, start here:

```powershell
cdxcore profile
```

That inspects your Codex MCP config and briefly starts each configured stdio MCP
server to time startup and the MCP handshake. It kills child processes after
profiling.

Example report:

```text
Server: notion
Status: fail
Cause: npx not found from Codex PATH
Evidence: command failed before MCP initialize
Suggested fix: use an absolute Node/npm path or add PATH in the MCP env block
Config source: C:\Users\<you>\.codex\config.toml
Secrets: redacted
```

Other commands:

```powershell
cdxcore inspect-config
cdxcore profile
cdxcore validate <server>
cdxcore diagnose-runtime <server>
cdxcore suggest-fixes
cdxcore serve
```

Add `--json` when you want machine-readable output:

```powershell
cdxcore profile --json
```

The JSON schema is `cdxcore.diagnostics.v1` and lives at
`schemas/cdxcore.diagnostics.v1.schema.json`.

Exit codes:

- `0`: completed with no failing diagnostics
- `1`: completed and at least one server failed
- `2`: bad CLI usage or input
- `3`: config could not be read or parsed enough to enumerate servers
- `4`: unexpected CDXCore error

Warnings alone still exit `0`.

## What `cdxcore serve` Is

`cdxcore serve` starts CDXCore as an MCP server for Codex.

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

That setup command is explicit. By default it installs the CDXCore MCP entry.
Add the Command Guard flags below when you want Codex to get command feedback
and repairs too.

Profiling launches configured stdio MCP servers only when you run `profile`,
`validate <server>`, or the matching MCP diagnostic tool.

## Command Guard

Command Guard is the workflow-helper side of CDXCore. It gives Codex a quick
pre-run check for shell commands, so Codex can see common command problems
before they waste a tool call.

You can use it together with the MCP diagnostics workflow, or install it on its
own after setup.

### v2a: Feedback Only

v2a reads the pending Bash hook payload and gives Codex extra context when a
command shape deserves a second look.

It can point out:

- Linux syntax in a PowerShell-backed session
- unquoted Windows paths with spaces
- validation/build/test commands chained in a way that may hide failure
- destructive-looking commands like `rm -rf`, `Remove-Item -Recurse`,
  `git reset --hard`, or `git clean -fd`

v2a is lightweight: it does not block, rewrite, execute, store, or write
anything. Bad hook input is a silent no-op.

Enable it:

```powershell
cdxcore setup codex --enable-command-guard
```

### v2b: Repeated Command-Shape Memory

v2b is v2a plus a tiny local ledger. It helps Codex notice loops in command
behavior.

It lets CDXCore say, “this same command shape has been seen repeatedly.” It
does not guess whether the command failed.

The ledger stores keyed hashes of normalized command shapes and cwd values. It
does not store raw commands, raw cwd, env values, stdout, stderr, transcript
text, tool responses, or secrets.

Enable it:

```powershell
cdxcore setup codex --enable-command-guard --enable-retry-ledger
```

Ledger files live under:

- `$CODEX_HOME/cdxcore/guard-ledger-v1.jsonl`
- `$CODEX_HOME/cdxcore/guard-ledger-key`
- `$CODEX_HOME/cdxcore/guard-ledger.lock`

If `CODEX_HOME` is not set, CDXCore uses `~/.codex/cdxcore`. The ledger is safe
to delete. A stale `guard-ledger.lock` is also safe to delete.

### v2c: PowerShell Repair

v2c can repair a very small set of obvious PowerShell command mistakes by
returning Codex `updatedInput`.

It only runs when command repair is enabled and PowerShell shell proof is
explicit. Windows alone is not proof.

Repairs v2c may apply:

- `cmd > /dev/null` -> `cmd > $null`
- `cmd 2> /dev/null` -> `cmd 2> $null`
- `rg 'literal text' src` -> `rg -F 'literal text' src`

It leaves broad or ambiguous commands alone: destructive commands, pipelines,
chains, multiline commands, input redirection, `grep -> rg`, `Select-String`,
commands with PowerShell variables/expressions, and commands containing
secret-looking material.

Install the repair hook:

```powershell
cdxcore setup codex --enable-command-guard --enable-command-repair
```

That installs the repair layer but keeps it in feedback mode until shell proof
is explicit. To allow the PowerShell repair allowlist:

```powershell
cdxcore setup codex --enable-command-guard --enable-command-repair --shell powershell
```

Setup installs only the PreToolUse hook. The `post-tool-use` entrypoint exists
for future/custom use, but setup does not install it by default.

The standalone hook example is at
`docs/examples/codex-command-guard-hooks.json`.

## Command Guard During Install

Windows:

```powershell
$installer = "$env:TEMP\install-cdxcore.ps1"
irm https://github.com/ikhdark/CDXCore/releases/latest/download/install.ps1 -OutFile $installer
powershell -NoProfile -ExecutionPolicy Bypass -File $installer -EnableCommandGuard
powershell -NoProfile -ExecutionPolicy Bypass -File $installer -EnableRetryLedger
powershell -NoProfile -ExecutionPolicy Bypass -File $installer -EnableCommandRepair -CommandRepairShell powershell
```

macOS/Linux:

```sh
curl -fsSL https://github.com/ikhdark/CDXCore/releases/latest/download/install.sh | sh -s -- --enable-command-guard
curl -fsSL https://github.com/ikhdark/CDXCore/releases/latest/download/install.sh | sh -s -- --enable-retry-ledger
curl -fsSL https://github.com/ikhdark/CDXCore/releases/latest/download/install.sh | sh -s -- --enable-command-repair --command-repair-shell powershell
```

## If PATH Is Weird

GUI apps often have a different PATH than your terminal.

If `cdxcore --version` works in a terminal but Codex cannot start CDXCore, use an
absolute path in `~/.codex/config.toml` or `$CODEX_HOME/config.toml`:

```toml
[mcp_servers.cdxcore]
startup_timeout_sec = 15
command = "C:\\Users\\you\\AppData\\Local\\CDXCore\\bin\\cdxcore.exe"
args = ["serve"]
```

For local developer testing, point at your local build:

```toml
[mcp_servers.cdxcore]
startup_timeout_sec = 15
command = "C:\\Users\\kuh\\Desktop\\CDXCore\\target\\release\\cdxcore.exe"
args = ["serve"]
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
  "args": ["serve"]
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

If you enabled command guard, remove the CDXCore PreToolUse entry from
`$CODEX_HOME/hooks.json` or `~/.codex/hooks.json`.

## Developer Notes

Normal users do not need Rust or this repo.

For local development:

```powershell
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings
```

Current release artifacts:

- Windows x64: `cdxcore-v0.1.3-x86_64-pc-windows-msvc.zip`
- Linux x64: `cdxcore-v0.1.3-x86_64-unknown-linux-gnu.tar.gz`
- macOS Apple Silicon: `cdxcore-v0.1.3-aarch64-apple-darwin.tar.gz`
- Installers: `install.ps1`, `install.sh`

Manual archives are published at:

```text
https://github.com/ikhdark/CDXCore/releases/tag/v0.1.3
```

Verify archive checksums against `SHA256SUMS.txt`. CDXCore binaries are not
signed yet.

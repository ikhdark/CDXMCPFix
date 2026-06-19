# CDXCore

CDXCore is a read-only Codex plugin, CLI, and MCP server for diagnosing MCP startup and configuration problems.

The v1 target is Codex MCP configuration. Claude Desktop, Cursor, Windsurf, and VS Code adapters are intentionally left for later.

## Install locally

```powershell
cargo build --release
```

Put the resulting `cdxcore` binary on the PATH used by the app that launches Codex, or change the plugin `.mcp.json` command to an absolute path. GUI-launched clients often have a different PATH than an interactive terminal, which is one of the failure modes CDXCore reports.

## Install as a local Codex plugin

Use the CLI directly if you only want terminal diagnostics. To test the plugin/MCP surface in Codex:

1. Build the binary:

   ```powershell
   cargo build --release
   ```

2. Decide how Codex will launch the MCP server. Keep `.mcp.json` as `command: "cdxcore"` only when the Codex GUI/client PATH can resolve that binary. Otherwise edit your local test copy of `.mcp.json` to use the absolute binary path, for example:

   ```json
   {
     "mcpServers": {
       "cdxcore": {
         "command": "C:\\Users\\you\\Desktop\\CDXCore\\target\\release\\cdxcore.exe",
         "args": ["serve"]
       }
     }
   }
   ```

3. Add this repo to a local Codex plugin marketplace entry that points at the repository root, then install it from that marketplace:

   ```json
   {
     "name": "cdxcore",
     "source": {
       "source": "local",
       "path": "C:\\Users\\you\\Desktop\\CDXCore"
     },
     "policy": {
       "installation": "AVAILABLE",
       "authentication": "ON_INSTALL"
     },
     "category": "Developer Tools"
   }
   ```

   ```powershell
   codex plugin add cdxcore@personal
   ```

   The default personal marketplace is discovered from `%USERPROFILE%\.agents\plugins\marketplace.json`. For a non-default local marketplace, add that marketplace first and install with its marketplace name instead of `personal`.

4. Start a new Codex thread and use the CDXCore MCP tools, or run:

   ```powershell
   cdxcore inspect-config
   cdxcore profile
   ```

## Commands

```powershell
cdxcore inspect-config
cdxcore profile
cdxcore validate <server>
cdxcore diagnose-runtime <server>
cdxcore suggest-fixes
cdxcore serve
```

Add `--json` for the stable `cdxcore.diagnostics.v1` JSON schema. The schema is committed at `schemas/cdxcore.diagnostics.v1.schema.json`.

Exit codes:

- `0`: completed with no failing diagnostics
- `1`: completed with at least one failing diagnostic
- warnings alone exit `0`
- `2`: CLI usage or input error
- `3`: config read or parse failure prevents enumeration
- `4`: unexpected CDXCore internal or profiler error

## Read-only contract

CDXCore does not edit config files, delete state, reset state, print raw env values by default, or call arbitrary MCP tools. It launches configured stdio servers only when the user explicitly runs `profile`, `validate <server>`, or the equivalent MCP diagnostic tool. Child processes are killed after profiling.

`safe_config_snippet` values use placeholders such as `${TOKEN_ENV_VAR}` or `<absolute path>` and never echo discovered secret values.

## Codex plugin packaging

This repo includes:

- `.codex-plugin/plugin.json`
- `.mcp.json`

The plugin manifest explicitly references `.mcp.json`. The default MCP entry runs:

```json
{
  "command": "cdxcore",
  "args": ["serve"]
}
```

If Codex cannot resolve `cdxcore` from its GUI/client PATH, use the absolute path to the binary in `.mcp.json`.

## Effective surface caveat

Effective Codex MCP surface reconstruction is best-effort in v1. CDXCore inspects raw user/project config and locally discoverable plugin cache MCP config. If it cannot prove plugin cache or bundled/injected server provenance offline, it marks the report incomplete instead of guessing.

When `cdxcore serve` is launched by Codex, `client_path` is authoritative for that CDXCore MCP process, but not necessarily every other Codex-managed child process. Standalone terminal runs include a PATH caveat.

## Secrets

Secret redaction applies to args, env literals, headers, OAuth fields, stderr/stdout evidence, safe snippets, and parsed JSON/TOML values under secret-like keys. CDXCore redacts keys or arguments containing:

`token`, `key`, `secret`, `password`, `bearer`, `auth`, `credential`, `cookie`, `session`, `api`, `oauth`.

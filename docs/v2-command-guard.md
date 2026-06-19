# CDXCore v2a - Command Guard

> **Status: optional v2a hook module.** CDXCore v1 remains the default MCP
> profiler. v2a is opt-in and feedback-only. v2b ledger behavior and v2c
> blocking behavior remain deferred.

## Contract

CDXCore v2a provides two optional Codex hook entrypoints:

```powershell
cdxcore guard-hook pre-tool-use
cdxcore guard-hook post-tool-use
```

The hook commands read Codex hook JSON from stdin and always exit `0`. Empty,
malformed, unrecognized, or irrelevant hook input is a silent no-op: empty
stdout and empty stderr.

Feedback output is limited to:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "additionalContext": "..."
  }
}
```

or the same shape with `PostToolUse`. The hook emits no other top-level fields.
Global CLI output flags such as `--json` do not affect hook stdout.

## Safety Boundary

Codex supports blocking and input rewrite in current hook output contracts, but
CDXCore v2a intentionally emits neither. External JSON fields such as
`hookSpecificOutput.permissionDecision`,
`hookSpecificOutput.permissionDecisionReason`,
`hookSpecificOutput.updatedInput`, `hookSpecificOutput.updatedMCPToolOutput`,
legacy `decision` / `reason`, and universal stop/suppress fields are never
emitted by v2a.

Forbidden v2a output fields are:

- `permissionDecision`
- `permissionDecisionReason`
- `updatedInput`
- `updatedMCPToolOutput`
- `suppressOutput`
- `interrupt`
- `decision`
- `reason`
- `continue`
- `stopReason`
- `systemMessage`
- `updatedPermissions`

The hook entrypoint commands themselves never:

- block tool execution
- rewrite tool input
- execute submitted commands
- read `transcript_path`
- write filesystem state
- store a retry ledger
- call MCP tools

The implementation only inspects canonical Bash hook payloads where
`tool_name == "Bash"` and `tool_input.command` is a string.

## Feedback Rules

PreToolUse may emit conservative feedback for:

- obvious platform/shell syntax mismatch
- unquoted Windows paths with spaces
- clearly risky command chaining that can hide validation/build/test failures
- destructive-looking commands such as `rm -rf`, `Remove-Item -Recurse`,
  `del /s`, `rmdir /s`, `git reset --hard`, or `git clean -fd`

Normal PowerShell pipelines and ordinary semicolon usage should not produce
feedback by default.

PostToolUse remains success-path only and does not include non-zero shell
exit-code rules in v2a. Current Codex behavior gates PostToolUse on the tool
success path, not directly on shell exit code, so `rg` / `grep` no-match
annotations are intentionally out of scope for v2a.

`additionalContext` is redacted, capped to 1200 Unicode scalar characters, and
redacted again before output. If the final context is empty, v2a emits no JSON.

## Optional Hook Installation

Do not wire hooks into the default `.codex-plugin/plugin.json`, and do not ship a
root `hooks/hooks.json`. To opt in after installing the CLI:

```powershell
cdxcore setup codex --enable-command-guard
```

That setup command is the explicit mutating opt-in path: it writes
`$CODEX_HOME/hooks.json` or `~/.codex/hooks.json`. It installs only the PreToolUse
guard hook in v2a. The `post-tool-use` entrypoint remains available for future
success-path rules and custom manual configs, but v2a does not install it by
default because it currently emits no feedback.

The standalone JSON example lives at
`docs/examples/codex-command-guard-hooks.json`.

If Codex cannot resolve `cdxcore` from the hook environment, use an absolute
binary path in the `command` string and quote it according to the shell used by
the hook runner.

## Deferred Work

v2b may add an opt-in ephemeral hashed retry ledger. v2c may add opt-in
deterministic blocking. Neither is enabled or implied by v2a.

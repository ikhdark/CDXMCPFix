# CDXCore v2 Command Guard

> **Status:** v2a is the default optional command guard. v2b is an explicit
> opt-in repeated risky shape ledger. v2c blocking behavior remains deferred.

## Contract

CDXCore provides two Codex hook entrypoints:

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

Codex supports blocking and input rewrite in current hook output contracts, but
CDXCore v2a/v2b intentionally emit neither. CDXCore never emits
`permissionDecision`, `permissionDecisionReason`, `updatedInput`,
`updatedMCPToolOutput`, `suppressOutput`, `interrupt`, `decision`, `reason`,
`continue`, `stopReason`, `systemMessage`, or `updatedPermissions`.
These external JSON fields are distinct from Codex's internal Rust outcome
fields such as `should_block`, `block_reason`, or `updated_input`.

The hook entrypoint commands themselves never block tool execution, rewrite tool
input, execute submitted commands, read `transcript_path`, write Codex config,
or call MCP tools. Only explicit user-run setup may write Codex hook
configuration.

## v2a Default

`cdxcore guard-hook pre-tool-use` is stateless v2a behavior. It performs no
ledger/key/lock reads or writes.

PreToolUse may emit conservative feedback for:

- obvious platform/shell syntax mismatch
- unquoted Windows paths with spaces
- clearly risky command chaining that can hide validation/build/test failures
- destructive-looking commands such as `rm -rf`, `Remove-Item -Recurse`,
  `del /s`, `rmdir /s`, `git reset --hard`, or `git clean -fd`

Normal PowerShell pipelines and ordinary semicolon usage should not produce
feedback by default.

PostToolUse remains a reserved no-op entrypoint in v2b. It is not installed by
default because v2b has no PostToolUse feedback rules. Current Codex behavior
runs PostToolUse through the tool-success path, so CDXCore v2a/v2b do not make
non-zero shell exit-code rules such as `rg` no-match annotations.

## v2b Retry-Shape Ledger

v2b is explicit opt-in:

```powershell
cdxcore guard-hook pre-tool-use --ledger
```

Ledger enablement precedence is:

1. `CDXCORE_GUARD_LEDGER=off`
2. `CDXCORE_GUARD_LEDGER=on`
3. `--ledger`
4. default off

The ledger tracks repeated risky command shapes before execution. It does not
claim commands failed.

Paths:

- ledger: `$CODEX_HOME/cdxcore/guard-ledger-v1.jsonl`
- key: `$CODEX_HOME/cdxcore/guard-ledger-key`
- lock: `$CODEX_HOME/cdxcore/guard-ledger.lock`
- fallback base: `~/.codex/cdxcore`

Paths are resolved from `$CODEX_HOME` or the user home directory, never from the
hook process cwd. The ledger key is created lazily only for risky
ledger-enabled PreToolUse commands. On Unix, CDXCore tries to create the key
with owner-only permissions. On Windows, storing it under the user's Codex home
profile path is the v2b privacy boundary.

Each JSONL line is one append-only observation:

```json
{
  "schema_version": 1,
  "timestamp_unix_ms": 1781840000000,
  "event_name": "PreToolUse",
  "command_shape_hash": "lowercase-hex-keyed-blake3",
  "cwd_hash": "lowercase-hex-keyed-blake3-or-null",
  "rule_ids": ["destructive.git_reset_hard"]
}
```

The ledger stores only keyed BLAKE3 hashes of redacted, normalized command
shapes and lexically normalized cwd values. It never stores raw commands, raw
cwd, env values, stdout/stderr, transcript text, tool responses, secrets, MCP
data, or the ledger key.

Repeated-shape feedback:

- first risky sighting: normal v2a feedback only
- second sighting within 24 hours: mild repeated-shape note
- third or later sighting within 24 hours: stronger repeated-loop note

Any ledger/key/parse/write/lock/compaction problem silently falls back to v2a or
best available already-read feedback: exit `0`, no stderr, no JSON error, and no
hook failure noise.

Ledger safety:

- malformed lines are ignored
- lines over 8 KiB are ignored
- records older than 24 hours are ignored
- future timestamps are invalid
- compaction is best-effort only when the ledger exceeds 256 KiB
- `guard-ledger.lock` uses best-effort create-new locking
- if the lock exists, writes are skipped immediately
- stale `guard-ledger.lock` files are safe to delete

## Optional Hook Installation

Do not wire hooks into the default `.codex-plugin/plugin.json`, and do not ship a
root `hooks/hooks.json`.

Install v2a command guard:

```powershell
cdxcore setup codex --enable-command-guard
```

Install v2b retry-shape ledger:

```powershell
cdxcore setup codex --enable-command-guard --enable-retry-ledger
```

The setup command is the explicit mutating opt-in path: it writes
`$CODEX_HOME/hooks.json` or `~/.codex/hooks.json`. v2a setup installs PreToolUse
without `--ledger`; v2b setup installs PreToolUse with `--ledger`.

The standalone JSON example lives at
`docs/examples/codex-command-guard-hooks.json` and remains v2a by default. If
Codex cannot resolve `cdxcore` from the hook environment, use an absolute binary
path in the `command` string and quote it according to the shell used by the hook
runner.

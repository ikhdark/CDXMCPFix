# CDXCore Roadmap (Index)

One-screen index of CDXCore's planned scope. CDXCore is CLI-first,
Codex-setup assisted, MCP-backed. **v1** is the default product surface.
**v2a** is an optional, feedback-only hook module. **v2b** is an explicit
opt-in repeated risky shape ledger. v2c and v3 are deferred design notes, not
implementation.

## v1 — Read-only Codex MCP config/startup profiler

CDXCore v1 is a strictly **read-only** Codex MCP **config/startup profiler**:
inspect Codex MCP config, statically diagnose it, and profile stdio server
startup/handshake on explicit command. **This is the default MVP surface.**

## v2a — Command-Guard (optional, feedback-only)

Hook-first command guard for AI-agent shell commands. v2a is shipped but inactive
by default and must be explicitly enabled with
`cdxcore setup codex --enable-command-guard`. It emits feedback only: no command
blocking, input rewrite, or retry ledger. See
[v2-command-guard.md](v2-command-guard.md).

## v2b — Repeated risky shape ledger (explicit opt-in)

v2b keeps the feedback-only boundary but can write a privacy-safe, hash-only
ledger when explicitly enabled with
`cdxcore setup codex --enable-command-guard --enable-retry-ledger`,
`cdxcore guard-hook pre-tool-use --ledger`, or `CDXCORE_GUARD_LEDGER=on`.

## v2c — Command-Guard blocking (deferred)

v2c blocking behavior is deferred and must remain manual opt-in if implemented
later.

## v3 — Cross-client fleet view (deferred)

Cross-client read-only fleet view: inspect MCP config/startup health across
installed agent clients in one aggregated report. **Requires extracting a
client-reader abstraction from the Codex-only v1 code, and remains read-only like
v1.** See [v3-fleet-view.md](v3-fleet-view.md).

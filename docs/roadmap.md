# CDXCore Roadmap (Index)

One-screen index of CDXCore's planned scope. Only **v1** is being built; v2 and v3
are deferred design notes, not implementation.

## v1 — Read-only Codex MCP config/startup profiler

CDXCore v1 is a strictly **read-only** Codex MCP **config/startup profiler**:
inspect Codex MCP config, statically diagnose it, and profile stdio server
startup/handshake on explicit command. **This is the only MVP being built.**

## v2 — Command-Guard (deferred)

Hook-first command guard for AI-agent shell commands. **Orthogonal to v1** — a
separate module on its own timeline, not part of the v1 MVP. See
[v2-command-guard.md](v2-command-guard.md).

## v3 — Cross-client fleet view (deferred)

Cross-client read-only fleet view: inspect MCP config/startup health across
installed agent clients in one aggregated report. **Requires extracting a
client-reader abstraction from the Codex-only v1 code, and remains read-only like
v1.** See [v3-fleet-view.md](v3-fleet-view.md).

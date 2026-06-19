# CDXCore Roadmap (Index)

One-screen index of CDXCore's planned scope. CDXCore is CLI-first, Codex-setup assisted, and MCP-backed. **v1 is the only product surface in this repository.**

## v1 — Read-only Codex MCP config/startup profiler

CDXCore v1 is a strictly **read-only** Codex MCP **config/startup profiler**: inspect Codex MCP config, statically diagnose it, and profile stdio server startup/handshake on explicit command. **This is the default MVP surface.**

## Command Guard — moved out

The former v2a/v2b/v2c Command Guard work now lives in `C:\Users\kuh\Desktop\CDXCoreGuard` and will be released as its own tool. Do not add Command Guard commands, hooks, tests, installer flags, or release packaging back into CDXCore v1.

## v3 — Cross-client fleet view (deferred)

Cross-client read-only fleet view: inspect MCP config/startup health across installed agent clients in one aggregated report. **Requires extracting a client-reader abstraction from the Codex-only v1 code, and remains read-only like v1.** See [v3-fleet-view.md](v3-fleet-view.md).

# CDXCore v2 — Command-Guard (Deferred Design Note)

> **Status: DEFERRED DESIGN NOTE — NOT PART OF THE CDXCore v1 MVP.**
> This document records design decisions and verified facts for a *future* v2
> module. Nothing here is implemented in v1. v1 ships no hooks, no guard code,
> no CLI subcommands, no config, no tests, no workspace crates, and no
> implementation stubs related to command guarding.

## v1 scope (unchanged)

CDXCore v1 is a **strictly read-only MCP startup/config profiler** only.

v1 does **not** include:

- any command guard behavior
- any hooks
- any retry ledger
- any blocking of commands
- any writable state introduced by guarding

The v1 trust proposition is a single sentence — *CDXCore cannot modify config or
state* — and v2 must not be allowed to weaken it. The guard is a separate module
shipped on its own timeline, after v1.

## Why a hook, not an MCP tool

Command-failure detection must be **hook-first**, not delivered as MCP tools.

An MCP tool only fires when the agent *chooses* to call it. The failures a guard
targets (e.g. misreading a no-match exit code as a crash, Unix syntax on Windows,
concatenated commands, retry loops) occur precisely when the agent does **not**
realize anything is wrong — so it will not think to call a diagnostic tool. A hook
fires automatically on the relevant lifecycle event with no agent decision
required, which is the only mechanism that reaches these cases.

## Verified Codex hook facts

These were verified against the Codex upstream reference and are the load-bearing
constraints for the entire v2 module:

- Codex supports **`PreToolUse`** and **`PostToolUse`** lifecycle hooks (among
  others).
- Command hooks use a **`command`** handler, with a platform-specific
  **`commandWindows`** variant for Windows invocation.
- Codex hook output can provide additional context, block execution, and, for
  allowed `PreToolUse` results, provide updated input.
- CDXCore v2a intentionally declines both block and rewrite behavior.

### Hook trust (current verified Codex behavior)

As currently verified, Codex gates whether a hook runs through a hook-trust model
(trust states such as managed / trusted / untrusted / modified, per-hook trust
hashes, and a managed-hooks-only mode). A CDXCore guard hook is therefore subject
to Codex's hook-trust model and **may not run if it is untrusted or unapproved** —
meaning the guard can be silently inactive until trusted. This describes Codex's
current behavior, not a permanent universal guarantee; the hook config surface may
evolve, but the constraint matters enough to preserve now.

### Structural consequence

Because Codex can block and rewrite, CDXCore must make its v2a boundary explicit
and testable: v2a emits feedback only. It never emits block decisions or
`updatedInput`; any "fix" travels as context for the agent to re-issue, never as
an in-place mutation from CDXCore v2a.

## v2 staging

The guard is introduced in stages so that *state* and *control* — two distinct
trust escalations — are added separately and can each be accepted or refused
independently:

- **v2a — stateless, feedback-only. The only default when v2 ships.** Hook
  emits context text only. No blocking. No input rewrite. No writable state.
  This is non-mutating advisory behavior, distinct from v1's read-only profiler.
  v2b and v2c are never enabled implicitly.
- **v2b — optional ephemeral hashed retry ledger. Opt-in.** Introduces the first
  writable state: a tightly-scoped, ephemeral, *hashed and redacted* per-thread
  ledger of failed-command fingerprints (never raw command text). Opt-in only;
  crosses only the state line.
- **v2c — deterministic blocking rules. Opt-in.** Promotes specific,
  deterministic rules to `stop` (block) only after they have proven themselves in
  feedback-only mode. Opt-in only; crosses the control line. Fuzzy /
  non-deterministic rules remain feedback-only indefinitely.

Default posture: **v2a is the only default.** Both v2b and v2c are opt-in and must
be explicitly enabled; neither is ever turned on by default.

## Reaffirmation

v1 remains a read-only MCP startup/config profiler only. No ledger, no blocking,
no hooks, and no command guard behavior ships in v1.

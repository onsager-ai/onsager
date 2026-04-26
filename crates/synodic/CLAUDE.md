# Synodic

AI agent governance — gate evaluation, policy, verdicts. Exposes a
small HTTP surface on port 3001 for the dashboard's governance views;
otherwise coordinates over the spine.

## The seam rule (canonical)

> HTTP APIs exist only at external boundaries:
> - **User-facing endpoints** called by the dashboard.
> - **Webhooks** called by external services (GitHub, etc.).
>
> Subsystems (`forge`, `stiglab`, `synodic`, `ising`) coordinate
> **exclusively** via the spine: events on the bus + reads against
> shared spine tables. No subsystem makes HTTP calls to another
> subsystem. No subsystem imports another subsystem's crate.

What this means for synodic specifically:

- **Allowed HTTP surfaces.** Dashboard-facing endpoints for
  governance UI (rule lists, verdict views, etc.). Webhooks from
  external policy sources, if any.
- **Forbidden HTTP surfaces.** Anything called from `forge`, `stiglab`,
  or `ising`. The legacy `HttpSynodicGate` path in
  `crates/forge/src/cmd/serve.rs:65–180` is the one remaining
  violation, and Lever C of spec #131 deletes it. Gate verdicts must
  flow as events: forge emits `forge.gate_requested`; synodic consumes
  it and emits `synodic.gate_verdict` with the decision.
- **`SYNODIC_FAIL_POLICY` (forge-side).** When forge cannot reach the
  HTTP gate today, it falls back per `SYNODIC_FAIL_POLICY` (default
  `escalate`). Once Lever C lands, the equivalent failure mode is "no
  `synodic.gate_verdict` arrived in window" — the same `escalate /
  deny / allow` choice applies to event-time math, not HTTP timeouts.
  The default stays `escalate` (forge invariant #5).
- **Cargo deps.** `synodic` may depend on `onsager-{artifact, protocol,
  spine}` (and on `onsager-protocol` only until Lever C deletes that
  crate). It must NOT depend on `forge`, `stiglab`, or `ising`. CI
  will hard-fail this once Lever B's architecture lint lands.
- **Verdict shape stays in the registry.** Per ADR 0003, `MergeRule`
  for verdict channels lives in `onsager-registry`. Synodic produces
  partial updates that get folded by the registry-declared rule — do
  not invent a private merge in this crate.

See [ADR 0001](../../docs/adr/0001-event-bus-coordination-model.md) for
the original decision and spec #131 for the six-lever enforcement plan.

## Build & Test

Run from repo root:

```bash
cargo build -p synodic
cargo test  -p synodic --lib
cargo clippy -p synodic --all-targets -- -D warnings
```

CI runs the workspace pass with `RUSTFLAGS="-D warnings"` against a
merge preview of `origin/main`; reproduce that locally via the
`onsager-pre-push` skill before pushing.

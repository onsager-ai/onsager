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
  or `ising`. **Lever C status (#148): no remaining violation** —
  `HttpSynodicGate` and the `POST /api/gate` route it called are gone
  as of phase 5. Gate verdicts flow as events: forge emits
  `forge.gate_requested`; synodic's `gate_listener` consumes it and
  emits `synodic.gate_verdict` with the decision.
- **Missing-in-window verdict policy (forge-side).** When `synodic.gate_verdict`
  doesn't arrive within the deadline, the forge pipeline applies the
  same `escalate / deny / allow` choice the legacy `SYNODIC_FAIL_POLICY`
  used to apply to HTTP timeouts — the default stays `escalate`
  (forge invariant #5). The phase-6 sub-issue (#186) tracks per-process
  cursor persistence so a forge restart mid-gate doesn't drop the
  verdict in flight.
- **Cargo deps.** `synodic` may depend on `onsager-{artifact,
  spine}` (the protocol DTOs now live in `onsager_spine::protocol`
  per #131 Lever C; the standalone `onsager-protocol` crate is gone).
  It must NOT depend on `forge`, `stiglab`, or `ising`. CI will
  hard-fail this once Lever B's architecture lint lands.
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

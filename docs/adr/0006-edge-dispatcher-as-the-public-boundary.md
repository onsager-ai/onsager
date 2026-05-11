# ADR 0006 — Edge dispatcher as the public boundary

- **Status**: Accepted
- **Date**: 2026-05-09 (accepted 2026-05-11)
- **Identity impact**: no
- **Tracking issues**: #283 (implementation spec); #222 (cleanup tail;
  the 2026-05-06 status update flagged the open infra-level dispatch
  question this ADR resolves).
- **Supersedes**: none
- **Superseded by**: none

## Context

ADR 0004's 2026-04-30 amendment named `portal` as the owner of
clause 1 of the seam rule — the external HTTP boundary itself.
Spec #222 carried that decision through six slices and a series of
follow-ups (#257, #259, and the final cleanup commit that landed
sessions / tasks / nodes plus the dashboard `API_BASE` cutover).
After all of that, every dashboard-facing `/api/*` route is
*implemented* on portal: routes, handlers, schema, auth, spine
reads and writes.

But the production deployment topology never caught up. The
production image is a single Docker container deployed to Railway,
and Railway exposes exactly one process to the outside. That
process is `stiglab`. Portal runs inside the same container, bound
to `127.0.0.1:3002` via `PORTAL_BIND` in the stiglab deploy
entrypoint (`crates/stiglab/deploy/entrypoint.sh`). Synodic and
forge run alongside, also internal. To make portal's routes
reachable from the dashboard, stiglab's router
(`crates/stiglab/src/server/mod.rs`) carries a wildcard catch-all
that loopback-forwards every `/api/*` request to portal at
`config.portal_url`:

```rust
.route("/api/{*path}", any(routes::portal::proxy));
```

PR #264 re-introduced this proxy after the Slice 6 cutover
precisely because the dashboard could not reach portal otherwise.

The result is a structural tension that no further refactor inside
the Onsager codebase can resolve:

- **Code-level boundary**: portal owns `/api/*`. Every dashboard
  route is portal-served. `lint-seams` and `check-api-contract`
  enforce this.
- **Process-level boundary**: stiglab is the externally-reachable
  process. Every external `/api/*` request hits stiglab first and
  is forwarded to portal over loopback. Stiglab is, in topology
  terms, still the edge.

ADR 0004's amendment closed the *code-level* gap (clause 1 has an
owner). The process-level gap is the same shape one layer down: the
route owner and the externally-exposed process disagree, and
nothing in the Onsager source tree can reconcile them because the
disagreement is between source code and deployment infrastructure.

The dev environment already runs the right shape.
`deploy/dev/Caddyfile` puts Caddy in front of every subsystem (one
container per subsystem under `docker-compose`) and routes `/api/*`
to `portal:3002`, `/agent*` to `stiglab:3000`, `/api/synodic/*` and
`/api/forge/*` to their respective subsystems with prefix-strip.
Same-origin for the dashboard, no proxy hop, no loopback. The dev
topology is what the seam rule's amendment implies; the production
topology is what we shipped to make Railway work.

The proxy's docstring already documents this:

> Post-#222 Slice 6 stiglab owns only `/agent/ws`. Everything else
> under `/api/` is handled by portal. In dev, Caddy routes those
> requests directly to portal; in the Railway single-container
> deployment stiglab is the only process reachable from outside, so
> this handler forwards the request over the loopback to portal at
> `config.portal_url`.

The proxy is no longer migration debt. It is the deployment
topology's compensation for portal not being directly reachable.
Either we remove the compensation (introduce an edge dispatcher in
production) or we accept that the process-level boundary will
permanently disagree with the code-level boundary.

Two further forces push toward resolution:

- **Future deployment targets.** Railway's single-container model is
  one shape among several. A multi-container deploy, a different
  cloud, or a self-hosted shape will each face the same question:
  who is externally reachable, and how does portal's address get
  surfaced? Pinning the answer once, in deployment infrastructure
  rather than in stiglab's process, is a portability gain.
- **Stiglab's identity.** Stiglab is the agent-session orchestrator.
  The proxy makes it incidentally also the production HTTP edge.
  That conflation is exactly what ADR 0004's amendment fought;
  finishing the alignment finishes the amendment.

## Decision

Introduce a real edge dispatcher in the production deployment so
the process-level boundary matches the route-level boundary that
ADR 0004 established. The dispatcher becomes the only
externally-reachable process; it routes `/api/*` to portal (which
keeps end-to-end ownership of those routes) and `/agent/ws` to
stiglab (the one route stiglab still legitimately owns). No
Onsager subsystem listens on a public port. The stiglab catch-all
proxy and the `crates/stiglab/src/server/routes/portal.rs` module
are deleted.

Concretely: add Caddy to the production Docker image
(`crates/stiglab/deploy/Dockerfile`). The entrypoint starts Caddy
on `$PORT` (the port Railway exposes); Caddy reverse-proxies
`/api/*` → `127.0.0.1:3002` (portal), `/agent/ws` →
`127.0.0.1:3000` (stiglab), and `/*` → static files
(`/app/static`, the dashboard build). The production Caddyfile
under `crates/stiglab/deploy/Caddyfile` is a peer of
`deploy/dev/Caddyfile` — same routing logic, different upstreams
(loopback in production vs. inter-container in dev).

The dev topology is unchanged. Same-origin behavior on the
dashboard is unchanged. The dashboard keeps `/api` as the base
URL (`apps/dashboard/src/lib/api/index.ts`) — exactly because the
dispatcher makes that true in both environments.

Going forward, the rule that owns this ADR is:

> The externally-reachable process is the edge dispatcher. The
> Onsager subsystems (`portal`, `stiglab`, `synodic`, `forge`,
> `ising`) are reachable only through the dispatcher's routing
> config. Portal owns `/api/*`; stiglab owns `/agent/ws`; the rest
> of the subsystems are not externally addressable.

Cross-cutting: this is *not* an ADR 0001 violation. ADR 0001
forbids synchronous HTTP between Onsager subsystems. The
dispatcher is not a subsystem — it is deployment infrastructure,
the same category as the database server or the OS process
supervisor. Caddy doesn't import any Onsager crate and doesn't
read or write the spine.

## Rejected alternatives

- **Keep the stiglab proxy as documented loopback infrastructure.**
  Cheapest. Land a docstring update + CLAUDE.md note that re-frames
  the proxy as deployment plumbing rather than migration debt. The
  code-level boundary is correct already; the process-level
  disagreement is an implementation detail of the Railway
  topology. Rejected because the same disagreement re-appears at
  every new deployment target, and because the proxy continues to
  give stiglab a public surface area it should not have.
- **Multi-service Railway deployment** (one Railway service per
  subsystem). Closer to the dev topology. Rejected because the
  reach is wide — separate Railway services need separate
  Dockerfile entries, separate logs, separate scaling configs,
  separate internal-DNS routing, separate health checks. The cost
  is a fork of every operational concern. The single-container
  image with an in-process dispatcher gets the same boundary
  alignment without the operational fork.
- **Switch off Railway entirely.** Real, but unrelated. The
  deployment shape question stands regardless of the cloud.
  Rejected as conflating two decisions.
- **Use Railway's own ingress to route paths to multiple services.**
  Equivalent to the multi-service alternative above; same
  operational cost, same rejection.
- **Move the proxy from stiglab to portal** (portal proxies its
  own externally-exposed surface to internal subsystems).
  Considered as a way to remove the stiglab proxy without
  introducing Caddy. Rejected because portal is then doing both
  edge dispatch and HTTP route serving in the same process — the
  same conflation the ADR is trying to remove, just relocated.
  Edge dispatch belongs in deployment infra, not in a subsystem.

## Consequences

### Positive

- The process-level boundary matches the route-level boundary.
  Stiglab is reachable from outside only for `/agent/ws`, which is
  what its ownership of clause 1 of the seam rule actually
  implies.
- The stiglab `/api/*` proxy and `routes/portal.rs` are deletable.
  `xtask check-api-contract` will report only `/agent/ws` as the
  stiglab dashboard surface (the goal already noted in #222's
  status update for slice 6).
- The dev and production topologies converge. The dev Caddyfile
  becomes a reviewable peer of the production Caddyfile; routing
  drift between environments becomes visible in diff form.
- New external integrations (GitLab, Slack, Linear) added to portal
  are reachable in production without touching stiglab. The
  process-level boundary is no longer a blocker for portal's
  growth.
- Future deployment targets inherit the same shape. The dispatcher
  is the public boundary; everything else is internal. Self-hosted
  or different-cloud deploys reuse the routing config with a
  different upstream block.

### Negative / trade-offs

- One more component in the production image (Caddy binary).
  Roughly ~40MB; a Caddyfile to maintain; one more thing whose
  health Railway must observe. Mitigation: pin Caddy version in
  the Dockerfile; smoke-test the routing in CI.
- The image's `ENTRYPOINT` becomes responsible for one more
  process. The existing entrypoint already manages four (synodic,
  portal, forge, stiglab); adding Caddy is incremental, not a
  structural change. Process supervision stays a `gosu` + `while
  true` loop; if Railway later adopts a real supervisor
  (s6-overlay, etc.) that is a separate decision.
- The dispatcher config becomes part of the deployment contract.
  A breaking route addition needs the Caddyfile updated in the
  same PR. This is a desirable kind of explicit; it is also a
  real per-route step that did not exist before.

### Neutral

- Dashboard API base URL is unchanged (`/api`). The dispatcher
  makes same-origin true in production; nothing in the frontend
  changes.
- ADR 0001's runtime invariant is preserved. Caddy is not a
  subsystem; no synchronous HTTP between Onsager subsystems is
  added.
- The dev Caddyfile is unchanged. Dev already had the right
  shape; this ADR aligns production with dev, not the other way
  around.

## Dev-process counterpart

Per ADR 0002, the dev-process analog of this decision: the dev
Caddyfile and the production Caddyfile are reviewable peers. A
new externally-reachable route (or a route ownership change)
requires both Caddyfiles updated in the same PR. The pattern
matches the "producer + consumer + manifest entry in one PR" rule
from ADR 0004 Lever E — both halves of the contract land
together so the half-wired drift pattern cannot recur on the
topology layer.

A small `xtask` check that diffs the route blocks of the two
Caddyfiles and fails on divergence is the natural mechanical
guardrail; it is filed as an item in the implementation spec
opened alongside this ADR but is not a blocking dependency for
landing the dispatcher itself.

## Adoption checklist

Execution lives in the implementation spec opened alongside this
ADR (linked under Tracking issues above). Status as of
2026-05-09:

- [ ] Add Caddy to `crates/stiglab/deploy/Dockerfile` (runtime
      stage).
- [ ] Production Caddyfile under `crates/stiglab/deploy/Caddyfile`,
      structurally mirroring `deploy/dev/Caddyfile` but with
      loopback upstreams.
- [ ] `crates/stiglab/deploy/entrypoint.sh` starts Caddy on `$PORT`;
      stiglab binds to `127.0.0.1:3000` (no longer to `0.0.0.0`);
      the `STIGLAB_HOST=0.0.0.0` env in the Dockerfile is removed.
- [ ] Delete `crates/stiglab/src/server/routes/portal.rs` and the
      `route("/api/{*path}", any(routes::portal::proxy))`
      registration in `crates/stiglab/src/server/mod.rs`.
- [ ] Stiglab no longer serves the static dashboard; the
      `STIGLAB_STATIC_DIR` env and the `nest_service("/assets",
      …)` / `fallback_service(…)` blocks in `mod.rs` move to the
      Caddyfile (`/assets` cache headers, `index.html` no-cache,
      SPA fallback).
- [ ] Update root `CLAUDE.md` ("Architecture" + "The seam rule")
      with the process-level edge clarification.
- [ ] Update ADR 0004's adoption checklist to note this ADR as the
      resolution of the process-level half of clause 1.
- [ ] Smoke test: `just smoke-test` against a deploy with Caddy in
      front of portal answers identically to today's stiglab-proxy
      shape.

## Out of scope

- **TLS termination.** Railway terminates TLS at its edge; Caddy
  inside the container speaks plain HTTP on `$PORT`. If a future
  deployment target needs in-image TLS that is a separate
  concern.
- **Health-check shape.** Railway's existing health check needs to
  point at `/api/health` (which now reaches portal via Caddy)
  rather than at stiglab directly; the implementation spec covers
  this but the protocol decision is unchanged.
- **Multi-tenant routing.** Out of scope; the dispatcher routes by
  path prefix, not by tenant. Workspace scoping continues to be a
  request-level concern enforced by `require_workspace_access` in
  portal.
- **Migration to a different platform.** This ADR is portable by
  design (the dispatcher pattern works on any single-process or
  multi-process target) but does not commit Onsager to or away
  from Railway.
- **A separate ADR for the dev Caddyfile**, which already exists
  and works — this ADR adopts the dev shape into production, not
  the other way around.

## Amendment 2026-05-09 — stiglab carve-out closed

The Decision section above carved out one exception to portal's
ownership of clause 1: *Portal owns `/api/*`; **stiglab owns
`/agent/ws`***. The carve-out was a pragmatic compromise to keep
this ADR's scope focused on the dispatcher work; it explicitly
preserved stiglab's ownership of the agent control-plane
WebSocket.

[ADR 0008](0008-portal-owns-the-agent-control-plane.md) closes
that carve-out. Portal becomes the route owner for `/agent/ws`
too, hosting it as a transparent WebSocket proxy that forwards
bytes bidirectionally to stiglab on loopback. Stiglab continues
to own the agent protocol (`AgentMessage`, node registration,
task dispatch) but stops accepting external connections.

The amended rule statement (as of 2026-05-09):

> The externally-reachable process is the edge dispatcher. The
> Onsager subsystems (`portal`, `stiglab`, `synodic`, `forge`,
> `ising`) are reachable only through the dispatcher's routing
> config. Portal owns every external route; no subsystem
> carve-outs.

Wherever this ADR's body says *"stiglab owns `/agent/ws`"* or
*"stiglab is externally reachable only for `/agent/ws`"*, read it
as the pre-amendment state. The post-amendment state is what ADR
0008 specifies.

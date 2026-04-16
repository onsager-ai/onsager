---
status: archived
created: 2026-03-11
priority: medium
tags:
- factory
- supply-chain
- context-delivery
- artifact-cache
- prefetch
- dependency-resolution
parent: 037-coding-factory-vision
depends_on:
- 039-assembly-line-abstraction
- 001-agent-workspace-persistence
created_at: 2026-03-11T00:00:00Z
updated_at: 2026-03-11T00:00:00Z
---

# Factory Supply Chain — Just-in-Time Context & Artifact Delivery

## Overview

Current design assumes agents have everything they need. There's no concept of just-in-time delivery of context, dependencies, or pre-computed artifacts to each station. This is like designing the assembly line but not the supply chain that delivers parts to each station exactly when needed.

Context windows are expensive. Feeding an agent the entire codebase when it only needs 3 files is wasteful. Pre-computing artifacts that multiple stations need avoids redundant work. Resolving external dependencies before BUILD prevents mid-build failures.

This spec builds the factory's supply chain: context delivery, artifact caching, dependency resolution, and predictive prefetch.

## Design

### Context Delivery

Each station has a **context budget** — the maximum context window allocation. The supply chain delivers exactly the context each station needs, no more.

```yaml
context_profile:
  station: build
  budget: 100k_tokens       # max context to deliver
  required:
    - spec: full             # the spec being implemented (always)
    - blueprint: full        # the design blueprint (always)
    - affected_files: full   # files identified in blueprint
  conditional:
    - test_fixtures: if_referenced  # only if blueprint mentions tests
    - api_specs: if_referenced      # only if touching API surface
  excluded:
    - unrelated_modules      # explicitly exclude to save context
  strategy: relevance_ranked # rank by relevance, fill to budget
```

Context assembly pipeline:
1. **Extract requirements** from the work item and station context profile
2. **Resolve references** — follow imports, type references, test fixtures
3. **Rank by relevance** — score each context chunk by its relevance to the task
4. **Pack to budget** — fill the context budget with highest-relevance items first
5. **Deliver** — provide the assembled context to the station agent

### Artifact Cache

Intermediate artifacts computed by one station are often useful to downstream stations or to future work items. The cache avoids recomputation:

**Cache layers:**

| Layer | Scope | Lifetime | Example |
|---|---|---|---|
| **Work-item cache** | Single work item across stations | Until work item completes | Blueprint, build report, review findings |
| **Project cache** | Across work items in same project | Until invalidated by code change | Type stubs, dependency graph, file index, test baseline |
| **Global cache** | Across projects | TTL-based | Library documentation, API reference summaries |

Cache invalidation:
- **Work-item cache:** Cleared on work item completion
- **Project cache:** Invalidated on git commits that affect cached content (track file→cache dependency)
- **Global cache:** TTL expiry (configurable per artifact type)

```json
{
  "cache_key": "project:synodic:type-stubs:src/pipeline",
  "artifact_type": "type_stubs",
  "created_by_station": "design",
  "content_hash": "sha256:abc123",
  "depends_on_files": ["src/pipeline.rs", "src/types.rs"],
  "last_valid_commit": "92bfef4",
  "size_tokens": 1200,
  "hit_count": 14
}
```

### Dependency Resolution

External dependencies are resolved before work reaches BUILD, preventing mid-build failures:

- **Library dependencies:** Verify that referenced libraries exist, are compatible, and are available
- **API dependencies:** Check that external APIs referenced in the spec are accessible and match expected schemas
- **Service dependencies:** Verify that dependent services (databases, message queues) are running and reachable
- **Tool dependencies:** Ensure required build tools, linters, and test frameworks are installed

Dependency resolution happens at the DESIGN stage exit gate. If unresolvable dependencies are detected, the work item is held with a clear dependency manifest until the issue is resolved.

```yaml
dependency_manifest:
  work_item: work-001
  resolved:
    - type: library
      name: tokio
      version: "1.35"
      status: available
    - type: tool
      name: rustfmt
      status: installed
  unresolved:
    - type: api
      name: github-api
      issue: "Rate limit exceeded, retry after 14:30"
      blocking: true
```

### Predictive Prefetch

The supply chain anticipates what downstream stations will need and prepares it in advance:

- **Blueprint prefetch:** When INTAKE completes a spec, start assembling the context for DESIGN (affected files, similar past specs, relevant documentation)
- **Build prefetch:** When DESIGN completes a blueprint, start resolving dependencies and pre-loading affected file contents for BUILD
- **Test prefetch:** When BUILD starts, begin preparing test fixtures and baseline metrics for INSPECT

Prefetch is speculative — it may not hit 100%. The heuristic is simple: look at the current station's output and the next station's context profile, start assembling the overlap.

Prefetch effectiveness is measured as a hit rate (% of prefetched artifacts actually used) and tracked in production metrics (spec 041).

### Integration with Workspace Persistence

The supply chain builds on spec 001 (agent workspace persistence) for durable context:

- **Memory sync:** Agent workspace state (spec 001) provides the baseline; supply chain adds task-specific context on top
- **Context recovery:** If an agent crashes mid-station, the supply chain can reconstruct the full context for a replacement agent
- **Context diff:** On rework, only deliver the delta context (review findings, specific file changes) rather than the full context again

## Plan

- [ ] Define context profile schema per station type (budget, required, conditional, excluded)
- [ ] Implement context assembly pipeline: extract requirements → resolve references → rank → pack → deliver
- [ ] Build artifact cache with three layers (work-item, project, global) and appropriate invalidation strategies
- [ ] Implement cache invalidation on git commits using file→cache dependency tracking
- [ ] Build dependency resolution for libraries, APIs, services, and tools at DESIGN exit gate
- [ ] Implement dependency manifest: resolved/unresolved tracking with blocking classification
- [ ] Build predictive prefetch: anticipate next-station context needs and pre-assemble
- [ ] Measure prefetch hit rate and feed into production metrics (spec 041)
- [ ] Integrate with workspace persistence (spec 001) for crash recovery and rework context deltas

## Test

- [ ] Context delivery for BUILD station includes only affected files and spec, not unrelated modules
- [ ] Context packing respects budget: a 100k token budget receives at most 100k tokens of context
- [ ] Artifact cache hit: INSPECT reuses type stubs computed by DESIGN without recomputation
- [ ] Cache invalidation: committing a change to `src/pipeline.rs` invalidates cached type stubs that depend on it
- [ ] Dependency resolution catches a missing library before BUILD starts
- [ ] Unresolved dependency holds the work item at DESIGN exit (doesn't proceed to BUILD)
- [ ] Prefetch hit rate is measured and reported in metrics
- [ ] Rework delivery includes only the delta context (review findings) not the full original context
- [ ] Agent crash recovery: replacement agent receives full reconstructed context and can continue

## Notes

The supply chain is the factory's invisible optimizer. Agents don't directly interact with it — they just receive the right context at the right time. The cost savings compound: less context per agent call means fewer tokens means lower cost per unit (tracked in spec 041's cost accounting).

The three-layer cache model is inspired by CPU cache hierarchies (L1/L2/L3). The work-item cache is "L1" (small, fast, per-item), project cache is "L2" (medium, per-project), and global cache is "L3" (large, shared, slow invalidation).
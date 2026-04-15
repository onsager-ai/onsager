# Context Mesh — Exploration: Consistent Memory & Durable Context

> Exploration document expanding on spec 026. Examines what a context mesh
> needs to become a real solution for consistent memory across agent sessions.

## Starting point

Spec 026 defines the context mesh as a shared reactive knowledge DAG where
agents publish, observe, and fill gaps autonomously. The three defining
properties:

1. **Reactive semantics** — changes push to observers
2. **Gap detection** — system reasons about what's *missing*
3. **Autonomous resolution** — gaps trigger agent spawning

Without all three, you have a shared cache. With all three, you approach
collective intelligence.

## Where the current model stops short

### 1. Context ≠ Knowledge

The current spec models context as knowledge claims: facts, findings,
constraints. But real agent context is richer:

| Layer | Content | Example |
|-------|---------|---------|
| **Knowledge** | Facts, findings, constraints | "The auth system uses JWT with RS256" |
| **Episodic** | What happened, what was tried, what failed | "We tried bcrypt but it caused timeout issues under load" |
| **Procedural** | How to do things, learned heuristics | "Always run integration tests after changing the auth middleware" |
| **Relational** | Agent capabilities, trust, collaboration history | "Agent-7 produces high-confidence security findings" |

A mesh of only knowledge nodes misses the memory that makes agents effective
across sessions and across tasks.

### 2. Persistence across sessions

The current DAG is ephemeral — it lives for one coordination run. But
consistent memory demands a **durable context mesh**:

- **Confidence decay**: Nodes lose confidence over time and when underlying
  code changes. A finding about auth patterns from 3 months ago is less
  reliable than one from yesterday.
- **Invalidation cascades**: When a node's dependency changes, downstream
  nodes must be re-evaluated. If `findings.auth.tokens` is stale because
  the token format changed, `design.auth-service` is also suspect.
- **Compaction**: Over time, fine-grained nodes should merge into summary
  nodes. 50 individual findings about error handling compact into a single
  "error handling patterns" summary with provenance links.
- **Cross-session continuity**: A new session should load relevant context
  from prior sessions. Not everything — a relevance-filtered projection.

### 3. Context mesh as substrate, not just primitive

Currently context mesh is one of 11 coordination primitives. But every other
primitive implicitly needs shared context:

- A **speculative swarm** needs shared understanding of the problem space
- A **pipeline** needs context to flow forward between stages
- A **generative-adversarial** pair needs shared criteria for quality
- **Stigmergic** coordination is literally communication through shared state

This raises a question: is context mesh a primitive that you *choose*, or is
it the **substrate** on which all primitives operate?

Possible answer: both. There's a *thin* context mesh (shared state bus) that
underlies all coordination, and a *thick* context mesh (the full reactive
knowledge DAG with gap detection) that you opt into for knowledge-intensive
tasks. The thin version is infrastructure; the thick version is a primitive.

## Proposed layered model

```
Context Mesh v2
│
├── Knowledge Layer (spec 026 as-is)
│   ├── Knowledge nodes: key, value, confidence, dependencies, author, version
│   ├── Reactive gap detection → autonomous spawn
│   └── Conflict resolution: compete-and-compare / LWW / manual
│
├── Memory Layer (new)
│   ├── Episodic: session transcripts, decision trails, failure records
│   │   → "What happened and why"
│   ├── Procedural: learned patterns, tool preferences, workflow heuristics
│   │   → "How to do things well"
│   └── Semantic: entity graph, codebase model, domain ontology
│       → "How things relate to each other"
│
├── Persistence Layer (new)
│   ├── TTL + confidence decay function
│   ├── Invalidation cascade engine (dependency change → stale propagation)
│   ├── Compaction (merge fine-grained → summary nodes, preserve provenance)
│   └── Session bootstrap (load relevant prior context on session start)
│
└── Projection Layer (new)
    ├── Per-agent context windows (relevance-filtered view of the mesh)
    ├── Attention scoring (what matters for *this* agent *right now*)
    └── Context budget management (fit within token/cost limits)
```

## The hardest problem: relevance filtering

Spec 026 acknowledges "context overload" as a failure mode but offers only
"narrow watch patterns" as mitigation. That's manual configuration.

The real challenge is **automatic relevance**: given a mesh with thousands of
nodes, which ones matter for a specific agent's current task?

This is the retrieval problem from RAG, but with more structure available:

| Signal | How it helps |
|--------|-------------|
| **Graph proximity** | Follow dependency edges from the current task node — nearby nodes are likely relevant |
| **Recency** | Recently created or updated nodes score higher |
| **Agent-role match** | An implementer doesn't need deep research nodes; a reviewer doesn't need implementation details |
| **Usage history** | Nodes that were observed but never acted on by similar agents lose score |
| **Confidence** | Higher-confidence nodes are more useful |
| **Staleness** | Nodes whose underlying dependencies have changed get penalized |

The projection layer would compute a relevance score for each node relative
to each agent and serve a filtered view that fits within the agent's context
budget.

## Relationship to existing approaches

| Approach | How it compares |
|----------|----------------|
| **RAG** | Context mesh is structured where RAG is unstructured. RAG retrieves text chunks; mesh retrieves typed knowledge nodes with dependency edges. Mesh knows what's *missing*; RAG only knows what *matches*. |
| **Shared filesystem** | Stigmergic coordination already uses shared artifacts. Context mesh adds structure, typing, confidence, and reactive semantics on top. |
| **Agent memory (MemGPT, etc.)** | Per-agent memory systems. Context mesh is *collective* memory — shared across agents with conflict resolution. |
| **Knowledge graphs** | Context mesh is a knowledge graph with three additions: reactive propagation, gap detection, and autonomous resolution. These make it *active* rather than passive. |

## Open questions

1. **Granularity**: What's the right node size? Too fine-grained (individual
   facts) creates overhead. Too coarse (entire documents) loses the
   dependency graph's value. Is there a natural granularity?

2. **Trust boundaries**: Should all agents see all nodes? In a fleet with
   agents from different providers or trust levels, some knowledge might
   need access control. But access control fights against the mesh's core
   value proposition (all knowledge visible to all).

3. **Bootstrapping**: How does a new session populate its initial mesh? Load
   everything from storage? Query based on the task description? Use a
   dedicated "context loader" agent?

4. **Garbage collection**: When does a node get deleted vs. compacted? What
   prevents unbounded growth of the mesh over months of use?

5. **Substrate vs. primitive**: If context mesh becomes the substrate for all
   coordination, does it change the operation algebra? Does every primitive
   implicitly get `observe` and `merge` for free?

## Next steps

- Define the memory layer's node types and schema
- Prototype a persistence layer with confidence decay
- Design the relevance scoring algorithm for the projection layer
- Test whether "substrate + primitive" framing changes the coordination model theory

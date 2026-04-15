---
status: planned
created: 2026-03-30
priority: critical
tags:
- governance
- feedback
- optimization
- convergence
- reward-function
- umbrella
depends_on:
- "068"
- "069"
created_at: 2026-03-30T00:00:00Z
updated_at: 2026-03-30T00:00:00Z
split_into:
- "072"
- "073"
- "074"
- "075"
- "076"
---

# Governance Feedback & Optimization — Umbrella Spec

> **Status**: split into 072-076 · **Priority**: critical · **Created**: 2026-03-30

## Splitting Rationale

This spec was originally a comprehensive design for closed-loop governance rule evolution. It has been split into 5 smaller, independently deliverable specs to support:

1. **Platform evolution**: Synodic is moving from a hooks-only CLI to a full platform with PostgreSQL, REST API, and web dashboard
2. **Incremental delivery**: Each spec can be implemented and tested independently
3. **Clearer ownership**: Database, UX, scoring, probing, and lifecycle are distinct concerns

## Child Specs

### [Spec 072: Governance Data Model & Storage Layer](../072-governance-data-model/README.md)

**Foundation** — PostgreSQL schema (SQLite for demo), storage abstractions, migrations.

**Key deliverables**:
- Tables: `rules`, `threat_categories`, `feedback_events`, `scoring_snapshots`, `probe_results`
- Storage trait with PG and SQLite implementations
- Migration system (sqlx)
- Seed data: 10 threat categories

**Why first**: Everything else depends on persistent storage.

### [Spec 073: Feedback Ingestion & Override UX](../073-feedback-ingestion-override/README.md)

**Critical path** — User override flow, feedback signals, reason clustering.

**Key deliverables**:
- Interactive override prompt in `intercept.sh` (TTY detection)
- `synodic feedback` command (records override/confirmed/ci_failure/incident)
- Override reason capture and semantic clustering
- API endpoints: `POST /api/feedback`, `GET /api/feedback`
- Bayesian Beta(α,β) updates on feedback

**Why second**: The entire feedback loop depends on override signals. This is the missing piece in the current implementation.

### [Spec 074: Governance Scoring Engine](../074-governance-scoring-engine/README.md)

**Analytics** — S(R), F(R), C(R) computation, constitutional constraints.

**Key deliverables**:
- Three golden functions: Safety, Friction, Coverage scores
- Coverage gap detection (uncovered threat categories)
- `synodic status` command with formatted output
- Constitutional constraints validation (read-freedom, bounded-scope, etc.)
- API endpoints: `GET /api/stats`, `GET /api/stats/history`
- Dashboard integration: Governance Health panel

**Why third**: Requires feedback data (spec 073) to compute meaningful scores.

### [Spec 075: Adversarial Probing](../075-adversarial-probing/README.md)

**Robustness** — Generate evasion variants, backtest expansions, escape local optima.

**Key deliverables**:
- 5 probe strategies: syntactic variation, indirection, encoding, semantic equivalence, path traversal
- Backtest framework (test expansions against known-safe commands)
- `synodic probe` command with interactive approval
- API endpoints: `POST /api/probe`, `GET /api/probe/:job_id`, `POST /api/probe/:job_id/apply`
- Scheduled weekly probes

**Why fourth**: Can be developed in parallel with spec 076. Depends on scoring (spec 074) for gap detection.

### [Spec 076: Rule Lifecycle & Convergence](../076-rule-lifecycle-convergence/README.md)

**Integration** — State machine, evidentiary standards, crystallization pipeline.

**Key deliverables**:
- Lifecycle states: candidate → active → tuned → crystallized/deprecated
- Evidentiary standards hierarchy (probable cause → beyond reasonable doubt)
- Convergence detection (per-rule and system-level)
- Crystallization pipeline (L2 → L1 git hook generation)
- CLI commands: `synodic optimize`, `synodic rules promote`, `synodic rules crystallize`
- Cross-project validation (optional central governance service)

**Why fifth**: Ties everything together. Requires feedback (073), scoring (074), and optionally probing (075).

## Implementation Order

**Dependency graph**:
```
072 (Data Model)
 │
 ├──> 073 (Feedback & Override)
 │     │
 │     └──> 074 (Scoring Engine)
 │            │
 │            ├──> 075 (Adversarial Probing)
 │            │
 │            └──> 076 (Rule Lifecycle)
```

**Recommended sequence**:
1. **Phase 1**: Spec 072 (database foundation)
2. **Phase 2**: Spec 073 (feedback loop — highest value)
3. **Phase 3**: Spec 074 (scoring and analytics)
4. **Phase 4**: Specs 075 + 076 in parallel (both depend on 074, no cross-dependency)

## Original Spec Summary

The original spec 071 covered:

- **The problem**: Open-loop governance (hand-authored rules, no learning) degrades as agent behavior evolves
- **Three golden functions**: S(R) safety, F(R) friction, C(R) coverage — together prevent local optima
- **Convergence framework**: Bayesian Beta distributions, adversarial probing, coverage constraints
- **Feedback mechanisms**: Override signals with reasoning, CI failures, incidents
- **Rule lifecycle**: candidate → active → tuned → crystallized → deprecated
- **Constitutional constraints**: Meta-rules that no optimization can violate
- **Evidentiary standards**: Different thresholds for different transitions

## Overview

Specs 068 and 069 define two governance axes (L1/L2) and two governance modes (intercept/audit). This produces a 2×2 matrix. But the matrix was missing a third dimension: **feedback** — using audit outcomes to improve intercept rules, and using intercept telemetry to refine audit focus.

Without feedback, governance is **open-loop**: rules are hand-authored, never learn from outcomes, and degrade silently as agent behavior evolves. Spec 069 sketched a "crystallization pipeline" (L2 audit → L1 rules) but didn't address the harder questions: what makes a good rule? how do you measure improvement? how do you know you're converging toward genuinely better governance rather than overfitting to seen patterns?

This spec answers those questions by defining:

1. The **three golden functions** (Safety, Friction, Coverage) that together form the governance reward signal
2. A **convergence framework** that provably avoids local optima via structural constraints and adversarial exploration
3. The **feedback mechanisms** that connect audit outputs to intercept inputs

### Why now

The intercept engine (spec 069 Phase 1) is built and ships 5 default rules. Those rules were hand-authored by reasoning about threat categories. But as Synodic gets deployed across projects, we need rules to improve from evidence — and we need guarantees that "improvement" means global improvement, not just overfitting to the last incident.

## Design

### The expanded capability matrix

The 2×2 becomes 2×3:

```
              Intercept           Audit              Optimize
              (pre-action)        (post-hoc)         (closed-loop)
            ┌─────────────────┬──────────────────┬──────────────────┐
   L1       │ Git Hooks       │ CI Pipeline      │ CI pattern →     │
   (static) │ pre-commit/push │ test, lint, cov  │   hook rule      │
            │ crystallized    │ failure attrib.  │ Crystallized L2  │
            │ L2 rules        │                  │   rules arrive   │
            ├─────────────────┼──────────────────┼──────────────────┤
   L2       │ Agent Hooks     │ Session Analysis │ Audit → new rule │
   (semant.)│ PreToolUse      │ Transcript review│ Hit-rate tuning  │
            │ rule engine     │ pattern mining   │ Adversarial probe│
            │                 │                  │ Coverage check   │
            └─────────────────┴──────────────────┴──────────────────┘
```

### The problem: why naive optimization finds local optima

A governance rule set is a classifier: for each agent action, it outputs allow or block. The natural optimization is to maximize precision (of all blocks, fraction truly harmful) and recall (of all harmful actions, fraction blocked).

**This finds local optima.** Here's why:

1. **Regex specificity trap**: A rule blocking `git push --force` is locally optimal on observed data. But `git push -f`, `git push --force-with-lease` to wrong branch, or `GIT_PUSH_FORCE=1 git push` all bypass it. The rule has high precision on its narrow scope but zero recall on variants.

2. **Seen-data bias**: Rules optimized for sessions observed so far miss entire threat categories that haven't occurred yet. A rule set with 99% precision and 99% recall on historical data may have 0% recall on novel threats.

3. **Precision-recall seesaw**: Tightening a rule to eliminate a false positive narrows its scope, creating false negatives that aren't detected until the next incident. The system oscillates.

4. **Coverage collapse**: Gradient-following on per-rule metrics drives resources toward the most-triggered rules, starving coverage of rare but severe threats.

These are all instances of the same fundamental problem: **optimizing a local objective (per-rule performance on observed data) does not guarantee improvement on the global objective (total harm prevented across all possible threats).**

### The three golden functions

The governance reward is not a single number. It is three functions that together define a constrained optimization landscape. The key insight: **the third function (Coverage) is a structural constraint that prevents collapse to local optima**, analogous to how curiosity-driven exploration in RL prevents policy collapse.

#### S(R): Safety Score

**Definition**: The probability that rule set R blocks a harmful action, evaluated against the **threat distribution** T.

```
S(R) = Σ_t∈T  P(t) · I(R blocks t)
```

Where:
- T is the set of all possible harmful actions
- P(t) is the probability/severity weight of threat t
- I(R blocks t) = 1 if at least one rule in R catches t

**The problem**: T is unknown. We never observe the full threat distribution. We can only estimate it from:
- Observed blocks (actions caught by current rules)
- Observed incidents (harmful actions that slipped through)
- Synthetic threats (adversarial probing)
- Threat taxonomy (domain knowledge)

**Estimation**: Maintain an empirical threat distribution T̂ from observed data, but supplement it with the threat taxonomy (see below) to ensure structural coverage.

#### F(R): Friction Score

**Definition**: The expected developer friction imposed by rule set R per session.

```
F(R) = (blocks + override_requests + governance_latency) / sessions
```

Components:
- **blocks**: Agent actions blocked by intercept rules (each one disrupts flow)
- **override_requests**: User asked to override a block (high-friction event)
- **governance_latency**: Time added by rule evaluation (should be <100ms, effectively zero)

**Target**: F(R) < 0.02 (fewer than 2 blocks per 100 tool calls). Governance that fires constantly gets disabled.

**Measurement**: Override events are the strongest signal. A block that the user overrides is almost certainly a false positive (the user saw the context and judged it safe). This is the governance analogue of "reward from human feedback."

#### C(R): Coverage Score

**Definition**: The fraction of the **threat taxonomy** covered by at least one active rule, weighted by threat severity.

```
C(R) = Σ_c∈Categories  w(c) · I(∃ r∈R: r covers c)  /  Σ_c  w(c)
```

Where:
- Categories is the governance threat taxonomy (see below)
- w(c) is the severity weight of category c
- I(∃ r∈R: r covers c) = 1 if at least one rule in R is mapped to category c

**Why this prevents local optima**: Per-rule optimization (S and F) can converge to a rule set that is excellent within its scope but has zero coverage of entire threat categories. The coverage constraint C(R) ≥ C_min forces the optimization to maintain breadth, not just depth. This is analogous to:
- **Entropy regularization in RL**: prevents policy collapse to a single action
- **Curiosity-driven exploration**: rewards visiting unvisited states
- **Minimum set cover**: ensures structural completeness

**Target**: C(R) ≥ 0.8 for critical categories, ≥ 0.5 overall.

### The overall objective

```
maximize  S(R) - λ·F(R)

subject to  C(R) ≥ C_min
```

Where:
- λ is the friction tolerance — how much safety are we willing to trade for less friction. Higher λ = more permissive governance. Default: λ = 0.5.
- C_min is the minimum coverage threshold. Default: 0.8 for critical, 0.5 for all.

**Why this formulation works**:
- S(R) alone → maximum blocking, unusable friction
- F(R) alone → no rules, no governance
- S - λF → Pareto trade-off, but can collapse coverage
- S - λF subject to C ≥ C_min → Pareto trade-off with structural floor

### Threat taxonomy: the structural prior

The threat taxonomy is a human-authored enumeration of harm categories. It provides **structural coverage guarantees** independent of observed data — the governance equivalent of a prior distribution in Bayesian learning.

```yaml
threat_taxonomy:
  - id: data-destruction
    severity: critical
    description: Irreversible data loss (rm -rf, DROP TABLE, git reset --hard)
    examples: [rm -rf /, git clean -fdx, DROP DATABASE]
    current_rules: [destructive-git, dangerous-rm]

  - id: credential-exposure
    severity: critical
    description: Secrets, tokens, keys exposed in code/logs/args
    examples: [echo $API_KEY, hardcoded passwords, .env in commit]
    current_rules: [secrets-in-args]

  - id: system-integrity
    severity: high
    description: Modifications to system files outside project scope
    examples: [write /etc/hosts, modify /usr/bin/, chmod 777 /]
    current_rules: [writes-outside-project, writes-to-system]

  - id: supply-chain
    severity: high
    description: Dependency manipulation, malicious package install
    examples: [npm install unknown-pkg, pip install from URL, curl | bash]
    current_rules: []  # GAP — coverage = 0

  - id: network-exfiltration
    severity: high
    description: Unauthorized data transmission to external endpoints
    examples: [curl -d @/etc/passwd, wget --post-file]
    current_rules: []  # GAP

  - id: privilege-escalation
    severity: high
    description: Gaining elevated permissions
    examples: [sudo, chmod u+s, chown root]
    current_rules: []  # GAP

  - id: persistence
    severity: medium
    description: Creating backdoors, cron jobs, startup scripts
    examples: [crontab -e, write to .bashrc, launchctl load]
    current_rules: []  # GAP

  - id: git-history-manipulation
    severity: medium
    description: Rewriting shared history, force-pushing
    examples: [git rebase published, git push --force to main]
    current_rules: [destructive-git]  # partial

  - id: resource-exhaustion
    severity: medium
    description: Consuming excessive CPU, memory, disk, or network
    examples: [fork bomb, while true, dd if=/dev/zero]
    current_rules: []  # GAP

  - id: ci-pipeline-manipulation
    severity: medium
    description: Modifying CI/CD configs to bypass checks
    examples: [edit .github/workflows, disable test step]
    current_rules: []  # GAP
```

The taxonomy immediately reveals coverage gaps. The current 5 rules cover 4 of 10 categories (3 critical/high, 1 medium). C(R) = ~0.45 weighted. The coverage constraint drives rule creation toward uncovered categories — this is how the structural prior prevents local optima.

**The taxonomy itself evolves**, but on a slower timescale than rules — updated quarterly by human review, not by the feedback loop. This separation of timescales prevents the optimization from gaming its own objective.

### Convergence framework

#### Why convergence is non-trivial

The governance optimization has properties that make convergence hard:
1. **Non-stationary environment**: Agent behavior evolves, new tools emerge
2. **Sparse reward signal**: Harmful actions are rare; most evaluations are true negatives
3. **Delayed feedback**: Some harm is only detectable weeks later (e.g., a vulnerability in deployed code)
4. **Adversarial dynamics**: If agents learn to evade rules, rules must co-evolve

#### Convergence guarantee via layered objectives

We define convergence as: **the rule set R reaches a state where S(R), F(R), and C(R) are all within their target ranges, and the rate of rule change decreases monotonically.**

The framework achieves this through three mechanisms that operate at different scales:

**1. Per-rule convergence (local optimization, fast)**

Each rule r maintains a **confidence state** modeled as a Beta distribution:

```
r.confidence = Beta(α, β)
  α = true_positives + prior_α     (correctly blocked harmful actions)
  β = false_positives + prior_β    (incorrectly blocked legitimate actions)

precision(r) = α / (α + β)
confidence_interval(r) = width of 95% credible interval
```

A rule converges when its confidence interval narrows below a threshold (enough observations to be certain about its precision). The Beta model is Bayesian — it incorporates prior knowledge (prior_α, prior_β from the threat taxonomy) and updates smoothly with evidence.

**Why Beta, not point estimates**: Point estimates (precision = TP/TP+FP) are noisy with small samples. The Beta distribution naturally handles uncertainty — a rule with 2 TP and 0 FP is not 100% precise; it's Beta(3,1) with a wide interval. This prevents premature crystallization and premature pruning.

**2. System-level coverage (structural constraint, medium)**

Coverage is checked periodically (e.g., weekly) against the threat taxonomy:

```
For each uncovered category c:
  1. Search audit logs for actions matching c's examples
  2. If found → evidence-based rule candidate
  3. If not found → synthetic probe (generate test cases for c)
  4. If probe reveals gap → create rule candidate
  5. If probe confirms no realistic path → accept gap (document why)
```

Coverage optimization cannot converge to a local optimum because it is defined against an external structure (the taxonomy) rather than against observed data. Adding a new rule for an uncovered category always increases C(R), regardless of what the per-rule metrics say.

**3. Adversarial probing (exploration, slow)**

Periodic adversarial probing tests whether rules are robust to evasion:

```
For each active rule r:
  1. Generate N adversarial variants of patterns r blocks
     - Character substitution: --force → -f, --force-with-lease
     - Encoding: base64, hex, URL-encoded commands
     - Indirection: alias creation, script wrapping, env vars
     - Semantic equivalence: different command, same effect
  2. Test each variant against r
  3. Variants that bypass r → expansion candidates
  4. Backtest expansion against historical allow decisions
  5. If precision preserved → expand rule
  6. If precision drops → flag for human review
```

Adversarial probing is the governance analogue of **simulated annealing perturbation** — it kicks the system out of local basins by testing whether nearby rule variations capture more threats. Combined with coverage constraints, it ensures:

- **Local search** (per-rule tuning) doesn't miss nearby variants
- **Global structure** (taxonomy coverage) doesn't miss distant categories
- **Robustness** (adversarial probing) doesn't miss evasion strategies

#### Why this avoids local optima: formal argument

A local optimum in governance is a rule set R* where:
- Every individual rule is well-tuned (high precision, stable confidence)
- But there exist threats t where P(t) > 0 and I(R* blocks t) = 0
- And no single-rule mutation of R* improves S(R*) on observed data (because t hasn't been observed)

Our framework escapes this because:

1. **Coverage constraint C(R) ≥ C_min** forces R to have rules in all major threat categories, not just the observed ones. This is a **hard constraint** on the feasible set, not a soft penalty. An R* that achieves perfect S and F but fails C is infeasible — the optimizer must add rules in uncovered categories before it can claim convergence.

2. **Adversarial probing** expands the effective observation set beyond historical data. It tests the system against the threat distribution T, not just the empirical distribution T̂. This reduces the gap between what we optimize (T̂) and what we care about (T).

3. **Threat taxonomy** provides a structural decomposition that is independent of the optimization process. The optimizer cannot game it (unlike a learned objective), because the taxonomy is maintained by humans on a separate cadence.

In optimization theory terms:
- Per-rule tuning = **local search** within a basin
- Coverage constraint = **feasibility constraint** that eliminates degenerate basins
- Adversarial probing = **perturbation** that tests basin boundaries
- Threat taxonomy = **problem decomposition** into sub-problems that must all be solved

Together, these guarantee that any convergent state covers all major threat categories, is robust to known evasion strategies, and has empirically validated precision — i.e., it is a **globally competitive** solution, not just a local one.

#### Convergence detection

The system has converged when all three conditions hold simultaneously for N consecutive evaluation periods:

```
1. Per-rule: All active rules have confidence_interval(r) < 0.1
   (enough observations to trust their precision)

2. System: C(R) ≥ C_min
   (structural coverage satisfied)

3. Stability: |ΔR| / |R| < 0.05
   (fewer than 5% of rules changed in last period)
```

When converged, optimization shifts from **active search** to **monitoring mode** — adversarial probes continue but at lower frequency, and rule changes require stronger evidence (higher α threshold).

#### Divergence detection and recovery

The system diverges when:
- Rule churn exceeds threshold (rules oscillating between active/inactive)
- S(R) and F(R) move in opposite directions for multiple periods (seesaw)
- Coverage score drops below C_min

Recovery:
1. Freeze rule changes (stop automated updates)
2. Diagnose: is the environment changing (new agent behavior) or is the optimization unstable?
3. If environment change → update threat taxonomy, restart optimization with new prior
4. If instability → reduce learning rate (require more evidence per update), increase human review

### Feedback signals

Where ground truth comes from, ordered by confidence:

#### Hard signals (high confidence, delayed)

| Signal | Source | What it tells us | Latency |
|--------|--------|-------------------|---------|
| Security incident | Incident response | False negative — action was allowed that caused harm | Days–weeks |
| CI failure after allow | CI pipeline | Possible false negative — allowed action led to failing code | Minutes–hours |
| Git revert of agent work | Git history | Session output was wrong/harmful | Hours–days |
| Vulnerability scan finding | SAST/DAST | Allowed action introduced vulnerability | Hours–days |

#### Soft signals (medium confidence, fast)

| Signal | Source | What it tells us | Latency |
|--------|--------|-------------------|---------|
| User override of block | Intercept hook | Likely false positive — user judged action as safe | Immediate |
| Block accepted (no override) | Intercept hook | Likely true positive — user agreed action was unsafe | Immediate |
| Post-session review finding | `synodic review` | Pattern that audit thinks is problematic | Minutes |
| Repeated rule trigger | Intercept telemetry | Rule is frequently relevant (or frequently wrong) | Sessions |

#### Proxy signals (low confidence, immediate)

| Signal | Source | What it tells us | Latency |
|--------|--------|-------------------|---------|
| Pattern similarity to known threat | Rule engine | New action resembles blocked patterns | Immediate |
| Anomalous tool call frequency | Session telemetry | Agent behavior deviating from baseline | Immediate |
| New tool/command not seen before | Intercept hook | Unknown action, may need new rule | Immediate |

### Appeals with reasoning

A binary override (yes/no) wastes the most valuable feedback signal: *why* the user disagreed. Structured appeals capture the reasoning, turning each override into a high-quality training example for rule tuning.

**Override protocol**:

```
Block fires →
  User sees: "Blocked: [rule description]. Override? (y/N) Reason (optional): "

  If override with reason:
    β++ for the rule
    Record: { rule, action, reason, timestamp, session_id }
    Reason is indexed for pattern detection

  If override without reason:
    β++ for the rule
    Record: { rule, action, reason: null, timestamp, session_id }
    (Still useful, but less informative)

  If block accepted (no override):
    α++ for the rule
    Record: { rule, action, accepted: true, timestamp, session_id }
```

**Why reasons matter for convergence**: Override reasons cluster into patterns. If 5 overrides for `destructive-git` all say "test environment" or "throwaway branch," that's a signal for a rule refinement (allow force-push on non-protected branches), not a signal to deprecate the rule entirely. Without reasons, β just accumulates and the rule gets deprecated — a false convergence that loses safety.

**Reason pattern detection**:

```
Cluster override reasons by semantic similarity:
  "test env" / "CI branch" / "throwaway" → context: non-production
    → Suggest: add exception for non-protected branches

  "I know what I'm doing" / "intentional" → context: expert override
    → No rule change — this is the system working as designed

  "this isn't actually dangerous" / "false alarm" → context: rule is wrong
    → Flag rule for review with examples
```

### Signal → update mapping

```
User override → increment rule β (false positive evidence)
  Record override reason for pattern analysis
  If β/(α+β) > 0.2 → flag rule for review
  If β/(α+β) > 0.4 → auto-disable rule, notify

Block accepted → increment rule α (true positive evidence)
  If α/(α+β) > 0.95 and α+β > 20 → rule is stable

CI failure after allow → search for matching rule candidate
  If pattern matches existing audit finding → promote to intercept rule
  If novel → create rule candidate, start accumulating evidence

Security incident → immediate: review all rules in relevant category
  Add rule for incident pattern
  Expand adversarial probes for that category

Post-session finding (≥3 occurrences) → rule candidate
  Backtest against historical sessions
  If precision > 0.9 → promote to active rule
```

### Rule lifecycle

```
                ┌──────────┐
                │  Threat  │
                │ Taxonomy │ (human-authored, quarterly review)
                └────┬─────┘
                     │ coverage gap detected
                     ▼
              ┌──────────────┐
         ┌───▶│  Candidate   │◀── audit finding (≥3 occurrences)
         │    │  Beta(1,1)   │◀── adversarial probe gap
         │    └──────┬───────┘
         │           │ backtest precision > 0.9
         │           ▼
         │    ┌──────────────┐
         │    │   Active     │ monitoring: α, β accumulate
         │    │  Beta(α,β)   │──── override → β++
         │    └──────┬───────┘     accepted → α++
         │           │
         │     ┌─────┴──────┐
         │     │            │
         │     ▼            ▼
         │  converged    problematic
         │  (CI < 0.1)   (β/(α+β) > 0.4)
         │     │            │
         │     ▼            ▼
         │  ┌────────┐  ┌────────────┐
         │  │ Tuned  │  │ Deprecated │
         │  │ stable │  │ disabled   │
         │  └───┬────┘  └────────────┘
         │      │
         │      │ crystallization criteria met
         │      ▼
         │  ┌──────────────────┐
         │  │  Crystallized    │ promoted to L1 git hook
         │  │  (L1 rule)       │ benefits all tools
         │  └──────────────────┘
         │
         └── adversarial probe finds regression → re-activate
```

#### Evidentiary standards

Not all lifecycle transitions are equal. Higher-impact decisions require stronger evidence — the governance equivalent of "probable cause" vs "beyond reasonable doubt."

| Transition | Standard | Threshold | Rationale |
|-----------|----------|-----------|-----------|
| Block an action (real-time) | **Probable cause** | Pattern match against active rule | Low bar — blocking is reversible (user can override). False positives are caught by feedback. |
| Promote candidate → active | **Clear and convincing** | Backtest precision > 0.9, ≥5 observations | Medium bar — active rules affect all sessions. Must demonstrate value before deployment. |
| Crystallize to L1 | **Beyond reasonable doubt** | α > 30, precision > 0.95, cross-project, CI < 0.1 | High bar — L1 rules affect all tools, are harder to revert, and cannot be overridden per-session. |
| Deprecate a rule | **Preponderance of evidence** | β/(α+β) > 0.4 or zero hits in 90 days | Medium bar — removing a rule reduces coverage. Must show it causes more harm (friction) than good (safety). |
| Expand rule (adversarial probe) | **Clear and convincing** | Backtest shows <1% false positive increase | Medium bar — expansion changes existing behavior. Must not break what's working. |
| Modify threat taxonomy | **Beyond reasonable doubt** | Human review + documented justification | Highest bar — the taxonomy constrains the entire optimization. Changes to it shift the objective function itself. |

The standard hierarchy prevents two failure modes:
- **Over-caution**: If blocking required "beyond reasonable doubt," rules would never fire in time. Probable cause is enough for a reversible action.
- **Over-eagerness**: If crystallization required only "probable cause," fragile rules would get baked into L1 hooks and afflict all tools.

#### Crystallization criteria (L2 → L1 promotion)

A rule is crystallized when it meets the **beyond reasonable doubt** standard:
1. **Converged**: confidence interval < 0.1
2. **High precision**: α/(α+β) > 0.95
3. **Sufficient evidence**: α + β > 30 observations
4. **Deterministic**: The rule can be expressed as a pure pattern match (no AI judgment needed)
5. **Cross-project**: Rule has been validated in ≥2 distinct project contexts (prevents overfitting)

Criterion 5 is key for avoiding local optima: a rule that works only in one project may be capturing a project-specific convention, not a universal threat.

### Adversarial probing: detailed design

Adversarial probing is the primary mechanism for **escaping local optima**. It works by expanding the observation set beyond historical data.

#### Probe generation strategies

```yaml
probe_strategies:
  - id: syntactic-variation
    description: Alter command syntax while preserving semantics
    techniques:
      - flag_forms: "--force → -f, --force-with-lease"
      - quoting: "rm -rf / → rm -rf '/'"
      - whitespace: "rm  -rf  /"
      - expansion: "$HOME vs ~"
    applicable_to: [Command rules]

  - id: indirection
    description: Achieve same effect through different mechanism
    techniques:
      - alias: "alias yeet='git push --force' && yeet"
      - script: "echo 'git push --force' > /tmp/x.sh && bash /tmp/x.sh"
      - env_var: "GIT_FORCE=1 git push (hypothetical)"
      - subshell: "bash -c 'git push --force'"
    applicable_to: [Command rules]

  - id: encoding
    description: Obfuscate payload to bypass pattern matching
    techniques:
      - base64: "echo Z2l0IHB1c2ggLS1mb3JjZQ== | base64 -d | bash"
      - hex: "echo -e '\x67\x69\x74' (git)"
      - variable_interpolation: "g=git; p=push; f=--force; $g $p $f"
    applicable_to: [Command rules, Pattern rules]

  - id: semantic-equivalence
    description: Different command, same harmful effect
    techniques:
      - alternative_tools: "rm vs unlink vs shred"
      - api_level: "file write vs echo redirect vs tee"
      - language_level: "bash vs python -c vs perl -e"
    applicable_to: [All rules]

  - id: path-traversal
    description: Reach restricted paths through indirection
    techniques:
      - symlinks: "ln -s /etc/passwd ./safe-file && edit safe-file"
      - relative: "../../../../../../etc/passwd"
      - canonical: "/etc/../etc/passwd"
    applicable_to: [Path rules]
```

#### Probe evaluation

```
For each probe p generated against rule r:
  result = intercept_engine.evaluate(p)
  if result == allow:
    # Probe bypassed the rule — this is a gap
    gap = Gap(rule=r, probe=p, strategy=p.strategy)

    # Generate candidate fix
    expanded_rule = expand_rule(r, p)

    # Backtest: does the expanded rule maintain precision?
    historical_allows = load_historical_allows()
    false_positives = [a for a in historical_allows if expanded_rule.blocks(a)]

    if len(false_positives) / len(historical_allows) < 0.01:
      # Safe to expand
      propose_rule_update(r, expanded_rule, evidence=gap)
    else:
      # Expansion would cause false positives — needs human judgment
      flag_for_review(gap, false_positives)
```

### Implementation: what to store

**No databases.** All state is git-tracked files, consistent with Synodic's design philosophy.

```
.synodic/
├── threat-taxonomy.yml           # Human-authored threat categories
├── rules/
│   ├── active/                   # Currently enforced rules
│   │   ├── destructive-git.yml   # Rule definition + Beta state
│   │   └── secrets-in-args.yml
│   ├── candidates/               # Proposed rules awaiting evidence
│   └── deprecated/               # Disabled rules (kept for history)
├── telemetry/
│   ├── summary.yml               # Aggregate metrics (S, F, C scores)
│   └── overrides.log             # User override events (append-only)
└── probes/
    └── last-run.yml              # Results of most recent adversarial probe
```

Rule file format:

```yaml
id: destructive-git
description: Block destructive git operations
category: data-destruction        # maps to threat taxonomy
tools: [Bash]
condition:
  type: command
  pattern: 'git\s+(reset\s+--hard|push\s+--force|push\s+-f|clean\s+-fd)'
lifecycle: tuned                  # candidate|active|tuned|crystallized|deprecated
confidence:
  alpha: 24                       # true positives + prior
  beta: 2                         # false positives + prior
  prior_alpha: 2                  # from threat taxonomy severity
  prior_beta: 1
last_updated: 2026-03-30
```

### Synodic CLI extensions

```bash
# Report a user override with reason (called by intercept.sh when user bypasses block)
synodic feedback --rule destructive-git --signal override --reason "test environment"

# Report a confirmed block (called by intercept.sh when block is accepted)
synodic feedback --rule destructive-git --signal confirmed

# Run adversarial probes against all active rules
synodic probe [--category data-destruction] [--strategy syntactic-variation]

# Show current S/F/C scores and convergence status
synodic status

# Propose rule candidates from audit findings
synodic optimize [--dry-run]

# Promote a candidate rule to active
synodic rules promote <rule-id>

# Promote a tuned rule to L1 (crystallize)
synodic rules crystallize <rule-id>
```

### Constitutional constraints

The threat taxonomy defines *what* to govern. Constitutional constraints define the **boundaries of governance itself** — meta-rules that no governance rule may violate, regardless of what the optimization process produces.

```yaml
constitutional_constraints:
  - id: right-of-appeal
    constraint: Override must always be available for any block
    rationale: >
      A governance system with no escape hatch becomes adversarial to the developer.
      Every block must be overridable with a recorded reason.

  - id: right-of-explanation
    constraint: Every block must include a human-readable reason and rule ID
    rationale: >
      An unexplained block is indistinguishable from a bug. The developer must
      be able to understand why an action was blocked and which rule triggered it.

  - id: read-freedom
    constraint: No rule may block file reads or directory listings
    rationale: >
      Reading is observation, not mutation. Blocking reads prevents the agent
      from understanding the codebase, which degrades output quality without
      preventing harm.

  - id: project-scope-presumption
    constraint: Actions within the project root are presumed legitimate unless a specific rule applies
    rationale: >
      The project directory is the agent's workspace. Governance should restrict
      actions that escape this scope (system files, network, other projects),
      not actions within it.

  - id: bounded-scope
    constraint: Every rule must specify target tools or paths; no rule may have unbounded blocking scope
    rationale: >
      A rule that matches "all tools, all inputs" is a kill switch, not governance.
      Scope must be explicit so that impact is predictable and auditable.

  - id: non-retroactivity
    constraint: New or modified rules apply only to future actions, never to past sessions
    rationale: >
      Retroactive application of new rules creates noise (flagging previously-approved
      work as violations), erodes trust, and conflates "the rule didn't exist then"
      with "the action was wrong." Past sessions were governed by the rules that
      existed at the time.
```

Constitutional constraints are **not subject to the optimization loop**. They cannot be weakened by evidence, overridden by adversarial probes, or deprecated by low hit rates. They are the fixed frame within which all rule evolution occurs.

The optimization's feasible set is: `{R : C(R) ≥ C_min ∧ ∀r ∈ R: r satisfies all constitutional constraints}`. Any proposed rule that violates a constraint is rejected before it enters the candidate pool.

### Human-in-the-loop gates

Automation proposes; humans approve:

| Action | Automated? | Human gate |
|--------|-----------|------------|
| Accumulate α/β evidence | Yes | No |
| Flag rule for review (high FP) | Yes | No |
| Disable rule (β/(α+β) > 0.4) | Yes | Notification |
| Create rule candidate from audit | Yes | No |
| Promote candidate → active | Requires human | `synodic rules promote` |
| Expand rule from adversarial probe | Requires human | Review + `synodic rules promote` |
| Crystallize to L1 | Requires human | `synodic rules crystallize` |
| Update threat taxonomy | Requires human | Direct edit + review |

## Plan

### Phase 1: Telemetry foundation

- [ ] Add `synodic feedback` command — records override/confirmed signals
- [ ] Extend intercept.sh to call `synodic feedback --signal confirmed` on block-accepted and `synodic feedback --signal override` on user bypass
- [ ] Implement Beta(α,β) confidence tracking per rule in rule YAML files
- [ ] Add `synodic status` command — displays S/F/C scores from telemetry

### Phase 2: Coverage constraints

- [ ] Author initial threat taxonomy (threat-taxonomy.yml) with 10 categories
- [ ] Implement coverage score C(R) computation against taxonomy
- [ ] Identify gaps: categories with no active rules
- [ ] Create rule candidates for top-priority uncovered categories (supply-chain, network-exfiltration, privilege-escalation)

### Phase 3: Adversarial probing

- [ ] Implement `synodic probe` — generates syntactic variants of blocked patterns
- [ ] Implement backtest: test expanded rules against historical allow decisions
- [ ] Generate expansion proposals with evidence
- [ ] At least: syntactic-variation and indirection strategies

### Phase 4: Feedback loop integration

- [ ] `synodic optimize` — scans audit logs, proposes rule candidates from recurring findings
- [ ] Implement rule lifecycle state machine (candidate → active → tuned → crystallized/deprecated)
- [ ] Convergence detection: alert when all three conditions met
- [ ] Divergence detection: alert when rule churn or seesaw detected

### Phase 5: Crystallization pipeline

- [ ] `synodic rules crystallize` — generates git hook check from tuned L2 rule
- [ ] Cross-project validation gate (criterion 5)
- [ ] Integration test: crystallized rule blocks in pre-commit for non-Claude tools

## Test

- [ ] `synodic feedback --signal override` increments β for the specified rule
- [ ] `synodic feedback --signal confirmed` increments α for the specified rule
- [ ] `synodic status` correctly computes S, F, C scores from rule state
- [ ] Coverage score drops when a rule covering a unique category is deprecated
- [ ] Rule with β/(α+β) > 0.4 is auto-disabled with notification
- [ ] `synodic probe` generates at least 3 syntactic variants per command rule
- [ ] Adversarial variant that bypasses a rule is flagged as a gap
- [ ] Expanded rule that causes false positives is flagged for human review (not auto-applied)
- [ ] `synodic optimize` generates rule candidate from audit finding with ≥3 occurrences
- [ ] Convergence detected when all three conditions hold for 3 consecutive periods
- [ ] Crystallized rule produces a working git hook check
- [ ] End-to-end: override signal → β increase → rule disabled → coverage gap → candidate created → backtest → promoted → converges
- [ ] Override with reason "test environment" is recorded and clustered with similar reasons
- [ ] Override reasons clustered as "non-production" suggest rule refinement, not deprecation
- [ ] Proposed rule that blocks file reads is rejected by constitutional constraint (read-freedom)
- [ ] Proposed rule with unbounded scope (no tools, no paths) is rejected by constitutional constraint (bounded-scope)
- [ ] New rule added today does not flag actions from yesterday's sessions (non-retroactivity)
- [ ] Candidate → active promotion requires "clear and convincing" standard (precision > 0.9, ≥5 observations)
- [ ] Crystallization requires "beyond reasonable doubt" standard (full criteria met)

## Notes

### Relationship to spec 042 (Continuous Improvement Loop)

Spec 042 described Kaizen for factory coordination (A/B testing of pipeline configurations, retrospectives, pattern libraries). That spec is archived because it assumed the factory infrastructure. This spec applies the same *philosophy* — closed-loop improvement with human oversight — but to the governance layer specifically, using the L1/L2 framework.

Key differences:
- 042 optimized pipeline *configurations* (which coordination primitive at which station)
- 071 optimizes governance *rules* (which patterns to block/allow)
- 042 used A/B testing; 071 uses Bayesian updating + adversarial probing
- Both share: human-in-the-loop gates, convergence detection, pattern accumulation

### Why not ML?

The feedback loop uses Bayesian statistics (Beta distributions), not ML models. Reasons:

1. **Sample size**: Governance events are rare. A rule might fire 5 times per month. ML needs thousands of examples.
2. **Interpretability**: Every rule must be human-readable. A neural classifier that blocks actions is a black box — unacceptable for governance.
3. **Auditability**: Beta(α,β) is fully interpretable: "this rule has 24 confirmed blocks and 2 false positives." Try explaining a gradient-boosted classifier's decision to an auditor.
4. **Determinism**: Governance decisions must be reproducible. Same input → same output. ML models have stochastic elements.

ML may be appropriate at the L2 audit layer (AI judge analyzing session transcripts). But the intercept layer — the one that blocks agent actions in real-time — must be deterministic, interpretable, and auditable.

### On the reward function and Goodhart's Law

Goodhart's Law: "When a measure becomes a target, it ceases to be a good measure." Applied to governance:

- If we optimize only for S(R), rules will block everything → F explodes
- If we optimize only for F(R), rules will allow everything → S drops to zero
- If we optimize S - λF without coverage constraint, rules will specialize on observed threats → coverage collapses

The three-function formulation resists Goodhart's by:
1. **Multi-objective**: No single metric can be gamed without degrading another
2. **Structural constraint**: Coverage is defined against an external taxonomy, not against the optimization's own data
3. **Human-maintained taxonomy**: The taxonomy is updated by humans on a separate cadence, preventing the optimizer from shifting the goalposts
4. **Adversarial validation**: Probing tests the system against what it *should* catch, not just what it *has* caught

---

## Migration Note

**⚠️ This spec has been split into 072-076.** The content below is preserved for historical context, but readers should refer to the child specs for implementation:

- **[072: Data Model](../072-governance-data-model/README.md)** — PostgreSQL schema, storage layer
- **[073: Feedback & Override](../073-feedback-ingestion-override/README.md)** — User override UX, signal recording
- **[074: Scoring Engine](../074-governance-scoring-engine/README.md)** — S/F/C computation, coverage gaps
- **[075: Adversarial Probing](../075-adversarial-probing/README.md)** — Evasion testing, rule expansion
- **[076: Rule Lifecycle](../076-rule-lifecycle-convergence/README.md)** — State machine, crystallization

The split was motivated by Synodic's evolution toward a platform architecture (PostgreSQL, REST API, web dashboard) rather than a file-based CLI tool.

---

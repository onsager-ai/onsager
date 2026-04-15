---
status: complete
created: 2026-03-30
completed: 2026-04-03
priority: high
tags:
- governance
- scoring
- metrics
- coverage
depends_on:
- "072"
- "073"
created_at: 2026-03-30T00:00:00Z
updated_at: 2026-03-30T00:00:00Z
---

# Governance Scoring Engine

> **Status**: complete · **Priority**: high · **Created**: 2026-03-30

## Overview

Governance quality is multi-dimensional. A rule set with 100% safety but 50% false positives is unusable. A rule set with 0% false positives but 20% coverage leaves major threat categories unprotected. This spec implements the **three golden functions** from spec 071 that together define governance effectiveness:

1. **S(R)**: Safety score — probability that harmful actions are blocked
2. **F(R)**: Friction score — developer friction per session
3. **C(R)**: Coverage score — fraction of threat taxonomy covered

These scores are computed from telemetry (spec 073) and threat taxonomy (spec 072), exposed via `synodic status` CLI command and dashboard API.

## Design

### The three golden functions

#### S(R): Safety Score

**Definition**: The probability that rule set R blocks a harmful action, evaluated against the threat distribution.

```
S(R) = Σ_t∈T  P(t) · I(R blocks t)
```

Where:
- T = set of all possible harmful actions
- P(t) = probability/severity weight of threat t
- I(R blocks t) = 1 if at least one rule in R catches t, else 0

**The problem**: We can't enumerate T (all possible threats). We estimate from:
1. **Observed blocks** — actions caught by rules (high confidence)
2. **Observed incidents** — harmful actions that slipped through (definite gaps)
3. **Threat taxonomy examples** — known attack patterns (medium confidence)

**Estimation**:

```rust
pub async fn compute_safety_score(storage: &dyn Storage) -> Result<f64> {
    let rules = storage.get_rules(active_only = true).await?;
    let categories = storage.get_threat_categories().await?;
    let incidents = storage.get_feedback(FeedbackFilters {
        signal_type: Some("incident".to_string()),
        since: Some(Utc::now() - Duration::days(90)), // Last 90 days
        ..Default::default()
    }).await?;

    let mut total_weight = 0.0;
    let mut covered_weight = 0.0;

    for category in categories {
        total_weight += category.severity_weight;

        // Check if any active rule covers this category
        let covered = rules.iter().any(|r| r.category_id == category.id);

        if covered {
            // Discount by recent incident rate for this category
            let incident_count = incidents.iter()
                .filter(|e| rule_for_category(&category.id, &rules).is_some())
                .count();

            let incident_penalty = 1.0 / (1.0 + incident_count as f64);
            covered_weight += category.severity_weight * incident_penalty;
        }
    }

    Ok(covered_weight / total_weight)
}
```

**Interpretation**:
- S(R) = 1.0 → Perfect safety (all threat categories covered, zero incidents)
- S(R) = 0.8 → Good (80% of weighted threats covered)
- S(R) = 0.5 → Mediocre (major gaps or frequent incidents)
- S(R) < 0.3 → Poor (governance is ineffective)

**Target**: S(R) ≥ 0.8 for production systems

#### F(R): Friction Score

**Definition**: Expected developer friction imposed by rule set R per session.

```
F(R) = (blocks + override_requests) / total_tool_calls
```

Components:
- **blocks**: Agent actions blocked (including overridden ones)
- **total_tool_calls**: All tool calls made (blocks + allows)

**Measurement**:

```rust
pub async fn compute_friction_score(storage: &dyn Storage, since: DateTime<Utc>) -> Result<f64> {
    // Count blocks (both confirmed and overridden)
    let blocks = storage.get_feedback(FeedbackFilters {
        signal_type: None, // All types
        since: Some(since),
        ..Default::default()
    }).await?
        .iter()
        .filter(|e| e.signal_type == "override" || e.signal_type == "confirmed")
        .count();

    // Estimate total tool calls (blocks + allows)
    // Note: We only track blocks in DB. Allows are not recorded (would be overwhelming).
    // Estimate: blocks / block_rate, where block_rate is learned from sessions with audit logs
    let estimated_total_calls = estimate_total_tool_calls(storage, since).await?;

    Ok(blocks as f64 / estimated_total_calls)
}

async fn estimate_total_tool_calls(storage: &dyn Storage, since: DateTime<Utc>) -> Result<f64> {
    // Heuristic: In observed sessions, block rate is ~1-5%.
    // Use 2% as default until we have session-level telemetry.
    // Future: Track total tool calls per session in a `sessions` table.
    const DEFAULT_BLOCK_RATE: f64 = 0.02;

    let blocks = storage.get_feedback(FeedbackFilters {
        since: Some(since),
        ..Default::default()
    }).await?.len() as f64;

    Ok(blocks / DEFAULT_BLOCK_RATE)
}
```

**Interpretation**:
- F(R) < 0.01 → Minimal friction (1 block per 100 tool calls)
- F(R) = 0.02 → Target level (2 blocks per 100 tool calls)
- F(R) > 0.05 → High friction (governance may be too aggressive)
- F(R) > 0.1 → Excessive friction (users will disable governance)

**Target**: F(R) < 0.02

**Note**: This is a simplified version. The full formula includes `governance_latency` (time added by rule evaluation), but our intercept engine runs <100ms so latency is negligible.

#### C(R): Coverage Score

**Definition**: Fraction of threat taxonomy covered by at least one active rule, weighted by severity.

```
C(R) = Σ_c∈Categories  w(c) · I(∃ r∈R: r covers c)  /  Σ_c  w(c)
```

Where:
- Categories = threat taxonomy (10 categories from spec 072)
- w(c) = severity weight (critical=1.0, high=0.7, medium=0.4, low=0.2)
- I(∃ r∈R: r covers c) = 1 if at least one rule maps to category c

**Measurement**:

```rust
pub async fn compute_coverage_score(storage: &dyn Storage) -> Result<CoverageScore> {
    let rules = storage.get_rules(active_only = true).await?;
    let categories = storage.get_threat_categories().await?;

    let mut total_weight = 0.0;
    let mut covered_weight = 0.0;
    let mut covered_categories = 0;
    let mut gaps = Vec::new();

    for category in &categories {
        total_weight += category.severity_weight;

        let covered = rules.iter().any(|r| r.category_id == category.id && r.enabled);

        if covered {
            covered_weight += category.severity_weight;
            covered_categories += 1;
        } else {
            gaps.push(CoverageGap {
                category_id: category.id.clone(),
                category_name: category.name.clone(),
                severity: category.severity.clone(),
                examples: category.examples.clone(),
            });
        }
    }

    Ok(CoverageScore {
        score: covered_weight / total_weight,
        covered_categories,
        total_categories: categories.len(),
        critical_coverage: compute_critical_coverage(&rules, &categories),
        gaps,
    })
}

fn compute_critical_coverage(rules: &[Rule], categories: &[ThreatCategory]) -> f64 {
    let critical_categories: Vec<_> = categories.iter()
        .filter(|c| c.severity == "critical")
        .collect();

    let covered_critical = critical_categories.iter()
        .filter(|c| rules.iter().any(|r| r.category_id == c.id && r.enabled))
        .count();

    covered_critical as f64 / critical_categories.len() as f64
}
```

**Interpretation**:
- C(R) = 1.0 → Full coverage (all categories have ≥1 rule)
- C(R) = 0.8 → Good (4 of 5 critical categories covered)
- C(R) = 0.5 → Mediocre (major gaps, especially in critical categories)
- C(R) < 0.3 → Poor (most categories uncovered)

**Target**: C(R) ≥ 0.8 for critical categories, ≥ 0.5 overall

**Why coverage prevents local optima**: Per-rule optimization (α, β) can converge to a rule set that is excellent within its scope but has zero coverage of entire threat categories. The coverage constraint forces breadth, not just depth.

### The overall objective

```
maximize  S(R) - λ·F(R)

subject to  C(R) ≥ C_min
```

Where:
- λ = friction tolerance (how much safety to trade for less friction). Default: λ = 0.5.
- C_min = minimum coverage threshold. Default: 0.8 for critical, 0.5 overall.

**Optimization interpretation**:
- If S(R) - 0.5·F(R) increases → improvement (more safety, less friction)
- If C(R) falls below threshold → violation (coverage floor not met)

### Constitutional constraints

Meta-rules that no governance rule may violate, regardless of optimization:

```rust
pub enum ConstitutionalConstraint {
    RightOfAppeal,        // Every block must be overridable
    RightOfExplanation,   // Every block must include reason + rule ID
    ReadFreedom,          // No rule may block file reads or listings
    ProjectScopePresumption, // Actions in project root are presumed legitimate
    BoundedScope,         // Every rule must specify target tools or paths
    NonRetroactivity,     // New rules apply only to future actions
}

pub fn validate_rule(rule: &Rule) -> Result<(), ConstraintViolation> {
    // ReadFreedom: No blocking Read tool
    if rule.tools.contains(&"Read".to_string()) {
        return Err(ConstraintViolation::ReadFreedom);
    }

    // BoundedScope: Must specify tools or paths
    if rule.tools.is_empty() && !rule.has_path_constraint() {
        return Err(ConstraintViolation::BoundedScope);
    }

    // RightOfExplanation: Description must be present
    if rule.description.is_empty() {
        return Err(ConstraintViolation::RightOfExplanation);
    }

    Ok(())
}
```

**Enforcement**: Constitutional constraints are checked on rule creation/update. Violating rules are rejected before entering the DB.

### `synodic status` command

```
synodic status [--project <id>] [--since <date>]
```

**Output**:
```
Governance Status

Safety Score (S):     0.82  ✅ Target: ≥0.80
Friction Score (F):   0.018 ✅ Target: <0.02
Coverage Score (C):   0.75  ⚠️  Target: ≥0.80

Coverage Gaps (3):
  ❌ supply-chain (high severity)
     Examples: npm install unknown-pkg, pip install from URL
  ❌ network-exfiltration (high severity)
     Examples: curl -d @/etc/passwd, wget --post-file
  ❌ privilege-escalation (high severity)
     Examples: sudo, chmod u+s, chown root

Active Rules (5):
  ✅ destructive-git (data-destruction) — α=24, β=2, precision=92%
  ✅ secrets-in-args (credential-exposure) — α=18, β=1, precision=95%
  ✅ writes-outside-project (system-integrity) — α=12, β=3, precision=80%
  ✅ writes-to-system (system-integrity) — α=8, β=1, precision=89%
  ✅ dangerous-rm (data-destruction) — α=31, β=0, precision=97%

Recommendations:
  1. Add rules for 3 uncovered high-severity categories
  2. Review 'writes-outside-project' (precision <90%, high β)
```

**Implementation**:

```rust
pub struct StatusCmd {
    project: Option<String>,
    since: Option<DateTime<Utc>>,
}

impl StatusCmd {
    pub async fn run(&self) -> Result<()> {
        let storage = get_storage().await?;
        let since = self.since.unwrap_or_else(|| Utc::now() - Duration::days(30));

        let safety = compute_safety_score(&storage).await?;
        let friction = compute_friction_score(&storage, since).await?;
        let coverage = compute_coverage_score(&storage).await?;

        // Print scores
        println!("Governance Status\n");
        print_score("Safety Score (S)", safety, 0.8);
        print_score("Friction Score (F)", friction, 0.02);
        print_score("Coverage Score (C)", coverage.score, 0.8);

        // Print gaps
        if !coverage.gaps.is_empty() {
            println!("\nCoverage Gaps ({}):", coverage.gaps.len());
            for gap in coverage.gaps {
                println!("  ❌ {} ({} severity)", gap.category_id, gap.severity);
                println!("     Examples: {}", gap.examples.join(", "));
            }
        }

        // Print active rules with Beta stats
        let rules = storage.get_rules(active_only = true).await?;
        println!("\nActive Rules ({}):", rules.len());
        for rule in rules {
            let precision = rule.alpha as f64 / (rule.alpha + rule.beta) as f64;
            println!("  ✅ {} ({}) — α={}, β={}, precision={:.0}%",
                rule.id, rule.category_id, rule.alpha, rule.beta, precision * 100.0);
        }

        // Recommendations
        println!("\nRecommendations:");
        if coverage.score < 0.8 {
            println!("  1. Add rules for {} uncovered high-severity categories",
                coverage.gaps.iter().filter(|g| g.severity == "high" || g.severity == "critical").count());
        }
        for rule in &rules {
            let precision = rule.alpha as f64 / (rule.alpha + rule.beta) as f64;
            if precision < 0.9 && rule.beta > 2 {
                println!("  2. Review '{}' (precision <90%, high β)", rule.id);
            }
        }

        Ok(())
    }
}

fn print_score(label: &str, value: f64, target: f64) {
    let status = if value >= target { "✅" } else { "⚠️" };
    println!("{:20} {:.2}  {} Target: ≥{:.2}", label, value, status, target);
}
```

### API endpoints

#### `GET /api/stats`

Get current governance scores.

**Response**:
```json
{
  "safety": 0.82,
  "friction": 0.018,
  "coverage": {
    "score": 0.75,
    "covered_categories": 7,
    "total_categories": 10,
    "critical_coverage": 1.0,
    "gaps": [
      {
        "category_id": "supply-chain",
        "category_name": "Supply Chain",
        "severity": "high",
        "examples": ["npm install unknown-pkg"]
      }
    ]
  },
  "computed_at": "2026-03-30T12:34:56Z"
}
```

#### `GET /api/stats/history?since=2026-03-01`

Get time-series of scores (from `scoring_snapshots` table).

**Response**:
```json
{
  "snapshots": [
    {
      "safety": 0.78,
      "friction": 0.022,
      "coverage": 0.70,
      "created_at": "2026-03-01T00:00:00Z"
    },
    {
      "safety": 0.82,
      "friction": 0.018,
      "coverage": 0.75,
      "created_at": "2026-03-15T00:00:00Z"
    }
  ]
}
```

### Dashboard integration

Add "Governance Health" panel to UI:

```typescript
// packages/ui/src/components/GovernanceHealth.tsx
export function GovernanceHealth() {
  const [stats, setStats] = useState<Stats | null>(null)

  useEffect(() => {
    fetchStats().then(setStats)
  }, [])

  return (
    <div className="governance-health">
      <h2>Governance Health</h2>
      <div className="scores">
        <Score label="Safety" value={stats.safety} target={0.8} />
        <Score label="Friction" value={stats.friction} target={0.02} inverse />
        <Score label="Coverage" value={stats.coverage.score} target={0.8} />
      </div>

      {stats.coverage.gaps.length > 0 && (
        <div className="gaps">
          <h3>Coverage Gaps</h3>
          {stats.coverage.gaps.map(gap => (
            <GapCard key={gap.category_id} gap={gap} />
          ))}
        </div>
      )}
    </div>
  )
}
```

**Score visualization**: Use a gauge or progress bar with color-coded status (green ≥ target, yellow close, red far).

### Periodic snapshots

Scores should be computed and stored periodically for trend analysis.

**Cron job** (runs daily):
```bash
# docker/cron/daily-snapshot.sh
#!/bin/bash
synodic status --json | jq '.safety, .friction, .coverage.score' | \
  xargs -I {} psql $DATABASE_URL -c \
    "INSERT INTO scoring_snapshots (safety_score, friction_score, coverage_score) VALUES ({})"
```

Or via Rust cron (using `tokio-cron-scheduler`):

```rust
// harness-http/src/jobs/snapshots.rs
use tokio_cron_scheduler::{JobScheduler, Job};

pub async fn schedule_snapshot_jobs(storage: Arc<dyn Storage>) -> Result<()> {
    let sched = JobScheduler::new().await?;

    sched.add(Job::new_async("0 0 * * *", move |_uuid, _l| {
        let storage = storage.clone();
        Box::pin(async move {
            let safety = compute_safety_score(&*storage).await?;
            let friction = compute_friction_score(&*storage, Utc::now() - Duration::days(1)).await?;
            let coverage = compute_coverage_score(&*storage).await?;

            storage.record_scores(GovernanceScores {
                safety_score: safety,
                friction_score: friction,
                coverage_score: coverage.score,
                // ... other fields
            }).await?;
        })
    }).await?).await?;

    sched.start().await?;
    Ok(())
}
```

## Plan

### Phase 1: Core scoring functions

- [ ] Implement `compute_safety_score()` in `harness-core/src/scoring.rs`
- [ ] Implement `compute_friction_score()` with estimation logic
- [ ] Implement `compute_coverage_score()` with gap detection
- [ ] Unit tests for each function

### Phase 2: Constitutional constraints

- [ ] Implement `ConstitutionalConstraint` enum
- [ ] Implement `validate_rule()` function
- [ ] Add constraint validation to `Storage::create_rule()` and `update_rule()`
- [ ] Test that violating rules are rejected

### Phase 3: `synodic status` command

- [ ] Add `StatusCmd` to CLI
- [ ] Implement formatted output (scores, gaps, rules, recommendations)
- [ ] Add `--json` flag for machine-readable output
- [ ] Test output formatting

### Phase 4: API endpoints

- [ ] `GET /api/stats` endpoint
- [ ] `GET /api/stats/history` endpoint
- [ ] Integration tests for both endpoints

### Phase 5: Dashboard integration

- [ ] Add `GovernanceHealth` component to UI
- [ ] Add score gauge/progress bar component
- [ ] Add coverage gaps display
- [ ] Add time-series chart for score trends

### Phase 6: Periodic snapshots

- [ ] Implement snapshot cron job (tokio-cron-scheduler)
- [ ] Store snapshots in `scoring_snapshots` table
- [ ] Verify snapshots appear in `/api/stats/history`

## Test

### Unit tests
- [ ] S(R) = 1.0 when all categories have active rules and zero incidents
- [ ] S(R) < 1.0 when recent incidents exist for covered categories
- [ ] F(R) = blocks / total_tool_calls (with estimation)
- [ ] C(R) = covered_weight / total_weight
- [ ] C(R) correctly weights critical categories higher than medium
- [ ] Constitutional constraint `ReadFreedom` rejects rule blocking Read tool
- [ ] Constitutional constraint `BoundedScope` rejects rule with no tools and no paths

### Integration tests
- [ ] `synodic status` outputs correct scores based on DB state
- [ ] `synodic status` lists coverage gaps for uncovered categories
- [ ] `synodic status --json` outputs valid JSON
- [ ] `GET /api/stats` returns scores matching CLI output
- [ ] `GET /api/stats/history` returns time-series snapshots

### End-to-end
- [ ] Create 5 rules covering 5 categories → C(R) = 0.5 (5/10)
- [ ] Add 5 more rules → C(R) = 1.0 (10/10)
- [ ] Record 10 overrides → F(R) increases
- [ ] Record incident for covered category → S(R) decreases

## Notes

### Why weighted coverage?

Unweighted coverage treats all threat categories equally: preventing `rm -rf /` (critical) has the same weight as preventing `fork bomb` (medium). This is wrong — not all threats are equally severe.

Weighted coverage (using `severity_weight`) ensures critical threats are prioritized. A rule set covering 2 critical categories is better than one covering 5 medium categories.

### Why estimate total tool calls instead of tracking exactly?

Tracking every tool call (allow decisions) would create massive DB write load:
- 100 tool calls per session × 1000 sessions/day = 100K DB writes/day
- Most are allows (98%+) which contain no governance signal
- Storage costs grow linearly with session count

Instead, we track only blocks (high signal, low volume) and estimate total calls from the block rate. This trades perfect accuracy for scalability.

**Future enhancement**: Track session-level aggregates (total_tool_calls per session) instead of per-call. This gives exact F(R) with 1 DB write per session instead of 100.

### Friction score edge case: what if blocks = 0?

If no blocks occur in the measurement window, F(R) = 0 (perfect friction score). This can happen in two scenarios:

1. **Governance is working**: Rules are well-tuned, no false positives, agents have learned to avoid violations
2. **Governance is absent**: No rules are active, or rules are so narrowly scoped they never trigger

Distinguish these by checking C(R):
- F(R) = 0 and C(R) = 1.0 → Excellent governance
- F(R) = 0 and C(R) < 0.5 → Governance is inactive

This is why we need all three scores together, not just one.

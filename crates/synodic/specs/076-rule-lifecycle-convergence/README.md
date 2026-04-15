---
status: complete
created: 2026-03-30
completed: 2026-04-03
priority: high
tags:
- governance
- lifecycle
- convergence
- crystallization
depends_on:
- "072"
- "073"
- "074"
created_at: 2026-03-30T00:00:00Z
updated_at: 2026-03-30T00:00:00Z
---

# Rule Lifecycle & Convergence

> **Status**: complete · **Priority**: high · **Created**: 2026-03-30

## Overview

Rules evolve through a lifecycle based on evidence. A rule candidate with 3 audit findings becomes active. An active rule with 30 confirmed blocks becomes tuned. A tuned rule validated across 2 projects becomes crystallized (promoted to L1 git hook). This spec defines:

1. The **rule lifecycle state machine** (candidate → active → tuned → crystallized/deprecated)
2. **Evidentiary standards** for each transition (probable cause → beyond reasonable doubt)
3. **Convergence detection** (when the system reaches a stable state)
4. **Crystallization pipeline** (L2 → L1 promotion)
5. CLI commands: `synodic optimize`, `synodic rules promote`, `synodic rules crystallize`

**Why lifecycle matters**: Without a structured lifecycle, rules are either hand-written (static) or automatically mutated (unstable). The lifecycle provides guardrails — rules can improve via feedback, but high-impact transitions (crystallization, deprecation) require strong evidence.

## Design

### Rule lifecycle states

```
┌──────────┐
│  Threat  │
│ Taxonomy │ (human-authored, quarterly review)
└────┬─────┘
     │ coverage gap detected
     ▼
┌──────────────┐
│  Candidate   │◀── audit finding (≥3 occurrences)
│  Beta(1,1)   │◀── adversarial probe gap
└──────┬───────┘
       │ backtest precision > 0.9, ≥5 observations
       ▼
┌──────────────┐
│   Active     │ monitoring: α, β accumulate
│  Beta(α,β)   │──── override → β++
└──────┬───────┘     confirmed → α++
       │
 ┌─────┴──────┐
 │            │
 ▼            ▼
converged    problematic
(CI < 0.1)   (β/(α+β) > 0.4)
 │            │
 ▼            ▼
┌────────┐  ┌────────────┐
│ Tuned  │  │ Deprecated │
│ stable │  │ disabled   │
└───┬────┘  └────────────┘
    │
    │ crystallization criteria met
    ▼
┌──────────────────┐
│  Crystallized    │ promoted to L1 git hook
│  (L1 rule)       │ benefits all tools
└──────────────────┘
```

**States**:
- **Candidate**: Proposed rule awaiting evidence
- **Active**: Deployed rule accumulating feedback
- **Tuned**: Converged rule (stable α/β, narrow confidence interval)
- **Crystallized**: Promoted to L1 git hook (deterministic, fast)
- **Deprecated**: Disabled rule (high false positive rate)

### Evidentiary standards

Not all transitions are equal. Higher-impact decisions require stronger evidence.

| Transition | Standard | Threshold | Rationale |
|-----------|----------|-----------|-----------|
| Block an action (real-time) | **Probable cause** | Pattern match | Low bar — blocking is reversible (user can override) |
| Promote candidate → active | **Clear and convincing** | Backtest precision > 0.9, ≥5 observations | Medium bar — active rules affect all sessions |
| Tune active → tuned | **Clear and convincing** | CI < 0.1 (confidence interval), α+β > 20 | Medium bar — indicates rule has converged |
| Crystallize tuned → L1 | **Beyond reasonable doubt** | α > 30, precision > 0.95, cross-project, CI < 0.1 | Highest bar — L1 rules affect all tools, harder to revert |
| Deprecate active → deprecated | **Preponderance of evidence** | β/(α+β) > 0.4 or zero hits in 90 days | Medium bar — removing a rule reduces coverage |
| Expand rule (adversarial probe) | **Clear and convincing** | Backtest shows <1% FP increase | Medium bar — expansion changes existing behavior |

**Standard hierarchy prevents failure modes**:
- **Over-caution**: If blocking required "beyond reasonable doubt," rules would never fire in time
- **Over-eagerness**: If crystallization required only "probable cause," fragile rules would get baked into L1

### Transition logic

#### Candidate → Active

**Trigger**: `synodic rules promote <rule-id>`

**Requirements** (clear and convincing):
1. Backtest precision > 0.9 (rule correctly identifies threats)
2. ≥5 observations (audit findings or probe results)
3. Rule passes constitutional constraints (spec 074)
4. No duplicate rule for the same threat category

**Implementation**:
```rust
pub async fn promote_candidate(storage: &dyn Storage, rule_id: &str) -> Result<()> {
    let mut rule = storage.get_rule(rule_id).await?
        .ok_or(Error::RuleNotFound)?;

    if rule.lifecycle != "candidate" {
        return Err(Error::InvalidTransition("Rule is not a candidate"));
    }

    // Check evidence
    let observations = storage.get_feedback(FeedbackFilters {
        rule_id: Some(rule_id.to_string()),
        ..Default::default()
    }).await?.len();

    if observations < 5 {
        return Err(Error::InsufficientEvidence(format!(
            "Need ≥5 observations, found {}", observations
        )));
    }

    // Backtest precision (using known-safe commands as proxy)
    let backtest = backtest_rule(storage, &rule).await?;
    if backtest.precision < 0.9 {
        return Err(Error::InsufficientEvidence(format!(
            "Backtest precision {:.1}% < 90%", backtest.precision * 100.0
        )));
    }

    // Constitutional constraints
    validate_rule(&rule)?;

    // Promote
    rule.lifecycle = "active".to_string();
    rule.enabled = true;
    storage.update_rule(rule_id, UpdateRule {
        lifecycle: Some("active".to_string()),
        enabled: Some(true),
        updated_at: Some(Utc::now()),
        ..Default::default()
    }).await?;

    println!("✅ Promoted rule '{}' to active (precision: {:.1}%)", rule_id, backtest.precision * 100.0);
    Ok(())
}
```

#### Active → Tuned

**Trigger**: Automatic when convergence criteria met

**Requirements** (clear and convincing):
1. Confidence interval < 0.1 (enough observations to trust precision)
2. α + β > 20 (sufficient evidence)
3. Rule has been active for ≥7 days (avoid premature convergence)

**Implementation**:
```rust
pub async fn check_convergence(storage: &dyn Storage) -> Result<Vec<String>> {
    let rules = storage.get_rules(active_only = true).await?;
    let mut converged_rules = vec![];

    for rule in rules {
        if rule.lifecycle != "active" {
            continue;
        }

        // Compute confidence interval (95% credible interval for Beta distribution)
        let ci = beta_confidence_interval(rule.alpha, rule.beta, 0.95);
        let observations = rule.alpha + rule.beta;
        let age_days = (Utc::now() - rule.created_at).num_days();

        if ci < 0.1 && observations > 20 && age_days >= 7 {
            // Rule has converged
            storage.update_rule(&rule.id, UpdateRule {
                lifecycle: Some("tuned".to_string()),
                updated_at: Some(Utc::now()),
                ..Default::default()
            }).await?;

            converged_rules.push(rule.id.clone());
            println!("✅ Rule '{}' converged (CI: {:.3}, α={}, β={})",
                rule.id, ci, rule.alpha, rule.beta);
        }
    }

    Ok(converged_rules)
}

fn beta_confidence_interval(alpha: i32, beta: i32, confidence: f64) -> f64 {
    // 95% credible interval width for Beta(α, β)
    // Using approximation: CI ≈ 2 * sqrt(α*β / ((α+β)^2 * (α+β+1)))
    let a = alpha as f64;
    let b = beta as f64;
    let variance = (a * b) / ((a + b).powi(2) * (a + b + 1.0));
    let std_dev = variance.sqrt();
    2.0 * 1.96 * std_dev // 1.96 for 95% CI
}
```

#### Tuned → Crystallized

**Trigger**: `synodic rules crystallize <rule-id>`

**Requirements** (beyond reasonable doubt):
1. α > 30 (30+ confirmed blocks)
2. Precision > 0.95 (α/(α+β) > 0.95)
3. CI < 0.1 (converged)
4. Cross-project validated (tested in ≥2 projects)
5. Deterministic (can be expressed as pure pattern match, no AI judgment)
6. Rule has been tuned for ≥30 days (avoid premature crystallization)

**Why cross-project validation matters**: A rule that works only in one project may be capturing a project-specific convention, not a universal threat. Crystallizing it to L1 would afflict all projects.

**Implementation**:
```rust
pub async fn crystallize_rule(storage: &dyn Storage, rule_id: &str) -> Result<()> {
    let rule = storage.get_rule(rule_id).await?
        .ok_or(Error::RuleNotFound)?;

    if rule.lifecycle != "tuned" {
        return Err(Error::InvalidTransition("Rule must be tuned before crystallization"));
    }

    // Check criteria
    let precision = rule.alpha as f64 / (rule.alpha + rule.beta) as f64;
    let ci = beta_confidence_interval(rule.alpha, rule.beta, 0.95);
    let age_days = (Utc::now() - rule.created_at).num_days();

    if rule.alpha < 30 {
        return Err(Error::InsufficientEvidence(format!("Need α ≥ 30, found {}", rule.alpha)));
    }
    if precision < 0.95 {
        return Err(Error::InsufficientEvidence(format!("Precision {:.1}% < 95%", precision * 100.0)));
    }
    if ci >= 0.1 {
        return Err(Error::InsufficientEvidence(format!("CI {:.3} ≥ 0.1", ci)));
    }
    if !rule.cross_project_validated {
        return Err(Error::InsufficientEvidence("Rule not validated across ≥2 projects"));
    }
    if age_days < 30 {
        return Err(Error::InsufficientEvidence(format!("Rule only {} days old, need ≥30", age_days)));
    }

    // Generate L1 git hook check
    let hook_check = generate_git_hook_check(&rule)?;

    // Write to .githooks/pre-commit
    let repo_root = find_repo_root()?;
    let hook_path = repo_root.join(".githooks/pre-commit");
    let mut content = fs::read_to_string(&hook_path)?;

    // Append crystallized check
    content.push_str(&format!("\n# Crystallized from L2 rule '{}'\n{}\n", rule.id, hook_check));
    fs::write(&hook_path, content)?;

    // Update rule
    storage.update_rule(rule_id, UpdateRule {
        lifecycle: Some("crystallized".to_string()),
        crystallized_at: Some(Utc::now()),
        updated_at: Some(Utc::now()),
        ..Default::default()
    }).await?;

    println!("✅ Crystallized rule '{}' to L1 git hook", rule_id);
    println!("   Added check to .githooks/pre-commit");

    Ok(())
}

fn generate_git_hook_check(rule: &Rule) -> Result<String> {
    // Generate bash check from rule condition
    match &rule.condition {
        InterceptCondition::Command { pattern } => {
            Ok(format!(
                r#"
# Block: {}
if git diff --cached --name-only | xargs -I {{}} sh -c 'grep -E "{}" "{{}}" 2>/dev/null'; then
    echo "❌ Blocked by crystallized rule '{}': {}"
    exit 1
fi
"#,
                rule.description, pattern, rule.id, rule.description
            ))
        }
        InterceptCondition::Path { glob } => {
            Ok(format!(
                r#"
# Block writes to: {}
if git diff --cached --name-only | grep -E "{}"; then
    echo "❌ Blocked by crystallized rule '{}': {}"
    exit 1
fi
"#,
                rule.description, glob_to_regex(glob), rule.id, rule.description
            ))
        }
        _ => Err(Error::NotDeterministic("Rule condition cannot be crystallized to git hook")),
    }
}
```

#### Active → Deprecated

**Trigger**: Automatic when false positive rate exceeds threshold

**Requirements** (preponderance of evidence):
1. β/(α+β) > 0.4 (40%+ false positive rate), OR
2. Zero hits in last 90 days (rule is unused)

**Implementation**:
```rust
pub async fn check_deprecation(storage: &dyn Storage) -> Result<Vec<String>> {
    let rules = storage.get_rules(active_only = true).await?;
    let mut deprecated_rules = vec![];

    for rule in rules {
        if rule.lifecycle == "deprecated" || rule.lifecycle == "crystallized" {
            continue;
        }

        let observations = rule.alpha + rule.beta;
        let false_positive_rate = if observations > 0 {
            rule.beta as f64 / observations as f64
        } else {
            0.0
        };

        // Check for high FP rate
        if false_positive_rate > 0.4 && observations >= 10 {
            storage.update_rule(&rule.id, UpdateRule {
                lifecycle: Some("deprecated".to_string()),
                enabled: Some(false),
                updated_at: Some(Utc::now()),
                ..Default::default()
            }).await?;

            deprecated_rules.push(rule.id.clone());
            eprintln!("⚠️ Deprecated rule '{}' (FP rate: {:.1}%)", rule.id, false_positive_rate * 100.0);
            continue;
        }

        // Check for zero hits in 90 days
        let recent_feedback = storage.get_feedback(FeedbackFilters {
            rule_id: Some(rule.id.clone()),
            since: Some(Utc::now() - Duration::days(90)),
            ..Default::default()
        }).await?;

        if recent_feedback.is_empty() && observations > 5 {
            // Rule has evidence (not brand new) but zero recent hits
            storage.update_rule(&rule.id, UpdateRule {
                lifecycle: Some("deprecated".to_string()),
                enabled: Some(false),
                updated_at: Some(Utc::now()),
                ..Default::default()
            }).await?;

            deprecated_rules.push(rule.id.clone());
            eprintln!("⚠️ Deprecated rule '{}' (zero hits in 90 days)", rule.id);
        }
    }

    Ok(deprecated_rules)
}
```

### Convergence detection (system-level)

The **system** has converged when all three conditions hold simultaneously for N consecutive evaluation periods:

```
1. Per-rule: All active rules have CI < 0.1
2. System: C(R) ≥ C_min (coverage constraint satisfied)
3. Stability: |ΔR| / |R| < 0.05 (fewer than 5% of rules changed in last period)
```

**Implementation**:
```rust
pub async fn check_system_convergence(storage: &dyn Storage) -> Result<ConvergenceState> {
    let rules = storage.get_rules(active_only = true).await?;

    // 1. Per-rule convergence
    let all_converged = rules.iter().all(|r| {
        let ci = beta_confidence_interval(r.alpha, r.beta, 0.95);
        ci < 0.1
    });

    // 2. Coverage constraint
    let coverage = compute_coverage_score(storage).await?;
    let coverage_met = coverage.score >= 0.8 && coverage.critical_coverage >= 0.8;

    // 3. Stability (rule churn)
    let rule_changes = count_rule_changes(storage, Duration::days(7)).await?;
    let churn_rate = rule_changes as f64 / rules.len() as f64;
    let stable = churn_rate < 0.05;

    let converged = all_converged && coverage_met && stable;

    Ok(ConvergenceState {
        converged,
        all_rules_converged: all_converged,
        coverage_satisfied: coverage_met,
        stable,
        churn_rate,
    })
}

async fn count_rule_changes(storage: &dyn Storage, window: Duration) -> Result<usize> {
    // Count rules created, updated, or deprecated in window
    let rules = storage.get_rules(active_only = false).await?;
    Ok(rules.iter().filter(|r| {
        let age = Utc::now() - r.updated_at;
        age < window
    }).count())
}
```

**When converged**: Shift from **active search** to **monitoring mode**:
- Adversarial probes run less frequently (monthly instead of weekly)
- Rule changes require stronger evidence (higher α threshold)
- Dashboard shows "✅ Governance converged" status

### Divergence detection and recovery

The system diverges when:
- Rule churn exceeds threshold (>10% per week for 3 consecutive weeks)
- S(R) and F(R) move in opposite directions (seesaw)
- Coverage score drops below C_min

**Recovery**:
1. Freeze rule changes (stop automated updates)
2. Diagnose: Is the environment changing (new agent behavior) or is optimization unstable?
3. If environment change → Update threat taxonomy, restart optimization
4. If instability → Reduce learning rate (require more evidence per update), increase human review

**Implementation**:
```rust
pub async fn check_divergence(storage: &dyn Storage) -> Result<Option<DivergenceAlert>> {
    let snapshots = storage.get_scores(None, Utc::now() - Duration::days(21)).await?;

    if snapshots.len() < 3 {
        return Ok(None); // Not enough data
    }

    // Check rule churn (last 3 weeks)
    let churn_rates: Vec<_> = snapshots.iter().map(|s| s.rule_churn_rate).collect();
    let high_churn = churn_rates.iter().all(|&r| r > 0.1);

    // Check S/F seesaw (safety and friction moving opposite directions)
    let safety_trend = snapshots.last().unwrap().safety_score - snapshots.first().unwrap().safety_score;
    let friction_trend = snapshots.last().unwrap().friction_score - snapshots.first().unwrap().friction_score;
    let seesaw = (safety_trend > 0.0 && friction_trend > 0.0) || (safety_trend < 0.0 && friction_trend < 0.0);

    // Check coverage drop
    let coverage_drop = snapshots.last().unwrap().coverage_score < 0.5;

    if high_churn || seesaw || coverage_drop {
        return Ok(Some(DivergenceAlert {
            high_churn,
            seesaw,
            coverage_drop,
            recommendation: "Freeze rule changes and diagnose root cause".to_string(),
        }));
    }

    Ok(None)
}
```

### CLI commands

#### `synodic optimize`

Scan audit logs, propose rule candidates from recurring findings.

```
synodic optimize [--dry-run] [--since <date>]
```

**Implementation**:
```rust
pub async fn optimize(storage: &dyn Storage, dry_run: bool, since: DateTime<Utc>) -> Result<()> {
    // Find audit findings with ≥3 occurrences
    let findings = find_recurring_patterns(storage, since).await?;

    println!("Found {} recurring patterns\n", findings.len());

    for finding in findings {
        println!("Pattern: {}", finding.pattern);
        println!("  Occurrences: {}", finding.count);
        println!("  Category: {}", finding.category);
        println!("  Example: {}", finding.example);

        if !dry_run {
            // Create rule candidate
            let rule = create_rule_candidate(storage, &finding).await?;
            println!("  ✅ Created candidate rule '{}'", rule.id);
        }
        println!();
    }

    Ok(())
}

async fn find_recurring_patterns(storage: &dyn Storage, since: DateTime<Utc>) -> Result<Vec<Finding>> {
    // Scan feedback events for CI failures and incidents
    let events = storage.get_feedback(FeedbackFilters {
        signal_type: Some("ci_failure".to_string()),
        since: Some(since),
        ..Default::default()
    }).await?;

    // Group by pattern (simplified: group by tool_name + command prefix)
    let mut pattern_counts: HashMap<String, Vec<FeedbackEvent>> = HashMap::new();

    for event in events {
        let pattern = extract_pattern(&event);
        pattern_counts.entry(pattern).or_insert_with(Vec::new).push(event);
    }

    // Filter to ≥3 occurrences
    let findings: Vec<_> = pattern_counts.into_iter()
        .filter(|(_, events)| events.len() >= 3)
        .map(|(pattern, events)| Finding {
            pattern,
            count: events.len(),
            category: infer_category(&events[0]),
            example: format!("{:?}", events[0].tool_input),
        })
        .collect();

    Ok(findings)
}
```

#### `synodic rules promote <rule-id>`

Promote candidate → active (manual approval required).

#### `synodic rules crystallize <rule-id>`

Crystallize tuned → L1 git hook (manual approval required).

### Cross-project validation

**Problem**: How does telemetry flow across projects? Each project has its own DB.

**Solution**: Centralized governance service (optional).

```
Project A (.synodic/rules.db) ──┐
Project B (.synodic/rules.db) ──┼──> Central Governance API
Project C (.synodic/rules.db) ──┘      (aggregates telemetry)
                                        ↓
                                  Global rule candidates
                                  Cross-project validation
```

**Local-first alternative**: Projects can opt-in to share telemetry by configuring a `SYNODIC_TELEMETRY_ENDPOINT`:

```bash
# .env
SYNODIC_TELEMETRY_ENDPOINT=https://governance.example.com/api/telemetry
```

When set, `synodic feedback` sends telemetry to both local DB and central endpoint.

**Cross-project validation check**:
```rust
async fn check_cross_project_validation(rule_id: &str) -> Result<bool> {
    let endpoint = env::var("SYNODIC_TELEMETRY_ENDPOINT").ok();

    if let Some(url) = endpoint {
        // Query central service
        let res: ProjectValidation = reqwest::get(format!("{}/rules/{}/projects", url, rule_id))
            .await?
            .json()
            .await?;

        Ok(res.validated_projects >= 2)
    } else {
        // Local-only: cannot validate cross-project
        Ok(false)
    }
}
```

## Plan

### Phase 1: State machine implementation

- [ ] Add lifecycle state transitions to `Storage` trait
- [ ] Implement `promote_candidate()` function
- [ ] Implement `check_convergence()` (active → tuned)
- [ ] Implement `check_deprecation()` (active → deprecated)

### Phase 2: Crystallization pipeline

- [ ] Implement `generate_git_hook_check()` for Command and Path rules
- [ ] Implement `crystallize_rule()` function
- [ ] Test crystallized check in `.githooks/pre-commit`
- [ ] Verify crystallized rule blocks non-Claude tools (e.g., manual bash)

### Phase 3: Convergence detection

- [ ] Implement `check_system_convergence()` function
- [ ] Implement `check_divergence()` function
- [ ] Add convergence status to `synodic status` output
- [ ] Add divergence alerts to dashboard

### Phase 4: CLI commands

- [ ] `synodic optimize` — scan audit logs, create candidates
- [ ] `synodic rules promote <id>` — candidate → active
- [ ] `synodic rules crystallize <id>` — tuned → L1
- [ ] `synodic rules deprecate <id>` — manual deprecation

### Phase 5: Cross-project validation

- [ ] Add `SYNODIC_TELEMETRY_ENDPOINT` env var support
- [ ] Implement telemetry upload to central service
- [ ] Implement cross-project validation check
- [ ] Build central governance API (optional, can defer)

## Test

### State transition tests
- [ ] Promote candidate with <5 observations → error (insufficient evidence)
- [ ] Promote candidate with precision <90% → error (backtest fails)
- [ ] Promote valid candidate → lifecycle changes to "active"
- [ ] Active rule with CI < 0.1 and α+β > 20 → auto-promotes to "tuned"
- [ ] Active rule with β/(α+β) > 0.4 → auto-deprecates

### Crystallization tests
- [ ] Crystallize tuned rule → L1 hook check generated
- [ ] L1 hook check blocks command in pre-commit
- [ ] Crystallize without cross-project validation → error
- [ ] Crystallize with α < 30 → error

### Convergence tests
- [ ] All rules converged + coverage met + stable → system converged
- [ ] Rule churn >10% for 3 weeks → divergence detected
- [ ] S(R) up and F(R) up → seesaw detected

### End-to-end
- [ ] Create candidate → accumulate 5 feedbacks → promote → accumulate 25 feedbacks → converge to tuned → validate in 2 projects → crystallize → verify L1 hook works

## Notes

### Why human approval for promotion and crystallization?

Automation proposes; humans approve. This prevents two failure modes:

1. **Runaway optimization**: Without human gates, the system could promote fragile rules to production based on spurious correlations
2. **Loss of agency**: Developers should understand and approve governance changes, not wake up to find 10 new rules blocking their work

The human-in-the-loop gates are:
- **Promote candidate → active**: `synodic rules promote` (requires human)
- **Crystallize tuned → L1**: `synodic rules crystallize` (requires human)

Auto-transitions (active → tuned, active → deprecated) are safe because:
- **Tuned** is still L2 (easily reverted)
- **Deprecated** disables the rule (reduces risk, doesn't add it)

### Crystallization criteria are conservative by design

Criteria for L1 promotion (beyond reasonable doubt):
- α > 30 (30+ confirmed blocks)
- Precision > 95%
- CI < 0.1 (converged)
- Cross-project validated
- Deterministic
- 30+ days tuned

This is deliberately strict because crystallized rules:
1. Affect all tools (not just Claude Code)
2. Cannot be overridden per-session
3. Harder to revert (git hook vs DB update)
4. Have higher blast radius (break builds, block contributors)

Better to have a rule stay in L2 longer than crystallize prematurely and break workflows.

### What if a crystallized rule needs updating?

Crystallized rules are in git hooks (code), not DB. To update:

1. Update the git hook manually (edit `.githooks/pre-commit`)
2. Or: Deprecate the L1 check, create a new L2 candidate with the fix, re-converge, re-crystallize

This is intentionally effortful — L1 rules should be **stable**. If a rule needs frequent updates, it shouldn't be crystallized.

### Convergence may never happen

For a fast-moving project with evolving agent behavior, the system may never fully converge — new threats emerge, rules are added, coverage gaps appear. This is OK.

Convergence is a **goal state**, not a requirement. The value is in the **direction** (rules improving via feedback), not reaching a fixed point.

Monitoring mode (slower probes, higher evidence threshold) can be triggered by **partial convergence** (e.g., ≥80% of rules converged, even if not all).

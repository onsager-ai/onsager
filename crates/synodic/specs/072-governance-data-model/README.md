---
status: complete
created: 2026-03-30
completed: 2026-04-03
priority: critical
tags:
- governance
- database
- storage
- data-model
depends_on:
- "068"
- "069"
created_at: 2026-03-30T00:00:00Z
updated_at: 2026-03-30T00:00:00Z
---

# Governance Data Model & Storage Layer

> **Status**: complete · **Priority**: critical · **Created**: 2026-03-30

## Overview

Synodic is evolving from a hooks-only tool to a full governance platform with a web dashboard. This requires persistent storage for rules, telemetry, feedback, and analytics. This spec defines the database schema, storage abstractions, and migration system.

**Philosophy shift**: The original "no databases" stance made sense for a minimal CLI tool. But with a web UI (`packages/ui/`), REST API (`harness-http`), and feedback loop requirements (spec 071), file-based state is no longer viable. We need:
- Queryable storage (aggregate across projects, time-series analytics)
- Concurrent access (multiple users viewing dashboard while agents run)
- ACID guarantees (rule updates must be atomic)
- Efficient indexing (fast lookups by rule ID, session ID, timestamp)

**Database choice**:
- **PostgreSQL** for production (default in Docker)
- **SQLite** for demo/local development (single-file, no server)
- Schema designed to be compatible with both (avoid PG-specific features)

## Design

### Database architecture

```
harness-core/
  src/
    storage/
      mod.rs          # Storage trait + shared types
      postgres.rs     # PostgreSQL implementation
      sqlite.rs       # SQLite implementation
    intercept.rs      # Uses Storage trait (read-only for rule lookup)

harness-http/
  src/
    main.rs           # API server setup
    api/
      events.rs       # Event CRUD endpoints
      rules.rs        # Rule management endpoints
      stats.rs        # Analytics endpoints
    storage.rs        # Owns Storage instance, connection pooling
```

**Key insight**: The intercept engine remains **stateless** at evaluation time. It reads rules from DB but doesn't write. All writes happen via the API (feedback signals, rule updates). This keeps latency <100ms for blocking decisions.

### Core storage trait

```rust
#[async_trait]
pub trait Storage: Send + Sync {
    // Rules
    async fn get_rules(&self, active_only: bool) -> Result<Vec<Rule>>;
    async fn get_rule(&self, id: &str) -> Result<Option<Rule>>;
    async fn create_rule(&self, rule: CreateRule) -> Result<Rule>;
    async fn update_rule(&self, id: &str, update: UpdateRule) -> Result<Rule>;
    async fn delete_rule(&self, id: &str) -> Result<()>;

    // Threat taxonomy
    async fn get_threat_categories(&self) -> Result<Vec<ThreatCategory>>;
    async fn update_threat_category(&self, id: &str, update: UpdateCategory) -> Result<ThreatCategory>;

    // Feedback events
    async fn record_feedback(&self, event: FeedbackEvent) -> Result<()>;
    async fn get_feedback(&self, filters: FeedbackFilters) -> Result<Vec<FeedbackEvent>>;

    // Override events (special case of feedback with reasons)
    async fn record_override(&self, event: OverrideEvent) -> Result<()>;
    async fn get_overrides(&self, filters: OverrideFilters) -> Result<Vec<OverrideEvent>>;

    // Scoring snapshots
    async fn record_scores(&self, scores: GovernanceScores) -> Result<()>;
    async fn get_scores(&self, project_id: &str, since: DateTime<Utc>) -> Result<Vec<GovernanceScores>>;

    // Probe results
    async fn record_probe(&self, result: ProbeResult) -> Result<()>;
    async fn get_probes(&self, rule_id: &str) -> Result<Vec<ProbeResult>>;
}
```

### Schema

#### `rules` table

```sql
CREATE TABLE rules (
    id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    category_id TEXT NOT NULL REFERENCES threat_categories(id),
    tools TEXT[], -- JSON array: ["Bash", "Write"] or empty = all
    condition_type TEXT NOT NULL, -- "pattern" | "path" | "command"
    condition_value TEXT NOT NULL, -- regex or glob pattern
    lifecycle TEXT NOT NULL, -- "candidate" | "active" | "tuned" | "crystallized" | "deprecated"

    -- Bayesian confidence tracking
    alpha INTEGER NOT NULL DEFAULT 1, -- true positives + prior
    beta INTEGER NOT NULL DEFAULT 1,  -- false positives + prior
    prior_alpha INTEGER NOT NULL DEFAULT 1, -- from threat category severity
    prior_beta INTEGER NOT NULL DEFAULT 1,

    -- Metadata
    enabled BOOLEAN NOT NULL DEFAULT true,
    project_id TEXT, -- NULL = global rule, non-NULL = project-specific
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),

    -- Crystallization metadata
    crystallized_at TIMESTAMP,
    cross_project_validated BOOLEAN DEFAULT false
);

CREATE INDEX idx_rules_category ON rules(category_id);
CREATE INDEX idx_rules_lifecycle ON rules(lifecycle);
CREATE INDEX idx_rules_project ON rules(project_id);
CREATE INDEX idx_rules_enabled ON rules(enabled);
```

**Notes**:
- `alpha` and `beta` are updated by feedback signals (spec 073)
- `lifecycle` state machine managed by spec 076
- `project_id` allows both global rules and project-specific overrides
- Arrays stored as JSON for SQLite compatibility

#### `threat_categories` table

```sql
CREATE TABLE threat_categories (
    id TEXT PRIMARY KEY, -- e.g., "data-destruction"
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    severity TEXT NOT NULL, -- "critical" | "high" | "medium" | "low"
    severity_weight REAL NOT NULL, -- 1.0 (critical), 0.7 (high), 0.4 (medium), 0.2 (low)
    examples TEXT[], -- JSON array: ["rm -rf /", "DROP DATABASE"]
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Seed data: 10 categories from spec 071
INSERT INTO threat_categories VALUES
    ('data-destruction', 'Data Destruction', 'Irreversible data loss', 'critical', 1.0,
     '["rm -rf /", "git reset --hard", "DROP DATABASE"]'),
    ('credential-exposure', 'Credential Exposure', 'Secrets exposed in code/logs/args', 'critical', 1.0,
     '["echo $API_KEY", "hardcoded passwords", ".env in commit"]'),
    ('system-integrity', 'System Integrity', 'Modifications to system files outside project', 'high', 0.7,
     '["write /etc/hosts", "modify /usr/bin/", "chmod 777 /"]'),
    ('supply-chain', 'Supply Chain', 'Dependency manipulation, malicious packages', 'high', 0.7,
     '["npm install unknown-pkg", "pip install from URL", "curl | bash"]'),
    ('network-exfiltration', 'Network Exfiltration', 'Unauthorized data transmission', 'high', 0.7,
     '["curl -d @/etc/passwd", "wget --post-file"]'),
    ('privilege-escalation', 'Privilege Escalation', 'Gaining elevated permissions', 'high', 0.7,
     '["sudo", "chmod u+s", "chown root"]'),
    ('persistence', 'Persistence Mechanisms', 'Creating backdoors, cron jobs, startup scripts', 'medium', 0.4,
     '["crontab -e", "write to .bashrc", "launchctl load"]'),
    ('git-history-manipulation', 'Git History Manipulation', 'Rewriting shared history', 'medium', 0.4,
     '["git rebase published", "git push --force to main"]'),
    ('resource-exhaustion', 'Resource Exhaustion', 'Consuming excessive resources', 'medium', 0.4,
     '["fork bomb", "while true", "dd if=/dev/zero"]'),
    ('ci-pipeline-manipulation', 'CI Pipeline Manipulation', 'Modifying CI/CD to bypass checks', 'medium', 0.4,
     '["edit .github/workflows", "disable test step"]');
```

#### `feedback_events` table

```sql
CREATE TABLE feedback_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    signal_type TEXT NOT NULL, -- "override" | "confirmed" | "ci_failure" | "incident"
    rule_id TEXT NOT NULL REFERENCES rules(id),
    session_id TEXT,
    tool_name TEXT NOT NULL,
    tool_input JSONB NOT NULL,

    -- Override-specific fields (NULL for other signal types)
    override_reason TEXT,

    -- CI failure / incident fields
    failure_type TEXT,
    evidence_url TEXT, -- Link to CI run, incident report, etc.

    project_id TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_feedback_rule ON feedback_events(rule_id);
CREATE INDEX idx_feedback_signal ON feedback_events(signal_type);
CREATE INDEX idx_feedback_session ON feedback_events(session_id);
CREATE INDEX idx_feedback_created ON feedback_events(created_at);
```

**Notes**:
- Single table for all feedback types (polymorphic)
- `override_reason` is key for spec 073 (appeals with reasoning)
- `tool_input` stored as JSONB for queryability (SQLite: TEXT with JSON functions)

#### `override_events` view

Convenience view for override-specific queries (used heavily by spec 073):

```sql
CREATE VIEW override_events AS
SELECT
    id, rule_id, session_id, tool_name, tool_input,
    override_reason, project_id, created_at
FROM feedback_events
WHERE signal_type = 'override';
```

#### `scoring_snapshots` table

```sql
CREATE TABLE scoring_snapshots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id TEXT,

    -- S(R): Safety score
    safety_score REAL NOT NULL,

    -- F(R): Friction score
    friction_score REAL NOT NULL,
    blocks_count INTEGER NOT NULL,
    override_count INTEGER NOT NULL,
    total_tool_calls INTEGER NOT NULL,

    -- C(R): Coverage score
    coverage_score REAL NOT NULL,
    covered_categories INTEGER NOT NULL,
    total_categories INTEGER NOT NULL,

    -- Convergence state
    converged BOOLEAN NOT NULL,
    rule_churn_rate REAL NOT NULL, -- |ΔR| / |R|

    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_scores_project ON scoring_snapshots(project_id);
CREATE INDEX idx_scores_created ON scoring_snapshots(created_at);
```

**Notes**:
- Snapshots computed periodically (daily or on-demand via `synodic status`)
- Time-series data for tracking convergence over time

#### `probe_results` table

```sql
CREATE TABLE probe_results (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    rule_id TEXT NOT NULL REFERENCES rules(id),
    strategy TEXT NOT NULL, -- "syntactic-variation" | "indirection" | "encoding" | etc.
    probe_input JSONB NOT NULL,
    bypassed BOOLEAN NOT NULL, -- true if probe evaded the rule

    -- If bypassed, proposed expansion
    proposed_expansion TEXT,
    expansion_precision_drop REAL, -- Impact on historical allows
    expansion_approved BOOLEAN,

    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_probe_rule ON probe_results(rule_id);
CREATE INDEX idx_probe_bypassed ON probe_results(bypassed);
CREATE INDEX idx_probe_created ON probe_results(created_at);
```

### Migration system

Use **sqlx** with compile-time verified queries:

```
harness-core/
  migrations/
    20260330_001_initial_schema.sql
    20260330_002_seed_threat_categories.sql
```

**Why sqlx**:
- Compile-time SQL verification (catches schema drift)
- Built-in migration runner
- Supports both PostgreSQL and SQLite
- Connection pooling
- Async runtime (Tokio)

### Connection configuration

**Environment variables**:
```bash
# PostgreSQL (production)
DATABASE_URL=postgresql://user:pass@localhost/synodic

# SQLite (local)
DATABASE_URL=sqlite://synodic.db
```

**Connection pooling**:
```rust
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;

pub async fn create_pool(database_url: &str) -> Result<Pool> {
    if database_url.starts_with("sqlite:") {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url).await?;
        sqlx::migrate!().run(&pool).await?; // Auto-migrate
        Ok(Pool::Sqlite(pool))
    } else {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(database_url).await?;
        sqlx::migrate!().run(&pool).await?;
        Ok(Pool::Postgres(pool))
    }
}
```

### Type compatibility

**JSON arrays** (`tools`, `examples`):
- PostgreSQL: Native `TEXT[]` type
- SQLite: Store as JSON text, use `json_each()` for queries

**UUIDs**:
- PostgreSQL: Native `UUID` type with `gen_random_uuid()`
- SQLite: Store as `TEXT`, generate with `uuid::Uuid::new_v4()` in Rust

**Timestamps**:
- PostgreSQL: `TIMESTAMP` with `NOW()`
- SQLite: `TEXT` in ISO 8601 format, use `datetime('now')` for default

### API integration

The `harness-http` crate consumes the Storage trait:

```rust
// harness-http/src/main.rs
use harness_core::storage::{Storage, create_pool};
use axum::{Router, Extension};

#[tokio::main]
async fn main() -> Result<()> {
    let database_url = env::var("DATABASE_URL")?;
    let pool = create_pool(&database_url).await?;

    let app = Router::new()
        .route("/api/rules", get(api::rules::list).post(api::rules::create))
        .route("/api/rules/:id", get(api::rules::get).patch(api::rules::update))
        .route("/api/feedback", post(api::feedback::record))
        .route("/api/stats", get(api::stats::compute))
        .layer(Extension(pool));

    axum::Server::bind(&"0.0.0.0:8080".parse()?)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
```

## Plan

### Phase 1: Schema and migrations

- [ ] Add `sqlx` dependency to `harness-core`
- [ ] Create `harness-core/src/storage/mod.rs` with Storage trait
- [ ] Write initial migration: `001_initial_schema.sql` (all tables)
- [ ] Write seed migration: `002_seed_threat_categories.sql` (10 categories)
- [ ] Test migrations on both PostgreSQL and SQLite

### Phase 2: Storage implementations

- [ ] Implement `PostgresStorage` in `harness-core/src/storage/postgres.rs`
- [ ] Implement `SqliteStorage` in `harness-core/src/storage/sqlite.rs`
- [ ] Connection pooling and auto-migration on startup
- [ ] Unit tests for both implementations (verify schema compatibility)

### Phase 3: Migrate default rules to DB

- [ ] Seed the 5 current default rules into DB on init
- [ ] Update `InterceptEngine::new()` to accept `Vec<Rule>` from DB instead of hardcoded
- [ ] Update `synodic intercept` to load rules from DB (cache for performance)
- [ ] Deprecate hardcoded rules in `intercept.rs` (DB is source of truth)

### Phase 4: CLI integration

- [ ] Add `--db-url` flag to `synodic init` (defaults to `sqlite://~/.synodic/synodic.db`)
- [ ] Auto-create SQLite DB on first init if not exists
- [ ] `synodic rules list` command (reads from DB)
- [ ] `synodic rules create` command (writes to DB)

## Test

- [ ] Migration runs successfully on fresh PostgreSQL database
- [ ] Migration runs successfully on fresh SQLite database
- [ ] All 10 threat categories seeded correctly
- [ ] `Storage::get_rules(active_only=true)` returns only enabled rules
- [ ] `Storage::create_rule()` inserts rule with correct Beta priors from threat category
- [ ] `Storage::record_feedback()` inserts feedback event with correct signal type
- [ ] Schema is identical between PostgreSQL and SQLite (modulo type mappings)
- [ ] Concurrent writes to feedback_events do not cause conflicts (ACID)
- [ ] `synodic intercept` loads rules from DB in <10ms (cached)
- [ ] `synodic rules list` outputs formatted table of all rules

## Notes

### Why not NoSQL?

Some might argue that event data (feedback, probes) fits a document store (MongoDB, etc.). Reasons we chose SQL:

1. **Structured queries**: Analytics (S/F/C scores) require aggregation across rules, categories, time ranges — SQL excels here
2. **Transactions**: Rule updates + feedback recording must be atomic
3. **Indexing**: Fast lookups by rule_id, session_id, timestamp are critical
4. **Tooling**: sqlx provides compile-time verification; NoSQL ORMs are less mature
5. **Deployment simplicity**: PostgreSQL/SQLite require no additional services

### Rule caching strategy

The intercept engine runs on every tool call (potentially hundreds per session). Loading rules from DB each time would add unacceptable latency. Caching strategy:

- **In-process cache**: `InterceptEngine` loads rules once on init, caches in memory
- **Invalidation**: CLI commands that modify rules (`synodic rules update`) invalidate cache
- **Staleness tolerance**: OK if agent uses rules from 1-2 seconds ago (rule changes are infrequent)
- **Future**: Add `NOTIFY`/`LISTEN` in PostgreSQL for real-time invalidation

### Cross-project telemetry

The schema supports both global and project-specific rules via `project_id`. How cross-project aggregation works:

- **Global rules** (`project_id = NULL`): Applied to all projects, telemetry aggregated across projects
- **Project rules** (`project_id = "myapp"`): Only applied to that project
- **Crystallization** (spec 076): Requires global rule validated in ≥2 projects before L1 promotion

This supports both centralized governance (security team maintains global rules) and project autonomy (teams can add project-specific rules).

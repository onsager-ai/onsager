# Onsager Monorepo Migration Plan

**Status**: Ready to execute
**Target**: Collapse `onsager-ai/onsager` + `onsager-ai/stiglab` + `onsager-ai/synodic` into a single monorepo at `github.com/onsager-ai/onsager`
**Out of scope**: `onsager-ai/ising` (different project), `onsager-ai/telegramable` (stays polyrepo)

-----

## 1. Context & locked decisions

This document is the authoritative plan. An AI dev agent executing this should **not** re-litigate these decisions — they’re the outcome of prior design discussion.

### 1.1 Why monorepo

The deciding factor is not code organization aesthetics — it’s operational:

- **Cloud session push pain**: Working across multiple repos inside Claude Code cloud sessions requires cross-repo credential management and cross-repo push workflows that are painful in ephemeral containers.
- **Git dependency fragility**: All 3 subsystems currently depend on `onsager` via `git = "https://github.com/onsager-ai/onsager.git"`, requiring `cargo update -p onsager` dance on every upstream change and breaking CI caching.
- **Shared config drift**: `CLAUDE.md`, hooks, `skills-lock.json` already show drift between repos.

Monorepo with `path` dependencies kills the git-dep problem instantly and gives Claude Code sessions a single tree to operate on.

### 1.2 Architectural principle: factory event bus

Onsager is a **factory event bus** architecture. Subsystems are **runtime-decoupled** via a shared PostgreSQL `events` / `events_ext` table + `pg_notify` channel. They coordinate through stigmergy (indirect signals via shared medium), not direct calls.

**This loose coupling MUST be preserved at the build-time dependency graph level.** The target dependency graph is:

```
         onsager-spine (event bus lib)
        /              \
   stiglab           synodic        ← do NOT depend on each other
```

`stiglab` and `synodic` must **not** import each other, and must **not** be statically linked into the same binary. A CLI orchestrator that deps both would re-couple them at build time — architectural violation.

### 1.3 Final crate layout (4 crates)

```
crates/
├── onsager-spine/   ← event bus lib (was: `onsager` repo root crate)
├── onsager/         ← dispatcher binary, ~100 LOC, NO business deps
├── stiglab/         ← lib + bin "stiglab"  (folded from 4 crates)
└── synodic/         ← lib + bin "synodic"  (folded from 2 crates)
```

**Binary naming**: `stiglab` and `synodic` directly (no `onsager-` prefix). The `onsager` dispatcher looks up both `onsager-<sub>` and `<sub>` on PATH, so `onsager stiglab serve` and `stiglab serve` both work.

**Why the rename `onsager` → `onsager-spine`**: the name `onsager` is reassigned to the dispatcher CLI, because users type `onsager ...` at the shell. The spine library keeps the `onsager-spine` name internally.

### 1.4 Stiglab: 4 → 1 crate fold

Current:

- `crates/stiglab-core` (640 LOC) — shared protocol types
- `crates/stiglab-server` (2421 LOC) — Axum server
- `crates/stiglab-agent` (660 LOC) — node agent
- `crates/stiglab` (322 LOC) — CLI wrapper with clap subcommands

Target: single `crates/stiglab/` crate with internal modules `core/`, `server/`, `agent/`.

```
crates/stiglab/
├── Cargo.toml       ← [lib] + [[bin]] name = "stiglab"
└── src/
    ├── lib.rs       ← pub mod core; pub mod server; pub mod agent;
    ├── main.rs      ← clap CLI (unchanged from current stiglab/src/main.rs)
    ├── runner.rs
    ├── core/        ← was stiglab-core/src/*
    ├── server/      ← was stiglab-server/src/*
    └── agent/       ← was stiglab-agent/src/*
```

### 1.5 Synodic: 2 → 1 crate fold + rename

Current:

- `rust/harness-core` (7003 LOC) — L2 interception engine, storage, scoring, clustering
- `rust/harness-cli` (3061 LOC) — clap CLI with `cmd/{orchestrate,intercept,serve,status,rules,optimize,run,probe,init,lifecycle,feedback}`

Target: single `crates/synodic/` crate.

```
crates/synodic/
├── Cargo.toml       ← [lib] + [[bin]] name = "synodic"
└── src/
    ├── lib.rs       ← pub mod core; pub mod cmd;
    ├── main.rs      ← was rust/harness-cli/src/main.rs
    ├── util.rs      ← was rust/harness-cli/src/util.rs
    ├── core/        ← was rust/harness-core/src/*
    └── cmd/         ← was rust/harness-cli/src/cmd/*
```

Crate rename: `harness-core` → (internal module `synodic::core`), `harness-cli` → (crate `synodic`, bin `synodic`).

### 1.6 UI: single dashboard app

Collapse both `packages/stiglab-ui` (React + shadcn/ui, ~4136 LOC) and `packages/ui` in synodic (~370 LOC, likely stub) into one `apps/dashboard/`.

**Strategy**: stiglab-ui is the base (mature, shadcn/ui, React Query, full auth/session/node views). synodic-ui’s `src/` is absorbed as `apps/dashboard/src/features/governance/`. synodic-ui’s own package.json / vite.config / tsconfig are **dropped** — stiglab-ui’s config wins.

```
apps/dashboard/
├── package.json           ← from stiglab-ui
├── vite.config.ts         ← from stiglab-ui
├── tsconfig.json          ← from stiglab-ui
└── src/
    ├── main.tsx
    ├── App.tsx            ← route shell: /sessions, /nodes, /governance, /factory
    ├── shared/            ← components/, api/, auth/, layout/
    └── features/
        ├── sessions/      ← from stiglab-ui
        ├── nodes/         ← from stiglab-ui
        ├── governance/    ← from synodic-ui/src/
        └── factory/       ← placeholder, new (cross-subsystem event overview)
```

Route reorganization and design-system unification of `governance/` visuals are **POST-migration PRs**, not part of the initial migration. Migration only does physical rehoming + ensures `pnpm build` passes.

### 1.7 What’s deleted in migration (no backwards compat)

User explicitly stated: no backwards compat needed.

- `packages/synodic-cli-npm` (`@codervisor/synodic` npm wrapper) — **deleted**
- `synodic/packages/cli` — **deleted**
- Separate `stiglab-server` / `stiglab-agent` / `harness-core` / `harness-cli` Cargo.toml files — **deleted**
- synodic deploy configs (`fly.toml`, `railway.json`, `render.yaml`, `Dockerfile`) — **kept but archived to `crates/synodic/deploy/`** (no live production to disrupt, confirmed by user)
- `stiglab/docker-compose.yml`, `stiglab/railway.toml`, `stiglab/Dockerfile` — **archived to `crates/stiglab/deploy/`**
- Crates.io publication — **not happening**; all internal deps are `path = "..."`

### 1.8 Specs stay per-crate

- `crates/onsager-spine/specs/` — 9 specs
- `crates/stiglab/specs/` — 2 specs
- `crates/synodic/specs/` — 78 specs

Per-subsystem numbering spaces are preserved (synodic’s spec-069 stays spec-069). No cross-crate renumbering.

### 1.9 Git history preserved

All 3 source repos’ git history is preserved via `git filter-repo --path-rename`. Final `git log --all` shows 4 ancestor trees (3 source repos + initial monorepo commit) converging.

-----

## 2. Pre-flight checklist

Before running any migration script:

- [ ] All open PRs in `onsager`, `stiglab`, `synodic` are either merged or explicitly abandoned.
- [ ] Each source repo has a `archive/pre-monorepo-2026-04` tag pushed on current `main`:
  
  ```bash
  for repo in onsager stiglab synodic; do
    git clone https://github.com/onsager-ai/$repo.git /tmp/tag-$repo
    cd /tmp/tag-$repo
    git tag archive/pre-monorepo-2026-04
    git push origin archive/pre-monorepo-2026-04
    cd - && rm -rf /tmp/tag-$repo
  done
  ```
- [ ] `git-filter-repo` installed (`pip install git-filter-repo` or `brew install git-filter-repo`).
- [ ] Rust toolchain: stable 1.75+ (synodic uses edition 2021, ising unused).
- [ ] pnpm 9+ installed.
- [ ] Write access to `onsager-ai` org on GitHub.
- [ ] GitHub CLI (`gh`) authenticated for the cutover step.

-----

## 3. Phase 1: Filter-repo each source repo

Work in `/tmp/onsager-migration/`. All operations are local; nothing is pushed until Phase 6.

```bash
MIG=/tmp/onsager-migration
rm -rf $MIG && mkdir -p $MIG && cd $MIG
```

### 3.1 Rewrite `onsager` (spine lib) → `crates/onsager-spine/`

```bash
git clone --no-local https://github.com/onsager-ai/onsager.git $MIG/onsager
cd $MIG/onsager

git filter-repo \
  --path-rename src/:crates/onsager-spine/src/ \
  --path-rename migrations/:crates/onsager-spine/migrations/ \
  --path-rename examples/:crates/onsager-spine/examples/ \
  --path-rename specs/:crates/onsager-spine/specs/ \
  --path-rename Cargo.toml:crates/onsager-spine/Cargo.toml \
  --path-rename README.md:crates/onsager-spine/README.md \
  --path-rename CLAUDE.md:crates/onsager-spine/CLAUDE.md \
  --path .gitignore \
  --invert-paths
```

Note the last two lines strip the repo-root `.gitignore` (will be replaced by a new monorepo-level one).

### 3.2 Rewrite `stiglab` → folded `crates/stiglab/` + `apps/dashboard/`

This is the most intricate filter-repo invocation because it’s doing the 4→1 crate fold at the path-rewrite level.

```bash
git clone --no-local https://github.com/onsager-ai/stiglab.git $MIG/stiglab
cd $MIG/stiglab

git filter-repo \
  --path-rename crates/stiglab-core/src/:crates/stiglab/src/core/ \
  --path-rename crates/stiglab-server/src/:crates/stiglab/src/server/ \
  --path-rename crates/stiglab-agent/src/:crates/stiglab/src/agent/ \
  --path-rename crates/stiglab/src/:crates/stiglab/src/ \
  --path-rename specs/:crates/stiglab/specs/ \
  --path-rename .claude/:crates/stiglab/.claude/ \
  --path-rename hooks/:crates/stiglab/hooks/ \
  --path-rename Dockerfile:crates/stiglab/deploy/Dockerfile \
  --path-rename docker-compose.yml:crates/stiglab/deploy/docker-compose.yml \
  --path-rename railway.toml:crates/stiglab/deploy/railway.toml \
  --path-rename .railwayignore:crates/stiglab/deploy/.railwayignore \
  --path-rename .dockerignore:crates/stiglab/deploy/.dockerignore \
  --path-rename packages/stiglab-ui/:apps/dashboard/ \
  --path crates/stiglab-core/Cargo.toml \
  --path crates/stiglab-server/Cargo.toml \
  --path crates/stiglab-agent/Cargo.toml \
  --path crates/stiglab/Cargo.toml \
  --path Cargo.toml \
  --path Cargo.lock \
  --path pnpm-workspace.yaml \
  --path package.json \
  --path pnpm-lock.yaml \
  --path packages \
  --path .github \
  --path .gitignore \
  --path BOOTSTRAP_PROMPT.md \
  --path CONTRIBUTING.md \
  --path CLAUDE.md \
  --path README.md \
  --path LICENSE \
  --path skills-lock.json \
  --invert-paths
```

What this does:

- Moves each of the 4 crate source trees into one unified `crates/stiglab/src/` layout
- Moves `packages/stiglab-ui/` entirely to `apps/dashboard/` (config files come with it intentionally — this is the base for the unified dashboard)
- Strips all old `Cargo.toml` files (they’ll be replaced by one new file in the fold commit)
- Strips root `Cargo.toml`, `Cargo.lock`, workspace config, package.json, CI, LICENSE, README — these are re-owned by the monorepo root

### 3.3 Rewrite `synodic` → folded `crates/synodic/` + dashboard fragment

```bash
git clone --no-local https://github.com/onsager-ai/synodic.git $MIG/synodic
cd $MIG/synodic

git filter-repo \
  --path-rename rust/harness-core/src/:crates/synodic/src/core/ \
  --path-rename rust/harness-cli/src/:crates/synodic/src/ \
  --path-rename specs/:crates/synodic/specs/ \
  --path-rename harness/:crates/synodic/harness/ \
  --path-rename hooks/:crates/synodic/hooks/ \
  --path-rename skills/:crates/synodic/skills/ \
  --path-rename .claude/:crates/synodic/.claude/ \
  --path-rename .harness/:crates/synodic/.harness/ \
  --path-rename .lean-spec/:crates/synodic/.lean-spec/ \
  --path-rename .githooks/:crates/synodic/.githooks/ \
  --path-rename docs/:crates/synodic/docs/ \
  --path-rename docs-site/:crates/synodic/docs-site/ \
  --path-rename deploy/:crates/synodic/deploy/ \
  --path-rename docker/:crates/synodic/docker/ \
  --path-rename scripts/:crates/synodic/scripts/ \
  --path-rename Makefile:crates/synodic/Makefile \
  --path-rename .mcp.json:crates/synodic/.mcp.json \
  --path-rename packages/ui/src/:apps/dashboard/src/features/governance/ \
  --path rust/harness-core/Cargo.toml \
  --path rust/harness-cli/Cargo.toml \
  --path rust/Cargo.toml \
  --path rust/Cargo.lock \
  --path packages/ui/package.json \
  --path packages/ui/vite.config.ts \
  --path packages/ui/tsconfig.json \
  --path packages/ui/tsconfig.app.json \
  --path packages/ui/tsconfig.node.json \
  --path packages/ui/index.html \
  --path packages/ui/eslint.config.js \
  --path packages/ui/public \
  --path packages/cli \
  --path package.json \
  --path pnpm-lock.yaml \
  --path pnpm-workspace.yaml \
  --path railway.toml \
  --path .github \
  --path .gitignore \
  --path CONTRIBUTING.md \
  --path CLAUDE.md \
  --path README.md \
  --path npm \
  --invert-paths
```

What this does:

- `rust/harness-core/src/*` becomes `crates/synodic/src/core/*`
- `rust/harness-cli/src/*` (which contains `main.rs`, `util.rs`, and `cmd/`) becomes `crates/synodic/src/*`
- `packages/ui/src/*` (the React source, not config) becomes `apps/dashboard/src/features/governance/*`
- synodic-ui’s config files (package.json, vite.config, tsconfigs, index.html) are **dropped** — they’d conflict with stiglab-ui’s config
- `packages/cli` (the npm wrapper) is **completely dropped** — no backwards compat per decision 1.7
- All deploy configs (`fly.toml`, `railway.json`, `render.yaml`, `docker/`) are rehomed to `crates/synodic/deploy/` and `crates/synodic/docker/`
- Lean-spec / hooks / harness / skills directories are kept under `crates/synodic/` (they’re subsystem-local tooling)

-----

## 4. Phase 2: Merge into new monorepo

```bash
mkdir $MIG/monorepo && cd $MIG/monorepo
git init -b main

# Seed with an initial empty commit so merges have a common root
git commit --allow-empty -m "chore: initialize onsager monorepo"

for sub in onsager stiglab synodic; do
  git remote add $sub $MIG/$sub
  git fetch $sub
  git merge --allow-unrelated-histories --no-edit \
    -m "chore: merge $sub into monorepo" $sub/main
  git remote remove $sub
done
```

After this, `$MIG/monorepo` has all 3 source trees merged into their new locations. No Cargo.toml at root yet, no workspace config, nothing builds. That’s expected — Phase 3 fixes it.

Verify expected layout:

```bash
cd $MIG/monorepo
tree -L 3 -d crates apps 2>/dev/null || find crates apps -maxdepth 3 -type d
```

Expected (roughly):

```
crates
├── onsager-spine
│   ├── examples
│   ├── migrations
│   ├── specs
│   └── src
├── stiglab
│   ├── deploy
│   ├── hooks
│   ├── specs
│   └── src
│       ├── agent
│       ├── core
│       └── server
└── synodic
    ├── deploy
    ├── docker
    ├── docs
    ├── docs-site
    ├── harness
    ├── hooks
    ├── scripts
    ├── skills
    ├── specs
    └── src
        ├── cmd
        └── core
apps
└── dashboard
    ├── public
    ├── src
    │   ├── features
    │   │   └── governance
    │   └── ...
    └── tests
```

-----

## 5. Phase 3: Fold commit — rewrite source content

`filter-repo` moved files around. Now we need to:

1. Rewrite `use` paths inside Rust source (since `stiglab_core::X` no longer exists — it’s `crate::core::X`)
1. Rename all `onsager::` imports to `onsager_spine::`
1. Replace each `crates/<name>/src/lib.rs` with a new one that declares the new module structure
1. Create new consolidated `Cargo.toml` files
1. Create the new dispatcher `crates/onsager/`
1. Create root workspace `Cargo.toml`

All of this in a single atomic commit (or a small sequence of commits all in the migration PR).

### 5.1 Rewrite stiglab use paths

```bash
cd $MIG/monorepo

# stiglab internal use path rewrites
rg -l 'stiglab_core::' crates/stiglab/src/ \
  | xargs -I {} sed -i.bak 's/stiglab_core::/crate::core::/g' {}
rg -l 'stiglab_server::' crates/stiglab/src/ \
  | xargs -I {} sed -i.bak 's/stiglab_server::/crate::server::/g' {}
rg -l 'stiglab_agent::' crates/stiglab/src/ \
  | xargs -I {} sed -i.bak 's/stiglab_agent::/crate::agent::/g' {}

# Extern crate → module paths for `use stiglab_core` at top level
rg -l 'use stiglab_core;' crates/stiglab/src/ \
  | xargs -I {} sed -i.bak 's/use stiglab_core;/use crate::core;/g' {}
# Same for server, agent

# Clean up .bak files
find crates/stiglab/src/ -name '*.bak' -delete
```

Important: the current `crates/stiglab/src/main.rs` has lines like:

```rust
use stiglab_agent::config::AgentConfig;
use stiglab_server::config::ServerConfig;
use stiglab_server::spine::SpineEmitter;
use stiglab_server::{db, state::AppState};
```

These become:

```rust
use crate::agent::config::AgentConfig;
use crate::server::config::ServerConfig;
use crate::server::spine::SpineEmitter;
use crate::server::{db, state::AppState};
```

The sed commands above handle this.

### 5.2 Rewrite synodic use paths

```bash
cd $MIG/monorepo

rg -l 'harness_core::' crates/synodic/src/ \
  | xargs -I {} sed -i.bak 's/harness_core::/crate::core::/g' {}
rg -l 'use harness_core;' crates/synodic/src/ \
  | xargs -I {} sed -i.bak 's/use harness_core;/use crate::core;/g' {}
find crates/synodic/src/ -name '*.bak' -delete
```

### 5.3 Rewrite `onsager::` → `onsager_spine::` (all crates)

```bash
cd $MIG/monorepo

for crate in stiglab synodic; do
  rg -l 'onsager::' crates/$crate/src/ \
    | xargs -I {} sed -i.bak 's/onsager::/onsager_spine::/g' {}
  rg -l 'use onsager;' crates/$crate/src/ \
    | xargs -I {} sed -i.bak 's/use onsager;/use onsager_spine;/g' {}
  rg -l 'use onsager ' crates/$crate/src/ \
    | xargs -I {} sed -i.bak 's/use onsager /use onsager_spine /g' {}
  find crates/$crate/src/ -name '*.bak' -delete
done
```

### 5.4 Create new `crates/stiglab/src/lib.rs`

The existing `crates/stiglab/src/lib.rs` (if present — it came from stiglab/src/main.rs’s sibling) needs to be replaced with a module-declaring lib.rs. Actually, from the source stiglab repo the `crates/stiglab/src/` directory only had `main.rs` and `runner.rs`, no lib.rs. So we’re creating a new lib.rs.

Create `crates/stiglab/src/lib.rs`:

```rust
//! # stiglab
//!
//! Distributed AI agent session orchestration. Part of the Onsager factory stack.
//!
//! This crate exposes `core` (shared types), `server` (control plane), and
//! `agent` (node agent runtime) as public modules so the `main.rs` CLI and
//! integration tests can reach into any of them. They are not intended as a
//! stable public API — other Onsager crates do NOT depend on this one.

pub mod core;
pub mod server;
pub mod agent;

// runner is CLI-internal
mod runner;
```

Now the folded sub-modules each need their own `mod.rs`. Each came from a separate crate’s `src/lib.rs`, which had its own module declarations. Rename them:

```bash
cd $MIG/monorepo
mv crates/stiglab/src/core/lib.rs crates/stiglab/src/core/mod.rs
mv crates/stiglab/src/server/lib.rs crates/stiglab/src/server/mod.rs
mv crates/stiglab/src/agent/lib.rs crates/stiglab/src/agent/mod.rs
```

`runner.rs` stays as-is (it was already at `crates/stiglab/src/runner.rs`).

**Gotcha**: sub-module `mod.rs` files may have `pub mod ...` declarations that assume they’re the crate root. Review each and ensure they don’t `use crate::X` in a way that only makes sense as a crate root. Since we’re just moving from `stiglab_server` crate to `stiglab::server` module, relative `mod` declarations work identically.

### 5.5 Create new `crates/synodic/src/lib.rs`

```rust
//! # synodic
//!
//! AI agent governance via hooks and event spine integration. Part of the
//! Onsager factory stack.
//!
//! Modules:
//! - `core`: interception engine, storage, scoring, clustering (was harness-core)
//! - `cmd`: CLI subcommand implementations (orchestrate, intercept, serve, rules, ...)

pub mod core;
pub mod cmd;
pub mod util;
```

Rename:

```bash
cd $MIG/monorepo
mv crates/synodic/src/core/lib.rs crates/synodic/src/core/mod.rs
# cmd/ came from harness-cli/src/cmd/, which already has a mod.rs, so no change
# util.rs is at crates/synodic/src/util.rs already
```

Also: the existing `crates/synodic/src/main.rs` (from `rust/harness-cli/src/main.rs`) already has `mod cmd;` and `mod util;` declarations. After fold, these need to become `use synodic::cmd;` and `use synodic::util;` — or actually, since main.rs is part of the same crate as lib.rs, it can just do `use crate::cmd;` and `use crate::util;`, which is equivalent.

Check `crates/synodic/src/main.rs` after filter-repo and adjust its top-of-file `mod` declarations if needed (they may need to be removed since lib.rs now owns them).

### 5.6 Create new `crates/stiglab/Cargo.toml`

Merge the dependencies from the 4 old Cargo.toml files (stiglab, stiglab-core, stiglab-server, stiglab-agent). The workspace-level dependencies were defined in the old stiglab root Cargo.toml, so we need to look at what each crate actually used.

```toml
[package]
name = "stiglab"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "Distributed AI agent session orchestration — part of the Onsager factory stack"

[lib]
name = "stiglab"
path = "src/lib.rs"

[[bin]]
name = "stiglab"
path = "src/main.rs"

[dependencies]
onsager-spine = { path = "../onsager-spine" }

# From stiglab-core
serde = { workspace = true }
serde_json = { workspace = true }
chrono = { workspace = true }
uuid = { workspace = true }
thiserror = { workspace = true }

# From stiglab-server
tokio = { workspace = true }
axum = { workspace = true }
sqlx = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
anyhow = { workspace = true }
tower-http = { workspace = true }
futures-util = { workspace = true }
reqwest = { workspace = true }
rand = { workspace = true }
base64 = { workspace = true }
ring = { workspace = true }
hex = { workspace = true }
axum-extra = { workspace = true }

# From stiglab-agent
tokio-tungstenite = { workspace = true }
clap = { workspace = true }
hostname = "0.4"
```

### 5.7 Create new `crates/synodic/Cargo.toml`

```toml
[package]
name = "synodic"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "AI agent governance — L1 git hooks + L2 Claude Code hooks + event spine integration"

[lib]
name = "synodic"
path = "src/lib.rs"

[[bin]]
name = "synodic"
path = "src/main.rs"

[features]
default = ["sqlite"]
postgres = ["sqlx/postgres"]
sqlite = ["sqlx/sqlite"]

[dependencies]
onsager-spine = { path = "../onsager-spine" }

# Merged from harness-core and harness-cli
serde = { workspace = true }
serde_json = { workspace = true }
serde_yaml = "0.9"
anyhow = { workspace = true }
regex = "1"
sqlx = { workspace = true, features = ["runtime-tokio", "chrono", "uuid", "json"] }
tokio = { workspace = true }
chrono = { workspace = true }
uuid = { workspace = true }
async-trait = "0.1"
indicatif = "0.17"
console = "0.15"
reqwest = { workspace = true, features = ["json", "rustls-tls"], default-features = false }
clap = { workspace = true }
axum = { workspace = true }
tower-http = { workspace = true, features = ["fs", "cors"] }
```

### 5.8 Create new `crates/onsager-spine/Cargo.toml`

This is largely the same as the existing `onsager/Cargo.toml` but renamed. The existing content was already a single-crate `[package]` (not `[workspace]`), so just rename `name`:

```toml
[package]
name = "onsager-spine"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "Client library for the Onsager event spine — shared PostgreSQL event stream coordination."

[dependencies]
tokio = { workspace = true }
sqlx = { workspace = true, features = ["runtime-tokio", "tls-rustls", "postgres", "chrono", "json", "uuid"] }
serde = { workspace = true }
serde_json = { workspace = true }
chrono = { workspace = true }
uuid = { workspace = true }
ulid = { version = "1", features = ["serde"] }
thiserror = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
async-trait = "0.1"

[dev-dependencies]
tracing-subscriber = { workspace = true }
```

Also: the existing `src/lib.rs` inside onsager-spine already references `pub mod factory_event; pub mod artifact; pub mod protocol; ...` — these all still exist since we just moved the directory. Verify this file didn’t get mangled by filter-repo.

### 5.9 Create new `crates/onsager/` (dispatcher)

This crate is entirely new — it has no git history because it didn’t exist in any source repo.

```bash
mkdir -p $MIG/monorepo/crates/onsager/src
```

`crates/onsager/Cargo.toml`:

```toml
[package]
name = "onsager"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "Onsager — unified dispatcher CLI for the AI factory stack"

[[bin]]
name = "onsager"
path = "src/main.rs"

[dependencies]
# Note: NO business crate deps here. This dispatcher must not link stiglab or
# synodic — that would break runtime loose coupling by forcing a rebuild on
# any subsystem change.
```

`crates/onsager/src/main.rs`:

```rust
//! onsager — dispatcher for the AI factory CLI stack.
//!
//! This binary is a thin git-style dispatcher. It does NOT depend on any
//! subsystem crate. Instead, it looks up subcommands on PATH:
//!
//!   $ onsager stiglab serve    →  exec `stiglab serve` (or `onsager-stiglab serve`)
//!   $ onsager synodic rules    →  exec `synodic rules` (or `onsager-synodic rules`)
//!
//! This preserves the architectural loose coupling between subsystems —
//! they are independent binaries that coordinate via the Onsager event spine,
//! never statically linked into a shared process.

use std::env;
use std::process::{exit, Command};

const HELP: &str = "\
onsager — AI factory dispatcher

USAGE:
    onsager <subcommand> [args...]
    onsager --help
    onsager --version

Subcommands are discovered on PATH. Any executable named `onsager-<name>`
or `<name>` (for known subsystems) is a valid subcommand.

KNOWN SUBCOMMANDS:
    stiglab     Distributed AI agent session orchestration
    synodic     AI agent governance

See `onsager <subcommand> --help` for subcommand-specific help.
";

const KNOWN: &[&str] = &["stiglab", "synodic"];

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        println!("{}", HELP);
        exit(0);
    }

    match args[1].as_str() {
        "-h" | "--help" | "help" => {
            println!("{}", HELP);
            exit(0);
        }
        "-V" | "--version" => {
            println!("onsager {}", env!("CARGO_PKG_VERSION"));
            exit(0);
        }
        sub => dispatch(sub, &args[2..]),
    }
}

fn dispatch(sub: &str, rest: &[String]) {
    // Try `onsager-<sub>` first, then `<sub>` if it's a known subsystem.
    // This supports both git-style prefixed binaries and direct binary names.
    let candidates: Vec<String> = if KNOWN.contains(&sub) {
        vec![format!("onsager-{}", sub), sub.to_string()]
    } else {
        vec![format!("onsager-{}", sub)]
    };

    for candidate in &candidates {
        match Command::new(candidate).args(rest).status() {
            Ok(status) => exit(status.code().unwrap_or(1)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                eprintln!("onsager: failed to exec `{}`: {}", candidate, e);
                exit(127);
            }
        }
    }

    eprintln!(
        "onsager: '{}' is not an onsager subcommand.\n\
         Tried: {}\n\
         Make sure one of them is in your PATH.",
        sub,
        candidates.join(", ")
    );
    exit(127);
}
```

### 5.10 Create root `Cargo.toml` (workspace)

```toml
[workspace]
resolver = "2"
members = [
    "crates/onsager-spine",
    "crates/onsager",
    "crates/stiglab",
    "crates/synodic",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "AGPL-3.0"
repository = "https://github.com/onsager-ai/onsager"

[workspace.dependencies]
# Core async
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Time and IDs
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4", "serde"] }

# Error handling
thiserror = "2"
anyhow = "1"

# Tracing
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Database
sqlx = { version = "0.8", features = ["runtime-tokio", "tls-rustls", "any", "postgres", "sqlite", "chrono", "json", "uuid"] }

# HTTP / Web
axum = { version = "0.8", features = ["ws"] }
axum-extra = { version = "0.10", features = ["cookie"] }
tower-http = { version = "0.6", features = ["fs", "cors", "trace"] }
reqwest = { version = "0.12", features = ["json"] }
tokio-tungstenite = { version = "0.24", features = ["native-tls"] }
futures-util = "0.3"

# CLI
clap = { version = "4", features = ["derive", "env"] }

# Crypto / encoding
rand = "0.8"
base64 = "0.22"
ring = "0.17"
hex = "0.4"
```

**Version reconciliation notes**:

- stiglab used axum 0.7, synodic (harness-cli) used axum 0.8. Monorepo standardizes on **0.8**. Stiglab server code may need minor adjustments (axum 0.7 → 0.8 migration is straightforward: `Router::route` API is stable, handler trait changes are mostly additive).
- stiglab used tower-http 0.5, synodic used 0.6. Standardize on **0.6**.
- stiglab used axum-extra 0.9, synodic used 0.10. Standardize on **0.10**.
- tokio-tungstenite is only used by stiglab, keep 0.24.

If axum 0.7 → 0.8 migration in `crates/stiglab/src/server/` ends up being non-trivial, fall back to axum 0.7 in workspace dep and let synodic adjust down (synodic-cli axum usage is minimal — just `serve` subcommand’s static file server).

### 5.11 Create root `.gitignore`

```
/target
**/*.rs.bak
/node_modules
**/node_modules
.DS_Store
*.swp
/.idea
/.vscode/settings.json
/Cargo.lock
**/dist
**/.turbo
**/.next
```

### 5.12 Create root `pnpm-workspace.yaml`

```yaml
packages:
  - "apps/*"
```

### 5.13 Create root `package.json`

```json
{
  "name": "onsager-monorepo",
  "version": "0.1.0",
  "private": true,
  "scripts": {
    "dev": "pnpm --filter dashboard dev",
    "build": "pnpm --filter dashboard build",
    "lint": "pnpm --filter dashboard lint",
    "test": "pnpm --filter dashboard test"
  },
  "packageManager": "pnpm@9.0.0"
}
```

### 5.14 Create root `justfile`

```make
# Onsager monorepo task runner.
# Rust + TS workspaces coexist; this file is just a command registry.

default:
    @just --list

# ── Build ────────────────────────────────────────────────────────────
build: build-rust build-ui

build-rust:
    cargo build --workspace

build-ui:
    pnpm install
    pnpm --filter dashboard build

# ── Test ─────────────────────────────────────────────────────────────
test: test-rust test-ui

test-rust:
    cargo test --workspace

test-ui:
    pnpm --filter dashboard test

# ── Lint ─────────────────────────────────────────────────────────────
lint: lint-rust lint-ui

lint-rust:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings

lint-ui:
    pnpm --filter dashboard lint

# ── Dev ──────────────────────────────────────────────────────────────
dev-dashboard:
    pnpm --filter dashboard dev

dev-stiglab:
    cargo run -p stiglab -- serve

dev-synodic:
    cargo run -p synodic -- serve

# ── DB ───────────────────────────────────────────────────────────────
db-migrate:
    psql "$DATABASE_URL" -f crates/onsager-spine/migrations/001_initial.sql
    psql "$DATABASE_URL" -f crates/onsager-spine/migrations/002_artifacts.sql

# ── Install from source ──────────────────────────────────────────────
install:
    cargo install --path crates/onsager
    cargo install --path crates/stiglab
    cargo install --path crates/synodic
```

### 5.15 Create root `rust-toolchain.toml`

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

### 5.16 Create root `CLAUDE.md`

Merge the 3 source CLAUDE.md files into one. The monorepo-level CLAUDE.md should cover:

- What Onsager is (factory event bus for AI agent stack)
- Workspace layout
- Build/test commands (`just build`, `just test`)
- Architectural principle (runtime loose coupling via `onsager-spine`)
- References to per-crate `CLAUDE.md` / `.claude/` directories for subsystem specifics

Subsystem-level CLAUDE.md files are preserved at `crates/stiglab/CLAUDE.md`, `crates/synodic/CLAUDE.md`, `crates/onsager-spine/CLAUDE.md`.

### 5.17 Create root `README.md`

```markdown
# Onsager

AI factory stack — unified monorepo.

## Subsystems

| Crate              | Role                                                  |
|--------------------|-------------------------------------------------------|
| `onsager-spine`    | Shared event bus library (PostgreSQL + pg_notify)     |
| `onsager`          | Unified CLI dispatcher (`onsager <subsystem> ...`)    |
| `stiglab`          | Distributed AI agent session orchestration            |
| `synodic`          | AI agent governance (hooks + spine integration)       |

All subsystems coordinate at runtime through the `onsager-spine` event bus.
They are **not** statically linked into a shared binary — loose coupling is
preserved at the build dependency graph level.

## Dashboard

A single React app at `apps/dashboard/` surfaces sessions (stiglab), nodes
(stiglab), governance (synodic), and factory events (onsager-spine) views.

## Build

    just build         # Rust workspace + dashboard
    just test
    just lint

## Run locally

    just dev-stiglab   # cargo run -p stiglab -- serve
    just dev-synodic   # cargo run -p synodic -- serve
    just dev-dashboard # pnpm --filter dashboard dev

## Install

    just install       # installs onsager, stiglab, synodic binaries

After install, both forms work:

    onsager stiglab serve
    stiglab serve
```

-----

## 6. Phase 4: Validation

Before pushing anything, verify the monorepo actually works locally.

```bash
cd $MIG/monorepo

# Rust workspace builds
cargo build --workspace 2>&1 | tee /tmp/build.log
# Expected: all 4 crates compile. Warnings OK, errors NOT OK.

# Rust tests (some may fail due to hardcoded DATABASE_URL; that's OK for first pass)
cargo test --workspace --no-run

# Clippy (allow warnings on first pass, fix in post-migration PR)
cargo clippy --workspace --all-targets 2>&1 | tee /tmp/clippy.log

# Dashboard builds
pnpm install
pnpm --filter dashboard build
# NOTE: the governance/ feature folder from synodic-ui will likely NOT integrate
# into routes yet — it's just sitting there. dashboard should still build as long
# as nothing imports from governance/. If build fails because of governance
# imports, temporarily comment them out; fixing is a post-migration PR.

# CLI dispatcher smoke test
cargo run -p onsager -- --help
cargo run -p onsager -- --version
cargo run -p onsager -- stiglab --help    # dispatches via PATH; may fail if not installed
```

### 6.1 Expected issues and fixes

**Issue**: `cargo build` fails because `stiglab_server::spine::SpineEmitter` references `onsager::EventStore` directly.
**Fix**: the sed script in 5.3 handles this; verify `onsager::` was fully replaced with `onsager_spine::`.

**Issue**: stiglab’s `axum 0.7` code doesn’t compile against workspace’s `axum 0.8`.
**Fix**: either (a) pin workspace to axum 0.7 and adjust synodic’s minimal axum usage, or (b) do the axum 0.7→0.8 migration in stiglab (adjust handler extractors, `Router::with_state` is the same, response types mostly unchanged). Recommended: (a) as migration-phase shortcut, (b) as post-migration cleanup.

**Issue**: `crates/stiglab/src/core/mod.rs` and `crates/stiglab/src/server/mod.rs` have top-level `use crate::X` that assumed they were crate roots.
**Fix**: these become `use crate::core::X` (prepend the new module prefix). Fix case-by-case during validation.

**Issue**: sqlx feature flags conflict. stiglab needed `["postgres", "sqlite"]` via `"any"`, synodic has feature flags `postgres`/`sqlite`. Workspace dep needs all features enabled.
**Fix**: the root Cargo.toml above already sets `features = ["runtime-tokio", "tls-rustls", "any", "postgres", "sqlite", "chrono", "json", "uuid"]`. If this causes conflicts, disable `default-features` in individual crate deps and let each crate enable what it needs.

**Issue**: dashboard `tsconfig` paths reference `@/` but the new governance/ folder isn’t aliased.
**Fix**: governance/ files imported by nothing initially, so this only matters when integration PRs start. Defer.

### 6.2 Commit the fold

Once `cargo build --workspace` is green:

```bash
cd $MIG/monorepo
git add -A
git commit -m "chore: fold 4 stiglab crates + 2 synodic crates into unified structure

- stiglab-core/server/agent/stiglab → crates/stiglab (internal modules)
- harness-core/harness-cli → crates/synodic (internal modules)
- onsager → crates/onsager-spine (renamed to free the 'onsager' name)
- crates/onsager: new dispatcher binary (git-style, ~100 LOC, zero business deps)
- Rewrite: use onsager:: → use onsager_spine::
- Rewrite: use stiglab_core:: → use crate::core::  (and analogues)
- Root workspace Cargo.toml + pnpm-workspace.yaml + justfile + README
- UI: packages/stiglab-ui → apps/dashboard (base)
- UI: synodic/packages/ui/src → apps/dashboard/src/features/governance (absorbed)
- npm compat wrappers (@codervisor/synodic) removed — no backwards compat
- Deploy configs rehomed to crates/<sub>/deploy/ (archived, synodic not live)
"
```

-----

## 7. Phase 5: CI workflows

Create `.github/workflows/rust.yml`:

```yaml
name: rust

on:
  push:
    branches: [main]
  pull_request:

env:
  CARGO_TERM_COLOR: always
  RUSTFLAGS: -D warnings

jobs:
  changes:
    runs-on: ubuntu-latest
    outputs:
      onsager-spine: ${{ steps.filter.outputs.onsager-spine }}
      stiglab: ${{ steps.filter.outputs.stiglab }}
      synodic: ${{ steps.filter.outputs.synodic }}
      onsager: ${{ steps.filter.outputs.onsager }}
      workspace: ${{ steps.filter.outputs.workspace }}
    steps:
      - uses: actions/checkout@v4
      - uses: dorny/paths-filter@v3
        id: filter
        with:
          filters: |
            workspace:
              - 'Cargo.toml'
              - 'Cargo.lock'
              - 'rust-toolchain.toml'
            onsager-spine:
              - 'crates/onsager-spine/**'
            stiglab:
              - 'crates/stiglab/**'
              - 'crates/onsager-spine/**'
            synodic:
              - 'crates/synodic/**'
              - 'crates/onsager-spine/**'
            onsager:
              - 'crates/onsager/**'

  check:
    needs: changes
    if: |
      needs.changes.outputs.workspace == 'true' ||
      needs.changes.outputs.onsager-spine == 'true' ||
      needs.changes.outputs.stiglab == 'true' ||
      needs.changes.outputs.synodic == 'true' ||
      needs.changes.outputs.onsager == 'true'
    runs-on: ubuntu-latest
    services:
      postgres:
        image: postgres:16
        env:
          POSTGRES_PASSWORD: postgres
        ports: [5432:5432]
        options: >-
          --health-cmd pg_isready
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all -- --check
      - run: cargo build --workspace
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
        env:
          DATABASE_URL: postgres://postgres:postgres@localhost:5432/postgres
```

Create `.github/workflows/frontend.yml`:

```yaml
name: frontend

on:
  push:
    branches: [main]
  pull_request:

jobs:
  changes:
    runs-on: ubuntu-latest
    outputs:
      dashboard: ${{ steps.filter.outputs.dashboard }}
    steps:
      - uses: actions/checkout@v4
      - uses: dorny/paths-filter@v3
        id: filter
        with:
          filters: |
            dashboard:
              - 'apps/dashboard/**'
              - 'pnpm-workspace.yaml'
              - 'package.json'

  build:
    needs: changes
    if: needs.changes.outputs.dashboard == 'true'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with:
          version: 9
      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: 'pnpm'
      - run: pnpm install --frozen-lockfile
      - run: pnpm --filter dashboard lint
      - run: pnpm --filter dashboard build
      - run: pnpm --filter dashboard test
```

-----

## 8. Phase 6: Cutover (force-push to `onsager-ai/onsager`)

User decision: force-push directly, no intermediate repo.

### 8.1 Pre-cutover safety

```bash
# Verify archive tags exist on the source repos
for repo in onsager stiglab synodic; do
  gh api repos/onsager-ai/$repo/git/ref/tags/archive/pre-monorepo-2026-04 \
    | grep -q '"ref"' && echo "$repo: tag OK" || echo "$repo: MISSING TAG"
done
```

If any tag is missing, go back to pre-flight 2. Do not proceed.

### 8.2 Force-push monorepo to `onsager-ai/onsager`

```bash
cd $MIG/monorepo
git remote add origin https://github.com/onsager-ai/onsager.git
git push --force --tags origin main
```

At this moment, `onsager-ai/onsager` becomes the monorepo. The old spine lib history is preserved under `crates/onsager-spine/` in the new history.

### 8.3 Archive the 2 sibling repos

```bash
# Rename + archive stiglab
gh api -X PATCH repos/onsager-ai/stiglab -f name=stiglab-archive
gh api -X PATCH repos/onsager-ai/stiglab-archive -F archived=true

# Rename + archive synodic
gh api -X PATCH repos/onsager-ai/synodic -f name=synodic-archive
gh api -X PATCH repos/onsager-ai/synodic-archive -F archived=true
```

Before archiving, add a README note to each archived repo pointing to the monorepo. Can be done via:

```bash
cd /tmp
git clone https://github.com/onsager-ai/stiglab-archive.git && cd stiglab-archive
cat > README.md <<'EOF'
# stiglab (archived)

This repository has been merged into the Onsager monorepo.

New location: https://github.com/onsager-ai/onsager/tree/main/crates/stiglab

All git history has been preserved in the monorepo via `git filter-repo`.
The last pre-monorepo commit is tagged `archive/pre-monorepo-2026-04` in this
repository for reference.
EOF
git add README.md && git commit -m "docs: point to monorepo" && git push
cd .. && rm -rf stiglab-archive

# Same for synodic-archive
```

### 8.4 Update GitHub repo settings on `onsager-ai/onsager`

- Description: “Onsager — AI factory stack (monorepo: onsager-spine, stiglab, synodic)”
- Topics: `rust`, `monorepo`, `ai-agent`, `event-sourcing`, `postgresql`
- Default branch: `main` (already)
- Copy over any secrets from the archived repos (DATABASE_URL, etc.) if their CI used them

-----

## 9. Phase 7: Post-migration PRs (NOT part of migration)

Each of these is an independent follow-up PR that should be opened after migration lands and CI is green.

### 9.1 PR-A: Dashboard route integration

Wire `apps/dashboard/src/features/governance/` into the main App.tsx router:

- Add sidebar entry
- Add `/governance/*` routes
- Auth guard as needed

### 9.2 PR-B: Governance UI design-system unification

Rewrite `features/governance/` components using the shadcn/ui primitives already present in `shared/components/`. Drop any residual `@base-ui/react` usage.

### 9.3 PR-C: Factory overview feature

Build `apps/dashboard/src/features/factory/` — a cross-subsystem view of `FactoryEvent` stream from onsager-spine. This is new functionality unlocked by the monorepo, not a migration artifact.

### 9.4 PR-D: Consolidated CLAUDE.md + skills

Merge hooks/ and skills/ from all 3 source crates into `.claude/` at the monorepo root, leaving only crate-specific content in `crates/<sub>/.claude/`. Add drift-detection.

### 9.5 PR-E: Axum version unification

If migration used the axum 0.7 fallback, migrate stiglab’s server code to axum 0.8 so synodic can drop the old version pin.

### 9.6 PR-F: Shared workspace deps audit

Review all `Cargo.toml` files for duplicate or inconsistent version specs that should be moved to `[workspace.dependencies]`.

-----

## 10. Rollback plan

If Phase 6 cutover goes wrong (e.g., force-push succeeds but later reveals a fatal issue):

```bash
# Restore onsager-ai/onsager to the archive tag
cd /tmp && git clone https://github.com/onsager-ai/onsager.git rollback
cd rollback
git checkout archive/pre-monorepo-2026-04
git push --force origin HEAD:main

# Un-archive the siblings
gh api -X PATCH repos/onsager-ai/stiglab-archive -F archived=false
gh api -X PATCH repos/onsager-ai/stiglab-archive -f name=stiglab
gh api -X PATCH repos/onsager-ai/synodic-archive -F archived=false
gh api -X PATCH repos/onsager-ai/synodic-archive -f name=synodic
```

Everyone goes back to polyrepo. Archive tags are the escape hatch.

-----

## 11. Execution checklist (for the AI dev agent)

- [ ] Pre-flight: archive tags pushed on all 3 source repos
- [ ] Phase 1: filter-repo run on all 3 repos, verify expected layouts
- [ ] Phase 2: merge into monorepo, verify `git log --all` shows 4 ancestor trees
- [ ] Phase 3: run all sed rewrites, create new lib.rs files, create all Cargo.toml, create dispatcher crate, create root config files
- [ ] Phase 4: `cargo build --workspace` passes (fix issues inline), `pnpm --filter dashboard build` passes
- [ ] Phase 4.5: commit the fold with a descriptive message
- [ ] Phase 5: create CI workflows, commit
- [ ] Phase 6: force-push to `onsager-ai/onsager`, archive siblings with README redirects
- [ ] Verify: `gh repo view onsager-ai/onsager` shows new monorepo, CI runs green on the first commit
- [ ] Open follow-up PRs A–F as separate issues (don’t execute them in the migration PR)

-----

## 12. Do-NOT list

Things the executing agent should explicitly **not** do:

- **Do NOT touch `onsager-ai/ising`** — different project, out of scope.
- **Do NOT touch `onsager-ai/telegramable`** — stays polyrepo by design.
- **Do NOT make `crates/onsager` (dispatcher) depend on `stiglab` or `synodic`** — breaks architectural loose coupling.
- **Do NOT attempt to unify stiglab-ui and synodic-ui visuals in the migration PR** — that’s PR-B, post-migration.
- **Do NOT renumber specs** — each subsystem keeps its own numbering space.
- **Do NOT publish any crate to crates.io** as part of migration.
- **Do NOT create any `@codervisor/*` npm wrapper packages** — no backwards compat.
- **Do NOT leave `git = "https://..."` deps anywhere** — all internal deps must be `path = "../..."`.
- **Do NOT create intermediate commits that don’t build** in the main migration sequence — use a topic branch with force-push during fold iteration, then fast-forward merge.

-----

## 13. Open questions the agent should surface (not decide)

If during execution the agent encounters any of the following, it should **stop and ask** rather than decide:

1. `axum` version reconciliation ends up requiring non-trivial code changes to stiglab’s server module (not a simple version bump).
1. `sqlx` feature flag conflicts cannot be resolved by workspace-level feature union.
1. A spec file collision between subsystems (shouldn’t happen — different numbering spaces — but flag if it does).
1. A `use` path in Rust source that doesn’t match any of the sed rewrite patterns (indicates an unforeseen naming convention).
1. dashboard pnpm install reveals a peer-dependency conflict between stiglab-ui and governance/ components (expected: they shared React major version, but verify).
1. Any file in a source repo that doesn’t obviously map to a target location in the new structure.

-----

*End of migration plan.*

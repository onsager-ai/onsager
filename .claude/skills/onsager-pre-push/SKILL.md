---
name: onsager-pre-push
description: Run before pushing code to the Onsager repo to catch what CI would fail on. Reproduces CI's merge preview + strict-warnings environment and renumbers colliding migrations. Triggers include "before push", "ready to push", "pre-push check", "push readiness", "prep for PR", or proactively before any git push on an Onsager branch.
---

# onsager-pre-push

Mechanical checklist that catches the CI failures this repo has actually had.
Runs in a few minutes end-to-end. **Never skip steps — they each exist because
something failed once.**

## Why

`rust.yml` runs on `pull_request`, which means GitHub checks out a **merge of
`origin/main` + the PR branch**, not the branch alone. The workflow also sets
`RUSTFLAGS: -D warnings`. Local `cargo build` without those two things is
insufficient. Skipping this checklist is how we got three red CI runs in a row.

## Steps

Run all of these from the repo root.

### 1. Sync main into the branch

```bash
git fetch origin main
git merge origin/main --no-edit
```

If there are conflicts, resolve them **now**, not on the PR page. Common
collision patterns in this repo:

- **Migrations**: both branches added `NNN_foo.sql`. Renumber yours to the
  next unused N, then update all three reference sites:
  `justfile` (`db-migrate`), `docker-compose.yml` (`migrate` service's
  `entrypoint`), `.github/workflows/rust.yml` (`Apply database migrations`
  step).
- **Enum variants**: main removed `Kind::{Report, Dataset, Config, ApiCall}`
  in favour of `Kind::PullRequest` once. If you see `variant not found`,
  grep for the old variant names and update each `match` arm.
- **Event envelope variants**: both branches added variants to
  `FactoryEventKind` — the auto-merge succeeds textually but duplicates or
  re-orders arms. Build verifies.

### 2. Build with CI's flags

```bash
RUSTFLAGS="-D warnings" cargo build --workspace
```

Treat **any** warning as a blocker. Do not `#[allow(dead_code)]` your way
past it; fix the root cause.

### 3. Test

```bash
RUSTFLAGS="-D warnings" cargo test --workspace --lib
```

`--lib` matches CI's main test pass. DB-gated integration tests under
`crates/onsager-spine/tests/` skip without `DATABASE_URL`; CI sets it, so if
you changed anything that touches Postgres, also run:

```bash
DATABASE_URL="postgres://onsager:onsager@localhost:5432/onsager" \
  cargo test -p onsager-spine -- --test-threads=1
```

(Requires `just dev-infra` running.)

### 4. Clippy with all targets

```bash
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets -- -D warnings
```

`--all-targets` catches warnings in tests + examples that the plain build
misses.

### 5. Format

```bash
cargo fmt --all
cargo fmt --all -- --check
```

First line applies, second verifies. If the second fails after the first,
something is interfering with rustfmt (rare — usually a tool config drift).

### 6. Push

```bash
git push -u origin <branch>
```

Retry up to 4 times with exponential backoff on transient network errors.
**Never** use `--force` on main or long-lived branches without explicit ask.

## Fast path

If nothing under `crates/` or migrations changed (e.g. docs only), steps 2–4
are optional. Step 1 is not — main still may have moved.

## What this skill does NOT cover

- Opening or managing the PR — see `onsager-pr-lifecycle`.
- Writing the code — that's the actual task.
- Smoke-testing Railway deploys — see the `railway` skill.

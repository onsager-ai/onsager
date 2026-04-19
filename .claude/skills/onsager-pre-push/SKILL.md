---
name: onsager-pre-push
description: Run before pushing code to the Onsager repo to catch what CI would fail on, and confirm the branch has a linked spec issue in a valid state. Reproduces CI's merge preview + strict-warnings environment and renumbers colliding migrations. Triggers include "before push", "ready to push", "pre-push check", "push readiness", "prep for PR", or proactively before any git push on an Onsager branch.
---

# onsager-pre-push

Mechanical checklist that catches the CI failures this repo has actually had,
plus a spec-link check that enforces the SDD loop locally. Runs in a few
minutes end-to-end. **Never skip steps — they each exist because something
failed once.**

## Why

`rust.yml` runs on `pull_request`, which means GitHub checks out a **merge of
`origin/main` + the PR branch**, not the branch alone. The workflow also sets
`RUSTFLAGS: -D warnings`. Local `cargo build` without those two things is
insufficient. Skipping this checklist is how we got three red CI runs in a row.

The spec-link step enforces "no PR without a spec or a `trivial` label" at
push time, before the PR is open — so the author sees the problem locally
instead of hearing about it from `pr-opened-progress` after the fact.

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

### 6. Spec-issue link check

Before pushing, confirm this branch corresponds to a known spec issue (or
is explicitly trivial). This is the local counterpart to the
`pr-opened-progress` routine — catching the miss here avoids a round-trip.

1. **Find the spec issue.** Search open issues with the `spec` label
   whose title or body matches the branch's purpose:

   ```
   mcp__github__list_issues  labels=[spec, planned]   state=open
   ```

   Or read your commit messages (`git log origin/main..HEAD`) for a
   `#N` reference. The SDD loop assumes you already know which spec this
   work closes or is part of — if you don't, stop and create one via
   `issue-spec` (or triage whether this is truly `trivial`).

2. **Confirm the spec's status is `planned` or `in-progress`.** If still
   `draft`, stop — the human-AI alignment gate has not been passed.
   Resolve open questions on the spec issue first.

3. **Draft the PR body linking line** so you can paste it into the PR:

   - `Closes #N` if this PR delivers the full spec.
   - `Part of #N` if it's one slice of a multi-PR spec.
   - `Fixes #N` for a defect referenced by a bug spec.

   Also draft a `## Delivers` subsection listing the exact Plan items you
   tick with this PR.

4. **If this is genuinely trivial** (typo, doc-only, one-line obvious
   fix), skip steps 6.1–6.3 and plan to apply the `trivial` label to the
   PR immediately after `mcp__github__create_pull_request`. Use sparingly.

### 7. Push

```bash
git push -u origin <branch>
```

Retry up to 4 times with exponential backoff on transient network errors.
**Never** use `--force` on main or long-lived branches without explicit ask.

After push, the `pr-opened-progress` routine (if configured) will flip the
linked spec to `in-progress` automatically. If routines aren't configured,
do it manually via `onsager-pr-lifecycle`.

## Fast path

If nothing under `crates/` or migrations changed (e.g. docs only), steps 2–4
are optional. Step 1 is not — main still may have moved. Step 6 is not —
the spec-link requirement applies to all non-trivial PRs.

## What this skill does NOT cover

- Writing the spec issue — see [`issue-spec`](../issue-spec/SKILL.md).
- Opening or managing the PR — see [`onsager-pr-lifecycle`](../onsager-pr-lifecycle/SKILL.md).
- The end-to-end dev loop — see [`onsager-dev-process`](../onsager-dev-process/SKILL.md).
- Smoke-testing Railway deploys — see the `railway` skill.

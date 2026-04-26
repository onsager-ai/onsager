---
name: onsager-pre-push
description: Run before pushing code to the Onsager repo to catch what CI would fail on, and confirm the branch has a linked spec issue in a valid state. Reproduces CI's merge preview + strict-warnings environment, renumbers colliding migrations, and walks through the repo's common merge-conflict patterns. Triggers include "before push", "ready to push", "pre-push check", "push readiness", "prep for PR", "resolve merge conflict", "merge conflict", "branch has conflicts", "sync with main", or proactively before any git push on an Onsager branch.
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
instead of hearing about it from `pr-spec-sync` after the fact.

## Steps

Run all of these from the repo root.

### 1. Sync main into the branch

```bash
git fetch origin main
git merge origin/main --no-edit
```

Resolve conflicts **locally**, before push — never on the PR "Resolve
conflicts" web editor (it bypasses `cargo build`/`clippy` and routinely
lands broken states). If the merge aborts cleanly, skip to step 2.

#### Resolving conflicts

1. **Inventory** what conflicted:

   ```bash
   git status --short                   # U* lines = unresolved paths
   git diff --name-only --diff-filter=U
   ```

2. **Work by pattern, not by file.** A single logical conflict (e.g. an
   enum variant change) usually spans several files. Match what you see
   against the patterns below before touching conflict markers — the
   right fix is often "take main's version and re-apply your change on
   top", not a line-by-line merge.

3. **Resolve**, then stage each resolved path with `git add <path>`.
   Re-run `git status` until no `U*` entries remain.

4. **Verify before committing the merge.** A passing merge means:

   ```bash
   RUSTFLAGS="-D warnings" cargo build --workspace
   RUSTFLAGS="-D warnings" cargo test --workspace --lib
   cargo fmt --all -- --check
   ```

   Only then:

   ```bash
   git commit --no-edit                 # default "Merge branch 'main' ..." message
   ```

5. **If you get lost**, bail and retry:

   ```bash
   git merge --abort
   ```

   This restores pre-merge state. Never `git reset --hard` or
   `git checkout --` without confirming nothing is staged you care
   about — the merge carries uncommitted resolutions.

   Prefer `merge` over `rebase` for syncing main here: the branch is
   likely already pushed, rebase rewrites history, and force-push is a
   destructive action per repo policy.

#### Common collision patterns in this repo

Match the symptom in `git status` / build output to the pattern:

- **Migrations (`migrations/NNN_*.sql`)**: both branches added the same
  `NNN`. Keep main's file at `NNN`, renumber yours to the next unused
  `NNN+k`, then update **all three** reference sites or the CI migrate
  step will skip it:
  - `justfile` — the `db-migrate` recipe.
  - `docker-compose.yml` — the `migrate` service's `entrypoint`.
  - `.github/workflows/rust.yml` — the `Apply database migrations` step.

  Sanity check: `git grep -n '<old-NNN>_'` should return zero hits
  after renumber.

- **Enum variants** (e.g. `Kind::{Report,Dataset,Config,ApiCall}` →
  `Kind::PullRequest`): main removed variants your branch still uses.
  Build reports `variant not found`. `git grep` each removed variant
  and update every `match` arm; don't add a catch-all `_ =>` just to
  silence the compiler — the exhaustive check is load-bearing.

- **Event envelope variants** (`FactoryEventKind`, `FactoryEvent`):
  both branches added arms. Textual merge succeeds but may duplicate
  or re-order variants. After merge, open the enum, dedupe, and
  re-run `cargo build`; serde-renamed variants sharing a tag will
  cause runtime dispatch errors that compile fine.

- **`Cargo.lock`**: always resolve by re-running `cargo build --workspace`
  after accepting either side — don't hand-edit. If both branches
  bumped the same dep differently, prefer main's version and let your
  change re-request an update.

- **`pnpm-lock.yaml` / `package.json`**: take main's `pnpm-lock.yaml`,
  re-apply your `package.json` edits, then `pnpm install` to
  regenerate the lockfile deterministically.

- **Spine event schema (`crates/onsager-spine/src/events/`)**: schema
  drift between branches silently changes wire format. After resolving,
  run `cargo test -p onsager-spine --lib` to catch serde round-trip
  failures before CI does.

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
`pr-spec-sync` workflow — catching the miss here avoids a round-trip.

1. **Find the spec issue.** Search open issues with the `spec` label
   whose title or body matches the branch's purpose:

   ```
   mcp__github__list_issues  labels=[spec]   state=open
   ```

   Then filter in memory for `planned` or `in-progress` status labels.

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

4. **Scan the branch's commit messages for implicit issue references**
   (advisory, not blocking):

   ```bash
   git log --format='%s%n%b' origin/main..HEAD | grep -oE '#[0-9]+' | sort -u
   ```

   For each `#N` returned, check whether the drafted PR body already
   mentions it on a linking line (`Closes` / `Fixes` / `Resolves` /
   `Part of` / `Refs` / `Related`). If not, decide deliberately:

   - **PR delivers that issue's acceptance** → add `Closes #N` to the
     body. Multi-issue `Closes` lines are fine (`Closes #27, Closes #30,
     Closes #33`). Auto-close doesn't fire for issues that are only
     *mentioned* in commit subjects — without an explicit `Closes` line
     those issues stay open after merge.
   - **PR only touches that issue** → use `Refs #N` so it cross-links
     without claiming closure.
   - **False positive** (issue number inside a code identifier, commit
     hash, etc.) → ignore.

   This is the step that catches the PR #43 failure mode: three
   acceptance criteria met across three commits, only one `#N` in the
   title, two issues silently left open after merge.

5. **If this is genuinely trivial** (typo, doc-only, one-line obvious
   fix), skip the spec-link substeps above (1–4) and plan to apply the
   `trivial` label to the PR immediately after
   `mcp__github__create_pull_request`. The seam-rule self-check in
   step 6.5 still runs — it's a no-op for genuinely doc-only diffs and
   takes seconds otherwise. Use the trivial label sparingly.

### 6.5. Seam-rule self-check

Before pushing, look at the diff once more against the canonical rule:

> HTTP APIs exist only at external boundaries:
> - **User-facing endpoints** called by the dashboard.
> - **Webhooks** called by external services (GitHub, etc.).
>
> Subsystems (`forge`, `stiglab`, `synodic`, `ising`) coordinate
> **exclusively** via the spine: events on the bus + reads against
> shared spine tables. No subsystem makes HTTP calls to another
> subsystem. No subsystem imports another subsystem's crate.

Run a focused diff scan — Lever B of spec #131 will eventually hard-fail
each of these in CI; until that lands, catch them locally:

```bash
git diff origin/main...HEAD -- 'crates/forge/**' 'crates/stiglab/**' \
  'crates/synodic/**' 'crates/ising/**'
```

Flag any of:

- **New sibling-subsystem HTTP client.** A new `reqwest::Client` /
  `hyper::Client` constructed in `forge|stiglab|synodic|ising` whose
  URL targets a sibling-subsystem port (3000, 3001, …) or service
  name. The right shape is event emit + listener pair.
- **New cross-subsystem Cargo dep.** Subsystem A's `Cargo.toml`
  declaring B as a dep, e.g. `forge` adding `stiglab = { path = ... }`.
  Subsystems may depend only on `onsager-{artifact, warehouse,
  protocol, spine, registry, delivery}` per their existing manifests.
- **New `serde(alias = …)`.** Renames must land atomically; aliases
  ossify (PR #107 is the canonical case).
- **New `*_mirror.rs` file or "translator" module.** The spine is
  already the source of truth; mirror modules drift (PR #129 / Lever
  D is removing the live one).
- **New `pub type X = Y` "for compatibility".** Same problem as a
  serde alias.
- **New `FactoryEventKind` variant without a consumer.** Producer +
  consumer should land together (PR #127 drift pattern). Lever E will
  make this CI-enforceable.
- **New API endpoint with no dashboard caller, or new dashboard
  client method with no backend handler.** PR #108 drift pattern;
  ship both halves in one PR (or two PRs gated by a contract test).

If any of these appear, fix the seam before pushing — do not file a
follow-up issue and ship the bridge. (Database schema migrations are
the only exception; they remain governed by `migrations/NNN_*.sql`.)

### 7. Push

```bash
git push -u origin <branch>
```

Retry up to 4 times with exponential backoff on transient network errors.
**Never** use `--force` on main or long-lived branches without explicit ask.

After push, the `pr-spec-sync.yml` workflow flips the linked spec to
`in-progress` automatically on PR open.

## Fast path

If nothing under `crates/` or migrations changed (e.g. docs only), steps 2–4
are optional. Step 1 is not — main still may have moved. Step 6 is not —
the spec-link requirement applies to all non-trivial PRs.

## What this skill does NOT cover

- Writing the spec issue — see [`issue-spec`](../issue-spec/SKILL.md).
- Opening or managing the PR — see [`onsager-pr-lifecycle`](../onsager-pr-lifecycle/SKILL.md).
- The end-to-end dev loop — see [`onsager-dev-process`](../onsager-dev-process/SKILL.md).
- Smoke-testing Railway deploys — see the `railway` skill.

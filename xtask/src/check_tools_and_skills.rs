//! `cargo run -p xtask -- check-tools-and-skills`
//!
//! Cross-references the portal MCP tool registry against the public
//! skills bundle (ADR 0007 / #288):
//!
//! 1. **Registry self-check (always runs).** Parses
//!    `crates/onsager-portal/src/mcp/registry.rs`, extracts every
//!    `ToolDescriptor { name: "..." }` entry, and verifies names are
//!    non-empty, unique, and valid `snake_case` identifiers. Catches
//!    the registry-drift failure mode (duplicate names, empty names,
//!    typos in names that wouldn't otherwise compile-fail).
//!
//! 2. **Skills cross-check (when the bundle is available).** The
//!    public skills bundle (`onsager-ai/onsager-skills`) lives in a
//!    sibling repo. To enable the cross-check, point
//!    `ONSAGER_SKILLS_DIR` at a local checkout of that repo (CI does
//!    this via a `git clone` step in the lint workflow; humans set it
//!    in their shell when both repos are checked out side-by-side).
//!    The lint then:
//!    - reads every `**/SKILL.md` under the directory,
//!    - parses the YAML frontmatter's `allowed_tools` list,
//!    - asserts every tool name in `allowed_tools` matches a
//!      registered tool, and
//!    - asserts every registered tool appears in at least one skill's
//!      `allowed_tools` (the half-wired drift pattern from PR #127
//!      applied to tools/skills).
//!
//! When `ONSAGER_SKILLS_DIR` is unset the lint prints a notice and
//! exits success — the registry self-check is hard-required, the
//! cross-check is opportunistic until the skills-bundle repo is
//! populated and CI wires it in.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::portal_registry::{self, ToolEntry};

const SKILLS_ENV: &str = "ONSAGER_SKILLS_DIR";

/// Tools that are intentionally registered in the portal MCP server
/// but do not yet have a paired skill grant in
/// `onsager-ai/onsager-skills`. Every entry is **debt** — the lint
/// catches accidental drift between registry and skills bundle, and
/// each exemption admits a known cross-repo-PR gap. The reason text
/// is grep-able and references the spec issue + the planned skill
/// name; landing the sibling-repo PR is a one-line removal here.
///
/// Same shape as the seam lint's `seam-allow` escape hatch: narrow,
/// labeled, trackable, never silently retained. Empty by default.
const PENDING_SKILL_GRANTS: &[(&str, &str)] = &[
    // Spec #395 — 0.2 substrate authoring tools. Paired skill
    // `onsager-author-substrate` is the planned sibling-repo PR; the
    // cloud session this PR was authored in is scoped to
    // `onsager-ai/onsager` only, so the skill grant lands in a
    // follow-up.
    (
        "submit_spec_plan",
        "spec #395 — pending `onsager-author-substrate` skill",
    ),
    (
        "update_spec",
        "spec #395 — pending `onsager-author-substrate` skill",
    ),
    (
        "list_spec_plans",
        "spec #395 — pending `onsager-author-substrate` skill",
    ),
    (
        "get_spec_plan",
        "spec #395 — pending `onsager-author-substrate` skill",
    ),
    (
        "compile_dry_run",
        "spec #395 — pending `onsager-author-substrate` skill",
    ),
    (
        "get_execution_plan",
        "spec #395 — pending `onsager-author-substrate` skill",
    ),
    (
        "submit_workflow",
        "spec #395 — pending `onsager-author-substrate` skill",
    ),
    (
        "update_workflow",
        "spec #395 — pending `onsager-author-substrate` skill",
    ),
    (
        "retire_workflow",
        "spec #395 — pending `onsager-author-substrate` skill",
    ),
    (
        "list_workflows_v2",
        "spec #395 — pending `onsager-author-substrate` skill",
    ),
    (
        "get_workflow_v2",
        "spec #395 — pending `onsager-author-substrate` skill",
    ),
];

pub fn run() -> Result<()> {
    let root = crate::workspace_root()?;
    let registry_path = root.join(portal_registry::REGISTRY_SRC);

    let tools = portal_registry::parse_registry(&registry_path)?;
    self_check(&tools)?;

    if let Ok(skills_dir) = std::env::var(SKILLS_ENV) {
        let skills_root = PathBuf::from(&skills_dir);
        if !skills_root.is_dir() {
            bail!(
                "{SKILLS_ENV}={skills_dir} but the path is not a directory; either unset \
                 the variable or point it at a local checkout of onsager-ai/onsager-skills"
            );
        }
        let skills = collect_skills(&skills_root)?;
        cross_check(&tools, &skills)?;
        println!(
            "check-tools-and-skills: {} tools, {} skills — all cross-references intact",
            tools.len(),
            skills.len(),
        );
    } else {
        println!(
            "check-tools-and-skills: {} tools, registry self-check passed. \
             Set {SKILLS_ENV} to a local checkout of onsager-ai/onsager-skills \
             to enable the skill cross-check.",
            tools.len(),
        );
    }
    Ok(())
}

fn self_check(tools: &[ToolEntry]) -> Result<()> {
    if tools.is_empty() {
        bail!("MCP tool registry is empty — at least one tool is required");
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for t in tools {
        if t.name.trim().is_empty() {
            bail!("MCP tool has an empty `name`");
        }
        if !is_snake_case(&t.name) {
            bail!(
                "MCP tool `{}` is not snake_case — names are wire identifiers and \
                 must match `^[a-z][a-z0-9_]*$`",
                t.name
            );
        }
        if !seen.insert(t.name.clone()) {
            bail!("MCP tool `{}` is registered twice", t.name);
        }
    }
    Ok(())
}

fn is_snake_case(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

// ---------------------------------------------------------------------------
// Skills bundle cross-check
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct SkillEntry {
    relative_path: String,
    allowed_tools: Vec<String>,
}

fn collect_skills(root: &Path) -> Result<Vec<SkillEntry>> {
    let mut out = Vec::new();
    walk_for_skills(root, root, &mut out)?;
    out.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(out)
}

fn walk_for_skills(root: &Path, dir: &Path, out: &mut Vec<SkillEntry>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        // Skip hidden directories and common non-skill folders.
        if name.starts_with('.') || name == "node_modules" || name == "target" {
            continue;
        }
        if path.is_dir() {
            walk_for_skills(root, &path, out)?;
        } else if name == "SKILL.md" {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("read {}", path.display()))?;
            let allowed_tools = parse_allowed_tools(&content)
                .with_context(|| format!("parse YAML frontmatter in {}", path.display()))?;
            out.push(SkillEntry {
                relative_path: relative,
                allowed_tools,
            });
        }
    }
    Ok(())
}

/// Pull `allowed_tools` from a SKILL.md's YAML frontmatter. Returns
/// an empty vec when the key isn't present — skills with no tool
/// grants are knowledge-only and pass the cross-check silently. The
/// only hard failures are (a) a skill granting a tool that isn't
/// registered, and (b) a registered tool that no skill grants. A
/// warning surface for knowledge-only skills is a possible follow-up;
/// today they're just neutral.
fn parse_allowed_tools(content: &str) -> Result<Vec<String>> {
    let frontmatter = extract_frontmatter(content)?;
    let mut tools = Vec::new();
    let mut in_block = false;
    for line in frontmatter.lines() {
        let trimmed = line.trim_end();
        if trimmed.starts_with("allowed_tools:") {
            // Inline form: `allowed_tools: [a, b]`
            if let Some(rest) = trimmed.strip_prefix("allowed_tools:") {
                let rest = rest.trim();
                if rest.starts_with('[') && rest.ends_with(']') {
                    let inner = &rest[1..rest.len() - 1];
                    for tok in inner.split(',') {
                        let cleaned = tok.trim().trim_matches(|c| c == '"' || c == '\'');
                        if !cleaned.is_empty() {
                            tools.push(cleaned.to_string());
                        }
                    }
                    in_block = false;
                    continue;
                }
            }
            in_block = true;
            continue;
        }
        if in_block {
            if let Some(item) = trimmed.strip_prefix("- ") {
                let cleaned = item.trim().trim_matches(|c| c == '"' || c == '\'');
                if !cleaned.is_empty() {
                    tools.push(cleaned.to_string());
                }
            } else if !trimmed.starts_with(' ') && !trimmed.starts_with('\t') {
                // Left the block — next top-level key.
                in_block = false;
            }
        }
    }
    Ok(tools)
}

fn extract_frontmatter(content: &str) -> Result<&str> {
    let rest = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
        .ok_or_else(|| anyhow!("SKILL.md does not start with `---` YAML frontmatter delimiter"))?;
    let end = rest
        .find("\n---")
        .ok_or_else(|| anyhow!("SKILL.md YAML frontmatter is not terminated by `---`"))?;
    Ok(&rest[..end])
}

fn cross_check(tools: &[ToolEntry], skills: &[SkillEntry]) -> Result<()> {
    let tool_names: BTreeSet<&str> = tools.iter().map(|t| t.name.as_str()).collect();

    // Every skill's allowed_tools must reference real tools.
    let mut errors: Vec<String> = Vec::new();
    let mut grants: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for skill in skills {
        for tool in &skill.allowed_tools {
            if !tool_names.contains(tool.as_str()) {
                errors.push(format!(
                    "skill `{}` grants unknown tool `{}`",
                    skill.relative_path, tool
                ));
            }
            grants
                .entry(tool.clone())
                .or_default()
                .push(skill.relative_path.clone());
        }
    }

    // Every registered tool must be granted by at least one skill.
    let pending: BTreeMap<&str, &str> = PENDING_SKILL_GRANTS.iter().copied().collect();
    let mut still_pending: Vec<(String, &str)> = Vec::new();
    for t in tools {
        let ungranted = grants.get(&t.name).map(|v| v.is_empty()).unwrap_or(true);
        if !ungranted {
            // Tool is granted: a `PENDING_SKILL_GRANTS` entry for it
            // is now stale and should be removed.
            if pending.contains_key(t.name.as_str()) {
                errors.push(format!(
                    "tool `{}` is now granted by a skill — remove its \
                     `PENDING_SKILL_GRANTS` entry in \
                     `xtask/src/check_tools_and_skills.rs`",
                    t.name
                ));
            }
            continue;
        }
        if let Some(reason) = pending.get(t.name.as_str()) {
            still_pending.push((t.name.clone(), reason));
            continue;
        }
        errors.push(format!(
            "tool `{}` is not granted by any skill — every public tool must \
             appear in at least one SKILL.md's allowed_tools",
            t.name
        ));
    }

    if !still_pending.is_empty() {
        // Print debt so it's visible on every lint run. The lint
        // itself doesn't fail on pending exemptions, but the noise
        // surfaces the gap on every CI / local run.
        println!(
            "check-tools-and-skills: {} pending skill grant(s) — debt tracked in \
             `PENDING_SKILL_GRANTS`:",
            still_pending.len()
        );
        for (tool, reason) in &still_pending {
            println!("  - {tool}: {reason}");
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        let header = format!(
            "check-tools-and-skills found {} cross-reference defect(s):",
            errors.len()
        );
        let body = errors.join("\n  - ");
        Err(anyhow!("{header}\n  - {body}"))
    }
}

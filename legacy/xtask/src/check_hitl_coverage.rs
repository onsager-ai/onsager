//! `cargo run -p xtask -- check-hitl-coverage`
//!
//! Enforces HITL principle 1 mechanically (spec #311 / #288):
//!
//! - Every mutation tool in `crates/onsager-portal/src/mcp/registry.rs`
//!   (i.e. category `Constructive` / `Diff` / `Destructive`) must have
//!   a corresponding `McpToolBinding` entry in
//!   `apps/dashboard/src/lib/mcp-tools.ts` with the matching slot and
//!   a `buildCard` factory.
//! - Every read-only tool in the Rust registry must have a binding in
//!   the TS file with `category: "read_only"` and a `renderInfo`
//!   factory (so chat renders a plain info block, not a card).
//! - No TS bindings without a matching Rust registry entry (the
//!   reverse drift — a UI for a tool that doesn't exist).
//!
//! The TS file is parsed by simple brace-matching against
//! `const <ident>: McpToolBinding = { ... }` blocks; we only need
//! `name`, `category`, and the presence/absence of `buildCard` /
//! `renderInfo` keys at the top of each block. A heavier parser
//! (swc, tsx-syn) is overkill for the four facts we check; the same
//! shape is used by `check_tools_and_skills.rs`.
//!
//! Counterpart of HITL principle 1 ("every mutation routes through a
//! HitlCard"). Same drift-shape as ADR 0004 Lever E (registry-backed
//! events) and `xtask check-tools-and-skills`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

use crate::portal_registry::{self, ToolCategory, ToolEntry};

const MCP_TOOLS_TS: &str = "apps/dashboard/src/lib/mcp-tools.ts";

pub fn run() -> Result<()> {
    let root = crate::workspace_root()?;
    let registry_path = root.join(portal_registry::REGISTRY_SRC);
    let tools_path = root.join(MCP_TOOLS_TS);

    let rust_tools = portal_registry::parse_registry(&registry_path)?;
    let ts_bindings = parse_ts_bindings(&tools_path)?;
    cross_check(&rust_tools, &ts_bindings)?;

    println!(
        "check-hitl-coverage: {} tools, {} dashboard bindings — HITL coverage intact",
        rust_tools.len(),
        ts_bindings.len(),
    );
    Ok(())
}

#[derive(Debug, Clone)]
struct TsBinding {
    name: String,
    category: String,
    has_build_card: bool,
    has_render_info: bool,
}

fn parse_ts_bindings(path: &Path) -> Result<Vec<TsBinding>> {
    let src = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let bytes = src.as_bytes();
    let mut bindings = Vec::new();
    let needle = b": McpToolBinding = {";
    let mut cursor = 0usize;
    while let Some(pos) = find_subslice(&bytes[cursor..], needle) {
        let absolute = cursor + pos;
        let open_brace = absolute + needle.len() - 1; // index of `{`
        let close_brace = match_brace(bytes, open_brace).ok_or_else(|| {
            anyhow!(
                "{}: unterminated `McpToolBinding` block at byte {}",
                path.display(),
                absolute
            )
        })?;
        let body = &src[open_brace..=close_brace];
        bindings.push(parse_binding_body(body)?);
        cursor = close_brace + 1;
    }
    Ok(bindings)
}

fn parse_binding_body(body: &str) -> Result<TsBinding> {
    let name = find_string_field(body, "name")
        .ok_or_else(|| anyhow!("McpToolBinding block has no `name: \"...\"` field"))?;
    let category = find_string_field(body, "category")
        .ok_or_else(|| anyhow!("McpToolBinding block has no `category: \"...\"` field"))?;
    let has_build_card = has_top_level_key(body, "buildCard");
    let has_render_info = has_top_level_key(body, "renderInfo");
    Ok(TsBinding {
        name,
        category,
        has_build_card,
        has_render_info,
    })
}

fn find_string_field(body: &str, key: &str) -> Option<String> {
    let bytes = body.as_bytes();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        if depth == 1 && is_word_start(bytes, i, key) {
            let after = i + key.len();
            let rest = &body[after..];
            let colon = rest.find(':')?;
            let post = &rest[colon + 1..];
            let trimmed = post.trim_start();
            let quote = trimmed.chars().next()?;
            if quote != '"' && quote != '\'' {
                return None;
            }
            let qpos = post.find(quote).unwrap();
            let after_q = &post[qpos + 1..];
            let close = after_q.find(quote)?;
            return Some(after_q[..close].to_string());
        }
        i += 1;
    }
    None
}

fn has_top_level_key(body: &str, key: &str) -> bool {
    let bytes = body.as_bytes();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        if depth == 1 && is_word_start(bytes, i, key) {
            // Look ahead for ':' (object key) — distinguishes from
            // `buildCard(...)` or comments mentioning the word.
            let after = i + key.len();
            for &c in bytes.iter().skip(after) {
                if c == b' ' || c == b'\t' {
                    continue;
                }
                return c == b':';
            }
            return false;
        }
        i += 1;
    }
    false
}

fn is_word_start(bytes: &[u8], i: usize, key: &str) -> bool {
    let k = key.as_bytes();
    if i + k.len() > bytes.len() {
        return false;
    }
    if &bytes[i..i + k.len()] != k {
        return false;
    }
    if i > 0 {
        let prev = bytes[i - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' {
            return false;
        }
    }
    let next_idx = i + k.len();
    if next_idx < bytes.len() {
        let next = bytes[next_idx];
        if next.is_ascii_alphanumeric() || next == b'_' {
            return false;
        }
    }
    true
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn match_brace(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn cross_check(rust: &[ToolEntry], ts: &[TsBinding]) -> Result<()> {
    let rust_by_name: BTreeMap<&str, &ToolEntry> =
        rust.iter().map(|t| (t.name.as_str(), t)).collect();
    let ts_by_name: BTreeMap<&str, &TsBinding> = ts.iter().map(|t| (t.name.as_str(), t)).collect();
    let mut errors: Vec<String> = Vec::new();

    // 1. Every Rust tool must have a TS binding with matching category +
    //    the right factory.
    for tool in rust {
        let binding = match ts_by_name.get(tool.name.as_str()) {
            Some(b) => b,
            None => {
                errors.push(format!(
                    "MCP tool `{}` ({}) has no entry in {} — every public tool needs a HitlCard \
                     slot assignment (or read-only info block) on the dashboard side",
                    tool.name, tool.category, MCP_TOOLS_TS,
                ));
                continue;
            }
        };
        let expected_ts = ts_category(tool.category);
        if binding.category != expected_ts {
            errors.push(format!(
                "MCP tool `{}` category mismatch: Rust registry says `{}`, dashboard binding says \
                 `{}` — they must match (`Constructive` ↔ `constructive`, etc.)",
                tool.name, tool.category, binding.category,
            ));
        }
        match tool.category {
            ToolCategory::Constructive | ToolCategory::Diff | ToolCategory::Destructive => {
                if !binding.has_build_card {
                    errors.push(format!(
                        "mutation tool `{}` is missing `buildCard:` in its dashboard binding — \
                         every mutation must route through a HitlCard (HITL principle 1)",
                        tool.name,
                    ));
                }
            }
            ToolCategory::ReadOnly => {
                if !binding.has_render_info {
                    errors.push(format!(
                        "read-only tool `{}` is missing `renderInfo:` in its dashboard binding — \
                         read-only tools render as plain info blocks in chat",
                        tool.name,
                    ));
                }
                if binding.has_build_card {
                    errors.push(format!(
                        "read-only tool `{}` declares `buildCard:` in its dashboard binding — \
                         only mutation tools render HitlCards",
                        tool.name,
                    ));
                }
            }
        }
    }

    // 2. No TS binding without a Rust registry entry (catches stale
    //    bindings after a tool is removed from the server).
    let rust_names: BTreeSet<&str> = rust_by_name.keys().copied().collect();
    for b in ts {
        if !rust_names.contains(b.name.as_str()) {
            errors.push(format!(
                "dashboard binding `{}` does not match any MCP tool in the Rust registry — \
                 remove it from {} or re-add the tool",
                b.name, MCP_TOOLS_TS,
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        let header = format!(
            "check-hitl-coverage found {} HITL-coverage defect(s):",
            errors.len()
        );
        let body = errors.join("\n  - ");
        bail!("{header}\n  - {body}");
    }
}

fn ts_category(c: ToolCategory) -> &'static str {
    match c {
        ToolCategory::Constructive => "constructive",
        ToolCategory::Diff => "diff",
        ToolCategory::Destructive => "destructive",
        ToolCategory::ReadOnly => "read_only",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ts_parser_extracts_name_category_and_factory_presence() {
        let src = r#"
const propose_workflow: McpToolBinding = {
  name: "propose_workflow",
  category: "constructive",
  title: (args) => "Create workflow",
  buildCard: (args) => ({
    kind: "constructive",
    title: "x",
    body: { fields: [] },
    commit: { label: "go", intent: "primary" },
    reject: { label: "no" },
  }),
}

const list_runs: McpToolBinding = {
  name: "list_runs",
  category: "read_only",
  title: () => "List runs",
  renderInfo: (args) => `Listing runs.`,
}
"#;
        let tmp = std::env::temp_dir().join("hitl_parse_test.ts");
        std::fs::write(&tmp, src).unwrap();
        let parsed = parse_ts_bindings(&tmp).unwrap();
        std::fs::remove_file(&tmp).ok();
        assert_eq!(parsed.len(), 2);
        let a = &parsed[0];
        assert_eq!(a.name, "propose_workflow");
        assert_eq!(a.category, "constructive");
        assert!(a.has_build_card);
        assert!(!a.has_render_info);
        let b = &parsed[1];
        assert_eq!(b.name, "list_runs");
        assert_eq!(b.category, "read_only");
        assert!(!b.has_build_card);
        assert!(b.has_render_info);
    }

    #[test]
    fn cross_check_flags_missing_build_card() {
        let rust = vec![ToolEntry {
            name: "propose_workflow".into(),
            category: ToolCategory::Constructive,
        }];
        let ts = vec![TsBinding {
            name: "propose_workflow".into(),
            category: "constructive".into(),
            has_build_card: false,
            has_render_info: false,
        }];
        let err = cross_check(&rust, &ts).unwrap_err().to_string();
        assert!(err.contains("missing `buildCard:`"), "got: {err}");
    }

    #[test]
    fn cross_check_flags_category_mismatch() {
        let rust = vec![ToolEntry {
            name: "edit_workflow".into(),
            category: ToolCategory::Diff,
        }];
        let ts = vec![TsBinding {
            name: "edit_workflow".into(),
            category: "constructive".into(),
            has_build_card: true,
            has_render_info: false,
        }];
        let err = cross_check(&rust, &ts).unwrap_err().to_string();
        assert!(err.contains("category mismatch"), "got: {err}");
    }

    #[test]
    fn cross_check_flags_extra_ts_binding() {
        let rust: Vec<ToolEntry> = Vec::new();
        let ts = vec![TsBinding {
            name: "ghost_tool".into(),
            category: "constructive".into(),
            has_build_card: true,
            has_render_info: false,
        }];
        let err = cross_check(&rust, &ts).unwrap_err().to_string();
        assert!(err.contains("does not match any MCP tool"), "got: {err}");
    }

    #[test]
    fn cross_check_passes_on_full_coverage() {
        let rust = vec![
            ToolEntry {
                name: "propose_workflow".into(),
                category: ToolCategory::Constructive,
            },
            ToolEntry {
                name: "list_runs".into(),
                category: ToolCategory::ReadOnly,
            },
        ];
        let ts = vec![
            TsBinding {
                name: "propose_workflow".into(),
                category: "constructive".into(),
                has_build_card: true,
                has_render_info: false,
            },
            TsBinding {
                name: "list_runs".into(),
                category: "read_only".into(),
                has_build_card: false,
                has_render_info: true,
            },
        ];
        cross_check(&rust, &ts).unwrap();
    }

    #[test]
    fn live_registry_and_dashboard_pass_hitl_coverage() {
        // Same end-to-end shape as `check_tools_and_skills`' self-test:
        // run the lint against the in-tree registry + dashboard
        // bindings. Catches drift between the two even when no fresh
        // commit touched either file.
        let root = crate::workspace_root().unwrap();
        let rust =
            portal_registry::parse_registry(&root.join(portal_registry::REGISTRY_SRC)).unwrap();
        let ts = parse_ts_bindings(&root.join(MCP_TOOLS_TS)).unwrap();
        cross_check(&rust, &ts).unwrap();
    }
}

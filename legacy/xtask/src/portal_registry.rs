//! Shared parser for the portal MCP tool registry. Both
//! `check_tools_and_skills` and `check_hitl_coverage` need to walk the
//! same `build_registry()` vec literal to pull out per-tool metadata;
//! keeping the parser in one place stops the two lints from drifting
//! against each other.
//!
//! Source of truth: `crates/onsager-portal/src/mcp/registry.rs`.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use syn::parse::Parser;
use syn::{Expr, ExprLit, ExprStruct, FieldValue, Item, Lit, Member, Stmt};

/// Path to the Rust registry, workspace-relative.
pub const REGISTRY_SRC: &str = "crates/onsager-portal/src/mcp/registry.rs";

/// Mirror of `crates::onsager_portal::mcp::registry::ToolCategory`. We
/// re-declare it here instead of importing because xtask is a leaf
/// crate that intentionally does not depend on portal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCategory {
    Constructive,
    Diff,
    Destructive,
    ReadOnly,
}

impl std::fmt::Display for ToolCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ToolCategory::Constructive => "Constructive",
            ToolCategory::Diff => "Diff",
            ToolCategory::Destructive => "Destructive",
            ToolCategory::ReadOnly => "ReadOnly",
        };
        f.write_str(s)
    }
}

/// One parsed entry from the registry's `vec![...]` literal.
#[derive(Debug, Clone)]
pub struct ToolEntry {
    pub name: String,
    pub category: ToolCategory,
}

/// Parse `crates/onsager-portal/src/mcp/registry.rs`'s `build_registry()`
/// body. Returns one `ToolEntry` per `ToolDescriptor { ... }` literal
/// in the trailing `vec![]`.
pub fn parse_registry(path: &Path) -> Result<Vec<ToolEntry>> {
    let src = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let file = syn::parse_file(&src).with_context(|| format!("parse {}", path.display()))?;

    let build_fn = file
        .items
        .iter()
        .find_map(|it| match it {
            Item::Fn(f) if f.sig.ident == "build_registry" => Some(f),
            _ => None,
        })
        .ok_or_else(|| anyhow!("could not find fn build_registry() in {}", path.display()))?;

    // The body must end with a `vec![ ToolDescriptor { ... }, ... ]`
    // expression. Require the **last** statement so a debug `dbg!(...)`
    // injected before the return expression fails loudly instead of
    // silently parsing the wrong value.
    let final_expr = match build_fn.block.stmts.last() {
        Some(Stmt::Expr(e, None)) => e,
        Some(_) => bail!("build_registry() body's last statement is not a bare expression"),
        None => bail!("build_registry() body is empty"),
    };

    let mac = match final_expr {
        Expr::Macro(m) => &m.mac,
        _ => bail!("build_registry()'s trailing expression is not `vec![...]`"),
    };
    if mac.path.segments.last().map(|s| s.ident.to_string()) != Some("vec".into()) {
        bail!("build_registry() must end with a `vec![...]` macro invocation");
    }

    let exprs = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated
        .parse2(mac.tokens.clone())
        .with_context(|| "parse build_registry()'s vec! body")?;

    let mut tools = Vec::new();
    for expr in exprs {
        let s = match expr {
            Expr::Struct(s) => s,
            other => bail!(
                "build_registry() vec! entries must be `ToolDescriptor {{ ... }}` struct \
                 literals; got {:?}",
                other
            ),
        };
        tools.push(extract_tool(&s)?);
    }
    Ok(tools)
}

fn extract_tool(s: &ExprStruct) -> Result<ToolEntry> {
    let name = field_str(&s.fields, "name")
        .ok_or_else(|| anyhow!("ToolDescriptor missing a `name: \"...\"` field"))?;
    let category = field_category(&s.fields, "category").ok_or_else(|| {
        anyhow!(
            "ToolDescriptor `{}` is missing a `category: ToolCategory::...` field",
            name,
        )
    })?;
    Ok(ToolEntry { name, category })
}

fn field_str(
    fields: &syn::punctuated::Punctuated<FieldValue, syn::Token![,]>,
    key: &str,
) -> Option<String> {
    for f in fields {
        let Member::Named(ident) = &f.member else {
            continue;
        };
        if ident != key {
            continue;
        }
        if let Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) = &f.expr
        {
            return Some(s.value());
        }
    }
    None
}

fn field_category(
    fields: &syn::punctuated::Punctuated<FieldValue, syn::Token![,]>,
    key: &str,
) -> Option<ToolCategory> {
    for f in fields {
        let Member::Named(ident) = &f.member else {
            continue;
        };
        if ident != key {
            continue;
        }
        if let Expr::Path(p) = &f.expr {
            // Match `ToolCategory::Constructive` etc. — last segment is the variant.
            let variant = p.path.segments.last()?.ident.to_string();
            return Some(match variant.as_str() {
                "Constructive" => ToolCategory::Constructive,
                "Diff" => ToolCategory::Diff,
                "Destructive" => ToolCategory::Destructive,
                "ReadOnly" => ToolCategory::ReadOnly,
                _ => return None,
            });
        }
    }
    None
}

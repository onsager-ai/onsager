//! Onsager workspace tasks. Currently:
//!
//!     cargo run -p xtask -- gen-event-docs           # write docs/events.md
//!     cargo run -p xtask -- gen-event-docs --check   # verify in sync
//!     cargo run -p xtask -- lint-seams               # check the seam rule
//!
//! The event catalog is derived from `crates/onsager-spine/src/factory_event.rs`
//! by parsing the `FactoryEventKind` enum and the `event_type()` /
//! `stream_type()` match arms. Adding a variant + its match arms automatically
//! extends the catalog on the next run; CI runs `--check` so a missing run
//! fails the build.
//!
//! `lint-seams` enforces the canonical seam rule from ADR 0004 / spec #131
//! Lever B — see [`lint_seams`] for the full check list.

mod lint_seams;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context, Result};
use syn::{Expr, ExprLit, Fields, ImplItem, Item, Lit, Meta, Pat, Stmt, Type, Variant, Visibility};

const SPINE_SRC: &str = "crates/onsager-spine/src/factory_event.rs";
const OUTPUT_DOC: &str = "docs/events.md";
const ENUM_NAME: &str = "FactoryEventKind";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let cmd = args.next();

    let result = match cmd.as_deref() {
        Some("gen-event-docs") => parse_gen_event_docs_flags(args).and_then(run_gen_event_docs),
        Some("lint-seams") => {
            if args.next().is_some() {
                Err(anyhow!("lint-seams takes no arguments"))
            } else {
                lint_seams::run()
            }
        }
        Some(other) => Err(anyhow!("unknown subcommand: {other}")),
        None => Err(anyhow!(
            "usage:\n  cargo run -p xtask -- gen-event-docs [--check]\n  cargo run -p xtask -- lint-seams"
        )),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Strict-parse the `gen-event-docs` flag set so typos like `--chek` fail
/// loudly instead of silently selecting the wrong mode.
fn parse_gen_event_docs_flags(args: impl IntoIterator<Item = String>) -> Result<bool> {
    let mut check = false;
    for arg in args {
        match arg.as_str() {
            "--check" => check = true,
            other => bail!("unknown flag for gen-event-docs: {other}"),
        }
    }
    Ok(check)
}

fn run_gen_event_docs(check: bool) -> Result<()> {
    let root = workspace_root()?;
    let src_path = root.join(SPINE_SRC);
    let out_path = root.join(OUTPUT_DOC);

    let src = std::fs::read_to_string(&src_path)
        .with_context(|| format!("read {}", src_path.display()))?;
    let file = syn::parse_file(&src).with_context(|| format!("parse {}", src_path.display()))?;

    let model = extract_model(&file)?;
    let rendered = render_markdown(&model);

    if check {
        let existing = std::fs::read_to_string(&out_path)
            .with_context(|| format!("read {}", out_path.display()))?;
        if existing != rendered {
            bail!(
                "{} is out of date.\n\nRegenerate with:\n    just gen-event-docs\n\nor:\n    cargo run -p xtask -- gen-event-docs",
                OUTPUT_DOC
            );
        }
        println!(
            "{OUTPUT_DOC} is up to date ({} events).",
            model.events.len()
        );
        return Ok(());
    }

    std::fs::write(&out_path, &rendered)
        .with_context(|| format!("write {}", out_path.display()))?;
    println!(
        "wrote {} ({} events, {} streams)",
        out_path.display(),
        model.events.len(),
        unique_streams(&model)
    );
    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .context("CARGO_MANIFEST_DIR not set; run via `cargo run -p xtask`")?;
    Ok(Path::new(&manifest)
        .parent()
        .ok_or_else(|| anyhow!("xtask manifest has no parent"))?
        .to_path_buf())
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Model {
    events: Vec<EventDoc>,
}

#[derive(Debug)]
struct EventDoc {
    /// Rust variant identifier, e.g. `ArtifactRegistered`.
    variant: String,
    /// Wire event type, e.g. `artifact.registered`.
    event_type: String,
    /// Logical stream / namespace, e.g. `artifact`.
    stream_type: String,
    /// Human description from the variant's doc-comment.
    description: String,
    /// Payload fields. Empty for unit variants.
    fields: Vec<FieldDoc>,
}

#[derive(Debug)]
struct FieldDoc {
    name: String,
    ty: String,
    description: String,
    optional: bool,
}

fn unique_streams(model: &Model) -> usize {
    let mut seen: Vec<&str> = model
        .events
        .iter()
        .map(|e| e.stream_type.as_str())
        .collect();
    seen.sort();
    seen.dedup();
    seen.len()
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

fn extract_model(file: &syn::File) -> Result<Model> {
    let mut variants: Option<Vec<Variant>> = None;
    let mut event_type_arms: Vec<(String, String)> = Vec::new();
    let mut stream_type_arms: Vec<(String, String)> = Vec::new();

    for item in &file.items {
        match item {
            Item::Enum(en) if en.ident == ENUM_NAME => {
                if !matches!(en.vis, Visibility::Public(_)) {
                    bail!("{ENUM_NAME} is not pub — refusing to document a private enum");
                }
                variants = Some(en.variants.iter().cloned().collect());
            }
            Item::Impl(im)
                if im.trait_.is_none() && type_ident(&im.self_ty).as_deref() == Some(ENUM_NAME) =>
            {
                for it in &im.items {
                    let ImplItem::Fn(f) = it else { continue };
                    let name = f.sig.ident.to_string();
                    if name == "event_type" {
                        event_type_arms = extract_match_arms(&f.block.stmts)?;
                    } else if name == "stream_type" {
                        stream_type_arms = extract_match_arms(&f.block.stmts)?;
                    }
                }
            }
            _ => {}
        }
    }

    let variants = variants
        .ok_or_else(|| anyhow!("could not find pub enum {ENUM_NAME} in factory_event.rs"))?;

    let mut events = Vec::with_capacity(variants.len());
    for v in &variants {
        let variant = v.ident.to_string();
        let event_type = event_type_arms
            .iter()
            .find(|(name, _)| name == &variant)
            .map(|(_, s)| s.clone())
            .ok_or_else(|| anyhow!("event_type() has no arm for {variant}"))?;
        let stream_type = stream_type_arms
            .iter()
            .find(|(name, _)| name == &variant)
            .map(|(_, s)| s.clone())
            .ok_or_else(|| anyhow!("stream_type() has no arm for {variant}"))?;
        let description = collect_doc(&v.attrs);
        let fields = match &v.fields {
            Fields::Named(named) => named
                .named
                .iter()
                .map(|f| -> Result<FieldDoc> {
                    let name = f
                        .ident
                        .as_ref()
                        .ok_or_else(|| {
                            anyhow!("variant {variant} has a named field without an identifier")
                        })?
                        .to_string();
                    let ty = type_to_string(&f.ty);
                    let optional = is_option(&f.ty);
                    let description = collect_doc(&f.attrs);
                    Ok(FieldDoc {
                        name,
                        ty,
                        description,
                        optional,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            Fields::Unit => Vec::new(),
            Fields::Unnamed(_) => bail!("variant {variant} uses tuple fields; not supported"),
        };
        events.push(EventDoc {
            variant,
            event_type,
            stream_type,
            description,
            fields,
        });
    }

    Ok(Model { events })
}

/// Walk a `match self { ... }` body and yield (variant_name, string_literal).
/// Handles both single-pattern arms (`Self::X { .. } => "x"`) and
/// or-patterns (`Self::A { .. } | Self::B { .. } => "ab"`).
fn extract_match_arms(stmts: &[Stmt]) -> Result<Vec<(String, String)>> {
    let match_expr = stmts
        .iter()
        .find_map(|s| match s {
            Stmt::Expr(Expr::Match(m), _) => Some(m),
            _ => None,
        })
        .ok_or_else(|| anyhow!("function body has no top-level match expression"))?;

    let mut out = Vec::new();
    for arm in &match_expr.arms {
        let lit = match arm.body.as_ref() {
            Expr::Lit(ExprLit {
                lit: Lit::Str(s), ..
            }) => s.value(),
            // event_type() returns &'static str literals. stream_type() likewise.
            // Anything else (formatted strings, .to_string(), etc.) is unexpected.
            _ => bail!("match arm body is not a string literal — generator can't handle it"),
        };
        for variant in collect_variants_from_pat(&arm.pat) {
            out.push((variant, lit.clone()));
        }
    }
    Ok(out)
}

/// From a pattern like `Self::A { .. } | Self::B { .. }` extract `["A", "B"]`.
fn collect_variants_from_pat(pat: &Pat) -> Vec<String> {
    let mut out = Vec::new();
    walk_pat(pat, &mut out);
    out
}

fn walk_pat(pat: &Pat, out: &mut Vec<String>) {
    match pat {
        Pat::Or(or) => {
            for p in &or.cases {
                walk_pat(p, out);
            }
        }
        Pat::Struct(s) => push_last_segment(&s.path, out),
        Pat::TupleStruct(t) => push_last_segment(&t.path, out),
        Pat::Path(p) => push_last_segment(&p.path, out),
        _ => {}
    }
}

fn push_last_segment(path: &syn::Path, out: &mut Vec<String>) {
    if let Some(seg) = path.segments.last() {
        out.push(seg.ident.to_string());
    }
}

fn type_ident(ty: &Type) -> Option<String> {
    if let Type::Path(p) = ty {
        p.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}

fn is_option(ty: &Type) -> bool {
    type_ident(ty).as_deref() == Some("Option")
}

fn type_to_string(ty: &Type) -> String {
    use quote::ToTokens;
    let mut s = ty.to_token_stream().to_string();
    // Collapse common spacing artifacts from `ToTokens` so generics, commas,
    // and path separators read as humans wrote them: `Foo < Bar >` → `Foo<Bar>`,
    // `Vec < T >` → `Vec<T>`, and `serde_json :: Value` → `serde_json::Value`.
    s = s
        .replace(" < ", "<")
        .replace(" >", ">")
        .replace(" ,", ",")
        .replace(" :: ", "::");
    s
}

fn collect_doc(attrs: &[syn::Attribute]) -> String {
    let mut lines = Vec::new();
    for a in attrs {
        let Meta::NameValue(nv) = &a.meta else {
            continue;
        };
        if !nv.path.is_ident("doc") {
            continue;
        }
        let Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) = &nv.value
        else {
            continue;
        };
        let mut line = s.value();
        if let Some(stripped) = line.strip_prefix(' ') {
            line = stripped.to_string();
        }
        lines.push(line);
    }
    // Collapse blank-line-separated paragraphs to a single space-joined string.
    // This keeps each event description compact in the rendered table while
    // preserving prose order.
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in lines {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(line.trim());
        }
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    paragraphs.join("\n\n")
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_markdown(model: &Model) -> String {
    let mut out = String::new();
    out.push_str(HEADER);

    // Group by stream_type, preserving first-seen stream order from the source.
    let mut stream_order: Vec<String> = Vec::new();
    for e in &model.events {
        if !stream_order.iter().any(|s| s == &e.stream_type) {
            stream_order.push(e.stream_type.clone());
        }
    }

    out.push_str("## Streams at a glance\n\n");
    out.push_str("| Stream | Producer subsystem | Event count |\n");
    out.push_str("|---|---|---|\n");
    for stream in &stream_order {
        let count = model
            .events
            .iter()
            .filter(|e| &e.stream_type == stream)
            .count();
        out.push_str(&format!(
            "| `{stream}` | {producer} | {count} |\n",
            producer = stream_producer(stream),
        ));
    }
    out.push('\n');

    out.push_str(NAVIGATION_HINT);

    for stream in &stream_order {
        out.push_str(&format!("## `{stream}` events\n\n"));
        out.push_str(&format!(
            "Producer subsystem: **{producer}**.\n\n",
            producer = stream_producer(stream)
        ));
        for e in model.events.iter().filter(|e| &e.stream_type == stream) {
            render_event(&mut out, e);
        }
    }

    out.push_str(FOOTER);
    out
}

fn render_event(out: &mut String, e: &EventDoc) {
    out.push_str(&format!("### `{}`\n\n", e.event_type));
    out.push_str(&format!("- Variant: `FactoryEventKind::{}`\n", e.variant));
    out.push_str(&format!("- Stream: `{}`\n", e.stream_type));
    out.push('\n');

    if !e.description.is_empty() {
        out.push_str(&e.description);
        out.push_str("\n\n");
    }

    if e.fields.is_empty() {
        out.push_str("_No payload fields._\n\n");
        return;
    }

    out.push_str("| Field | Type | Description |\n");
    out.push_str("|---|---|---|\n");
    for f in &e.fields {
        let mut desc = if f.description.is_empty() {
            String::new()
        } else {
            f.description.replace('\n', " ").replace('|', "\\|")
        };
        if f.optional && !desc.is_empty() {
            desc.push_str(" _(optional)_");
        } else if f.optional {
            desc.push_str("_(optional)_");
        }
        out.push_str(&format!(
            "| `{name}` | `{ty}` | {desc} |\n",
            name = f.name,
            ty = escape_md_code(&f.ty),
        ));
    }
    out.push('\n');
}

fn escape_md_code(s: &str) -> String {
    s.replace('|', "\\|")
}

/// Map a stream / namespace → the subsystem that owns and produces it. Hand-
/// curated because "stream" is a logical bus partition, not a code-derived
/// fact. If the spine ever grows a registry of producers, swap this for that.
fn stream_producer(stream: &str) -> &'static str {
    match stream {
        "artifact" => "forge",
        "warehouse" => "warehouse worker (forge)",
        "delivery" => "delivery worker (forge)",
        "deliverable" => "forge",
        "git" => "onsager-portal (GitHub webhooks)",
        "forge" => "forge",
        "stiglab" => "stiglab",
        "synodic" => "synodic",
        "ising" => "ising",
        "refract" => "refract",
        "workflow" => "stiglab (trigger) / forge (stage)",
        "registry" => "synodic (catalog crud)",
        "gate" => "onsager-portal (GitHub) / forge (manual)",
        _ => "(unknown — update `stream_producer` in xtask)",
    }
}

// ---------------------------------------------------------------------------
// Static prose
// ---------------------------------------------------------------------------

const HEADER: &str = "<!--
Generated by `cargo run -p xtask -- gen-event-docs`. Do not edit by hand.
Source of truth: crates/onsager-spine/src/factory_event.rs
CI runs `--check`; out-of-sync diffs will fail the build.
-->

# Onsager event catalog

This is the wire-level reference for every event written to the factory event
spine. It's auto-generated from the `FactoryEventKind` enum in
`crates/onsager-spine/src/factory_event.rs` — the single typed vocabulary
shared by every subsystem.

For the architectural rationale (why a bus, not direct calls) see
[ADR 0001](adr/0001-event-bus-coordination-model.md).

## Envelope

Every event is wrapped in a `FactoryEvent`:

```rust
pub struct FactoryEvent {
    pub event: FactoryEventKind,        // typed payload, see below
    pub correlation_id: Option<String>, // trace a causal chain
    pub causation_id: Option<i64>,      // id of the event that caused this one
    pub actor: String,                  // who emitted it
    pub timestamp: DateTime<Utc>,       // db-assigned at append time
}
```

Persisted in two tables (see `crates/onsager-spine/migrations/001_initial.sql`):

- **`events`** — typed factory events. Discriminator: `event_type` column,
  matches the wire string in each section below (e.g. `artifact.registered`).
  `stream_type` + `stream_id` partition the log by entity.
- **`events_ext`** — namespaced extension events for subsystem-private data
  that doesn't (yet) belong on the typed bus. Carries `(namespace, event_type)`
  and a free-form JSON payload. Validated by
  [`Namespace`](../crates/onsager-spine/src/namespace.rs) against the
  well-known set: `forge`, `stiglab`, `synodic`, `ising`, `telegramable`,
  `workflow`.

## Versioning

Event payload versioning is **not yet decided** — see ADR 0001. Until then,
treat the schema as additive: new fields land as `Option<T>` with
`#[serde(default, skip_serializing_if = \"Option::is_none\")]`, and existing
fields are not renamed or repurposed. Removing a field is a breaking change
that requires a coordinated rollout.

";

const NAVIGATION_HINT: &str =
    "Each section below covers one stream. Inside a section, every event lists \
its wire `event_type` string, the Rust variant name, the variant's doc \
comment, and a payload field table (where the field's own doc comment is the \
description).

";

const FOOTER: &str = "
---

_Regenerate with `just gen-event-docs`._
";

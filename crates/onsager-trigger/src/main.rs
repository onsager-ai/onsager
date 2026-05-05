//! `onsager trigger` — manual + replay trigger CLI (#241).
//!
//! Two surfaces:
//!
//!   onsager trigger fire   <workflow_id> --manual <name> [--payload <json>]
//!   onsager trigger replay <workflow_id> --event <id>
//!
//! Both emit `FactoryEventKind::TriggerFired` to the spine directly via
//! `EventStore` — no HTTP indirection, independent of portal (#222). The
//! consumer side is forge's existing `trigger_subscriber`; manual fires
//! flow through the same path as webhook fires.
//!
//! Every fire also emits a `workflow.manual_triggered` audit event with
//! `actor = "cli"` (or `"replay"`) and the user identity if available
//! via the `ONSAGER_USER` env var. Audit events live on the spine in the
//! `audit` namespace, not consumed by any subsystem (audit-only).
//!
//! Per #241 resolution: any authenticated user can fire / replay in v1.
//! The CLI inherits the user's shell environment as its "auth" — there's
//! no separate token; the CLI is operator-grade by design.

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use clap::{Args, Parser, Subcommand};
use onsager_registry::TRIGGERS;
use onsager_spine::factory_event::{FactoryEvent, FactoryEventKind};
use onsager_spine::{EventMetadata, EventStore, TriggerKind};
use serde_json::Value;

const DEFAULT_DATABASE_URL_VAR: &str = "DATABASE_URL";

#[derive(Parser)]
#[command(name = "onsager-trigger", about = "Fire or replay workflow triggers")]
struct Cli {
    /// Postgres connection URL (defaults to $DATABASE_URL).
    #[arg(long, global = true, env = DEFAULT_DATABASE_URL_VAR)]
    database_url: String,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Fire a manual trigger immediately.
    Fire(FireArgs),
    /// Replay a past `TriggerFired` event by id.
    Replay(ReplayArgs),
}

#[derive(Args)]
struct FireArgs {
    /// Workflow id to fire.
    workflow_id: String,
    /// Manual trigger name. Must match the workflow's declared
    /// `Manual { name }` trigger.
    #[arg(long)]
    manual: String,
    /// Optional JSON payload to attach.
    #[arg(long)]
    payload: Option<String>,
}

#[derive(Args)]
struct ReplayArgs {
    /// Workflow id to replay against (the replay re-fires under this
    /// workflow, not necessarily the workflow that produced the source).
    workflow_id: String,
    /// Event id of the past `TriggerFired` to re-emit.
    #[arg(long)]
    event: i64,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

#[tokio::main(flavor = "current_thread")]
async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("onsager_trigger=info")
        .compact()
        .init();

    let cli = Cli::parse();
    let store = EventStore::connect(&cli.database_url)
        .await
        .with_context(|| format!("connecting to {}", cli.database_url))?;

    let actor = std::env::var("ONSAGER_USER").unwrap_or_else(|_| "cli".to_string());

    match cli.cmd {
        Cmd::Fire(args) => fire(&store, &args, &actor).await,
        Cmd::Replay(args) => replay(&store, &args, &actor).await,
    }
}

async fn fire(store: &EventStore, args: &FireArgs, actor: &str) -> Result<()> {
    if TRIGGERS.lookup("manual").is_none() {
        return Err(anyhow!("registry manifest is missing the `manual` row"));
    }

    let workflow = load_workflow(store, &args.workflow_id).await?;
    let trigger_matches = matches!(
        &workflow.trigger,
        TriggerKind::Manual { name } if name == &args.manual
    );
    if !trigger_matches {
        return Err(anyhow!(
            "workflow `{}` does not declare a manual trigger named `{}` \
             (its trigger is {})",
            workflow.id,
            args.manual,
            workflow.trigger.kind_tag()
        ));
    }

    let extra_payload = match args.payload.as_deref() {
        Some(s) => Some(serde_json::from_str::<Value>(s).context("--payload is not valid JSON")?),
        None => None,
    };

    let now = Utc::now();
    let mut payload = serde_json::json!({
        "trigger_kind": "manual",
        "workflow_id": workflow.id,
        "workspace_id": workflow.workspace_id,
        "name": args.manual,
        "fired_at": now,
        "actor": actor,
        "source": "cli",
    });
    if let Some(extra) = extra_payload {
        if let (Value::Object(ref mut map), Value::Object(extra_map)) = (&mut payload, extra) {
            for (k, v) in extra_map {
                map.insert(k, v);
            }
        }
    }

    emit_trigger_fired(store, &workflow, "manual", payload, actor, now).await?;
    emit_audit(
        store,
        &workflow,
        "cli_fire",
        serde_json::json!({ "manual_name": args.manual, "actor": actor }),
        actor,
        now,
    )
    .await?;
    println!(
        "fired manual trigger `{}` for workflow {} (workspace {})",
        args.manual, workflow.id, workflow.workspace_id
    );
    Ok(())
}

async fn replay(store: &EventStore, args: &ReplayArgs, actor: &str) -> Result<()> {
    let source = load_trigger_fired(store, args.event).await?;
    let workflow = load_workflow(store, &args.workflow_id).await?;

    let now = Utc::now();
    let mut payload = match source.payload.clone() {
        Value::Object(map) => Value::Object(map),
        other => serde_json::json!({ "original_payload": other }),
    };
    if let Value::Object(ref mut map) = payload {
        // Accumulate the replay chain — each replay layer marks the
        // event id of its source. Replay-of-replay is allowed (#241
        // resolution); the chain is visible via these markers.
        let mut chain: Vec<i64> = map
            .get("replay_chain")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|x| x.as_i64()).collect())
            .unwrap_or_default();
        chain.push(args.event);
        map.insert("replay_of".into(), Value::from(args.event));
        map.insert(
            "replay_chain".into(),
            Value::Array(chain.into_iter().map(Value::from).collect()),
        );
        map.insert("replay_actor".into(), Value::String(actor.to_string()));
        map.insert("replay_fired_at".into(), Value::String(now.to_rfc3339()));
        // Stamp the canonical workflow + workspace; the source event
        // may have run under a different workflow.
        map.insert("workflow_id".into(), Value::String(workflow.id.clone()));
        map.insert(
            "workspace_id".into(),
            Value::String(workflow.workspace_id.clone()),
        );
        map.insert("trigger_kind".into(), Value::String("replay".into()));
        map.insert("source".into(), Value::String("cli_replay".into()));
    }

    emit_trigger_fired(store, &workflow, "replay", payload, actor, now).await?;
    emit_audit(
        store,
        &workflow,
        "cli_replay",
        serde_json::json!({
            "source_event_id": args.event,
            "source_trigger_kind": source.trigger_kind,
            "actor": actor,
        }),
        actor,
        now,
    )
    .await?;
    println!(
        "replayed event {} as `{}` fire for workflow {}",
        args.event, source.trigger_kind, workflow.id
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Spine helpers
// ---------------------------------------------------------------------------

/// Minimal `workflows`-row projection — all this CLI needs for emit /
/// validation.
struct WorkflowRow {
    id: String,
    workspace_id: String,
    trigger: TriggerKind,
    active: bool,
}

async fn load_workflow(store: &EventStore, workflow_id: &str) -> Result<WorkflowRow> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT workflow_id, workspace_id, active, trigger_kind, trigger_config \
           FROM workflows WHERE workflow_id = $1",
    )
    .bind(workflow_id)
    .fetch_optional(store.pool())
    .await
    .context("loading workflow row")?
    .ok_or_else(|| anyhow!("workflow `{workflow_id}` not found"))?;

    let id: String = row.try_get("workflow_id")?;
    let workspace_id: String = row.try_get("workspace_id")?;
    let active: bool = row.try_get("active")?;
    let kind_tag: String = row.try_get("trigger_kind")?;
    let cfg: Value = row.try_get("trigger_config")?;
    let trigger = TriggerKind::from_storage(&kind_tag, &cfg)
        .with_context(|| format!("workflow `{id}` has unparseable trigger config"))?;
    if !active {
        return Err(anyhow!("workflow `{id}` is not active"));
    }
    Ok(WorkflowRow {
        id,
        workspace_id,
        trigger,
        active,
    })
}

struct PastTriggerFired {
    trigger_kind: String,
    payload: Value,
}

async fn load_trigger_fired(store: &EventStore, event_id: i64) -> Result<PastTriggerFired> {
    let row = store
        .get_ext_event_by_id(event_id)
        .await
        .context("fetching source event")?
        .ok_or_else(|| anyhow!("event `{event_id}` not found in events_ext"))?;

    // The events_ext.data column may carry either the FactoryEvent
    // envelope or the bare FactoryEventKind. Try both.
    let kind = if let Ok(env) = serde_json::from_value::<FactoryEvent>(row.data.clone()) {
        env.event
    } else {
        serde_json::from_value::<FactoryEventKind>(row.data)
            .map_err(|e| anyhow!("event `{event_id}` is not a FactoryEvent: {e}"))?
    };

    match kind {
        FactoryEventKind::TriggerFired {
            trigger_kind,
            payload,
            ..
        } => Ok(PastTriggerFired {
            trigger_kind,
            payload,
        }),
        _ => Err(anyhow!(
            "event `{event_id}` is not a TriggerFired event (was {:?})",
            std::any::type_name_of_val(&kind)
        )),
    }
}

async fn emit_trigger_fired(
    store: &EventStore,
    workflow: &WorkflowRow,
    trigger_kind: &str,
    payload: Value,
    actor: &str,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    let envelope = FactoryEvent {
        event: FactoryEventKind::TriggerFired {
            workflow_id: workflow.id.clone(),
            trigger_kind: trigger_kind.to_string(),
            payload,
        },
        correlation_id: None,
        causation_id: None,
        actor: actor.to_string(),
        timestamp: now,
    };
    let data = serde_json::to_value(&envelope)?;
    let metadata = EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: actor.to_string(),
    };
    store
        .append_ext(
            &workflow.workspace_id,
            &format!("workflow:{}", workflow.id),
            "workflow",
            "trigger.fired",
            data,
            &metadata,
            None,
        )
        .await
        .context("appending trigger.fired")?;
    Ok(())
}

async fn emit_audit(
    store: &EventStore,
    workflow: &WorkflowRow,
    event_subtype: &str,
    detail: Value,
    actor: &str,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    let payload = serde_json::json!({
        "workflow_id": workflow.id,
        "workspace_id": workflow.workspace_id,
        "actor": actor,
        "event_subtype": event_subtype,
        "fired_at": now,
        "detail": detail,
    });
    let metadata = EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: actor.to_string(),
    };
    store
        .append_ext(
            &workflow.workspace_id,
            &format!("audit:workflow:{}", workflow.id),
            "audit",
            "workflow.manual_triggered",
            payload,
            &metadata,
            None,
        )
        .await
        .context("appending workflow.manual_triggered")?;
    Ok(())
}

// Quiet the unused-field lint when the binary is built without all helpers
// in use (e.g. cargo build with cfg gating).
#[allow(dead_code)]
fn _force_workflow_row_fields(w: &WorkflowRow) -> bool {
    w.active
}

//! Per-adapter reconciliation poller (spec #121, spine-emit half
//! landed via spec #430).
//!
//! Webhooks miss deliveries; the reconciler is the backstop. One
//! background task per project ticks at the configured interval,
//! reads the adapter's view via `Adapter::poll_since`, routes each
//! observed update through the shared [`translator`] into
//! `RoutedEvent`s, and persists them on the spine via
//! [`emit::emit_routed_events`]. The cursor advances only when the
//! batch emit succeeds (`(adapter_id, external_ref)` dedup makes
//! retrying an already-written event a silent no-op).
//!
//! The scheduler honors the per-project `ingestion_mode` column:
//!   * `webhook+reconciler` (default) — poll at the reconciler
//!     interval (300 s) as a backstop;
//!   * `polling-only` — poll at the polling-only interval (60 s),
//!     no public URL or webhook secret required;
//!   * `webhook-only` — no scheduler task spawned for this project.
//!
//! Interval values come from [`IngestionMode::poll_interval`] with a
//! 30 s floor (spec #121 § Alignment / "Human decides"). The floor
//! prevents a runaway config from hammering GitHub.

pub mod emit;
pub mod mode;
pub mod scheduler;
pub mod state;
pub mod translator;

pub use emit::{EmitOutcome, emit_routed_events};
pub use mode::{IngestionMode, MIN_POLL_INTERVAL, POLLING_ONLY_INTERVAL, RECONCILER_INTERVAL};
pub use scheduler::spawn_all;
pub use state::{load_state, touch_polled_at, upsert_state};
pub use translator::{GITHUB_ADAPTER_ID, translate_issue, translate_pull_request};

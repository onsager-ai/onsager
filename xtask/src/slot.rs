//! Slot allocator for per-worktree dev environments (spec #194).
//!
//! Each git worktree maps to a numbered slot recorded in
//! `.dev-slots.json` at the repo root. The slot deterministically
//! derives a 10-port block (`9000 + 10*N + offset`) and a docker-compose
//! project name `onsager-slot{N}`.
//!
//! Slot 0 is **special-cased to the legacy port layout** (5432, 5173,
//! 3000, 3001, 3003) for the main checkout. Slots 1..=99 use the
//! stride-by-10 scheme. The hard cap is 100; `max_slots` in the
//! manifest provides a soft cap.
//!
//! Subcommands:
//!
//!     cargo run -p xtask -- slot alloc <name> [--worktree <path>] [--branch <branch>]
//!     cargo run -p xtask -- slot free  <name>
//!     cargo run -p xtask -- slot list  [--json]
//!     cargo run -p xtask -- slot env   <name>
//!     cargo run -p xtask -- slot tunnel <name> [--host <host>]
//!     cargo run -p xtask -- slot get   <name>
//!     cargo run -p xtask -- slot project <name>     # docker-compose project name
//!
//! Slot 0 / "main" is reserved for the main checkout — `alloc` rejects
//! the names "main" and "0", and `env`/`get`/`project`/`tunnel` accept
//! either spelling as a shortcut for the main checkout.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

pub const MANIFEST_FILE: &str = ".dev-slots.json";
pub const HARD_MAX_SLOTS: u8 = 100;
pub const DEFAULT_MAX_SLOTS: u8 = 100;
pub const SLOT_PORT_BASE: u16 = 9000;
pub const SLOT_PORT_STRIDE: u16 = 10;

/// Names that can never be allocated. "main" and "0" address slot 0
/// (the main checkout), so handing them out would create entries that
/// `entry_or_main()` shadows — visible in `slot list` but invisible to
/// `slot env`/`get`/`project`/`tunnel`.
const RESERVED_NAMES: &[&str] = &["main", "0"];

/// Legacy port layout for slot 0 (the main checkout). Keeping this
/// special-cased preserves zero behavioral change for single-checkout
/// developers who never create a worktree. The legacy stiglab / synodic /
/// forge ports (3000 / 3001 / 3003) are owned by `just dev` directly,
/// not by the slot allocator — they aren't part of `SlotPorts` because
/// no slot 1..=99 publishes a parallel set on the host.
const SLOT0_EDGE_PORT: u16 = 5173;
const SLOT0_POSTGRES_PORT: u16 = 5432;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    /// Soft cap (1..=HARD_MAX_SLOTS) on how many slots may be allocated.
    /// Defaults to HARD_MAX_SLOTS when absent. Users on resource-constrained
    /// VMs can lower this to fail-fast before hitting RAM/disk pressure.
    #[serde(default = "default_max_slots")]
    pub max_slots: u8,
    #[serde(default)]
    pub slots: Vec<SlotEntry>,
}

fn default_max_slots() -> u8 {
    DEFAULT_MAX_SLOTS
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            max_slots: DEFAULT_MAX_SLOTS,
            slots: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SlotEntry {
    pub slot: u8,
    pub name: String,
    /// Path to the worktree, relative to the repo root. `.` for the
    /// main checkout in slot 0.
    pub worktree: String,
    /// Git branch the worktree is on. Tracked so `slot list` can show
    /// it without shelling out to `git`.
    #[serde(default)]
    pub branch: String,
}

/// Resolved port assignments for a slot — pure derivation from `slot`.
///
/// Only `edge` and `postgres` are published on the VM host by
/// `docker-compose.slot.yml`; everything else stays on the compose
/// network. The dashboard reaches stiglab/synodic/forge via Caddy's
/// same-origin reverse proxy at `edge`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotPorts {
    pub slot: u8,
    pub edge: u16,
    pub postgres: u16,
}

impl Manifest {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(MANIFEST_FILE);
        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                serde_json::from_str(&contents).with_context(|| format!("parse {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(anyhow!(e)).with_context(|| format!("read {}", path.display())),
        }
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let path = root.join(MANIFEST_FILE);
        let mut s = serde_json::to_string_pretty(self).context("serialize manifest")?;
        s.push('\n');
        std::fs::write(&path, s).with_context(|| format!("write {}", path.display()))
    }

    /// Find the lowest-free slot number, or fail when `max_slots` is
    /// reached. Slot 0 is reserved for the main checkout — the allocator
    /// hands out 1..=(max_slots-1) for worktrees.
    pub fn allocate(&self, name: &str) -> Result<u8> {
        if name.trim().is_empty() {
            bail!("slot name must not be empty");
        }
        if RESERVED_NAMES.contains(&name) {
            bail!("slot name {name:?} is reserved for the main checkout (slot 0)");
        }
        if self.slots.iter().any(|s| s.name == name) {
            bail!("a slot named {name:?} already exists");
        }
        if self.max_slots == 0 || self.max_slots > HARD_MAX_SLOTS {
            bail!(
                "max_slots {} out of range (1..={})",
                self.max_slots,
                HARD_MAX_SLOTS
            );
        }
        // Reserve slot 0 for the main checkout. Worktree slots start at 1.
        let used: std::collections::BTreeSet<u8> = self.slots.iter().map(|s| s.slot).collect();
        for candidate in 1..self.max_slots {
            if !used.contains(&candidate) {
                return Ok(candidate);
            }
        }
        bail!(
            "all {} slots in use (max_slots={}); free one with `just worktree-rm <name>`",
            self.max_slots.saturating_sub(1),
            self.max_slots
        );
    }

    pub fn find(&self, name: &str) -> Option<&SlotEntry> {
        self.slots.iter().find(|s| s.name == name)
    }

    pub fn remove(&mut self, name: &str) -> Result<SlotEntry> {
        let pos = self
            .slots
            .iter()
            .position(|s| s.name == name)
            .ok_or_else(|| anyhow!("no slot named {name:?}"))?;
        Ok(self.slots.remove(pos))
    }

    pub fn upsert_main(&mut self) {
        // The main checkout always occupies slot 0. We synthesize the
        // entry on demand so a fresh repo doesn't need a manifest at all.
        if !self.slots.iter().any(|s| s.slot == 0) {
            self.slots.push(SlotEntry {
                slot: 0,
                name: "main".to_string(),
                worktree: ".".to_string(),
                branch: String::new(),
            });
            self.slots.sort_by_key(|s| s.slot);
        }
    }
}

/// Map a slot number to its port assignments. Pure function — no I/O.
pub fn slot_ports(slot: u8) -> SlotPorts {
    if slot == 0 {
        return SlotPorts {
            slot,
            edge: SLOT0_EDGE_PORT,
            postgres: SLOT0_POSTGRES_PORT,
        };
    }
    let base = SLOT_PORT_BASE + SLOT_PORT_STRIDE * slot as u16;
    SlotPorts {
        slot,
        edge: base,
        postgres: base + 1,
        // Offsets 2..=9 are reserved within the slot's 10-port block for
        // ad-hoc debugger / profiler / direct-service ports a developer
        // can bind via `docker compose run --service-ports` at debug
        // time. They are intentionally not pre-published.
    }
}

pub fn project_name(slot: u8) -> String {
    format!("onsager-slot{slot}")
}

pub fn postgres_volume(slot: u8) -> String {
    format!("postgres-slot{slot}")
}

pub fn target_volume(slot: u8) -> String {
    format!("target-slot{slot}")
}

/// Render the env vars a slot's docker-compose project consumes. Written
/// to `worktrees/<name>/.env.slot` and sourced by the compose project.
pub fn slot_env(entry: &SlotEntry) -> Vec<(String, String)> {
    let p = slot_ports(entry.slot);
    let mut out = vec![
        ("ONSAGER_SLOT".into(), entry.slot.to_string()),
        ("ONSAGER_SLOT_NAME".into(), entry.name.clone()),
        ("ONSAGER_COMPOSE_PROJECT".into(), project_name(entry.slot)),
        (
            "ONSAGER_POSTGRES_VOLUME".into(),
            postgres_volume(entry.slot),
        ),
        ("ONSAGER_TARGET_VOLUME".into(), target_volume(entry.slot)),
        ("SLOT_EDGE_PORT".into(), p.edge.to_string()),
        ("SLOT_POSTGRES_PORT".into(), p.postgres.to_string()),
    ];
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// CLI driver
// ---------------------------------------------------------------------------

pub fn run(args: Vec<String>) -> Result<()> {
    let mut iter = args.into_iter();
    let sub = iter.next().ok_or_else(|| {
        anyhow!("slot: missing subcommand (alloc|free|list|env|get|project|tunnel)")
    })?;
    match sub.as_str() {
        "alloc" => cmd_alloc(iter.collect()),
        "free" => cmd_free(iter.collect()),
        "list" => cmd_list(iter.collect()),
        "env" => cmd_env(iter.collect()),
        "get" => cmd_get(iter.collect()),
        "project" => cmd_project(iter.collect()),
        "tunnel" => cmd_tunnel(iter.collect()),
        other => bail!("slot: unknown subcommand {other:?}"),
    }
}

fn cmd_alloc(args: Vec<String>) -> Result<()> {
    let mut name: Option<String> = None;
    let mut worktree: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--worktree" => {
                worktree = Some(
                    it.next()
                        .ok_or_else(|| anyhow!("--worktree needs a value"))?,
                )
            }
            "--branch" => {
                branch = Some(it.next().ok_or_else(|| anyhow!("--branch needs a value"))?)
            }
            other if other.starts_with("--") => bail!("unknown flag {other:?}"),
            other => {
                if name.is_some() {
                    bail!("unexpected positional arg {other:?}");
                }
                name = Some(other.to_string());
            }
        }
    }
    let name = name.ok_or_else(|| anyhow!("slot alloc: missing <name>"))?;
    let root = workspace_root()?;
    let mut manifest = Manifest::load(&root)?;
    let slot = manifest.allocate(&name)?;
    let entry = SlotEntry {
        slot,
        name: name.clone(),
        worktree: worktree.unwrap_or_else(|| format!("worktrees/{name}")),
        branch: branch.unwrap_or_else(|| name.clone()),
    };
    manifest.slots.push(entry.clone());
    manifest.slots.sort_by_key(|s| s.slot);
    manifest.save(&root)?;
    // Emit machine-parseable output on stdout so just recipes can capture
    // it without grep gymnastics. Human-readable summary on stderr.
    let p = slot_ports(slot);
    eprintln!(
        "allocated slot {slot} for {name:?} (edge {edge}, postgres {pg}, project {proj})",
        slot = slot,
        name = name,
        edge = p.edge,
        pg = p.postgres,
        proj = project_name(slot),
    );
    println!("{slot}");
    Ok(())
}

fn cmd_free(args: Vec<String>) -> Result<()> {
    let mut name: Option<String> = None;
    for a in args {
        match a.as_str() {
            other if other.starts_with("--") => bail!("unknown flag {other:?}"),
            other => {
                if name.is_some() {
                    bail!("unexpected positional arg {other:?}");
                }
                name = Some(other.to_string());
            }
        }
    }
    let name = name.ok_or_else(|| anyhow!("slot free: missing <name>"))?;
    let root = workspace_root()?;
    let mut manifest = Manifest::load(&root)?;
    let entry = manifest.remove(&name)?;
    manifest.save(&root)?;
    eprintln!("freed slot {} ({})", entry.slot, entry.name);
    println!("{}", entry.slot);
    Ok(())
}

fn cmd_list(args: Vec<String>) -> Result<()> {
    let mut json = false;
    for a in args {
        match a.as_str() {
            "--json" => json = true,
            other => bail!("unknown flag {other:?}"),
        }
    }
    let root = workspace_root()?;
    let mut manifest = Manifest::load(&root)?;
    manifest.upsert_main();
    if json {
        let s = serde_json::to_string_pretty(&manifest).context("serialize manifest")?;
        println!("{s}");
        return Ok(());
    }
    println!(
        "{:<5}  {:<20}  {:<7}  {:<7}  {:<25}  worktree",
        "slot", "name", "edge", "pg", "project"
    );
    for s in &manifest.slots {
        let p = slot_ports(s.slot);
        println!(
            "{:<5}  {:<20}  {:<7}  {:<7}  {:<25}  {}",
            s.slot,
            s.name,
            p.edge,
            p.postgres,
            project_name(s.slot),
            s.worktree
        );
    }
    Ok(())
}

fn cmd_env(args: Vec<String>) -> Result<()> {
    let name = args
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("slot env: missing <name>"))?;
    let root = workspace_root()?;
    let manifest = Manifest::load(&root)?;
    let entry = entry_or_main(&manifest, &name)?;
    for (k, v) in slot_env(&entry) {
        println!("{k}={v}");
    }
    Ok(())
}

fn cmd_get(args: Vec<String>) -> Result<()> {
    let name = args
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("slot get: missing <name>"))?;
    let root = workspace_root()?;
    let manifest = Manifest::load(&root)?;
    let entry = entry_or_main(&manifest, &name)?;
    println!("{}", entry.slot);
    Ok(())
}

fn cmd_project(args: Vec<String>) -> Result<()> {
    let name = args
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("slot project: missing <name>"))?;
    let root = workspace_root()?;
    let manifest = Manifest::load(&root)?;
    let entry = entry_or_main(&manifest, &name)?;
    println!("{}", project_name(entry.slot));
    Ok(())
}

fn cmd_tunnel(args: Vec<String>) -> Result<()> {
    let mut name: Option<String> = None;
    let mut host: Option<String> = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--host" => host = Some(it.next().ok_or_else(|| anyhow!("--host needs a value"))?),
            other if other.starts_with("--host=") => host = Some(other[7..].to_string()),
            other if other.starts_with("--") => bail!("unknown flag {other:?}"),
            other => {
                if name.is_some() {
                    bail!("unexpected positional arg {other:?}");
                }
                name = Some(other.to_string());
            }
        }
    }
    let name = name.ok_or_else(|| anyhow!("slot tunnel: missing <name>"))?;
    let host = host.unwrap_or_else(|| "vm".to_string());
    let root = workspace_root()?;
    let manifest = Manifest::load(&root)?;
    let entry = entry_or_main(&manifest, &name)?;
    let p = slot_ports(entry.slot);
    let mut flags = vec![
        format!("-L {edge}:localhost:{edge}", edge = p.edge),
        format!("-L {pg}:localhost:{pg}", pg = p.postgres),
    ];
    flags.push(host.to_string());
    println!("ssh {}", flags.join(" "));
    eprintln!(
        "slot {} ({}) — open http://localhost:{}/ after the tunnel is up",
        entry.slot, entry.name, p.edge
    );
    Ok(())
}

fn entry_or_main(manifest: &Manifest, name: &str) -> Result<SlotEntry> {
    if name == "main" || name == "0" {
        return Ok(SlotEntry {
            slot: 0,
            name: "main".to_string(),
            worktree: ".".to_string(),
            branch: String::new(),
        });
    }
    manifest
        .find(name)
        .cloned()
        .ok_or_else(|| anyhow!("no slot named {name:?}"))
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(slot: u8, name: &str) -> SlotEntry {
        SlotEntry {
            slot,
            name: name.into(),
            worktree: format!("worktrees/{name}"),
            branch: name.into(),
        }
    }

    #[test]
    fn allocate_picks_lowest_free_slot_starting_at_one() {
        let m = Manifest::default();
        // Slot 0 is reserved for the main checkout; allocator hands out 1+.
        assert_eq!(m.allocate("feat-a").unwrap(), 1);
    }

    #[test]
    fn allocate_skips_used_slots() {
        let mut m = Manifest::default();
        m.slots.push(entry(1, "a"));
        m.slots.push(entry(3, "c"));
        assert_eq!(m.allocate("b").unwrap(), 2);
        m.slots.push(entry(2, "b"));
        assert_eq!(m.allocate("d").unwrap(), 4);
    }

    #[test]
    fn allocate_rejects_when_max_slots_reached() {
        let m = Manifest {
            max_slots: 3,
            slots: vec![entry(1, "a"), entry(2, "b")],
        };
        // max_slots=3 means slots 0,1,2 are addressable; slot 0 is main,
        // so worktrees can occupy 1 and 2. A third allocation must fail.
        let err = m.allocate("c").unwrap_err().to_string();
        assert!(err.contains("all"), "expected exhaustion error, got: {err}");
    }

    #[test]
    fn allocate_rejects_duplicate_name() {
        let mut m = Manifest::default();
        m.slots.push(entry(1, "feat-a"));
        let err = m.allocate("feat-a").unwrap_err().to_string();
        assert!(err.contains("already exists"), "got: {err}");
    }

    #[test]
    fn allocate_rejects_empty_name() {
        let m = Manifest::default();
        assert!(m.allocate("").is_err());
        assert!(m.allocate("   ").is_err());
    }

    #[test]
    fn allocate_rejects_max_slots_zero_or_overflow() {
        let m = Manifest {
            max_slots: 0,
            slots: vec![],
        };
        assert!(m.allocate("a").is_err());
        let m = Manifest {
            max_slots: 200,
            slots: vec![],
        };
        assert!(m.allocate("a").is_err());
    }

    #[test]
    fn slot_zero_uses_legacy_ports() {
        let p = slot_ports(0);
        assert_eq!(p.edge, 5173);
        assert_eq!(p.postgres, 5432);
    }

    #[test]
    fn slot_n_uses_stride_by_ten() {
        let p1 = slot_ports(1);
        assert_eq!(p1.edge, 9010);
        assert_eq!(p1.postgres, 9011);
        let p7 = slot_ports(7);
        assert_eq!(p7.edge, 9070);
        assert_eq!(p7.postgres, 9071);
        let p99 = slot_ports(99);
        assert_eq!(p99.edge, 9990);
        assert_eq!(p99.postgres, 9991);
    }

    #[test]
    fn slot_ports_are_collision_free_across_slots() {
        let mut all: Vec<u16> = Vec::new();
        for n in 1..=99u8 {
            let p = slot_ports(n);
            all.extend([p.edge, p.postgres]);
        }
        let mut sorted = all.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            all.len(),
            "port collision detected across slots 1..=99"
        );
    }

    #[test]
    fn allocate_rejects_reserved_names() {
        let m = Manifest::default();
        for n in ["main", "0"] {
            let err = m.allocate(n).unwrap_err().to_string();
            assert!(err.contains("reserved"), "name {n:?} got: {err}");
        }
    }

    #[test]
    fn project_and_volume_names_follow_convention() {
        assert_eq!(project_name(0), "onsager-slot0");
        assert_eq!(project_name(7), "onsager-slot7");
        assert_eq!(postgres_volume(7), "postgres-slot7");
        assert_eq!(target_volume(7), "target-slot7");
    }

    #[test]
    fn manifest_load_missing_file_returns_default() {
        let dir = std::env::temp_dir().join(format!("onsager-slot-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // No manifest written — load must succeed with defaults.
        let m = Manifest::load(&dir).unwrap();
        assert_eq!(m.max_slots, DEFAULT_MAX_SLOTS);
        assert!(m.slots.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn manifest_round_trip_preserves_entries() {
        let dir = std::env::temp_dir().join(format!("onsager-slot-test-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut m = Manifest::default();
        m.slots.push(entry(1, "feat-a"));
        m.slots.push(entry(3, "feat-c"));
        m.save(&dir).unwrap();
        let loaded = Manifest::load(&dir).unwrap();
        assert_eq!(loaded, m);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upsert_main_is_idempotent() {
        let mut m = Manifest::default();
        m.upsert_main();
        m.upsert_main();
        m.upsert_main();
        assert_eq!(m.slots.iter().filter(|s| s.slot == 0).count(), 1);
        assert_eq!(m.slots[0].name, "main");
    }

    #[test]
    fn slot_env_includes_port_block_and_project_name() {
        let e = entry(2, "feat-b");
        let env: std::collections::BTreeMap<_, _> = slot_env(&e).into_iter().collect();
        assert_eq!(env.get("ONSAGER_SLOT").map(String::as_str), Some("2"));
        assert_eq!(env.get("SLOT_EDGE_PORT").map(String::as_str), Some("9020"));
        assert_eq!(
            env.get("SLOT_POSTGRES_PORT").map(String::as_str),
            Some("9021")
        );
        assert_eq!(
            env.get("ONSAGER_COMPOSE_PROJECT").map(String::as_str),
            Some("onsager-slot2")
        );
        assert_eq!(
            env.get("ONSAGER_POSTGRES_VOLUME").map(String::as_str),
            Some("postgres-slot2")
        );
        assert_eq!(
            env.get("ONSAGER_TARGET_VOLUME").map(String::as_str),
            Some("target-slot2")
        );
    }
}

//! `BATTLE_PLAN_VIEWER_CHECK_PIECE`: a piece's crossing leaves the mirrors where a reset would have.
//!
//! At a boundary the playhead carries each table's crossing instead of a reset
//! ([`playhead::boundary::crossed`]), and the prepare still rebuilds every table's state on the
//! fetchers. The judge takes those states and compares each one, table by table, with the mirror
//! the crossing's change sets left ([`super::mirror::Mirror::differs`]), failing by name. Every
//! layer's product then equals what a reset and a whole compose make, because a compose reads
//! nothing but the mirrors.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

static CHECKED: AtomicU64 = AtomicU64::new(0);
static DIFFERING: AtomicU64 = AtomicU64::new(0);

pub fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BATTLE_PLAN_VIEWER_CHECK_PIECE").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_PIECE")).is_some_and(|value| !value.is_empty() && value != "0"))
}

/// One more table judged, whatever it found.
pub fn judged() {
    CHECKED.fetch_add(1, Relaxed);
}

pub fn found(table: replay_data::rows::TableId, hour: replay_data::Hour, why: &str) {
    DIFFERING.fetch_add(1, Relaxed);
    eprintln!("BATTLE_PLAN_VIEWER_CHECK_PIECE: crossing into hour {hour}: table {table:?}: {why}");
}

/// Tables judged and differences found, for the run's report.
pub fn counted() -> (u64, u64) {
    (CHECKED.load(Relaxed), DIFFERING.load(Relaxed))
}

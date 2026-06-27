//! The per-tick RE-BIND digest (Stage 1 deliverable 5, immutability half).
//!
//! Moving the scrubber re-binds every open surface from the new frame's snapshot. The
//! no-fabrication / immutability invariant (M7 hardening list) is: each re-bind builds
//! a FRESH report from `&Snapshot` (the bind layer never mutates the `Arc<Snapshot>`),
//! so scrubbing A→B→A reproduces A's report BYTE-IDENTICALLY — no value from the B
//! visit leaks into the second A bind.
//!
//! This module is the single place the app computes a comparable digest of a bound
//! frame (topbar fields + flag identity + state-panel fields + building rows +
//! occupation). The live render path and the headless immutability test both call
//! [`rebind_digest`], so the test proves the SAME code the app runs is leak-free.
//!
//! It is GPU-free + window-free: it exercises `bind::apply_topbar` (into a throwaway
//! scene-graph) and `bind::bind_state` against a `&Snapshot`, exactly as a live tick
//! does, minus the draw. The flag identity is included because the resolved flag
//! sprite depends on TAG + ruling-party ideology — the exact stale-cache trap Codex
//! flagged (§4): if a tick reused the previous hour's flag, this digest would change.

use bind::{apply_topbar, bind_state, TopbarSpec};
use gui_runtime::scene::Node;
use snapshot::Snapshot;

/// A stable, comparable digest of a fully bound frame for one observed country + one
/// selected state. Identical inputs (same snapshot, country, state) → identical
/// string; ANY carried-over/fabricated value would perturb it.
pub fn rebind_digest(snap: &Snapshot, country: &str, state_id: &str, state_name: &str) -> String {
    // TOPBAR: bind into a THROWAWAY scene-graph (we only read the report here).
    let spec = TopbarSpec::topbar();
    let mut throwaway: Vec<Node> = Vec::new();
    let tb = apply_topbar(&mut throwaway, &spec, snap, country);

    // STATE PANEL: bind the selected state.
    let sp = bind_state(snap, state_id, state_name);

    let mut s = String::new();
    for f in &tb.fields {
        s.push_str(&format!(
            "TB:{}={}|{}\n",
            f.element, f.value, f.not_captured
        ));
    }
    // FLAG identity (TAG + ideology) — the resolved sprite depends on BOTH, so this
    // catches a stale flag carried across a scrub (the Codex §4 cache-key trap).
    s.push_str(&format!("FLAG:{:?}/{:?}\n", tb.flag_tag, tb.flag_ideology));
    for f in &sp.fields {
        s.push_str(&format!(
            "SP:{}={}|{}\n",
            f.element, f.value, f.not_captured
        ));
    }
    for b in sp
        .state_buildings
        .iter()
        .chain(sp.province_buildings.iter())
    {
        s.push_str(&format!("BR:{}={}@{}\n", b.key, b.level, b.frame));
    }
    s.push_str(&format!(
        "OWN:{:?} CTRL:{:?} OCC:{}\n",
        sp.owner_tag,
        sp.controller_tag,
        sp.occupation.is_some()
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use scrub::{Fetch, Timeline};
    use serde_json::json;
    use snapshot::Store;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    /// A tiny two-snapshot on-disk store, parsed through the REAL store reader (not an
    /// in-memory shortcut), so the LRU/timeline path is exercised end to end.
    fn tiny_store() -> (PathBuf, Store, String) {
        let dir = std::env::temp_dir().join(format!(
            "app_rebind_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let game = "g".to_string();
        let gdir = dir.join(&game);
        fs::create_dir_all(&gdir).unwrap();
        // hour 100: GER fascism pp=10, state 64 owner+controller GER, vp 5.
        let a = json!({
            "game_id":"g","date":"1940.1.1.12","hour_index":100,
            "countries":{"GER":{"politics":{"stability":0.5,"political_power":10.0,"ruling_party":"fascism"}}},
            "states":{"64":{"owner":"GER","controller":"GER","category":"city","victory_points_total":5,
                "manpower_pool":{"total":1000},"buildings":{"infrastructure":3},"occupation":null,
                "occupation_law":null,"resources":null}}
        });
        // hour 200: GER DEMOCRATIC now (flag ideology flips!), pp=99; state 64
        // controller flips to SOV, occupation appears, an extra building.
        let b = json!({
            "game_id":"g","date":"1941.1.1.12","hour_index":200,
            "countries":{"GER":{"politics":{"stability":0.9,"political_power":99.0,"ruling_party":"democratic"}}},
            "states":{"64":{"owner":"GER","controller":"SOV","category":"city","victory_points_total":5,
                "manpower_pool":{"total":900},"buildings":{"infrastructure":3,"arms_factory":2},
                "occupation":{"resistance":10.0,"compliance":50.0},"occupation_law":null,"resources":null}}
        });
        fs::write(
            gdir.join("000000000100.snapshot.json"),
            serde_json::to_string(&a).unwrap(),
        )
        .unwrap();
        fs::write(
            gdir.join("000000000200.snapshot.json"),
            serde_json::to_string(&b).unwrap(),
        )
        .unwrap();
        let index = json!({"game_id":"g","schema_version":"1","snapshots":[
            {"hour_index":100,"date":"1940.1.1.12","file":"000000000100.snapshot.json"},
            {"hour_index":200,"date":"1941.1.1.12","file":"000000000200.snapshot.json"}
        ]});
        fs::write(
            gdir.join("index.json"),
            serde_json::to_string(&index).unwrap(),
        )
        .unwrap();
        let store = Store::open(&dir);
        (dir, store, game)
    }

    /// THE deliverable-5d test: the LRU is immutable under re-bind. Scrub A→B→A
    /// through the REAL Timeline; the second A is a WARM hit (same Arc); the re-bind
    /// digest of A-after must equal A-before. The bind builds a fresh report from the
    /// shared snapshot, so nothing from the B visit can persist.
    #[test]
    fn lru_immutable_under_rebind_a_b_a() {
        let (dir, store, game) = tiny_store();
        let mut tl = Timeline::open(store, game, 4).unwrap();

        // A (hour 100) — cold.
        let fa1 = tl.seek_to(0).unwrap();
        assert_eq!(fa1.fetch, Fetch::Cold);
        let da1 = rebind_digest(&fa1.snapshot, "GER", "64", "Brandenburg");

        // B (hour 200) — cold. Binds against a DIFFERENT shared snapshot.
        let fb = tl.seek_to(1).unwrap();
        assert_eq!(fb.fetch, Fetch::Cold);
        let db = rebind_digest(&fb.snapshot, "GER", "64", "Brandenburg");
        assert_ne!(da1, db, "A and B must differ, else the test proves nothing");
        // the flag ideology actually changed (fascism -> democratic) — the cache-key trap.
        assert!(da1.contains("fascism") || da1.contains("\"fascism\""));
        assert!(db.contains("democratic"));

        // Back to A — WARM (same Arc).
        let fa2 = tl.seek_to(0).unwrap();
        assert_eq!(fa2.fetch, Fetch::Warm, "returning to A must be a cache hit");
        assert!(
            Arc::ptr_eq(&fa1.snapshot, &fa2.snapshot),
            "A revisit must be the SAME parse"
        );
        let da2 = rebind_digest(&fa2.snapshot, "GER", "64", "Brandenburg");

        assert_eq!(
            da1, da2,
            "A->B->A: the A re-bind carried a value from the B visit"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// The cached snapshot cannot be mutated while the cache holds a clone (the type
    /// guarantee underpinning the immutability), re-proved at the app layer.
    #[test]
    fn cached_snapshot_is_immutable_to_app() {
        let (dir, store, game) = tiny_store();
        let mut tl = Timeline::open(store, game, 4).unwrap();
        let mut f = tl.seek_to(0).unwrap();
        // refcount >= 2 (cache + this Frame) -> no &mut into the snapshot.
        assert!(
            Arc::get_mut(&mut f.snapshot).is_none(),
            "the app must not be able to mutate a cached snapshot"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

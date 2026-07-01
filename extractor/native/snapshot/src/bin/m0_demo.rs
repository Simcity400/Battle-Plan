//! M0 demo â€” prove the Python-store <-> Rust-reader boundary.
//!
//! Loads a real snapshot from the on-disk store and prints a country's scalars +
//! a state's buildings, distinguishing measured-zero / captured-null / never-captured.
//!
//! Usage: m0_demo [store_root] [game_id] [hour_index] [country_tag] [state_id]
//! Defaults target the committed demo frame (game "replay", 1941.4.13, GER, state 1).
//! The default store_root is RELATIVE ("replay_interface/replay/snapshots") â€” no personal path is
//! ever baked in; pass an explicit root to point elsewhere.

use serde_json::Value;
use snapshot::Store;
use std::env;

fn main() {
    let a: Vec<String> = env::args().collect();
    let root = a
        .get(1)
        .map(String::as_str)
        .unwrap_or("replay_interface/replay/snapshots")
        .to_string();
    let game = a.get(2).map(String::as_str).unwrap_or("replay").to_string();
    let hour: i64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(17005619);
    let tag = a.get(4).map(String::as_str).unwrap_or("GER").to_string();
    let sid = a.get(5).map(String::as_str).unwrap_or("1").to_string();

    let store = Store::open(&root);

    match store.index(&game) {
        Ok(idx) => println!(
            "index: game={} schema={:?} snapshots={}",
            idx.game_id,
            idx.schema_version,
            idx.snapshots.len()
        ),
        Err(e) => eprintln!("index error: {e}"),
    }

    let t = std::time::Instant::now();
    let snap = match store.load(&game, hour) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("load error ({game} @ {hour}): {e}");
            std::process::exit(1);
        }
    };
    let load_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!(
        "loaded game={} date={} hour={} in {:.1} ms  ({} countries / {} states)",
        snap.game_id.as_deref().unwrap_or("?"),
        snap.date.as_deref().unwrap_or("?"),
        snap.hour_index.unwrap_or(-1),
        load_ms,
        snap.countries().map(|m| m.len()).unwrap_or(0),
        snap.states().map(|m| m.len()).unwrap_or(0),
    );

    println!("-- country {tag} --");
    for p in [
        "politics.stability",
        "politics.war_support",
        "politics.political_power",
        "politics.ruling_party",
        "economy.factories.civ",
        "economy.factories.mil",
        "economy.fuel.current",
        "military.manpower.available",
    ] {
        let cell = match snap.country_path(&tag, p) {
            None => "<absent: never captured>".to_string(),
            Some(Value::Null) => "<null: not captured this frame>".to_string(),
            Some(v) => v.to_string(),
        };
        println!("  {p:34} = {cell}");
    }

    println!("-- state {sid} --");
    match snap.state(&sid) {
        None => println!("  <state absent>"),
        Some(st) => {
            let g = |k: &str| st.get(k).cloned().unwrap_or(Value::Null);
            println!(
                "  owner={} controller={} category={}",
                g("owner"),
                g("controller"),
                g("category")
            );
            match st.get("buildings").and_then(Value::as_object) {
                Some(b) => {
                    for (k, v) in b {
                        println!("  building {k:22} = {v}");
                    }
                }
                None => println!("  buildings: <absent>"),
            }
            println!("  victory_points_total = {}", g("victory_points_total"));
        }
    }

    // scrub-latency micro-bench: how fast can we re-load a full-world frame?
    let k = 20;
    let t = std::time::Instant::now();
    for _ in 0..k {
        let _ = store.load(&game, hour);
    }
    let avg = t.elapsed().as_secs_f64() * 1000.0 / k as f64;
    println!("scrub-load avg over {k} reloads: {avg:.2} ms/snapshot");
}

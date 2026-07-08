//! NAME-RE engine-baseline gate against the 63-label oracle (RE-TITLE).
//!
//! `tests/fixtures/mapname/engine_label_oracle.json` is the ANSWER KEY: 63
//! country-title labels decoded from the vanilla 1936 `mapname` vertex buffers
//! across the EU + SA + other captures, each with the engine's primary anchor
//! (x, y map px), baseline angle (deg), and arc length (map px). This gate is
//! broader than the frame4700 EU gate: it includes the deep-south CHILE label,
//! whose narrow-south anchor is the acid test for the engine OLS
//! dual-regression + longest-internal-owned-segment placement (`§6.3`,
//!
//! Captures validate, they never drive (`MAP_LABEL_LAW.md` §5): the law
//! recomputes placements from the bundle raster and must reproduce the engine
//! anchors. SKIPS when the untracked map bundle is absent.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use map_viewer::{compute_country_label_placements, LabelLawInputs, ProvinceKind};

struct Bundle {
    width: usize,
    height: usize,
    province_ids: Vec<u32>,
    province_to_state: BTreeMap<u32, u32>,
    owners: BTreeMap<u32, String>,
    impassable_states: BTreeSet<u32>,
    province_importance: BTreeMap<u32, u64>,
    province_type: BTreeMap<u32, ProvinceKind>,
}

fn bundle_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("out/map_bundle")
}

fn load_bundle(dir: &Path) -> Option<Bundle> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).ok()?).ok()?;
    let width = manifest["dimensions"]["width"].as_u64()? as usize;
    let height = manifest["dimensions"]["height"].as_u64()? as usize;
    let file = |key: &str, default: &str| -> String {
        manifest["files"][key]
            .as_str()
            .unwrap_or(default)
            .to_string()
    };
    let bytes = std::fs::read(dir.join(file("province_ids", "province_ids.r32"))).ok()?;
    if bytes.len() != width * height * 4 {
        return None;
    }
    let province_ids = bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let p2s: BTreeMap<String, u32> = serde_json::from_slice(
        &std::fs::read(dir.join(file("province_to_state", "province_to_state.json"))).ok()?,
    )
    .ok()?;
    let province_to_state = p2s
        .into_iter()
        .filter_map(|(k, v)| k.parse::<u32>().ok().map(|k| (k, v)))
        .collect();
    let own: BTreeMap<String, String> = serde_json::from_slice(
        &std::fs::read(dir.join(file("state_owner_1936", "state_owner_1936.json"))).ok()?,
    )
    .ok()?;
    let owners = own
        .into_iter()
        .filter_map(|(k, v)| k.parse::<u32>().ok().map(|k| (k, v)))
        .collect();
    let meta: BTreeMap<String, serde_json::Value> = serde_json::from_slice(
        &std::fs::read(dir.join(file("state_meta", "state_meta.json"))).ok()?,
    )
    .ok()?;
    let mut impassable_states = BTreeSet::new();
    let mut province_importance = BTreeMap::new();
    for (id, value) in &meta {
        let Ok(id) = id.parse::<u32>() else { continue };
        if value["impassable"].as_bool() == Some(true) {
            impassable_states.insert(id);
        }
        if let Some(vps) = value["victory_points"].as_object() {
            for (pid, weight) in vps {
                if let (Ok(pid), Some(w)) = (pid.parse::<u32>(), weight.as_u64()) {
                    province_importance.insert(pid, w);
                }
            }
        }
    }
    let pdefs: BTreeMap<String, serde_json::Value> = serde_json::from_slice(
        &std::fs::read(dir.join(file("province_definitions", "province_definitions.json"))).ok()?,
    )
    .ok()?;
    let mut province_type = BTreeMap::new();
    for (pid, def) in &pdefs {
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        let kind = def
            .get("province_type")
            .and_then(|t| t.as_str())
            .map(ProvinceKind::from_type_str)
            .unwrap_or(ProvinceKind::Land);
        province_type.insert(pid, kind);
    }
    Some(Bundle {
        width,
        height,
        province_ids,
        province_to_state,
        owners,
        impassable_states,
        province_importance,
        province_type,
    })
}

#[test]
fn oracle63_engine_baseline_reproduces() {
    let dir = bundle_dir();
    let Some(bundle) = load_bundle(&dir) else {
        eprintln!("SKIP oracle63 gate: no map bundle at {}", dir.display());
        return;
    };
    let inputs = LabelLawInputs {
        province_ids: &bundle.province_ids,
        width: bundle.width,
        height: bundle.height,
        province_to_state: &bundle.province_to_state,
        owners: &bundle.owners,
        impassable_states: &bundle.impassable_states,
        province_importance: &bundle.province_importance,
        province_type: &bundle.province_type,
    };
    let t0 = std::time::Instant::now();
    let placements = compute_country_label_placements(&inputs);
    let elapsed = t0.elapsed();
    assert!(!placements.is_empty(), "1936 bundle must place labels");

    let raw = include_str!("fixtures/mapname/engine_label_oracle.json");
    let v: serde_json::Value = serde_json::from_str(raw).expect("oracle parses");
    let rows = v["labels"].as_array().expect("labels array");

    let mut dists: Vec<f32> = Vec::new();
    let mut chile: Option<f32> = None;
    let mut worst: Vec<(String, f32)> = Vec::new();
    for r in rows {
        let tag = r["tag"].as_str().unwrap();
        let ex = r["anchor_x"].as_f64().unwrap() as f32;
        let ey = r["anchor_y"].as_f64().unwrap() as f32;
        // Nearest same-tag placement (any component: a country's primary label
        // may be a colony), measuring anchor accuracy not component identity.
        let best = placements
            .iter()
            .filter(|p| p.tag == tag)
            .map(|p| (p.anchor_x - ex).hypot(p.anchor_y - ey))
            .min_by(f32::total_cmp);
        let Some(d) = best else { continue };
        dists.push(d);
        worst.push((tag.to_string(), d));
        if tag == "CHL" {
            chile = Some(d);
        }
    }
    worst.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut sorted = dists.clone();
    sorted.sort_by(f32::total_cmp);
    let median = sorted[sorted.len() / 2];
    let p90 = sorted[(sorted.len() as f32 * 0.9) as usize];
    eprintln!(
        "oracle63: n={} median={:.1} p90={:.1} max={:.1}  CHILE={:.1}  compute={:?}",
        dists.len(),
        median,
        p90,
        sorted[sorted.len() - 1],
        chile.unwrap_or(-1.0),
        elapsed,
    );
    eprintln!("  worst 8: {:?}", &worst[..worst.len().min(8)]);

    // RE-TITLE port asserts (byte-faithful engine baseline, full raster). The
    // port gives median ~4.2, CHILE ~8.1; these thresholds have headroom over
    // that but are far tighter than the pre-port PCA/core law.
    assert!(
        dists.len() >= 60,
        "only {}/63 oracle anchors matched a same-tag placement",
        dists.len()
    );
    assert!(
        median < 8.0,
        "engine-baseline anchor median drifted: {median:.1} px (port baseline ~4.2)"
    );
    let chile = chile.expect("CHILE must place");
    assert!(
        chile < 15.0,
        "CHILE anchor drifted: {chile:.1} px — the deep-south thin-country acid \
         test (port baseline ~8.1; the stride-4 grid shattered it at >250)"
    );
    // A handful of tags are genuine component-selection residuals (the engine's
    // primary label sits on a colony/exclave our de-clutter drops, e.g. FRANCE
    // West Africa ~214, OMAN Musandam ~92); the p90 stays bounded well below
    // those to prove the bulk of the set is tight.
    assert!(
        p90 < 30.0,
        "engine-baseline anchor p90 drifted: {p90:.1} px (port baseline ~22)"
    );
}


use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use map_viewer::{
    compute_country_label_placements, CountryLabelPlacement, LabelLawInputs, ProvinceKind,
};

const CAP_PX: f32 = 55.0;

#[derive(Debug)]
struct KeyRow {
    tag: String,
    text: String,
    anchor_x: f32,
    anchor_y: f32,
    arc_w: f32,
    h_cap: f32,
    track_px: f32,
    w_nat_px: f32,
}

fn fixture_rows() -> Vec<KeyRow> {
    let raw = include_str!("fixtures/mapname/frame4700_labels.json");
    let v: serde_json::Value = serde_json::from_str(raw).expect("answer key parses");
    assert_eq!(
        v["schema"].as_str(),
        Some("battle-plan-frame4700-mapname-answer-key-v1")
    );
    assert_eq!(v["cap_px"].as_f64(), Some(f64::from(CAP_PX)));
    v["labels"]
        .as_array()
        .expect("labels array")
        .iter()
        .map(|r| KeyRow {
            tag: r["tag"].as_str().unwrap().to_string(),
            text: r["text"].as_str().unwrap().to_string(),
            anchor_x: r["anchor_x"].as_f64().unwrap() as f32,
            anchor_y: r["anchor_y"].as_f64().unwrap() as f32,
            arc_w: r["arc_w"].as_f64().unwrap() as f32,
            h_cap: r["h_cap"].as_f64().unwrap() as f32,
            track_px: r["track_px"].as_f64().unwrap() as f32,
            w_nat_px: r["w_nat_px"].as_f64().unwrap() as f32,
        })
        .collect()
}

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

fn join<'a>(
    rows: &'a [KeyRow],
    placements: &'a [CountryLabelPlacement],
) -> Vec<(&'a KeyRow, &'a CountryLabelPlacement)> {
    let mut cands: Vec<(f32, usize, usize)> = Vec::new();
    for (ri, row) in rows.iter().enumerate() {
        for (pi, p) in placements.iter().enumerate() {
            if p.tag != row.tag {
                continue;
            }
            let d = (p.anchor_x - row.anchor_x).hypot(p.anchor_y - row.anchor_y);
            if d <= (0.6 * p.half_length).max(25.0) {
                cands.push((d, ri, pi));
            }
        }
    }
    cands.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut used_r = BTreeSet::new();
    let mut used_p = BTreeSet::new();
    let mut pairs = Vec::new();
    for (_, ri, pi) in cands {
        if used_r.insert(ri) && used_p.insert(pi) {
            pairs.push((&rows[ri], &placements[pi]));
        }
    }
    pairs
}

#[test]
#[cfg_attr(not(gate_map_bundle), ignore = "the map bundle directory at map_viewer/out/map_bundle")]
fn frame4700_engine_label_sizes_reproduce() {
    let dir = replay_test_gates::require_existing_dir(bundle_dir());
    let bundle = load_bundle(&dir).expect("load map bundle");
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
    let placements = compute_country_label_placements(&inputs);
    assert!(!placements.is_empty(), "1936 bundle must place labels");

    let rows = fixture_rows();
    let pairs = join(&rows, &placements);
    assert!(
        pairs.len() >= 36,
        "placement regression: only {}/38 engine labels joined a same-tag \
         placement (anchor law drifted?)",
        pairs.len()
    );

    const LINE_HEIGHT: f32 = 84.0;
    let mut ratios: Vec<f32> = Vec::new();
    let mut capped_misses = Vec::new();
    for (row, p) in &pairs {
        let gs = p.engine_glyph_scale(row.w_nat_px, LINE_HEIGHT);
        let h_pred = gs.scale * CAP_PX;
        let ratio = h_pred / row.h_cap;
        ratios.push(ratio);
        let gaps = (row.text.chars().filter(|c| !c.is_whitespace()).count() - 1).max(1) as f32;
        let engine_fill = row.arc_w / row.w_nat_px * CAP_PX;
        if row.track_px * gaps / row.w_nat_px > 0.10 && row.h_cap < 0.8 * engine_fill {
            let cap_ratio = h_pred / row.h_cap;
            if !gs.cap_bound || !(0.7..=1.4).contains(&cap_ratio) {
                capped_misses.push(format!(
                    "{} '{}': cap_bound={} pred {:.2} vs engine {} (x{:.2})",
                    row.tag, row.text, gs.cap_bound, h_pred, row.h_cap, cap_ratio
                ));
            }
        }
    }
    assert!(
        capped_misses.is_empty(),
        "breadth-cap law missed letterspaced engine labels:\n{}",
        capped_misses.join("\n")
    );
    let within25 =
        ratios.iter().filter(|r| (0.8..1.25).contains(*r)).count() as f32 / ratios.len() as f32;
    let mut sorted = ratios.clone();
    sorted.sort_by(f32::total_cmp);
    let median = sorted[sorted.len() / 2];
    assert!(
        within25 >= 0.78,
        "size law degraded: {:.0}% of {} pairs within ±25% (fit baseline 89%)",
        within25 * 100.0,
        ratios.len()
    );
    assert!(
        (0.90..=1.10).contains(&median),
        "size law bias: median predicted/engine = {median:.3} (fit baseline 0.988)"
    );
}

#[test]
#[cfg_attr(not(gate_map_bundle), ignore = "the map bundle directory at map_viewer/out/map_bundle")]
fn frame4700_engine_anchors_reproduce() {
    let dir = replay_test_gates::require_existing_dir(bundle_dir());
    let bundle = load_bundle(&dir).expect("load map bundle");
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
    let placements = compute_country_label_placements(&inputs);

    let raw = include_str!("fixtures/mapname/frame4700_placement_gate.json");
    let v: serde_json::Value = serde_json::from_str(raw).expect("placement gate parses");
    assert_eq!(
        v["schema"].as_str(),
        Some("battle-plan-frame4700-placement-gate-v1")
    );
    let rows = v["labels"].as_array().expect("labels array");

    let mut dists: Vec<f32> = Vec::new();
    let mut norway: Option<f32> = None;
    for r in rows {
        let tag = r["tag"].as_str().unwrap();
        let ex = r["anchor_x"].as_f64().unwrap() as f32;
        let ey = r["anchor_y"].as_f64().unwrap() as f32;
        let best = placements
            .iter()
            .filter(|p| p.tag == tag)
            .map(|p| (p.anchor_x - ex).hypot(p.anchor_y - ey))
            .min_by(f32::total_cmp);
        let Some(d) = best else { continue };
        dists.push(d);
        if tag == "NOR" {
            norway = Some(d);
        }
    }
    assert!(
        dists.len() >= 17,
        "only {}/19 engine anchors matched a same-tag placement",
        dists.len()
    );
    let nor = norway.expect("NORWAY must place");
    assert!(
        nor <= 30.0,
        "NAME3 regression: NORWAY anchor {nor:.0} px from the engine (raw-centroid law was 122; core-anchor fix ~9)"
    );
    let mut sorted = dists.clone();
    sorted.sort_by(f32::total_cmp);
    let median = sorted[sorted.len() / 2];
    assert!(
        median <= 40.0,
        "NAME3 anchor law degraded: median engine-anchor distance {median:.0} px over {} tags",
        dists.len()
    );
}

//! NAME2 cross-session size gate (audit F-024 confrontation, 2026-07-11).
//!
//! A BLIND dual-derivation session (rounds 2-3, `rederive-titlesize2/3`)
//! byte-decoded eight country labels from `postfx_frame162662` — a capture
//! session and camera height (1060.04) entirely disjoint from the frame4700
//! oracle the law was fitted on — and measured their world cap heights. Its
//! free two-coefficient fits over ITS OWN ray-reconstructed drivers left an
//! 8.54% floor, which is consistent with (not in excess of) this law's
//! long-documented accuracy envelope. W11B subsequently traced the engine
//! breadth numerator to the full province-area raster count in
//! instruction skeleton only, not DAT values or upstream/helper semantics. This
//! independent research gate prints every joined per-label
//! ratio while retaining the established aggregate band.
//!
//! This gate closes the confrontation: OUR pipeline (bundle raster -> OLS
//! owned-segment arc -> `engine_glyph_scale`) must reproduce the session's
//! independently measured heights within the SAME envelope as frame4700
//! (every joined label within +/-25%, median within [0.90, 1.10]). A future
//! drift of the size law now trips against two independent capture sessions.
//!
//! SKIPS (visible) when the untracked map bundle is absent.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use map_viewer::{
    compute_country_label_placements, CountryLabelPlacement, LabelLawInputs, ProvinceKind,
};

const CAP_PX: f32 = 55.0;
const LINE_HEIGHT: f32 = 84.0;

struct KeyRow {
    tag: String,
    text: String,
    h_cap: f32,
    arc_w: f32,
    w_nat_px: f32,
    anchor_x: f32,
    anchor_y: f32,
}

fn fixture_rows() -> Vec<KeyRow> {
    let raw = include_str!("fixtures/mapname/frame162662_labels.json");
    let v: serde_json::Value = serde_json::from_str(raw).expect("answer key parses");
    assert_eq!(
        v["schema"].as_str(),
        Some("battle-plan-frame162662-mapname-answer-key-v1")
    );
    assert_eq!(v["cap_px"].as_f64(), Some(f64::from(CAP_PX)));
    v["labels"]
        .as_array()
        .expect("labels array")
        .iter()
        .map(|r| KeyRow {
            tag: r["tag"].as_str().unwrap().to_string(),
            text: r["text"].as_str().unwrap().to_string(),
            h_cap: r["h_cap"].as_f64().unwrap() as f32,
            arc_w: r["arc_w"].as_f64().unwrap() as f32,
            w_nat_px: r["w_nat_px"].as_f64().unwrap() as f32,
            anchor_x: r["anchor_x"].as_f64().unwrap() as f32,
            anchor_y: r["anchor_y"].as_f64().unwrap() as f32,
        })
        .collect()
}

/// Bundle reader duplicated from `frame4700_label_size_gate.rs` (integration
/// tests cannot link bins).
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

/// Geographic instance join (frame4700 convention): the session's label
/// anchors were extracted from its cited glyph blobs, so each key row joins
/// the nearest same-tag placement within `max(25, 0.6 x half_length)` map px
/// — Portugal's metropole label joins Iberia, never a colonial component.
fn matching_instance<'a>(
    placements: &'a [CountryLabelPlacement],
    row: &KeyRow,
) -> Option<&'a CountryLabelPlacement> {
    placements
        .iter()
        .filter(|p| p.tag == row.tag)
        .filter(|p| {
            let d = (p.anchor_x - row.anchor_x).hypot(p.anchor_y - row.anchor_y);
            d <= (0.6 * p.half_length).max(25.0)
        })
        .min_by(|a, b| {
            let da = (a.anchor_x - row.anchor_x).hypot(a.anchor_y - row.anchor_y);
            let db = (b.anchor_x - row.anchor_x).hypot(b.anchor_y - row.anchor_y);
            da.total_cmp(&db)
        })
}

#[test]
#[cfg_attr(not(gate_map_bundle), ignore = "the map bundle directory at map_viewer/out/map_bundle")]
fn frame162662_cross_session_label_sizes_reproduce() {
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

    let mut ratios: Vec<f32> = Vec::new();
    let mut misses = Vec::new();
    for row in fixture_rows() {
        let Some(p) = matching_instance(&placements, &row) else {
            misses.push(format!("{}: no placement", row.tag));
            continue;
        };
        let gs = p.engine_glyph_scale(row.w_nat_px, LINE_HEIGHT);
        let h_pred = gs.scale * CAP_PX;
        let ratio = h_pred / row.h_cap;
        eprintln!(
            "frame162662 {} '{}': ratio={ratio:.3} pred={h_pred:.3} engine={:.3} cap_bound={}",
            row.tag, row.text, row.h_cap, gs.cap_bound
        );
        ratios.push(ratio);
        if !(0.8..1.25).contains(&ratio) {
            misses.push(format!(
                "{} '{}': pred {:.3} vs measured {:.3} (x{:.3}; our arc {:.1} vs decoded {:.1})",
                row.tag,
                row.text,
                h_pred,
                row.h_cap,
                ratio,
                2.0 * p.half_length,
                row.arc_w
            ));
        }
    }
    assert!(
        misses.is_empty(),
        "cross-session size law misses (frame4700-envelope):\n{}",
        misses.join("\n")
    );
    let mut sorted = ratios.clone();
    sorted.sort_by(f32::total_cmp);
    let median = sorted[sorted.len() / 2];
    assert!(
        (0.90..=1.10).contains(&median),
        "cross-session size bias: median predicted/measured = {median:.3}"
    );
}

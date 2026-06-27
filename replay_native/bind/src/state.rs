//! STATE-PANEL BINDING (M5) — the declarative element→snapshot-path→formatter→
//! fidelity map for `province_state.json`, realising the STATE PANEL section of
//! `replay_ui/BINDING_MAP_topbar_state.md` against a real selected state.
//!
//! Like the topbar spec ([`crate::TopbarSpec`]) this is the no-fabrication choke
//! point for the state view: every field is resolved from a [`snapshot::Snapshot`]
//! state object, formatted, and any ABSENT / NULL / `unavailable`-badged element
//! renders the not-captured glyph ("—"), never a fabricated `0`. The buildings and
//! resources GRIDBOXES are bound here too: their bound array is extracted as an
//! ordered, icon-mapped row list (the row-factory instantiates one row per entry).
//!
//! Codex-confirmed corrections folded in (BINDING_MAP §2a):
//!   * state-panel population = `manpower_pool.total` (NOT `.available`),
//!   * `victory_points_total` is the exact VP int,
//!   * resistance/compliance are 0..100 percents (bar fraction = value/100),
//!   * `resources`, `occupation_law`, `is_claimed_by` are always-null in the
//!     current snapshots → `unavailable` (render "—"), never substituted.

use crate::format::{self, NOT_CAPTURED};
use crate::{classify, Fidelity, ValueState};
use serde_json::Value;
use snapshot::Snapshot;

/// A bound scalar field of the state panel (mirrors the topbar's [`BoundField`]).
#[derive(Debug, Clone)]
pub struct StateField {
    pub element: String,
    pub value: String,
    pub fidelity: Fidelity,
    pub not_captured: bool,
}

/// One building row for a gridbox row-factory: the building key, its level, and
/// the sprite frame (into `GFX_buildings_strip` / `GFX_resources_strip`).
#[derive(Debug, Clone)]
pub struct BuildingRow {
    pub key: String,
    /// Display level string (a measured int, or "—" if somehow non-numeric).
    pub level: String,
    /// 0-based frame index into the building/resource strip.
    pub frame: u32,
    pub fidelity: Fidelity,
}

/// The fully bound state panel: the scalar fields, the building rows (state +
/// province scoped, ordered), the occupation bars, and the flags.
#[derive(Debug, Clone, Default)]
pub struct StateBindReport {
    pub state_id: String,
    pub fields: Vec<StateField>,
    /// State-scoped buildings (industrial_complex, arms_factory, …) as rows.
    pub state_buildings: Vec<BuildingRow>,
    /// Province-scoped buildings folded to the state (rail_way, naval_base, …).
    pub province_buildings: Vec<BuildingRow>,
    /// Resource rows — EMPTY + unavailable in current data (resources == null).
    pub resource_rows: Vec<BuildingRow>,
    pub owner_tag: Option<String>,
    pub controller_tag: Option<String>,
    /// occupation present → Some((resistance%, compliance%)); else None.
    pub occupation: Option<Occupation>,
}

#[derive(Debug, Clone, Copy)]
pub struct Occupation {
    /// 0..100 percent.
    pub resistance: f64,
    pub compliance: f64,
}

impl StateBindReport {
    pub fn get(&self, element: &str) -> Option<&StateField> {
        self.fields.iter().find(|f| f.element == element)
    }
}

/// The canonical ORDER + icon-frame map for state-scoped buildings. The frame
/// index is into `GFX_buildings_strip` (31 frames). The order is the in-game
/// state-view order; the frames are the game's building strip layout (icon-mapped,
/// a static map per the binding map, NOT data).
const STATE_BUILDINGS: &[(&str, u32)] = &[
    ("infrastructure", 0),
    ("industrial_complex", 1),
    ("arms_factory", 2),
    ("dockyard", 3),
    ("air_base", 5),
    ("anti_air_building", 6),
    ("radar_station", 7),
    ("rocket_site", 8),
    ("nuclear_reactor", 9),
    ("synthetic_refinery", 10),
    ("fuel_silo", 11),
];

/// Province-scoped buildings (folded to the state) order + frame map.
const PROVINCE_BUILDINGS: &[(&str, u32)] = &[
    ("naval_base", 4),
    ("rail_way", 12),
    ("supply_node", 13),
    ("naval_supply_hub", 14),
    ("coastal_bunker", 15),
    ("bunker", 16),
    ("air_facility", 17),
    ("land_facility", 18),
    ("floating_harbor", 19),
];

/// Resource order + frame map (into `GFX_resources_strip`). Used only when the
/// `resources` domain is populated; it is `null` in current data → no rows.
const RESOURCES: &[(&str, u32)] = &[
    ("aluminium", 0),
    ("chromium", 1),
    ("oil", 2),
    ("rubber", 3),
    ("steel", 4),
    ("tungsten", 5),
];

/// Extract ordered building rows from a state's `buildings` / `province_buildings`
/// object. Only PRESENT keys with a measured non-zero level become rows (the game
/// shows only built buildings). A measured level of `0` is omitted (no building);
/// this is NOT a fabrication choice — a `0`-level building is simply not built.
fn building_rows(obj: Option<&Value>, order: &[(&str, u32)]) -> Vec<BuildingRow> {
    let mut rows = Vec::new();
    let map = match obj.and_then(Value::as_object) {
        Some(m) => m,
        None => return rows,
    };
    for (key, frame) in order {
        if let Some(v) = map.get(*key) {
            // a measured numeric level; skip null/non-number and 0 (not built).
            if let Some(n) = v.as_f64() {
                if n > 0.0 {
                    rows.push(BuildingRow {
                        key: key.to_string(),
                        level: format::int(v),
                        frame: *frame,
                        fidelity: Fidelity::Exact,
                    });
                }
            }
        }
    }
    rows
}

/// Build the resource rows. `resources` is always `null` in the current snapshot
/// generator (BINDING_MAP §2a) → an EMPTY list badged unavailable (never the
/// country-level aggregate substituted in). Present here so the gridbox renders an
/// empty/"—" state honestly when the domain is unfed.
fn resource_rows(obj: Option<&Value>) -> Vec<BuildingRow> {
    let mut rows = Vec::new();
    let map = match obj.and_then(Value::as_object) {
        Some(m) => m,
        None => return rows, // null/absent → no rows (unavailable)
    };
    for (key, frame) in RESOURCES {
        if let Some(v) = map.get(*key) {
            if let Some(n) = v.as_f64() {
                if n > 0.0 {
                    rows.push(BuildingRow {
                        key: key.to_string(),
                        level: format::int(v),
                        frame: *frame,
                        fidelity: Fidelity::Exact,
                    });
                }
            }
        }
    }
    rows
}

/// A scalar state field: classify the path, run a formatter on a measured value,
/// else not-captured. `unavailable` short-circuits to "—" (never consults data).
fn scalar(
    snap: &Snapshot,
    sid: &str,
    element: &str,
    path: &str,
    fmt: fn(&Value) -> String,
    fidelity: Fidelity,
) -> StateField {
    if fidelity == Fidelity::Unavailable {
        return StateField {
            element: element.to_string(),
            value: NOT_CAPTURED.to_string(),
            fidelity,
            not_captured: true,
        };
    }
    let cell = snap.state_path(sid, path);
    let (value, nc) = match classify(cell) {
        ValueState::Measured => {
            let s = fmt(cell.expect("measured implies Some"));
            let nc = s == NOT_CAPTURED;
            (s, nc)
        }
        _ => (NOT_CAPTURED.to_string(), true),
    };
    StateField {
        element: element.to_string(),
        value,
        fidelity,
        not_captured: nc,
    }
}

/// Resolve a state's `<TAG>` flag field (owner / controller). A measured string
/// tag → the tag (the demo resolves the flag image); null/absent → "—".
fn flag_field(snap: &Snapshot, sid: &str, element: &str, path: &str) -> (StateField, Option<String>) {
    let cell = snap.state_path(sid, path);
    match cell.and_then(Value::as_str) {
        Some(tag) => (
            StateField {
                element: element.to_string(),
                value: tag.to_string(),
                fidelity: Fidelity::Exact,
                not_captured: false,
            },
            Some(tag.to_string()),
        ),
        None => (
            StateField {
                element: element.to_string(),
                value: NOT_CAPTURED.to_string(),
                fidelity: Fidelity::Exact,
                not_captured: true,
            },
            None,
        ),
    }
}

/// BIND the state panel for a selected state id. `state_name` is the resolved
/// display name (from static map meta + loc, supplied by the caller because it is
/// not in the snapshot). Returns a full [`StateBindReport`]; the demo paints flags
/// + instantiates the gridbox rows from it.
pub fn bind_state(snap: &Snapshot, sid: &str, state_name: &str) -> StateBindReport {
    let mut report = StateBindReport {
        state_id: sid.to_string(),
        ..Default::default()
    };

    // --- state name (exact: from static map meta + loc, NOT the snapshot). ---
    report.fields.push(StateField {
        element: "state_name".to_string(),
        value: state_name.to_string(),
        fidelity: Fidelity::Exact,
        not_captured: state_name.is_empty(),
    });

    // --- category (exact). ---
    report.fields.push(scalar(
        snap, sid, "category", "category", format::category, Fidelity::Exact,
    ));

    // --- victory points (exact int). ---
    report.fields.push(scalar(
        snap, sid, "victory_points", "victory_points_total", format::int, Fidelity::Exact,
    ));

    // --- manpower: state population = manpower_pool.total (Codex correction). ---
    report.fields.push(scalar(
        snap, sid, "manpower", "manpower_pool.total", format::manpower, Fidelity::Exact,
    ));

    // --- owner / controller flags (exact). ---
    let (owner_f, owner_tag) = flag_field(snap, sid, "state_owners", "owner");
    report.fields.push(owner_f);
    report.owner_tag = owner_tag;
    let (ctrl_f, ctrl_tag) = flag_field(snap, sid, "controller_flag", "controller");
    report.fields.push(ctrl_f);
    report.controller_tag = ctrl_tag;

    // --- occupation: resistance / compliance (exact; 0..100 percent). ---
    let occ = snap.state_path(sid, "occupation");
    match occ {
        Some(Value::Object(o)) => {
            let res = o.get("resistance").and_then(Value::as_f64);
            let comp = o.get("compliance").and_then(Value::as_f64);
            // percent_100 prints a 0..100 value directly with a % sign.
            report.fields.push(StateField {
                element: "state_resistance_status_container.progress_text".to_string(),
                value: res.map(format::percent_100).unwrap_or_else(|| NOT_CAPTURED.to_string()),
                fidelity: Fidelity::Exact,
                not_captured: res.is_none(),
            });
            report.fields.push(StateField {
                element: "state_compliance_status_container.progress_text".to_string(),
                value: comp.map(format::percent_100).unwrap_or_else(|| NOT_CAPTURED.to_string()),
                fidelity: Fidelity::Exact,
                not_captured: comp.is_none(),
            });
            if let (Some(r), Some(c)) = (res, comp) {
                report.occupation = Some(Occupation {
                    resistance: r,
                    compliance: c,
                });
            }
        }
        _ => {
            // no occupation (core/own territory) → bars not shown; fields "—".
            report.fields.push(StateField {
                element: "state_resistance_status_container.progress_text".to_string(),
                value: NOT_CAPTURED.to_string(),
                fidelity: Fidelity::Exact,
                not_captured: true,
            });
            report.fields.push(StateField {
                element: "state_compliance_status_container.progress_text".to_string(),
                value: NOT_CAPTURED.to_string(),
                fidelity: Fidelity::Exact,
                not_captured: true,
            });
        }
    }

    // --- occupation_law: always null in current data → unavailable ("—"). ---
    report.fields.push(scalar(
        snap, sid, "select_law_icon", "occupation_law", format::int, Fidelity::Unavailable,
    ));

    // --- energy_text: no per-state energy field → unavailable. ---
    report.fields.push(scalar(
        snap, sid, "energy_text", "energy", format::int, Fidelity::Unavailable,
    ));

    // --- buildings gridboxes (row sources). ---
    report.state_buildings = building_rows(snap.state_path(sid, "buildings"), STATE_BUILDINGS);
    report.province_buildings =
        building_rows(snap.state_path(sid, "province_buildings"), PROVINCE_BUILDINGS);

    // --- resources gridbox: always-null domain → empty + unavailable. ---
    report.resource_rows = resource_rows(snap.state_path(sid, "resources"));

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn berlin_snapshot() -> Snapshot {
        // a faithful fragment of the real state 64 (Brandenburg) + an occupied one.
        let j = json!({
            "date": "1941.4.13.11",
            "states": {
                "64": {
                    "owner": "GER", "controller": "GER", "category": "megalopolis",
                    "victory_points_total": 56,
                    "manpower_pool": {"available": 331620, "locked": 6883615, "total": 7265029},
                    "buildings": {
                        "infrastructure": 5, "arms_factory": 10, "industrial_complex": 11,
                        "anti_air_building": 2, "air_base": 6, "synthetic_refinery": 3,
                        "fuel_silo": 2, "dockyard": 0
                    },
                    "province_buildings": {"rail_way": 5, "supply_node": 1, "land_facility": 1},
                    "occupation": null, "occupation_law": null, "resources": null
                },
                "1": {
                    "owner": "ITA", "controller": "ITA", "category": "town",
                    "victory_points_total": 1,
                    "manpower_pool": {"available": 667, "total": 322900},
                    "buildings": {"infrastructure": 2},
                    "occupation": {"resistance": 4.525, "compliance": 97.06527},
                    "occupation_law": null, "resources": null
                }
            }
        });
        serde_json::from_value(j).unwrap()
    }

    #[test]
    fn binds_real_berlin_values() {
        let snap = berlin_snapshot();
        let r = bind_state(&snap, "64", "Brandenburg");
        assert_eq!(r.get("state_name").unwrap().value, "Brandenburg");
        assert_eq!(r.get("category").unwrap().value, "megalopolis");
        assert_eq!(r.get("victory_points").unwrap().value, "56");
        // Codex: population = manpower_pool.total, not .available.
        assert_eq!(r.get("manpower").unwrap().value, "7.27M");
        assert_eq!(r.get("state_owners").unwrap().value, "GER");
        assert_eq!(r.get("controller_flag").unwrap().value, "GER");
        assert_eq!(r.owner_tag.as_deref(), Some("GER"));
    }

    #[test]
    fn building_rows_skip_zero_and_keep_order() {
        let snap = berlin_snapshot();
        let r = bind_state(&snap, "64", "Brandenburg");
        // 7 built state buildings (dockyard=0 is omitted, not fabricated).
        let keys: Vec<&str> = r.state_buildings.iter().map(|b| b.key.as_str()).collect();
        assert!(keys.contains(&"industrial_complex"));
        assert!(keys.contains(&"arms_factory"));
        assert!(!keys.contains(&"dockyard")); // level 0 → not built → no row
        // canonical order: infrastructure before industrial_complex before arms.
        let pos = |k: &str| keys.iter().position(|x| *x == k).unwrap();
        assert!(pos("infrastructure") < pos("industrial_complex"));
        assert!(pos("industrial_complex") < pos("arms_factory"));
        // levels are the measured ints.
        let ic = r.state_buildings.iter().find(|b| b.key == "industrial_complex").unwrap();
        assert_eq!(ic.level, "11");
        assert_eq!(ic.frame, 1);
        // 7 state buildings overflow a 6-slot box (proves the overflow source).
        assert_eq!(r.state_buildings.len(), 7);
    }

    #[test]
    fn occupied_state_binds_resistance_compliance() {
        let snap = berlin_snapshot();
        let r = bind_state(&snap, "1", "Aosta");
        assert!(r.occupation.is_some());
        let occ = r.occupation.unwrap();
        assert!((occ.resistance - 4.525).abs() < 1e-6);
        assert!((occ.compliance - 97.06527).abs() < 1e-6);
        // the text fields show the rounded percents.
        assert_eq!(
            r.get("state_resistance_status_container.progress_text").unwrap().value,
            "5%"
        );
        assert_eq!(
            r.get("state_compliance_status_container.progress_text").unwrap().value,
            "97%"
        );
    }

    #[test]
    fn unavailable_fields_render_dash_never_zero() {
        let snap = berlin_snapshot();
        let r = bind_state(&snap, "64", "Brandenburg");
        // occupation_law (null), energy (no field) → "—", never "0".
        for el in ["select_law_icon", "energy_text"] {
            let f = r.get(el).unwrap();
            assert_eq!(f.value, NOT_CAPTURED, "{el} fabricated a value");
            assert!(f.not_captured);
            assert_ne!(f.value, "0");
        }
        // resources gridbox: empty (null domain), never substituted with country data.
        assert!(r.resource_rows.is_empty());
        // a non-occupied state shows "—" for resistance/compliance (not 0%).
        assert_eq!(
            r.get("state_resistance_status_container.progress_text").unwrap().value,
            NOT_CAPTURED
        );
    }

    #[test]
    fn measured_zero_vp_is_shown_not_dashed() {
        // a state with a measured 0 VP must show "0", proving "—" = not-captured.
        let j = json!({"states":{"9":{"owner":"X","victory_points_total":0,
            "manpower_pool":{"total":0},"category":"rural"}}});
        let snap: Snapshot = serde_json::from_value(j).unwrap();
        let r = bind_state(&snap, "9", "Nowhere");
        assert_eq!(r.get("victory_points").unwrap().value, "0");
        assert_eq!(r.get("manpower").unwrap().value, "0");
    }
}

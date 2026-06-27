//! APPLY layer — substitute bound snapshot values into a `gui_runtime`
//! scene-graph. The single no-fabrication choke point lives here:
//! [`resolve_text`] returns `format::NOT_CAPTURED` ("—") for ANY binding that is
//! `Unavailable`, or whose path resolves to absent/null/non-scalar — a formatter
//! runs ONLY on a measured value, so a fabricated `0` can never be produced.

use crate::format::{self, NOT_CAPTURED};
use crate::{classify, Fidelity, Formatter, TextBinding, TopbarSpec, ValueState};
use gui_runtime::scene::{ElementKind, Node};
use serde_json::Value;
use snapshot::Snapshot;

/// What a single text binding produced, for the demo's verifiable report.
#[derive(Debug, Clone)]
pub struct BoundField {
    pub element: String,
    pub value: String,
    pub fidelity: Fidelity,
    /// True when the field rendered the not-captured glyph (absent/null/unavailable).
    pub not_captured: bool,
}

/// The result of applying the topbar spec: per-field outcomes + the flag's
/// TAG/ideology (the demo resolves the flag image separately, via `FlagResolver`).
#[derive(Debug, Clone, Default)]
pub struct BindReport {
    pub fields: Vec<BoundField>,
    /// The TAG bound (for the flag) and the ruling-party ideology, if measured.
    pub flag_tag: Option<String>,
    pub flag_ideology: Option<String>,
}

impl BindReport {
    pub fn get(&self, element: &str) -> Option<&BoundField> {
        self.fields.iter().find(|f| f.element == element)
    }
}

/// Compute the display string for ONE text binding against the snapshot/country.
/// This is the no-fabrication boundary — see module docs.
pub fn resolve_text(b: &TextBinding, snap: &Snapshot, tag: &str) -> (String, bool) {
    // Hard rule: an `unavailable` binding NEVER consults a value — it is, by the
    // map's contract, not-captured. This makes a fabricated 0 structurally
    // impossible for these elements regardless of what the path holds.
    if b.fidelity == Fidelity::Unavailable {
        return (NOT_CAPTURED.to_string(), true);
    }

    match b.formatter {
        // Special: factories read three independent paths under economy.factories.
        Formatter::FactoryTriple => {
            let civ = snap.country_path(tag, "economy.factories.civ");
            let mil = snap.country_path(tag, "economy.factories.mil");
            let dock = snap.country_path(tag, "economy.factories.dockyard");
            let s = format::factory_triple(measured(civ), measured(mil), measured(dock));
            // not-captured iff EVERY slot is not-captured (all "—").
            let nc = s.chars().all(|c| c == '—' || c == '/');
            (s, nc)
        }
        // Special: the clock reads the top-level snapshot date, not a country path.
        Formatter::Clock => match snap.date.as_deref() {
            Some(d) => {
                let s = format::clock(d);
                let nc = s == NOT_CAPTURED;
                (s, nc)
            }
            None => (NOT_CAPTURED.to_string(), true),
        },
        // Special (derived): convoys = sum of military.convoys[].amount (a list).
        Formatter::Int if b.element == "convoys_count" => sum_convoys(snap, tag),
        // The generic scalar path: classify -> measured -> formatter, else "—".
        f => {
            let path = match b.path {
                Some(p) => p,
                None => return (NOT_CAPTURED.to_string(), true),
            };
            let cell = snap.country_path(tag, path);
            match classify(cell) {
                ValueState::Measured => {
                    let v = cell.expect("measured implies Some");
                    let s = run_scalar(f, v);
                    // run_scalar only fabricates "—" on a type mismatch.
                    let nc = s == NOT_CAPTURED;
                    (s, nc)
                }
                // absent / null / non-scalar -> not-captured, never a 0.
                _ => (NOT_CAPTURED.to_string(), true),
            }
        }
    }
}

/// Return `Some(v)` only if `v` is a measured number (for triple slots).
fn measured(opt: Option<&Value>) -> Option<&Value> {
    match opt {
        Some(v) if v.is_number() => Some(v),
        _ => None,
    }
}

/// Run a scalar formatter on a measured value.
fn run_scalar(f: Formatter, v: &Value) -> String {
    match f {
        Formatter::Percent => format::percent(v),
        Formatter::UnsignedInt => format::unsigned_int(v),
        Formatter::Int => format::int(v),
        Formatter::Manpower => format::manpower(v),
        Formatter::Fuel => format::fuel(v),
        // The specials are handled before this is reached.
        Formatter::FactoryTriple | Formatter::Clock => NOT_CAPTURED.to_string(),
    }
}

/// Derived: sum `military.convoys[].amount`. The list itself is a measured
/// structure; an ABSENT/NULL `convoys` is not-captured (not a fabricated 0).
fn sum_convoys(snap: &Snapshot, tag: &str) -> (String, bool) {
    match snap.country_path(tag, "military.convoys") {
        Some(Value::Array(items)) => {
            let mut total = 0.0_f64;
            for it in items {
                if let Some(a) = it.get("amount").and_then(Value::as_f64) {
                    total += a;
                }
            }
            (format!("{}", total.round() as i64), false)
        }
        _ => (NOT_CAPTURED.to_string(), true),
    }
}

/// Apply the topbar spec to a scene-graph IN PLACE: substitute each bound text
/// element's resolved value, and record the flag TAG/ideology. Returns a
/// [`BindReport`] for verification. The flag IMAGE is resolved + uploaded by the
/// caller (it needs the renderer); here we only record TAG + ideology and leave
/// the `player_flag` chrome for the caller to repaint.
pub fn apply_topbar(
    roots: &mut [Node],
    spec: &TopbarSpec,
    snap: &Snapshot,
    tag: &str,
) -> BindReport {
    let mut report = BindReport {
        flag_tag: Some(tag.to_string()),
        ..Default::default()
    };

    // Ruling party drives the flag ideology; a measured string only.
    report.flag_ideology = snap
        .country_path(tag, "politics.ruling_party")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    for b in &spec.texts {
        let (value, not_captured) = resolve_text(b, snap, tag);
        for root in roots.iter_mut() {
            set_element_text(root, b.element, &value);
        }
        report.fields.push(BoundField {
            element: b.element.to_string(),
            value,
            fidelity: b.fidelity,
            not_captured,
        });
    }

    report
}

/// Set the resolved display text of the first element named `name` in the tree.
/// Works for text boxes AND button captions. Returns true if found.
pub fn set_element_text(node: &mut Node, name: &str, text: &str) -> bool {
    if node.name == name {
        match &mut node.kind {
            ElementKind::Text(t) => {
                t.text = text.to_string();
                return true;
            }
            ElementKind::Button { text: Some(t), .. } => {
                t.text = text.to_string();
                return true;
            }
            _ => { /* fall through: a non-text element by this name */ }
        }
    }
    for child in node.children.iter_mut() {
        if set_element_text(child, name, text) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ger_snapshot() -> Snapshot {
        // A minimal but faithful GER fragment mirroring the real snapshot.
        let j = json!({
            "date": "1941.4.13.11",
            "countries": {
                "GER": {
                    "politics": {
                        "stability": 0.75, "war_support": 0.85,
                        "political_power": 247.3735, "command_power": 182.36,
                        "ruling_party": "fascism"
                    },
                    "economy": {
                        "factories": {"civ": 110, "mil": 131, "dockyard": 16, "under_construction": 12},
                        "fuel": {"current": 849000.0, "max": 849000.0}
                    },
                    "military": {
                        "manpower": {"available": 525000},
                        "xp": {},
                        "convoys": [
                            {"archetype": "convoy_1", "amount": 1147.0},
                            {"archetype": "convoy_1", "amount": 17.0}
                        ]
                    }
                }
            }
        });
        serde_json::from_value(j).unwrap()
    }

    #[test]
    fn binds_real_ger_values() {
        let snap = ger_snapshot();
        let spec = TopbarSpec::topbar();
        let get = |el: &str| {
            let b = spec.texts.iter().find(|b| b.element == el).unwrap();
            resolve_text(b, &snap, "GER").0
        };
        assert_eq!(get("pol_power"), "247");
        assert_eq!(get("command_power_value"), "182");
        assert_eq!(get("stability"), "75%");
        assert_eq!(get("war_support"), "85%");
        assert_eq!(get("manpower"), "525.00K");
        assert_eq!(get("fuel_value"), "849.0K");
        assert_eq!(get("industrial_capacity"), "110/131/16");
        assert_eq!(get("DateText"), "11:00 13, April 1941");
        assert_eq!(get("convoys_count"), "1164"); // 1147 + 17
    }

    #[test]
    fn unavailable_never_zero() {
        let snap = ger_snapshot();
        let spec = TopbarSpec::topbar();
        // Every Unavailable binding renders the em dash, NEVER "0".
        for b in spec
            .texts
            .iter()
            .filter(|b| b.fidelity == Fidelity::Unavailable)
        {
            let (s, nc) = resolve_text(b, &snap, "GER");
            assert_eq!(s, NOT_CAPTURED, "element {} fabricated a value", b.element);
            assert!(nc, "element {} not flagged not-captured", b.element);
            assert_ne!(s, "0");
        }
    }

    #[test]
    fn empty_xp_object_is_not_captured_not_zero() {
        // military.xp = {} -> xp.army path is ABSENT -> not captured (not 0),
        // even though the binding is also fidelity-gated. Prove the path layer too.
        let snap = ger_snapshot();
        assert_eq!(
            classify(snap.country_path("GER", "military.xp.army")),
            ValueState::Absent
        );
    }

    #[test]
    fn measured_zero_is_shown_not_dashed() {
        // A measured 0 must show as "0", proving "—" is reserved for not-captured.
        let j = json!({"date":"1936.1.1.12","countries":{"X":{"politics":{"stability":0.0,"political_power":0.0}}}});
        let snap: Snapshot = serde_json::from_value(j).unwrap();
        let spec = TopbarSpec::topbar();
        let stab = spec
            .texts
            .iter()
            .find(|b| b.element == "stability")
            .unwrap();
        let pp = spec
            .texts
            .iter()
            .find(|b| b.element == "pol_power")
            .unwrap();
        assert_eq!(resolve_text(stab, &snap, "X"), ("0%".to_string(), false));
        assert_eq!(resolve_text(pp, &snap, "X"), ("0".to_string(), false));
    }

    #[test]
    fn applies_into_scene_text() {
        use gui_runtime::scene::{Rect, TextAlign, TextSpec};
        let mut root = Node {
            name: "root".into(),
            raw_kind: "containerWindowType".into(),
            kind: ElementKind::Container { background: None },
            pos: (0.0, 0.0),
            size_w: None,
            size_h: None,
            orientation: gui_runtime::scene::Anchor::UpperLeft,
            origo: gui_runtime::scene::Anchor::UpperLeft,
            clipping: false,
            alwaystransparent: false,
            rect: Rect::default(),
            clip: None,
            children: vec![Node {
                name: "stability".into(),
                raw_kind: "instantTextBoxType".into(),
                kind: ElementKind::Text(TextSpec {
                    text: "100%".into(),
                    raw: "100%".into(),
                    font: "hoi_18mbs".into(),
                    align: TextAlign::Right,
                    max_width: Some(57.0),
                    max_height: Some(24.0),
                }),
                pos: (0.0, 0.0),
                size_w: None,
                size_h: None,
                orientation: gui_runtime::scene::Anchor::UpperLeft,
                origo: gui_runtime::scene::Anchor::UpperLeft,
                clipping: false,
                alwaystransparent: false,
                rect: Rect::default(),
                clip: None,
                children: vec![],
            }],
        };
        let snap = ger_snapshot();
        let spec = TopbarSpec::topbar();
        let mut roots = std::slice::from_mut(&mut root);
        let report = apply_topbar(&mut roots, &spec, &snap, "GER");
        // the scene node now carries the bound text
        if let ElementKind::Text(t) = &root.children[0].kind {
            assert_eq!(t.text, "75%");
        } else {
            panic!("not text");
        }
        assert_eq!(report.flag_ideology.as_deref(), Some("fascism"));
        assert_eq!(report.get("stability").unwrap().value, "75%");
    }
}

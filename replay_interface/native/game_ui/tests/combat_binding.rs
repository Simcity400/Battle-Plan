//! U1 combat-detail panel gates: a LAYOUT gate (parse `landcombat.gui` via the
//! shared `game_ui` parser and resolve named child rects through the LAW-solid
//! `resolve_element_rect`) plus a BINDING gate (drive the real
//! `replay_core::ReplaySource`/`WorldTimeline` through the `fx_division_movement`
//! packed fixture and assert `combat_panel_from_frame` renders the exact
//! generator combat values, appearing/disappearing at the exact ticks).
//!
//! Expected values are transcribed from the fixture GENERATOR SOURCE
//! (`replay_fixture_gen::fx_division_movement`), never from the code under test:
//! ticks 6..=9 carry one combat `{combat_id:9001, kind:"land",
//! location_province: (1001 while tick<8 else 2000), attackers:[101],
//! defenders:[201]}`; every other tick has no active combat.

use game_ui::combat_binding::combat_panel_from_frame;
use game_ui::compose::{resolve_element_rect, ComposeOptions, Rect};
use game_ui::gui::{GuiElement, LayoutSet};
use replay_core::{open_recording, WorldTimeline};
use std::path::{Path, PathBuf};

/// The vanilla install root. Panics rather than returning `None`: every
/// caller is gated on `gate_hoi4_install`, so an absent install here is a
/// failure, never a silent skip that libtest would report as a pass.
fn game_root() -> PathBuf {
    replay_test_gates::require_vanilla_game_root()
}

fn parse_layout(root: &Path, rel: &str) -> LayoutSet {
    let mut layouts = LayoutSet::default();
    layouts
        .parse_file(&root.join(rel), rel)
        .expect("parse vanilla gui");
    layouts
}

fn find<'a>(layouts: &'a LayoutSet, name: &str, kind: &str) -> &'a GuiElement {
    layouts
        .find_by_name(name)
        .into_iter()
        .find(|element| element.element_type.eq_ignore_ascii_case(kind))
        .unwrap_or_else(|| panic!("{kind} {name} missing"))
}

fn child<'a>(parent: &'a GuiElement, name: &str) -> &'a GuiElement {
    parent
        .children
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("child {name} missing"))
}

fn fixture(name: &str) -> WorldTimeline {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/positive")
        .join(format!("{name}.battleplan"));
    let source = open_recording(&path)
        .unwrap_or_else(|err| panic!("open {name}: {err} ({})", path.display()));
    WorldTimeline::open(source).unwrap_or_else(|err| panic!("timeline {name}: {err}"))
}

/// LAYOUT GATE — `landcombat.gui` parses with 0 recoveries and `landcombatview`
/// resolves to its file-declared rect; its direct combat-detail children
/// resolve to engine-law pixel positions (center anchor + explicit position).
#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn landcombatview_layout_resolves_from_files() {
    let root = game_root();
    let layouts = parse_layout(&root, "interface/landcombat.gui");
    assert_eq!(
        layouts.stats.recovery_count, 0,
        "landcombat.gui parses clean"
    );

    let view = find(&layouts, "landcombatview", "containerWindowType");
    // Declared size 567x617; orientation center; position {-512,-256}.
    let options = ComposeOptions::new(1920, 1080);
    let screen = Rect::new(0, 0, 1920, 1080);
    let view_rect = resolve_element_rect(view, screen, (0, 0), &options);
    // center anchor = (960,540); + position (-512,-256) = (448,284), size 567x617.
    assert_eq!(view_rect, Rect::new(448, 284, 567, 617));

    // Direct children resolve relative to the window rect (UpperLeft default
    // anchor, explicit positions; textboxes/icons carry no size → natural 0x0).
    let cases = [
        ("window_title", 55, 2),
        ("terrain_picture", 77, 37),
        ("combat_progress", 185, 116),
        ("land_battle_progress_frame", 183, 114),
    ];
    for (name, px, py) in cases {
        let el = child(view, name);
        let rect = resolve_element_rect(el, view_rect, (0, 0), &options);
        assert_eq!(
            (rect.x(), rect.y()),
            (view_rect.x() + px, view_rect.y() + py),
            "child {name} rect"
        );
    }
    // The two participant sub-windows are present (division/reserve lists).
    assert!(view.children.iter().any(|c| c.name == "attacker_window"));
    assert!(view.children.iter().any(|c| c.name == "defender_window"));
}

/// BINDING GATE 1 — the combat appears at tick 6, holds through tick 9, and is
/// absent before/after, with the exact generator identity + participant counts.
#[test]
fn combat_panel_flips_at_generator_ticks() {
    let tl = fixture("fx_division_movement");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    for (t, &hour) in hours.iter().enumerate() {
        let b = combat_panel_from_frame(&tl.frame(hour).unwrap());
        if (6..=9).contains(&t) {
            assert!(b.available, "combat active @tick{t}");
            assert_eq!(b.combats.len(), 1, "one combat @tick{t}");
            let row = &b.combats[0];
            assert_eq!(row.combat_id, Some(9001), "combat_id @tick{t}");
            assert_eq!(row.kind.as_deref(), Some("land"), "kind @tick{t}");
            let expected_loc = if t < 8 { 1001 } else { 2000 };
            assert_eq!(
                row.location_province,
                Some(expected_loc),
                "location @tick{t}"
            );
            assert_eq!(
                row.attacker_divisions,
                Some(vec![101]),
                "attackers @tick{t}"
            );
            assert_eq!(
                row.defender_divisions,
                Some(vec![201]),
                "defenders @tick{t}"
            );
            assert_eq!(row.attacker_division_count(), Some(1));
            assert_eq!(row.defender_division_count(), Some(1));
            assert_eq!(row.terrain.as_deref(), Some("plains"));
            assert_eq!(row.progress, Some(t as f64 / 10.0));
            assert_eq!(row.combat_width, Some(80.0));
        } else {
            assert!(!b.available, "no combat @tick{t}");
            assert!(b.combats.is_empty(), "no combat rows @tick{t}");
        }
    }
}

/// BINDING GATE 2 — typed producer fields render exactly while genuinely
/// external selectors remain fail-closed.
#[test]
fn typed_display_fields_bind_and_external_selectors_fail_closed() {
    let tl = fixture("fx_division_movement");
    let hours = tl.hours().to_vec();
    let b = combat_panel_from_frame(&tl.frame(hours[7]).unwrap());
    let row = &b.combats[0];
    assert_eq!(row.progress, Some(0.7), "producer progress");
    assert_eq!(row.combat_width, Some(80.0), "producer combat width");
    assert_eq!(row.terrain.as_deref(), Some("plains"), "producer terrain");
    assert_eq!(row.bombardment_active, None, "A16 bombardment fail-closed");
    assert_eq!(row.air_superiority_state, None, "A16 air-sup fail-closed");
}

// ---------------------------------------------------------------------------
// FW1 status consumer — availability is owned by `WorldFrame::combats_status`
// ---------------------------------------------------------------------------

struct MemorySource {
    hours: Vec<i64>,
    frames: std::collections::BTreeMap<i64, serde_json::Value>,
}

impl replay_core::ReplaySource for MemorySource {
    fn game_id(&self) -> &str {
        "combat_status_test"
    }
    fn hours(&self) -> &[i64] {
        &self.hours
    }
    fn warnings(&self) -> &[String] {
        &[]
    }
    fn frame(
        &self,
        hour_index: i64,
    ) -> Result<replay_core::RawFrame, replay_core::SourceError> {
        let rest = self
            .frames
            .get(&hour_index)
            .and_then(serde_json::Value::as_object)
            .cloned()
            .ok_or_else(|| replay_core::SourceError::Open(format!("no frame {hour_index}")))?;
        Ok(replay_core::RawFrame {
            hour_index,
            date: None,
            save_version: None,
            schema_version: None,
            rest,
        })
    }
}

fn memory_frames(frames: Vec<(i64, serde_json::Value)>) -> WorldTimeline {
    let hours: Vec<i64> = frames.iter().map(|(h, _)| *h).collect();
    let source = MemorySource {
        hours,
        frames: frames.into_iter().collect(),
    };
    WorldTimeline::open(Box::new(source)).expect("open memory timeline")
}

/// The combats collection's own status decides availability — a frame whose
/// collection is unavailable (or absent) binds NO rows and is distinguishable
/// from the measured "no combats were running", instead of both collapsing
/// into the same empty vector.
#[test]
fn combat_panel_availability_is_status_owned_not_emptiness_derived() {
    use replay_core::ViewStatus;
    let tl = memory_frames(vec![
        // Present with one active combat.
        (
            0,
            serde_json::json!({ "combats": [ { "combat_id": 9001, "kind": "land" } ] }),
        ),
        // Present, measured empty: no combats were running.
        (1, serde_json::json!({ "combats": [] })),
        // Explicitly unobtained collection this hour.
        (2, serde_json::json!({ "combats": null })),
        // Domain absent: the hour carried no combats key at all.
        (3, serde_json::json!({})),
    ]);

    let active = combat_panel_from_frame(&tl.frame(0).unwrap());
    assert_eq!(active.combats_status, ViewStatus::Present);
    assert!(active.available);
    assert_eq!(active.combats.len(), 1);

    let measured_empty = combat_panel_from_frame(&tl.frame(1).unwrap());
    assert_eq!(measured_empty.combats_status, ViewStatus::Present);
    assert!(!measured_empty.available, "no combat => window closed");
    assert!(measured_empty.combats.is_empty());

    let unobtained = combat_panel_from_frame(&tl.frame(2).unwrap());
    assert_eq!(unobtained.combats_status, ViewStatus::DomainUnavailable);
    assert!(!unobtained.available);
    assert!(unobtained.combats.is_empty(), "fail-closed: no rows bound");

    let absent = combat_panel_from_frame(&tl.frame(3).unwrap());
    assert_eq!(absent.combats_status, ViewStatus::DomainAbsent);
    assert!(!absent.available);

    // The three empty-vector shapes are now distinguishable by status.
    assert_ne!(measured_empty.combats_status, unobtained.combats_status);
    assert_ne!(measured_empty.combats_status, absent.combats_status);
}

/// RES1 R1 — the XUI20 land vector consumes the same `combats_status`. The
/// binder is fail-closed at the frame seam (no rows bound under a non-Present
/// collection) and the bound status makes the measured-empty hour
/// distinguishable from the unreadable one, so no XUI20 land surface can read
/// as measured off row presence alone.
#[test]
fn xui20_land_binding_is_owned_by_combats_status() {
    use game_ui::combat_detail_losses::combat_detail_losses_from_frame;
    use replay_core::ViewStatus;
    let tl = memory_frames(vec![
        (
            0,
            serde_json::json!({ "combats": [ { "combat_id": 9001, "kind": "land" } ] }),
        ),
        (1, serde_json::json!({ "combats": [] })),
        (2, serde_json::json!({ "combats": null })),
        (3, serde_json::json!({})),
    ]);

    let active = combat_detail_losses_from_frame(&tl.frame(0).unwrap());
    assert_eq!(active.land_status, ViewStatus::Present);
    assert_eq!(active.land.len(), 1);
    assert!(active.has_any_measured_surface());

    let measured_empty = combat_detail_losses_from_frame(&tl.frame(1).unwrap());
    assert_eq!(measured_empty.land_status, ViewStatus::Present);
    assert!(measured_empty.land.is_empty());

    let unobtained = combat_detail_losses_from_frame(&tl.frame(2).unwrap());
    assert_eq!(unobtained.land_status, ViewStatus::DomainUnavailable);
    assert!(
        unobtained.land.is_empty(),
        "fail-closed: an unreadable collection binds no land rows"
    );

    let absent = combat_detail_losses_from_frame(&tl.frame(3).unwrap());
    assert_eq!(absent.land_status, ViewStatus::DomainAbsent);
    assert!(absent.land.is_empty());

    assert_ne!(measured_empty.land_status, unobtained.land_status);
    assert_ne!(measured_empty.land_status, absent.land_status);
}

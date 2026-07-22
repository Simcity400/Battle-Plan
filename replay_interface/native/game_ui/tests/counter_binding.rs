//! U1 counter-content binding gates (REPLAY_PRODUCTION_PLAN §6a / C1 content).
//!
//! Drives the packed snapshot-flavor fixtures through the real `replay_core`
//! `ReplaySource`/`WorldTimeline` and asserts that
//! `game_ui::binding_counters::counter_contents_for_pov` populates every counter
//! field with the exact fixture ground truth, across multiple POV countries at
//! every probed hour, with fail-closed absence honesty and a reported NATO-symbol
//! resolution rate.
//!
//! Expected values are derived from the fixture GENERATOR SOURCE
//! (`replay_fixture_gen/src/main.rs`), never by running the code under test:
//!   * `fx_division_movement` — `division(101,1,"GER",loc,…)` and
//!     `division(201,1,"SOV",2000,…)`; both use `template_role="infantry"`,
//!     `template_name="Fixture Infantry"`, `strength=0.95`, `organisation=42.0`.
//!     Div 101: `moving = tick<8`, `move_target = Some(2000) if tick<8`,
//!     `in_combat = true` for ticks 6..=9 (mutate override), else false.
//!     Div 201: not moving, never in combat.
//!   * `fx_naval_air_activity` — base divisions `101:1`/`201:1` (unchanged),
//!     one task force `501:1` (owner ENG, `ship_count = 8+tick`, hull "capital"),
//!     one air wing `800:1` (owner ENG, `name="Fighter Wing"`,
//!     `plane_count = 100+tick`, `mission="air_superiority"`).
//!
//! Tick↔hour: `fixture_time(base_offset+tick)` is strictly increasing in tick,
//! so `tl.hours()[i]` is the frame for tick `i`.

use game_ui::binding_counters::{
    counter_contents_for_pov, nato_symbol_resolution, CounterContent, CounterDomain, NatoKeySource,
    ServedBadge, KNOWN_LAND_SYMBOL_KEYS_SAMPLE,
};
use replay_core::{open_recording, ViewStatus, WorldTimeline};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn fixture(name: &str) -> WorldTimeline {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/positive")
        .join(format!("{name}.battleplan"));
    let source = open_recording(&path)
        .unwrap_or_else(|err| panic!("open {name}: {err} ({})", path.display()));
    WorldTimeline::open(source).unwrap_or_else(|err| panic!("timeline {name}: {err}"))
}

fn known_land_keys() -> BTreeSet<String> {
    KNOWN_LAND_SYMBOL_KEYS_SAMPLE
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Assert the common infantry-division content shared by both fixtures.
fn assert_infantry_land(c: &CounterContent, uid: &str, owner: &str) {
    assert_eq!(c.uid, uid);
    assert_eq!(c.domain, CounterDomain::Land);
    assert_eq!(c.owner.as_deref(), Some(owner));
    assert_eq!(c.country_color_key.as_deref(), Some(owner));
    assert_eq!(c.name.as_deref(), Some("Fixture Infantry"));
    assert_eq!(
        c.nato_symbol_key.as_deref(),
        Some("GFX_unit_infantry_icon_medium_white"),
        "{uid} nato key"
    );
    assert_eq!(c.symbol_source, NatoKeySource::LandTemplateRole);
    assert_eq!(c.symbol_hint, None);
    assert_eq!(c.strength, Some(0.95));
    assert_eq!(c.organisation, Some(42.0));
    assert_eq!(c.str_bar_fraction, Some(0.95));
    // This legacy fixture carries no authenticated max-org denominator.
    assert_eq!(c.org_bar_fraction, None);
    assert_eq!(c.max_organisation, None);
    assert_eq!(c.current_supply, None);
    assert_eq!(c.max_supply, None);
    assert_eq!(c.supply_ratio, None);
    assert_eq!(c.remaining_supply, replay_core::Leaf::Absent);
    // Experience is a directly recorded scalar and is surfaced as itself; it
    // is never reused as the engine's evaluated training predicate.
    assert_eq!(c.experience, Some(0.2));
    assert_eq!(c.dig_in_fraction, Some(0.3));
    assert_eq!(c.current_order_kind, replay_core::Leaf::Absent);
    assert_eq!(c.active_ability_kinds, replay_core::Leaf::Absent);
    assert_eq!(c.count, None);
    // The row is present but predates the evaluated A15 observations, so the
    // three row-owned badges are explicitly unavailable this hour. Separate
    // child-domain badges remain structurally absent.
    assert_eq!(c.badges.low_supply, ServedBadge::NotSurfaced);
    assert_eq!(c.badges.dig_in, ServedBadge::AbsentThisHour);
    assert_eq!(c.badges.training, ServedBadge::AbsentThisHour);
    assert_eq!(c.badges.attrition, ServedBadge::NotSurfaced);
    assert_eq!(c.badges.active_ability, ServedBadge::NotSurfaced);
}

/// GATE U1a — `fx_division_movement`: div 101 (GER) and 201 (SOV) counter
/// content matches ground truth at every hour, across two POVs, with the served
/// orders/combat badges tracking `moving`/`in_combat` per tick and the untouched
/// A15 badges fail-closed.
#[test]
fn division_movement_land_counter_content_per_pov_per_hour() {
    let tl = fixture("fx_division_movement");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    for (tick, &hour) in hours.iter().enumerate() {
        let frame = tl.frame(hour).unwrap();

        // --- GER POV: exactly div 101:1, no naval/air ---
        let ger = counter_contents_for_pov(&frame, "GER");
        assert_eq!(ger.land_status, ViewStatus::Present);
        assert_eq!(ger.land.len(), 1, "GER land count @tick{tick}");
        assert!(
            ger.naval.is_empty() && ger.air.is_empty(),
            "GER navy/air empty"
        );
        let d101 = &ger.land[0];
        assert_infantry_land(d101, "101:1", "GER");
        // This legacy fixture has no D20 primary-order authority. Movement is
        // not relabelled as an order-link observation.
        assert_eq!(d101.badges.orders, ServedBadge::NotSurfaced);
        assert_eq!(
            d101.badges.combat,
            ServedBadge::Measured((6..=9).contains(&tick)),
            "GER 101 combat @tick{tick}"
        );

        // --- SOV POV: exactly div 201:1, never moving/in combat ---
        let sov = counter_contents_for_pov(&frame, "SOV");
        assert_eq!(sov.land.len(), 1, "SOV land count @tick{tick}");
        let d201 = &sov.land[0];
        assert_infantry_land(d201, "201:1", "SOV");
        assert_eq!(d201.badges.orders, ServedBadge::NotSurfaced);
        assert_eq!(d201.badges.combat, ServedBadge::Measured(false));

        // --- ENG POV: no owned units (never fabricated) ---
        let eng = counter_contents_for_pov(&frame, "ENG");
        assert!(eng.is_empty(), "ENG has no counters @tick{tick}");

        // NATO-symbol resolution over GER's counters: 1 keyed, 1 resolved.
        let (resolved, keyed) = nato_symbol_resolution(ger.all(), &known_land_keys());
        assert_eq!((resolved, keyed), (1, 1), "GER nato resolution @tick{tick}");
    }
}

/// GATE U1b — `fx_naval_air_activity`: ENG owns the task force + air wing;
/// counter content (count, name, symbol hint, all-not-surfaced badges) matches
/// ground truth. GER still owns the base infantry division. Naval/air carry no
/// subuniticons symbol key (separate symbol systems), so land-symbol resolution
/// is 0/0 for the ENG POV.
#[test]
fn naval_air_counter_content_per_pov_per_hour() {
    let tl = fixture("fx_naval_air_activity");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    for (tick, &hour) in hours.iter().enumerate() {
        let tick = tick as i64;
        let frame = tl.frame(hour).unwrap();

        // --- ENG POV: one naval, one air, no land ---
        let eng = counter_contents_for_pov(&frame, "ENG");
        assert_eq!(eng.naval_status, ViewStatus::Present);
        assert_eq!(eng.air_status, ViewStatus::Present);
        assert!(eng.land.is_empty(), "ENG owns no division @tick{tick}");
        assert_eq!(eng.naval.len(), 1);
        assert_eq!(eng.air.len(), 1);

        let tf = &eng.naval[0];
        assert_eq!(tf.uid, "501:1");
        assert_eq!(tf.domain, CounterDomain::Naval);
        assert_eq!(tf.owner.as_deref(), Some("ENG"));
        assert_eq!(tf.country_color_key.as_deref(), Some("ENG"));
        // GAP: FleetUnitRow serves no task-force name.
        assert_eq!(tf.name, None);
        assert_eq!(tf.nato_symbol_key, None);
        assert_eq!(tf.symbol_source, NatoKeySource::NavalHullClassNotMapped);
        assert_eq!(tf.symbol_hint.as_deref(), Some("capital"));
        assert_eq!(tf.strength, None);
        assert_eq!(tf.organisation, None);
        assert_eq!(tf.str_bar_fraction, None);
        assert_eq!(tf.org_bar_fraction, None);
        assert_eq!(tf.count, Some(8 + tick), "TF ship_count @tick{tick}");
        for badge in [
            tf.badges.orders,
            tf.badges.combat,
            tf.badges.low_supply,
            tf.badges.dig_in,
            tf.badges.training,
            tf.badges.attrition,
            tf.badges.active_ability,
        ] {
            assert_eq!(badge, ServedBadge::NotSurfaced);
        }

        let wing = &eng.air[0];
        assert_eq!(wing.uid, "800:1");
        assert_eq!(wing.domain, CounterDomain::Air);
        assert_eq!(wing.owner.as_deref(), Some("ENG"));
        assert_eq!(wing.name.as_deref(), Some("Fighter Wing"));
        assert_eq!(wing.nato_symbol_key, None);
        assert_eq!(wing.symbol_source, NatoKeySource::AirMissionNotMapped);
        assert_eq!(wing.symbol_hint.as_deref(), Some("air_superiority"));
        assert_eq!(wing.count, Some(100 + tick), "wing plane_count @tick{tick}");
        assert_eq!(wing.str_bar_fraction, None);
        assert_eq!(wing.org_bar_fraction, None);

        // No subuniticons key on naval/air → 0 keyed, 0 resolved for ENG.
        let (resolved, keyed) = nato_symbol_resolution(eng.all(), &known_land_keys());
        assert_eq!((resolved, keyed), (0, 0), "ENG nato resolution @tick{tick}");

        // --- GER POV: the base infantry division, resolving 1/1 ---
        let ger = counter_contents_for_pov(&frame, "GER");
        assert_eq!(ger.land.len(), 1);
        assert_infantry_land(&ger.land[0], "101:1", "GER");
        let (resolved, keyed) = nato_symbol_resolution(ger.all(), &known_land_keys());
        assert_eq!((resolved, keyed), (1, 1), "GER nato resolution @tick{tick}");
    }
}

/// GATE U1c — domain view status is carried through fail-closed. Both fixtures
/// keep all three unit domains `Present` (arrays, possibly empty), so an
/// empty-but-present domain is distinguishable from an unavailable one.
#[test]
fn domain_status_is_carried_not_inferred() {
    let tl = fixture("fx_division_movement");
    let frame = tl.frame(tl.first_hour()).unwrap();
    let ger = counter_contents_for_pov(&frame, "GER");
    // naval/air are empty for GER but the domain is Present (a real empty array
    // in the recording), not Unavailable.
    assert_eq!(ger.naval_status, ViewStatus::Present);
    assert_eq!(ger.air_status, ViewStatus::Present);
    assert!(ger.naval.is_empty() && ger.air.is_empty());
}

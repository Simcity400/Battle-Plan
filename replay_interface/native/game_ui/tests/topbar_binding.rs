//! D1 + D2 binding gates.
//!
//! These drive the *packed snapshot-flavor* fixtures through the real
//! `replay_core` `ReplaySource`/`WorldTimeline` and assert that the
//! `game_ui::binding` population produces the exact generator values
//! (string-level compare) at every probed hour, that the ideology flip
//! re-keys the flag/name candidates at the exact flip hour, and that
//! unavailable views render the fail-closed absent form (never a default).
//!
//! Expected values are derived from the fixture GENERATOR SOURCE
//! (`replay_fixture_gen/src/main.rs`), not by running the code under test.

use game_ui::assets::AssetResolver;
use game_ui::binding::{
    country_name_key_candidates, topbar_binding_from_frame, topbar_flag_key_candidates,
};
use game_ui::probe::parse_gfx_files;
use game_ui::topbar::{compose_topbar, FactoryCounts};
use replay_core::{open_recording, WorldTimeline};
use std::path::PathBuf;

fn fixture(name: &str) -> WorldTimeline {
    fixture_in("positive", name)
}

fn fixture_in(group: &str, name: &str) -> WorldTimeline {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(group)
        .join(format!("{name}.battleplan"));
    let source = open_recording(&path)
        .unwrap_or_else(|err| panic!("open {name}: {err} ({})", path.display()));
    WorldTimeline::open(source).unwrap_or_else(|err| panic!("timeline {name}: {err}"))
}

/// The vanilla install root. Panics rather than returning `None`: every
/// caller is gated on `gate_hoi4_install`, so an absent install here is a
/// failure, never a silent skip that libtest would report as a pass.
fn game_root() -> PathBuf {
    replay_test_gates::require_vanilla_game_root()
}

// --- generator formulas, transcribed from replay_fixture_gen::fx_topbar_scalars_sweep ---
// base per tag: GER=0, ENG=20, SOV=40. t = tick index (== hours[t]).
fn base_for(tag: &str) -> i64 {
    match tag {
        "GER" => 0,
        "ENG" => 20,
        _ => 40,
    }
}

/// GATE 1 — every topbar scalar the view can serve renders == generator value
/// at every probed hour, for multiple POVs (per-POV keying).
#[test]
fn topbar_scalar_sweep_renders_generator_values() {
    let tl = fixture("fx_topbar_scalars_sweep");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    for (t_idx, &hour) in hours.iter().enumerate() {
        let t = t_idx as i64;
        let frame = tl.frame(hour).unwrap();
        for tag in ["GER", "ENG", "SOV"] {
            let base = base_for(tag);
            let b = topbar_binding_from_frame(&frame, tag);

            // Raw-rendered scalars (UNKNOWN formatting → format!("{}") of the
            // exact generator f64/i64; see binding.rs module header).
            assert_eq!(
                b.political_power.as_deref(),
                Some(format!("{}", (base + t * 3) as f64).as_str()),
                "{tag} pp @t{t}"
            );
            // stability / war_support are recorded 0.0..=1.0 ratios and take
            // the SAME percent projection as the sibling world-tension field:
            // `topbar::format_percent` of the generator fraction, exactly.
            assert_eq!(
                b.stability_percent,
                Some(game_ui::topbar::format_percent(
                    (0.40 + (t as f64 * 0.02)) as f32
                )),
                "{tag} stability @t{t}"
            );
            assert_eq!(
                b.war_support_percent,
                Some(game_ui::topbar::format_percent(
                    (0.30 + (t as f64 * 0.015)) as f32
                )),
                "{tag} war_support @t{t}"
            );
            assert_eq!(
                b.command_power,
                Some(format!("{}", (base + 10 + t) as f64)),
                "{tag} command_power @t{t}"
            );
            assert_eq!(
                b.manpower,
                Some(format!("{}", 100_000 + base * 1000 + t * 100)),
                "{tag} manpower @t{t}"
            );
            assert_eq!(
                b.army_experience,
                Some(format!("{}", t as f64)),
                "{tag} xp_army @t{t}"
            );
            assert_eq!(
                b.navy_experience,
                Some(format!("{}", (t * 2) as f64)),
                "{tag} xp_navy @t{t}"
            );
            assert_eq!(
                b.air_experience,
                Some(format!("{}", (t * 3) as f64)),
                "{tag} xp_air @t{t}"
            );
            assert_eq!(
                b.fuel,
                Some(format!("{}", (500 + base * 10 + t * 25) as f64)),
                "{tag} fuel @t{t}"
            );
            // This legacy fixture predates the exact d83 sidecar and retained
            // D18 numeric join, so its scalar cannot reach the product UI.
            assert_eq!(b.nukes, None, "{tag} nukes exact authority @t{t}");
            assert_eq!(
                b.factory_counts,
                Some(FactoryCounts {
                    civilian: (base + 10 + t) as u32,
                    military: (base + 5 + t) as u32,
                    naval: (base + 2) as u32,
                }),
                "{tag} factories @t{t}"
            );

            // Convoys: served per-tick since the A3 convoys_count alias landed:
            // generator writes military.convoys_count = base + 50 + tick.
            assert_eq!(
                b.convoys,
                Some(format!("{}", (base + 50 + t) as f64)),
                "{tag} convoys @t{t}"
            );

            // GATE 4 (in-sweep absence): world_tension lives on a path the view
            // does not read for this fixture → fail-closed None.
            assert_eq!(b.world_tension_percent, None, "{tag} world_tension @t{t}");
            assert_eq!(b.world_tension, None, "{tag} world_tension ratio @t{t}");
            // supply is served since the A3 scalars.supply_* alias landed:
            // generator writes supply_value = 12.0 + tick.
            assert_eq!(
                b.supply,
                Some(format!("{}", 12.0 + t as f64)),
                "{tag} supply @t{t}"
            );
            assert_eq!(
                b.industrial_ratio,
                Some((0.5 + t as f64 * 0.01) as f32),
                "{tag} industrial ratio @t{t}"
            );
            assert_eq!(
                b.convoys_ratio,
                Some((0.9 - t as f64 * 0.01) as f32),
                "{tag} convoy ratio @t{t}"
            );
            assert_eq!(
                b.fuel_ratio,
                Some((0.25 + t as f64 * 0.02) as f32),
                "{tag} fuel ratio @t{t}"
            );
            assert_eq!(
                b.supply_ratio,
                Some((0.8 - t as f64 * 0.01) as f32),
                "{tag} supply ratio @t{t}"
            );
            // One recorded xp_ratio cannot be joined to three distinct bars.
            assert_eq!(b.army_xp_ratio, None);
            assert_eq!(b.navy_xp_ratio, None);
            assert_eq!(b.air_xp_ratio, None);
        }
    }
}

/// GATE 4 — a POV absent from an otherwise-present frame renders an
/// all-absent scalar binding (never defaulted to zero), while still naming the
/// tag so the flag can fall back to the bare inventory.
#[test]
fn absent_pov_renders_fail_closed_binding() {
    let tl = fixture("fx_topbar_scalars_sweep");
    let frame = tl.frame(tl.first_hour()).unwrap();
    let b = topbar_binding_from_frame(&frame, "USA"); // not in the fixture
    assert_eq!(b.country_tag.as_deref(), Some("USA"));
    assert_eq!(b.political_power, None);
    assert_eq!(b.manpower, None);
    assert_eq!(b.fuel, None);
    assert_eq!(b.factory_counts, None);
    assert_eq!(b.country_ideology, None);
    assert_eq!(b.cosmetic_tag, None);
}

/// End-to-end XUI27 unavailable-data gate. A POV omitted from a present
/// `WorldFrame` drives every dynamic topbar leaf to `None`, then the real
/// game-file composer omits those elements. This proves the renderer cannot
/// expose `topbar.gui` placeholders or select zero/default progress frames.
#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn absent_world_frame_data_is_omitted_by_topbar_composition() {
    let root = game_root();
    let tl = fixture("fx_topbar_scalars_sweep");
    let frame = tl.frame(tl.first_hour()).unwrap();
    let binding = topbar_binding_from_frame(&frame, "USA");
    let resolver = AssetResolver::new(vec![root]);
    let (registry, _) = parse_gfx_files(&resolver).expect("parse GFX");
    let rendered = compose_topbar(&resolver, &registry, &binding).expect("compose absent topbar");
    let rendered_again =
        compose_topbar(&resolver, &registry, &binding).expect("recompose absent topbar");

    assert_eq!(rendered.output.image, rendered_again.output.image);
    assert_eq!(
        rendered.output.debug_rects,
        rendered_again.output.debug_rects
    );
    assert_eq!(
        (rendered.output.image.width, rendered.output.image.height),
        (2346, 87)
    );

    let names = rendered
        .output
        .debug_rects
        .iter()
        .map(|rect| rect.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for name in [
        "pol_power",
        "manpower",
        "stability",
        "war_support",
        "command_power_value",
        "experience_value",
        "navy_experience_value",
        "air_experience_value",
        "nukes_value",
        "industrial_capacity",
        "convoys_count",
        "fuel_value",
        "supply_value",
        "industrial_ratio_bar",
        "convoys_ratio_bar",
        "fuel_ratio_bar",
        "supply_ratio_bar",
        "threat_button",
        "threat_icon_glow",
        "threat_value_button",
        "threat_value",
        "DateText",
        "button_speedstep1",
        "button_speedstep2",
        "button_speedstep3",
        "button_speedstep4",
        "button_speedstep5",
        "initiative_text",
        "decisionview_amount_timeout_bg",
        "decisionview_amount_timeout_items",
        "decisionview_amount_to_take_bg",
        "decisionview_amount_to_take_items",
        "intelview_ops_prepared_or_completed_bg",
        "intelview_ops_prepared_or_completed_items",
        "intelview_ops_available_bg",
        "intelview_ops_available_items",
        "prototype_rewards_aquired_bg",
        "prototype_rewards_aquired",
        "non_delivering_contracts",
        "non_delivering_contract_amount",
        "delivering_contracts",
        "ongoing_contract_amount",
    ] {
        assert!(
            !names.contains(name),
            "unavailable `{name}` reached the render tree"
        );
    }
}

/// The anomaly fixture changes one typed scalar from numeric to wrong-typed at
/// the crossing hour. `WorldFrame` classifies that leaf as `Unobtained`; the
/// composed topbar must omit only that value while retaining measured peers.
#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn unobtained_world_frame_leaf_is_selectively_omitted_by_topbar_composition() {
    let root = game_root();
    let tl = fixture_in("anomaly", "fx_anomaly_type_flip");
    let crossing_hour = tl.hours()[6];
    let frame = tl.frame(crossing_hour).unwrap();
    let binding = topbar_binding_from_frame(&frame, "GER");
    assert_eq!(binding.political_power, None);
    assert!(
        frame
            .coverage_warnings()
            .iter()
            .any(|warning| warning.contains("political_power") && warning.contains("unobtained")),
        "crossing frame did not preserve the wrong-type/unobtained finding"
    );

    let resolver = AssetResolver::new(vec![root]);
    let (registry, _) = parse_gfx_files(&resolver).expect("parse GFX");
    let rendered =
        compose_topbar(&resolver, &registry, &binding).expect("compose unobtained topbar");
    let names = rendered
        .output
        .debug_rects
        .iter()
        .map(|rect| rect.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(!names.contains("pol_power"));
    assert!(
        names.contains("manpower"),
        "measured peer was incorrectly omitted"
    );
}

/// GATE 2 — the ideology/cosmetic flip re-keys the flag and display-name
/// candidates at the exact flip hour and nowhere else.
#[test]
fn ideology_flip_rekeys_flag_and_name_at_flip_hour() {
    let tl = fixture("fx_cosmetic_ideology");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    // Generator: tick<4 fascism/none, 4<=tick<8 democratic/GER_civil_war_democratic,
    // tick>=8 neutrality/GER_reichskommissariat.
    let ideology_at = |t: usize| match t {
        0..=3 => ("fascism", None),
        4..=7 => ("democratic", Some("GER_civil_war_democratic")),
        _ => ("neutrality", Some("GER_reichskommissariat")),
    };

    for (t, &hour) in hours.iter().enumerate() {
        let frame = tl.frame(hour).unwrap();
        let b = topbar_binding_from_frame(&frame, "GER");
        let (ideology, cosmetic) = ideology_at(t);

        // tick<4: extra.cosmetic_tag is JSON null, the producer's MEASURED "this
        // country has no cosmetic override" → `Leaf::Measured(None)` → no
        // override in the binding, so the shield keys off the bare tag.
        assert_eq!(
            b.country_ideology.as_deref(),
            Some(ideology),
            "ideology @t{t}"
        );
        assert_eq!(b.cosmetic_tag.as_deref(), cosmetic, "cosmetic @t{t}");

        let identity = frame.country_identity("GER").unwrap();
        let flags = topbar_flag_key_candidates("GER", identity);
        let names = country_name_key_candidates("GER", Some(ideology), cosmetic);

        match t {
            0..=3 => {
                assert_eq!(flags[0], "gfx/flags/medium/GER_fascism.tga", "flag @t{t}");
                assert_eq!(names, vec!["GER_fascism".to_string(), "GER".to_string()]);
            }
            4..=7 => {
                assert_eq!(
                    flags[0], "gfx/flags/medium/GER_civil_war_democratic_democratic.tga",
                    "flag @t{t}"
                );
                assert_eq!(
                    names,
                    vec![
                        "GER_civil_war_democratic_democratic".to_string(),
                        "GER_civil_war_democratic".to_string(),
                        "GER_democratic".to_string(),
                        "GER".to_string(),
                    ]
                );
            }
            _ => {
                assert_eq!(
                    flags[0], "gfx/flags/medium/GER_reichskommissariat_neutrality.tga",
                    "flag @t{t}"
                );
                assert_eq!(names[0], "GER_reichskommissariat_neutrality");
            }
        }
    }

    // The flip is exact: hour 3 differs from hour 4.
    let f3 = tl.frame(hours[3]).unwrap();
    let f4 = tl.frame(hours[4]).unwrap();
    let flags3 = topbar_flag_key_candidates("GER", f3.country_identity("GER").unwrap());
    let flags4 = topbar_flag_key_candidates("GER", f4.country_identity("GER").unwrap());
    assert_ne!(
        flags3, flags4,
        "flag candidates must change at the flip hour"
    );
}

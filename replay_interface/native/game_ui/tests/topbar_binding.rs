
use game_ui::assets::AssetResolver;
use game_ui::binding::{
    country_name_key_candidates, topbar_binding_from_frame, topbar_flag_key_candidates,
};
use game_ui::probe::parse_gfx_files;
use game_ui::topbar::{compose_topbar, FactoryCounts};
use replay_core::{open_recording, WorldTimeline};
use std::path::PathBuf;

/// A TAG this frame's census admits. A test that means to name a country says
/// so here; a test about a contended key asks `named_country` and gets `None`.
fn named<'tag>(
    frame: &replay_core::WorldFrame,
    tag: &'tag str,
) -> replay_core::NamedCountry<'tag> {
    frame.named_country(tag).expect("census-admitted tag")
}

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

fn game_root() -> PathBuf {
    replay_test_gates::require_vanilla_game_root()
}

fn base_for(tag: &str) -> i64 {
    match tag {
        "GER" => 0,
        "ENG" => 20,
        _ => 40,
    }
}

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
            let b = topbar_binding_from_frame(&frame, named(&frame, tag));

            assert_eq!(
                b.political_power.as_deref(),
                Some(format!("{}", (base + t * 3) as f64).as_str()),
                "{tag} pp @t{t}"
            );
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

            assert_eq!(
                b.convoys,
                Some(format!("{}", (base + 50 + t) as f64)),
                "{tag} convoys @t{t}"
            );

            assert_eq!(b.world_tension_percent, None, "{tag} world_tension @t{t}");
            assert_eq!(b.world_tension, None, "{tag} world_tension ratio @t{t}");
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
            assert_eq!(b.army_xp_ratio, None);
            assert_eq!(b.navy_xp_ratio, None);
            assert_eq!(b.air_xp_ratio, None);
        }
    }
}

#[test]
fn absent_pov_renders_fail_closed_binding() {
    let tl = fixture("fx_topbar_scalars_sweep");
    let frame = tl.frame(tl.first_hour()).unwrap();
    let b = topbar_binding_from_frame(&frame, named(&frame, "USA"));
    assert_eq!(b.country_tag.as_deref(), Some("USA"));
    assert_eq!(b.political_power, None);
    assert_eq!(b.manpower, None);
    assert_eq!(b.fuel, None);
    assert_eq!(b.factory_counts, None);
    assert_eq!(b.country_ideology, None);
    assert_eq!(b.cosmetic_tag, None);
}

#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn absent_world_frame_data_is_omitted_by_topbar_composition() {
    let root = game_root();
    let tl = fixture("fx_topbar_scalars_sweep");
    let frame = tl.frame(tl.first_hour()).unwrap();
    let binding = topbar_binding_from_frame(&frame, named(&frame, "USA"));
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
    let binding = topbar_binding_from_frame(&frame, named(&frame, "GER"));
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

#[test]
fn ideology_flip_rekeys_flag_and_name_at_flip_hour() {
    let tl = fixture("fx_cosmetic_ideology");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    let ideology_at = |t: usize| match t {
        0..=3 => ("fascism", None),
        4..=7 => ("democratic", Some("GER_civil_war_democratic")),
        _ => ("neutrality", Some("GER_reichskommissariat")),
    };

    for (t, &hour) in hours.iter().enumerate() {
        let frame = tl.frame(hour).unwrap();
        let b = topbar_binding_from_frame(&frame, named(&frame, "GER"));
        let (ideology, cosmetic) = ideology_at(t);

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

    let f3 = tl.frame(hours[3]).unwrap();
    let f4 = tl.frame(hours[4]).unwrap();
    let flags3 = topbar_flag_key_candidates("GER", f3.country_identity("GER").unwrap());
    let flags4 = topbar_flag_key_candidates("GER", f4.country_identity("GER").unwrap());
    assert_ne!(
        flags3, flags4,
        "flag candidates must change at the flip hour"
    );
}

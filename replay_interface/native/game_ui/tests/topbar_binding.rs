//! D1 + D2 binding gates (REPLAY_PRODUCTION_PLAN Phase D).
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

use game_ui::binding::{
    country_name_key_candidates, topbar_binding_from_frame, topbar_flag_key_candidates,
};
use game_ui::topbar::FactoryCounts;
use replay_core::{open_recording, WorldTimeline};
use std::path::PathBuf;

fn fixture(name: &str) -> WorldTimeline {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/positive")
        .join(format!("{name}.battleplan"));
    let source = open_recording(&path)
        .unwrap_or_else(|err| panic!("open {name}: {err} ({})", path.display()));
    WorldTimeline::open(source).unwrap_or_else(|err| panic!("timeline {name}: {err}"))
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

/// Assert a rendered scalar string decodes to `expected` (± 1e-9). Used for the
/// two genuinely-fractional fields whose JSON round-trip is not bit-exact.
fn assert_close(got: &Option<String>, expected: f64, _label: &str, tag: &str, t: i64) {
    let s = got
        .as_deref()
        .unwrap_or_else(|| panic!("{tag} scalar @t{t} was fail-closed None, expected {expected}"));
    let v: f64 = s
        .parse()
        .unwrap_or_else(|_| panic!("{tag} scalar @t{t} not a number: {s:?}"));
    assert!(
        (v - expected).abs() < 1e-9,
        "{tag} scalar @t{t}: rendered {s:?} ({v}) != generator {expected}"
    );
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
            // stability / war_support are the only genuinely-fractional
            // scalars; JSON serialization is not bit-round-trip-exact, so the
            // raw string is validated to DECODE to the generator value (still a
            // string the binding produced, numerically pinned to the formula).
            assert_close(
                &b.stability_percent,
                0.40 + (t as f64 * 0.02),
                "{tag} stability",
                tag,
                t,
            );
            assert_close(
                &b.war_support_percent,
                0.30 + (t as f64 * 0.015),
                "{tag} war_support",
                tag,
                t,
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
            assert_eq!(b.nukes, Some(format!("{}", t / 4)), "{tag} nukes @t{t}");
            assert_eq!(
                b.factory_counts,
                Some(FactoryCounts {
                    civilian: (base + 10 + t) as u32,
                    military: (base + 5 + t) as u32,
                    naval: (base + 2) as u32,
                }),
                "{tag} factories @t{t}"
            );

            // Convoys: the view sums military.convoys[].amount (base 50.0); the
            // generator's per-tick military.convoys_count is on a path the view
            // does not read (replay_core gap, see LANE_REPORT.md). The binding
            // faithfully renders the value the view actually serves.
            assert_eq!(b.convoys, Some("50".to_string()), "{tag} convoys @t{t}");

            // GATE 4 (in-sweep absence): world_tension and supply live on paths
            // the view does not read for this fixture → fail-closed None.
            assert_eq!(b.world_tension_percent, None, "{tag} world_tension @t{t}");
            assert_eq!(b.world_tension, None, "{tag} world_tension ratio @t{t}");
            assert_eq!(b.supply, None, "{tag} supply @t{t}");
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

        // tick<4: extra.cosmetic_tag is JSON null → Unobtained → None binding.
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

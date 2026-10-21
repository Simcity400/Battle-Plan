//! The calculator's rules on synthetic definitions and pools, slot indices chosen freely so no
//! executable metadata is read.

use super::kernel::*;
use installed_content_core::equipment::STAT_COUNT;

fn slots() -> Slots {
    let names = [
        "max_strength",
        "max_organisation",
        "default_morale",
        "recon",
        "entrenchment",
        "initiative",
        "casualty_trickleback",
        "supply_consumption",
        "supply_consumption_factor",
        "suppression",
        "suppression_factor",
        "experience_loss_factor",
        "equipment_capture_factor",
        "reliability",
        "reliability_factor",
        "weight",
        "fuel_capacity",
        "fuel_consumption",
        "fuel_consumption_factor",
        "soft_attack",
        "hard_attack",
        "air_attack",
        "defense",
        "breakthrough",
        "armor_value",
        "ap_attack",
        "hardness",
        "maximum_speed",
        "build_cost_ic",
    ];
    Slots::new(|name| names.iter().position(|known| *known == name)).unwrap()
}

const fn fixed(units: i64) -> i64 {
    units * ONE
}

fn definition(words: &[(usize, i64)], requirements: Vec<(usize, i64)>) -> Definition {
    let mut stats = [0; STAT_COUNT];
    for &(slot, value) in words {
        stats[slot] = value;
    }
    Definition {
        stats,
        combat_width: fixed(2),
        manpower: 1000,
        training_time: 90,
        support_type: false,
        support_group: false,
        categories: vec!["category_line".into()],
        grants: Vec::new(),
        requirements,
        critical: Vec::new(),
        transport: None,
        own_fuel_consumption: ONE,
        affects_speed: true,
    }
}

fn design(words: &[(usize, i64)], archetype: usize, amount: i64) -> Held {
    let mut stats = [0; STAT_COUNT];
    for &(slot, value) in words {
        stats[slot] = value;
    }
    Held {
        stats,
        archetype,
        amount,
    }
}

fn operands() -> Operands {
    Operands {
        manpower_fill: ONE,
        fuel_fill: ONE,
        armor_vs_average: 40_000,
        pen_vs_average: 40_000,
        out_of_fuel_equipment: 10_000,
        fuel_capacity_hours: fixed(96),
        army_fuel_cost: 50_000,
        slowest_speed: 1_000,
    }
}

/// Two line groups and a support company, fully equipped: sums, means over every real cell,
/// hardness over the non-support cells only, and the armour blend of the largest group and the
/// average.
#[test]
fn a_full_composition_follows_each_stats_rule() {
    let s = slots();
    let mut line = definition(
        &[(s.max_strength, fixed(25)), (s.max_organisation, fixed(60))],
        vec![(1, fixed(100))],
    );
    line.stats[s.soft_attack] = 0;
    let mut armour = definition(
        &[(s.max_strength, fixed(2)), (s.max_organisation, fixed(10))],
        vec![(2, fixed(50))],
    );
    armour.stats[s.hardness] = 0;
    let mut support = definition(
        &[
            (s.max_strength, fixed(2)),
            (s.max_organisation, fixed(20)),
            (s.soft_attack, -50_000),
        ],
        vec![(1, fixed(20))],
    );
    support.support_type = true;
    support.support_group = true;
    let groups = [
        Group {
            definition: &line,
            count: 3,
            critical: ONE,
        },
        Group {
            definition: &armour,
            count: 1,
            critical: ONE,
        },
        Group {
            definition: &support,
            count: 1,
            critical: ONE,
        },
    ];
    let rifles = design(
        &[
            (s.soft_attack, fixed(3)),
            (s.hardness, 0),
            (s.reliability, 90_000),
        ],
        1,
        fixed(320),
    );
    let tanks = design(
        &[
            (s.soft_attack, fixed(10)),
            (s.hardness, 80_000),
            (s.armor_value, fixed(20)),
        ],
        2,
        fixed(50),
    );
    let block = calculate(&s, &groups, &[rifles, tanks], &operands());
    // HP: 3 x 25 + 2 + 2.
    assert_eq!(block.stats[s.max_strength], Some(fixed(79)));
    // Organisation: (3 x 60 + 10 + 20) / 5.
    assert_eq!(block.stats[s.max_organisation], Some(fixed(42)));
    // Soft attack: 3 x 3, + 10, + the support company's 3 at half.
    assert_eq!(
        block.stats[s.soft_attack],
        Some(fixed(9) + fixed(10) + 150_000)
    );
    // Hardness: the tanks' 0.8 over the four non-support cells.
    assert_eq!(block.stats[s.hardness], Some(20_000));
    // Armour: 0.4 of the largest group's 20, 0.6 of the 20 over five cells.
    assert_eq!(
        block.stats[s.armor_value],
        Some(fixed(8) + mul(fixed(4), 60_000))
    );
    // Reliability: each rifle user takes its design's 0.9 over its one requirement, the tanks
    // none; the sum of 3.6 over five cells.
    assert_eq!(block.stats[s.reliability], Some(72_000));
    assert_eq!(block.combat_width, fixed(10));
    assert_eq!(block.manpower, 5000);
    assert_eq!(block.training_time, 90);
    // No design has speed: each group's zero offer is taken, then floored.
    assert_eq!(
        block.stats[s.maximum_speed],
        Some(operands().slowest_speed)
    );
}

/// A support company's grants: an additive grant adds to its targets' base words, a factor
/// scales their equipment terms; a source counted twice grants once.
#[test]
fn support_grants_reach_their_category() {
    let s = slots();
    let line = definition(&[(s.entrenchment, 0)], vec![(1, fixed(100))]);
    let mut engineers = definition(&[], vec![]);
    engineers.support_group = true;
    engineers.support_type = true;
    engineers.categories = vec!["category_support".into()];
    engineers.grants = vec![
        Grant {
            category: "CATEGORY_LINE".into(),
            add: true,
            stats: vec![(s.entrenchment, 20_000)],
        },
        Grant {
            category: "category_line".into(),
            add: false,
            stats: vec![(s.soft_attack, 10_000)],
        },
    ];
    let groups = [
        Group {
            definition: &line,
            count: 2,
            critical: ONE,
        },
        Group {
            definition: &engineers,
            count: 2,
            critical: ONE,
        },
    ];
    let rifles = design(&[(s.soft_attack, fixed(3))], 1, fixed(200));
    let block = calculate(&s, &groups, &[rifles], &operands());
    // 2 x 0.2 entrenchment; 2 x 3 x 1.1 soft attack.
    assert_eq!(block.stats[s.entrenchment], Some(40_000));
    assert_eq!(block.stats[s.soft_attack], Some(660_000));
}

/// Held equipment above the requirement shares out, truncating at each stage; below it, the
/// terms fall with the share; a transport restriction keeps another archetype's strength out.
#[test]
fn held_shares_and_transport() {
    let s = slots();
    let mut line = definition(&[], vec![(1, fixed(100)), (3, fixed(10))]);
    line.transport = Some(3);
    let groups = [Group {
        definition: &line,
        count: 1,
        critical: ONE,
    }];
    let rifles = |amount| {
        design(
            &[(s.soft_attack, fixed(3)), (s.max_strength, fixed(10))],
            1,
            amount,
        )
    };
    let trucks = design(&[(s.max_strength, fixed(5))], 3, fixed(10));
    let over = calculate(
        &s,
        &groups,
        &[rifles(fixed(150)), trucks.clone()],
        &operands(),
    );
    // 100/150 of 150 is 0.99999 of the requirement after truncation.
    assert_eq!(over.stats[s.soft_attack], Some(299_997));
    // Only the trucks' strength passes the restriction.
    assert_eq!(over.stats[s.max_strength], Some(fixed(5)));
    let half = calculate(&s, &groups, &[rifles(fixed(50)), trucks], &operands());
    assert_eq!(half.stats[s.soft_attack], Some(150_000));
}

/// Each group that affects speed offers its equipment speed per sub-unit and the slowest offer
/// wins; a group that does not keeps the previous speed; the floor applies after every group,
/// so a first group that does not affect speed leaves a floor a faster group cannot raise.
#[test]
fn speed_is_the_slowest_offer_floored_after_each_group() {
    let s = slots();
    let line = |archetype| definition(&[], vec![(archetype, fixed(100))]);
    let (fast, slow, carts) = (line(1), line(2), {
        let mut support = line(3);
        support.affects_speed = false;
        support
    });
    let pool = [
        design(&[(s.maximum_speed, fixed(6))], 1, fixed(200)),
        design(&[(s.maximum_speed, fixed(4))], 2, fixed(100)),
        design(&[(s.maximum_speed, fixed(1))], 3, fixed(100)),
    ];
    let group = |definition, count| Group {
        definition,
        count,
        critical: ONE,
    };
    let block = calculate(
        &s,
        &[group(&fast, 2), group(&carts, 1), group(&slow, 1)],
        &pool,
        &operands(),
    );
    assert_eq!(block.stats[s.maximum_speed], Some(fixed(4)));
    let block = calculate(
        &s,
        &[group(&carts, 1), group(&fast, 2)],
        &pool,
        &operands(),
    );
    assert_eq!(block.stats[s.maximum_speed], Some(operands().slowest_speed));
    let empty = calculate(&s, &[], &[], &operands());
    assert_eq!(empty.stats[s.maximum_speed], Some(0));
}

/// A group without critical equipment fills one; otherwise its critical types' holdings and
/// requirements, each weighted by the type's own build cost and truncated, divide as totals,
/// no more than the manpower fill; a holding is clamped to zero and to its requirement.
#[test]
fn critical_fills_weigh_each_type_by_its_build_cost() {
    let mut line = definition(&[], vec![(1, fixed(100)), (2, fixed(10))]);
    let required = [(1, fixed(200)), (2, fixed(20))];
    let rifles = design(&[], 1, fixed(250));
    let cost = |archetype: usize| if archetype == 1 { fixed(5) } else { fixed(2) };
    assert_eq!(
        super::critical_fill(&line, &required, &[rifles.clone()], ONE, &cost),
        ONE
    );
    line.critical = vec![1, 2];
    // Rifles held in full (clamped to 200), trucks not at all: 1000 / (1000 + 40), truncated.
    assert_eq!(
        super::critical_fill(&line, &required, &[rifles.clone()], ONE, &cost),
        96_153
    );
    assert_eq!(
        super::critical_fill(&line, &required, &[rifles.clone()], 50_000, &cost),
        50_000
    );
    // A negative holding counts as none.
    let owed = design(&[], 2, -fixed(5));
    assert_eq!(
        super::critical_fill(&line, &required, &[rifles, owed], ONE, &cost),
        96_153
    );
    // Nothing required: one.
    assert_eq!(super::critical_fill(&line, &[], &[], ONE, &cost), ONE);
}

/// No composition: every computed stat is zero, as the game's reset leaves it.
#[test]
fn an_empty_composition_is_zero() {
    let s = slots();
    let block = calculate(&s, &[], &[], &operands());
    assert_eq!(block.stats[s.max_strength], Some(0));
    assert_eq!(block.stats[s.hardness], Some(0));
    assert_eq!(block.combat_width, 0);
}

#[test]
fn fixed5_arithmetic_truncates_and_guards() {
    assert_eq!(mul(-150_000, 50_000), -75_000);
    assert_eq!(mul(1, 50_000), 0);
    assert_eq!(div(fixed(1), fixed(3)), 33_333);
    assert_eq!(div(-fixed(1), fixed(3)), -33_333);
    assert_eq!(div(5, 0), UNDEFINED);
}

mod bonuses {
    use super::super::definitions::{Block, Bonus, apply, authored, reaching};
    use super::*;
    use dashboard_text::subunit_requirements::{Content, Grant as AuthoredGrant, SubUnit};

    /// A mod's own definition reads every stat word the executable registers, a later word
    /// replacing an earlier one, ignores its other scalar fields, and refuses a grant word it does
    /// not reproduce by name.
    #[test]
    fn a_mod_definition_reads_registered_stats_and_refuses_unknown_grant_words() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for stat metadata");
        let words = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(key, text)| ((*key).to_owned(), (*text).to_owned()))
                .collect()
        };
        let mut unit = SubUnit::default();
        unit.key = "synthetic_ranger".into();
        unit.sheet.words = words(&[
            ("abbreviation", "RNG"),
            ("night_penalty", "-0.25"),
            ("soft_attack", "0.1"),
            ("soft_attack", "-0.2"),
        ]);
        unit.sheet.grants = vec![AuthoredGrant {
            category: "category_line".into(),
            add: true,
            words: words(&[("entrenchment", "0.5"), ("display_as_percentage", "no")]),
        }];
        let index = |name: &str| installed_content_core::equipment::stat_index(name.as_bytes()).unwrap();
        let definition = authored(&Content::default(), &unit, &[]).unwrap();
        assert_eq!(definition.stats[index("night_penalty")], -25_000);
        assert_eq!(definition.stats[index("soft_attack")], -20_000);
        assert_eq!(
            definition.grants[0].stats,
            [(index("entrenchment"), 50_000)]
        );
        unit.sheet.grants[0]
            .words
            .push(("synthetic_unknown".into(), "1".into()));
        let refused = authored(&Content::default(), &unit, &[]).unwrap_err();
        assert!(
            refused.contains("battalion_mult field synthetic_unknown"),
            "{refused}"
        );
    }

    fn subunit() -> SubUnit {
        let mut subunit = SubUnit::default();
        subunit.key = "ranger".into();
        subunit.sheet.categories = vec!["category_light".into(), "category_all".into()];
        subunit
    }

    fn block(target: &str, category: bool, stats: Vec<(usize, i64)>) -> Block {
        Block {
            target: target.into(),
            category,
            stats,
            ..Block::default()
        }
    }

    /// The definition's own block applies first, then its categories' in their order, and only
    /// the first block of each target: a repeated target and a category it lacks do not.
    /// A technology researched at a level applies its words once per level; a level the
    /// recording holds as zero or less applies them once.
    #[test]
    fn a_technology_applies_once_per_researched_level() {
        use super::super::technology_multiplier;
        let s = slots();
        assert_eq!(technology_multiplier(3), fixed(3));
        assert_eq!(technology_multiplier(1), ONE);
        assert_eq!(technology_multiplier(0), ONE);
        assert_eq!(technology_multiplier(-2), ONE);
        let mut target = super::definition(&[], Vec::new());
        let bonus = block("infantry", false, vec![(s.soft_attack, 5_000)]);
        apply(&mut target, &s, &bonus, technology_multiplier(3));
        assert_eq!(target.stats[s.soft_attack], 15_000);
    }

    #[test]
    fn the_first_block_of_each_target_applies_in_order() {
        let bonus = Bonus {
            blocks: vec![
                block("category_all", true, vec![(0, 3)]),
                block("RANGER", false, vec![(0, 1)]),
                block("ranger", false, vec![(0, 100)]),
                block("category_light", true, vec![(0, 2)]),
                block("category_heavy", true, vec![(0, 50)]),
                block("category_light", false, vec![(0, 70)]),
            ],
            multiplier: ONE,
            source: "test".into(),
        };
        let reached: Vec<_> = reaching(&subunit(), &bonus)
            .into_iter()
            .map(|block| block.stats[0].1)
            .collect();
        assert_eq!(reached, [1, 2, 3]);
    }

    /// A technology's words scale by its applied count, fuel words add once, grants merge by
    /// category and kind, and reliability is floored at zero after each block.
    #[test]
    fn blocks_add_scaled_words_and_merge_grants() {
        let s = slots();
        let mut definition = Definition {
            stats: [0; STAT_COUNT],
            combat_width: fixed(2),
            manpower: 0,
            training_time: 0,
            support_type: false,
            support_group: true,
            categories: Vec::new(),
            grants: vec![Grant {
                category: "category_line".into(),
                add: true,
                stats: vec![(s.entrenchment, 10_000)],
            }],
            requirements: Vec::new(),
            critical: Vec::new(),
            transport: None,
            own_fuel_consumption: ONE,
            affects_speed: true,
        };
        let mut bonus = block(
            "ranger",
            false,
            vec![
                (s.max_organisation, fixed(2)),
                (s.fuel_consumption, 5_000),
                (s.reliability, -fixed(1)),
            ],
        );
        bonus.combat_width = -50_000;
        bonus.grants = vec![
            Grant {
                category: "CATEGORY_LINE".into(),
                add: true,
                stats: vec![(s.entrenchment, 10_000), (s.defense, 1_000)],
            },
            Grant {
                category: "category_line".into(),
                add: false,
                stats: vec![(s.defense, 1_000)],
            },
        ];
        apply(&mut definition, &s, &bonus, fixed(3));
        assert_eq!(definition.stats[s.max_organisation], fixed(6));
        assert_eq!(definition.stats[s.fuel_consumption], 5_000);
        assert_eq!(definition.stats[s.reliability], 0);
        assert_eq!(definition.stats[s.build_cost_ic], ONE / 10);
        assert_eq!(definition.combat_width, fixed(2) - 150_000);
        assert_eq!(definition.grants.len(), 2);
        assert_eq!(
            definition.grants[0].stats,
            [(s.entrenchment, 40_000), (s.defense, 3_000)]
        );
        assert_eq!(definition.grants[1].stats, [(s.defense, 3_000)]);
    }
}

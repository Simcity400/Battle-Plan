use super::*;
use content_reader_save::{Decimal, LandCombat, combat_details::LandDetails, production::Equipment};
use replay_data::rows::LandCombatProgressRow;
use replay_data::rows::LandEquipmentLoss;
use replay_data::rows::Recorded;
use replay_data::rows::TableId;
use replay_data::rows::TableRows;
use replay_frame::frame::TableView;

const COMBAT: u64 = 7 << 32 | 62;
const GROUP: u64 = 7 << 32 | 53;
const TEMPLATE: u64 = 4 << 32 | 54;
const UNIT: u64 = 9 << 32 | 51;
const LEADER: u64 = 8 << 32 | 64;
const VARIANT: u64 = 12 << 32 | 70;

fn number(literal: &str) -> Decimal {
    Decimal::parse(literal.as_bytes()).unwrap()
}

fn pool(entries: &[(u64, &str)], allow_zero: bool) -> Pool {
    Pool {
        equipment: entries
            .iter()
            .map(|&(variant, amount)| Equipment {
                variant,
                amount: number(amount),
            })
            .collect(),
        allow_zero_entries: Some(allow_zero),
    }
}

fn saved_observer() -> Observer {
    Observer {
        side: BattleSide {
            manpower: Some(3),
            air_factor: Some(number("0.125")),
            equipment: Some(pool(&[(VARIANT, "3"), (VARIANT + (1 << 32), "0.5")], false)),
            captured_by_enemy: Some(pool(&[(VARIANT, "0"), (VARIANT, "-0.25")], true)),
            recovered: Some(pool(&[(VARIANT, "0.125")], false)),
            modifiers: vec![(11, -2), (11, 4)],
            tags: vec![b"AAA".to_vec(), b"AAA".to_vec(), b"BBB".to_vec()],
            leader: Some(LEADER),
        },
        activities: vec![
            Activity {
                group: GROUP,
                templates: vec![TEMPLATE, TEMPLATE, TEMPLATE + (1 << 32)],
                units: vec![UNIT, UNIT + (1 << 32)],
                strength: vec![number("0.9195"), number("-0.00001")],
                equipment: Some(pool(&[(VARIANT, "0"), (VARIANT + (1 << 32), "0.25")], true)),
                dealer: Some(b"AAA".to_vec()),
                taker: Some(b"BBB".to_vec()),
            },
            Activity {
                group: GROUP + (1 << 32),
                templates: Vec::new(),
                units: Vec::new(),
                strength: Vec::new(),
                equipment: None,
                dealer: Some(b"BBB".to_vec()),
                taker: None,
            },
        ],
        leaders: vec![(LEADER, -1), (LEADER + (1 << 32), 4)],
        damage: vec![
            ObserverDamage {
                from: b"AAA".to_vec(),
                receiver: b"BBB".to_vec(),
                to: b"AAA".to_vec(),
                value: number("1.25"),
            },
            ObserverDamage {
                from: b"BBB".to_vec(),
                receiver: b"AAA".to_vec(),
                to: b"BBB".to_vec(),
                value: number("-0.00001"),
            },
        ],
        total_damage: Some(number("2.5")),
        progress: Some(number("0.25")),
        counters: (0..30).collect(),
        snow: Some(true),
        victory: Some(true),
    }
}

fn fixture() -> (DecodedFrame, Save) {
    let (mut frame, mut saved, ..) = recorder_oracle_support::fixtures::fixture();
    let mut entries = vec![
        (kind::OBSERVER_STATE, 0, [250_000, 25_000, 3, 0, 0]),
        (kind::SIDE, 0, [0, 12_500, LEADER, 0, 0]),
        (kind::POOL, 0, [1, 0, 0, 0, 0]),
        (kind::POOL, 1, [1, 1, 0, 0, 0]),
        (kind::POOL, 2, [1, 0, 0, 0, 0]),
        (kind::EQUIPMENT, 1, [0, VARIANT, 0, 0, 0]),
        (kind::EQUIPMENT, 1, [1, VARIANT, -25_000_i64 as u64, 0, 0]),
        (kind::EQUIPMENT, 2, [0, VARIANT, 12_500, 0, 0]),
        (kind::MODIFIER, 0, [0, 11, -2_i64 as u64, 0, 0]),
        (kind::MODIFIER, 0, [1, 11, 4, 0, 0]),
        (kind::COUNTRY, 0, [0, 4, 0, 0, 0]),
        (kind::COUNTRY, 0, [1, 4, 0, 0, 0]),
        (kind::COUNTRY, 0, [2, 9, 0, 0, 0]),
        (kind::OBSERVER_LEADER, 0, [LEADER, -1_i64 as u64, 0, 0, 0]),
        (kind::OBSERVER_LEADER, 1, [LEADER + (1 << 32), 4, 0, 0, 0]),
        (kind::OBSERVER_DAMAGE, 0, [4, 9, 4, 125_000, 0]),
        (kind::OBSERVER_DAMAGE, 1, [9, 4, 9, -1_i64 as u64, 0]),
        (kind::ACTIVITY, 0, [GROUP, 4, 9, 0, 0]),
        (kind::ACTIVITY_TEMPLATE, 0, [0, TEMPLATE, 0, 0, 0]),
        (kind::ACTIVITY_TEMPLATE, 0, [1, TEMPLATE, 0, 0, 0]),
        (
            kind::ACTIVITY_TEMPLATE,
            0,
            [2, TEMPLATE + (1 << 32), 0, 0, 0],
        ),
        (kind::ACTIVITY_DAMAGE, 0, [0, UNIT, 91_950, 0, 0]),
        (
            kind::ACTIVITY_DAMAGE,
            0,
            [1, UNIT + (1 << 32), -1_i64 as u64, 0, 0],
        ),
        (kind::ACTIVITY_EQUIPMENT, 0, [u64::MAX, 1, 1, 0, 0]),
        (kind::ACTIVITY_EQUIPMENT, 0, [0, VARIANT, 0, 0, 0]),
        (
            kind::ACTIVITY_EQUIPMENT,
            0,
            [1, VARIANT + (1 << 32), 25_000, 0, 0],
        ),
        (kind::ACTIVITY, 1, [GROUP + (1 << 32), 9, 0, 0, 0]),
    ];
    for block in 0..6 {
        entries.push((
            kind::OBSERVER_MODIFIERS,
            block * 5,
            std::array::from_fn(|i| block * 5 + i as u64),
        ));
    }
    let rows = [true, false]
        .into_iter()
        .flat_map(|attacker| {
            entries
                .iter()
                .enumerate()
                .map(move |(ordinal, &(kind, item, values))| {
                    let mut row =
                        CombatLogObserverRow::from_fact(COMBAT, attacker, kind as u8, item, values)
                            .unwrap();
                    row.ordinal = ordinal as u64;
                    row
                })
        })
        .collect();
    *frame.tables[TableId::CombatLogObservers.index()] =
        TableView::Complete(TableRows::CombatLogObservers(rows));
    let equipment = vec![
        LandEquipmentLoss {
            variant: VARIANT,
            amount_raw: 300_000,
        },
        LandEquipmentLoss {
            variant: VARIANT + (1 << 32),
            amount_raw: 50_000,
        },
        LandEquipmentLoss {
            variant: VARIANT,
            amount_raw: 0,
        },
    ];
    *frame.tables[TableId::LandCombatProgress.index()] =
        TableView::Complete(TableRows::LandCombatProgress(vec![LandCombatProgressRow {
            combat: COMBAT,
            attacker_manpower_loss: Recorded::Value(3),
            defender_manpower_loss: Recorded::Value(3),
            attacker_equipment_losses: Recorded::Value(equipment.clone()),
            defender_equipment_losses: Recorded::Value(equipment),
            ..Default::default()
        }]));
    let mut details = LandDetails::default();
    for side in &mut details.sides {
        side.log = Some(saved_observer());
    }
    saved.land_combats.insert(
        COMBAT,
        LandCombat {
            id: COMBAT,
            province: 2,
            day: 0,
            duration: 0,
            attackers: Vec::new(),
            defenders: Vec::new(),
            details,
        },
    );
    (frame, saved)
}

fn run(frame: &DecodedFrame, saved: &Save) -> Report {
    let countries = [
        CountryRow {
            tag: b"AAA".to_vec(),
            ..Default::default()
        },
        CountryRow {
            tag: b"BBB".to_vec(),
            ..Default::default()
        },
    ];
    let links = BTreeMap::from([(4, &countries[0]), (9, &countries[1])]);
    let mut report = Report::default();
    compare(frame, saved, &links, &mut report).unwrap();
    report
}

#[test]
fn observer_comparison_uses_exact_values_across_save_decimal_spellings() {
    let (frame, saved) = fixture();
    assert_ne!(number("3"), decimal(300_000).unwrap());
    let report = run(&frame, &saved);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CombatLogObservers"], 2);
}

#[test]
fn observer_comparison_retains_one_unit_numeric_differences() {
    let (frame, mut saved) = fixture();
    assert!(run(&frame, &saved).differences.is_empty());
    let cases: &[(&str, fn(&mut Observer))] = &[
        ("air factor", |s| {
            s.side.air_factor = Some(number("0.12501"))
        }),
        ("lost equipment", |s| {
            s.side.equipment.as_mut().unwrap().equipment[0].amount = number("3.00001")
        }),
        ("captured zero equipment", |s| {
            s.side.captured_by_enemy.as_mut().unwrap().equipment[0].amount = number("0.00001")
        }),
        ("recovered equipment", |s| {
            s.side.recovered.as_mut().unwrap().equipment[0].amount = number("0.12501")
        }),
        ("activity strength", |s| {
            s.activities[0].strength[0] = number("0.91951")
        }),
        ("activity equipment", |s| {
            s.activities[0].equipment.as_mut().unwrap().equipment[1].amount = number("0.25001")
        }),
        ("attributed damage", |s| {
            s.damage[0].value = number("1.25001")
        }),
        ("negative damage", |s| {
            s.damage[1].value = number("-0.00002")
        }),
        ("total damage", |s| s.total_damage = Some(number("2.50001"))),
        ("progress", |s| s.progress = Some(number("0.25001"))),
    ];
    for &(label, change) in cases {
        let mut observer = saved_observer();
        change(&mut observer);
        saved.land_combats.get_mut(&COMBAT).unwrap().details.sides[0].log = Some(observer);
        let report = run(&frame, &saved);
        assert_eq!(
            report.differences.len(),
            1,
            "{label}: {:?}",
            report.differences
        );
    }
}

#[test]
fn observer_comparison_retains_presence_membership_and_collection_order() {
    let (frame, mut saved) = fixture();
    assert!(run(&frame, &saved).differences.is_empty());
    let cases: &[(&str, fn(&mut Observer))] = &[
        ("manpower", |s| s.side.manpower = Some(4)),
        ("air factor presence", |s| s.side.air_factor = None),
        ("lost pool presence", |s| s.side.equipment = None),
        ("captured pool presence", |s| {
            s.side.captured_by_enemy = None
        }),
        ("recovered pool presence", |s| s.side.recovered = None),
        ("pool policy presence", |s| {
            s.side.equipment.as_mut().unwrap().allow_zero_entries = None
        }),
        ("pool zero policy", |s| {
            s.side
                .captured_by_enemy
                .as_mut()
                .unwrap()
                .allow_zero_entries = Some(false)
        }),
        ("equipment identity", |s| {
            s.side.equipment.as_mut().unwrap().equipment[0].variant += 1
        }),
        ("equipment order", |s| {
            s.side.equipment.as_mut().unwrap().equipment.swap(0, 1)
        }),
        ("repeated equipment membership", |s| {
            s.side
                .captured_by_enemy
                .as_mut()
                .unwrap()
                .equipment
                .remove(0);
        }),
        ("empty pool membership", |s| {
            s.side.recovered.as_mut().unwrap().equipment.clear()
        }),
        ("modifier order", |s| s.side.modifiers.swap(0, 1)),
        ("country order", |s| s.side.tags.swap(0, 2)),
        ("repeated country membership", |s| {
            s.side.tags.remove(0);
        }),
        ("side leader", |s| s.side.leader = None),
        ("activity membership", |s| {
            s.activities.pop();
        }),
        ("activity order", |s| s.activities.swap(0, 1)),
        ("activity group", |s| s.activities[0].group += 1),
        ("repeated template membership", |s| {
            s.activities[0].templates.remove(0);
        }),
        ("template order", |s| s.activities[0].templates.swap(0, 2)),
        ("unit order", |s| s.activities[0].units.swap(0, 1)),
        ("strength order", |s| s.activities[0].strength.swap(0, 1)),
        ("strength membership", |s| {
            s.activities[0].strength.pop();
        }),
        ("activity pool presence", |s| {
            s.activities[0].equipment = None
        }),
        ("absent versus empty activity pool", |s| {
            s.activities[1].equipment = Some(pool(&[], false))
        }),
        ("activity equipment order", |s| {
            s.activities[0]
                .equipment
                .as_mut()
                .unwrap()
                .equipment
                .swap(0, 1)
        }),
        ("activity pool policy", |s| {
            s.activities[0]
                .equipment
                .as_mut()
                .unwrap()
                .allow_zero_entries = None
        }),
        ("activity dealer", |s| s.activities[0].dealer = None),
        ("activity taker", |s| s.activities[0].taker = None),
        ("leader membership", |s| {
            s.leaders.pop();
        }),
        ("leader order", |s| s.leaders.swap(0, 1)),
        ("leader hours", |s| s.leaders[0].1 += 1),
        ("damage membership", |s| {
            s.damage.pop();
        }),
        ("damage order", |s| s.damage.swap(0, 1)),
        ("damage source", |s| s.damage[0].from = b"BBB".to_vec()),
        ("damage receiver", |s| {
            s.damage[0].receiver = b"AAA".to_vec()
        }),
        ("damage target", |s| s.damage[0].to = b"BBB".to_vec()),
        ("total damage presence", |s| s.total_damage = None),
        ("progress presence", |s| s.progress = None),
        ("counter value", |s| s.counters[0] += 1),
        ("counter membership", |s| {
            s.counters.pop();
        }),
        ("snow presence", |s| s.snow = None),
        ("victory truth", |s| s.victory = Some(false)),
    ];
    for &(label, change) in cases {
        let mut observer = saved_observer();
        change(&mut observer);
        saved.land_combats.get_mut(&COMBAT).unwrap().details.sides[0].log = Some(observer);
        let report = run(&frame, &saved);
        assert_eq!(
            report.differences.len(),
            1,
            "{label}: {:?}",
            report.differences
        );
    }
    saved.land_combats.get_mut(&COMBAT).unwrap().details.sides[0].log = None;
    assert_eq!(run(&frame, &saved).differences.len(), 1);
    saved.land_combats.clear();
    assert_eq!(run(&frame, &saved).differences.len(), 2);
}

#[test]
fn observer_comparison_reconstructs_native_order_from_noncontiguous_stable_labels() {
    let (mut frame, saved) = fixture();
    let TableView::Complete(TableRows::CombatLogObservers(rows)) =
        &mut *frame.tables[TableId::CombatLogObservers.index()]
    else {
        unreachable!()
    };
    for row in rows.iter_mut() {
        row.ordinal = 1_000_000 - row.ordinal * 13;
    }
    rows.sort_unstable_by_key(CombatLogObserverRow::key);
    let report = run(&frame, &saved);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CombatLogObservers"], 2);
}

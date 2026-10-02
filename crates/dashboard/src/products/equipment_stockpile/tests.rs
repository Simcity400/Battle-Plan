use super::*;
use crate::test_fixtures::{CAMPAIGN, records, replay, stockpile_frame};
use replay::frame::{DecodedFrame, TableView};
use replay::rows::TableRows;

/// Equipment from one authored file.
fn types(text: &[u8]) -> Database {
    installed_content::equipment::fixture::load(&[(
        "common/units/equipment/fixture.txt",
        std::str::from_utf8(text).unwrap(),
    )])
    .0
}

#[test]
fn native_equipment_redefinitions_and_diagnostics_are_kept() {
    let units = types(
        b"equipments = { guns = { is_archetype = maybe } tanks = { is_archetype = yes type = armor } TANKS = { is_archetype = yes type = fighter } }",
    );
    assert!(units.lost.is_none());
    assert_eq!(
        archetype_of(&units, "tanks"),
        Ok(("tanks".into(), 1, Domain::Air))
    );
    assert!(
        archetype_of(&units, "guns")
            .unwrap_err()
            .contains("no known archetype")
    );
    let unknown = types(b"duplicate_archetypes = { copy = { archetype = missing } }");
    assert!(
        equipment_warnings(&unknown)
            .iter()
            .any(|note| note.contains("copy") && note.contains("missing"))
    );
    assert!(archetype_of(&unknown, "copy").is_err());
}

#[test]
fn a_variant_s_type_groups_by_its_archetype_in_database_order_with_its_domain() {
    let units = types(
        br#"
        equipments = {
            infantry_equipment = { is_archetype = yes type = infantry }
            infantry_equipment_1 = { archetype = infantry_equipment }
            fighter_equipment = { is_archetype = yes type = fighter }
            fighter_equipment_0 = { archetype = fighter_equipment }
            convoy = { is_archetype = yes type = convoy }
        }
    "#,
    );
    // The recorded spelling may differ in ASCII case from the authored key.
    assert_eq!(
        archetype_of(&units, "Infantry_Equipment_1"),
        Ok(("infantry_equipment".to_owned(), 0, Domain::Land))
    );
    assert_eq!(
        archetype_of(&units, "fighter_equipment_0"),
        Ok(("fighter_equipment".to_owned(), 2, Domain::Air))
    );
    assert_eq!(
        archetype_of(&units, "convoy").map(|(_, _, domain)| domain),
        Ok(Domain::Naval)
    );
    assert_eq!(
        archetype_of(&units, "mod_gun"),
        Err("mod_gun is not authored equipment".to_owned())
    );
}

/// A mod layer adds its own archetypes (one in a new arm for a vanilla
/// chassis via `duplicate_archetypes`), and replaces a vanilla file whole,
/// dropping `supply_truck` from trucks. Types, arms, database order and need
/// rules follow the recorded content stack, never vanilla names.
#[test]
fn a_mod_s_equipment_types_resolve_through_the_recorded_content_stack() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_dashboard_mod_equipment_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let write = |layer: &str, path: &str, text: &str| {
        let path = root.join(layer).join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        "base",
        "common/units/equipment/infantry.txt",
        "equipments = { infantry_equipment = { is_archetype = yes type = infantry }
           infantry_equipment_1 = { archetype = infantry_equipment }
           motorized_equipment = { is_archetype = yes type = motorized supply_truck = yes } }",
    );
    write(
        "base",
        "common/units/equipment/tanks.txt",
        "equipments = { light_tank_chassis = { is_archetype = yes type = armor } }",
    );
    write(
        "mod",
        "common/units/equipment/infantry.txt",
        "equipments = { infantry_equipment = { is_archetype = yes type = infantry }
           infantry_equipment_1 = { archetype = infantry_equipment }
           motorized_equipment = { is_archetype = yes type = motorized } }",
    );
    write(
        "mod",
        "common/units/equipment/mod_walkers.txt",
        "equipments = { mod_walker_equipment = { is_archetype = yes type = armor }
           mod_walker_equipment_1 = { archetype = mod_walker_equipment }
           mod_airship_equipment = { is_archetype = yes type = strategic_bomber } }",
    );
    // Copies load after what they copy: the game reads the folder in path order.
    write(
        "mod",
        "common/units/equipment/x_mod_duplicates.txt",
        "duplicate_archetypes = { mod_sky_tank = { archetype = light_tank_chassis type = fighter } }",
    );
    let layers = installed_content::ResolvedLayers::from_folders(&[
        (&root.join("base"), &[][..]),
        (&root.join("mod"), &[][..]),
    ])
    .unwrap();
    let units = Database::load_complete(&layers).unwrap();
    // The mod's own types, by the recorded spelling, in the arm their category puts them.
    let (walker, _, walker_domain) = archetype_of(&units, "MOD_WALKER_EQUIPMENT_1").unwrap();
    assert_eq!(
        (walker.as_str(), walker_domain),
        ("mod_walker_equipment", Domain::Land)
    );
    assert_eq!(
        archetype_of(&units, "mod_airship_equipment").unwrap().2,
        Domain::Air
    );
    // A vanilla chassis cloned into a new arm by the mod.
    assert_eq!(archetype_of(&units, "mod_sky_tank").unwrap().2, Domain::Air);
    // Database order follows the stack's files, the mod's among them.
    let archetypes: Vec<_> = units
        .equipment
        .iter()
        .filter(|kind| kind.is_archetype)
        .collect();
    let place = |key: &str| {
        archetypes
            .iter()
            .position(|item| item.name == key.as_bytes())
            .unwrap()
    };
    assert!(place("motorized_equipment") < place("mod_walker_equipment"));
    assert!(place("mod_airship_equipment") < place("light_tank_chassis"));
    // The mod's trucks drop supply_truck, so their need is derived; its own types' too.
    assert_eq!(need_note(&units, "motorized_equipment").unwrap(), None);
    assert_eq!(need_note(&units, "mod_walker_equipment").unwrap(), None);
    // A token no layer declares is refused by name, never assumed.
    assert_eq!(
        archetype_of(&units, "vanilla_only_equipment"),
        Err("vanilla_only_equipment is not authored equipment".to_owned())
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn need_is_derived_only_where_the_available_stock_is_the_recorded_stock() {
    let units = types(
        br#"
        equipments = {
            infantry_equipment = { is_archetype = yes type = infantry }
            motorized_equipment = { is_archetype = yes type = motorized supply_truck = yes }
            mechanized_equipment = { is_archetype = yes type = mechanized supply_truck = no }
            train_equipment = { is_archetype = yes type = train }
            convoy = { is_archetype = yes type = convoy }
        }
    "#,
    );
    let note = |key| need_note(&units, key).unwrap();
    assert_eq!(note("infantry_equipment"), None);
    assert_eq!(note("mechanized_equipment"), None);
    assert!(note("motorized_equipment").unwrap().contains("trucks"));
    assert!(note("train_equipment").unwrap().contains("trains"));
    assert!(note("convoy").unwrap().contains("convoy"));
}

/// The stockpile slot's hour on day `day`.
fn stockpile_slot(day: i32) -> i32 {
    let replay::cadence::Cadence::Daily(slot) = replay::cadence::cadence(TableId::Stockpiles)
    else {
        panic!("Stockpiles is read in a daily slot");
    };
    day * replay::cadence::STAGGER_PERIOD + slot as i32
}

/// A stockpile frame at the logistics slot's hour `hour` whose LogisticsEquipment
/// table holds `rows` (country link, archetype token, balance, stock); `read`
/// times the table's walk; `None` records it as unavailable. Token 1 is
/// `infantry_equipment_1`'s spelling in the fixture, used here as an archetype token.
fn logistics_frame(hour: i32, read: bool, rows: Option<&[(u32, u32, i64, i64)]>) -> DecodedFrame {
    use replay::rows::LogisticsEquipmentRow;
    let mut frame = stockpile_frame(hour, false, &[], Some(&[]));
    *frame.tables[TableId::LogisticsEquipment.index()] = match rows {
        Some(rows) => TableView::Complete(TableRows::LogisticsEquipment(
            rows.iter()
                .enumerate()
                .map(
                    |(index, &(country, token, balance_raw, stock_raw))| LogisticsEquipmentRow {
                        country,
                        archetype: index as u32,
                        token,
                        balance_raw,
                        stock_raw,
                    },
                )
                .collect(),
        )),
        None => TableView::Unavailable {
            reason: "fixture".into(),
        },
    };
    if read {
        frame.metrics.table_ns[TableId::LogisticsEquipment.index()] = 1;
    }
    frame
}

#[test]
fn production_reads_are_dated_apart_from_the_stockpile() {
    use crate::products::equipment_stockpile::scan;
    let slot = stockpile_slot(1000);
    // The slot read has lines but no stockpile table.
    let mut frame = stockpile_frame(slot, true, &[], Some(&[(10, 360_045, 2_250_000, 0)]));
    *frame.tables[TableId::Stockpiles.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let scanned = scan(
        &replay(vec![
            frame,
            stockpile_frame(slot + 12, false, &[], Some(&[])),
        ]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(scanned.reads.is_empty());
    assert_eq!(scanned.line_reads, [slot]);
}

#[test]
fn a_deferred_intel_walk_leaves_the_logistics_record_carried() {
    use crate::products::equipment_stockpile::scan;
    let day = 1000 * replay::cadence::STAGGER_PERIOD;
    // The retry hour holds different rows (a later in-memory state), but only
    // CountryIntel was walked there: the slot read's values stay, carried.
    let mut retry = logistics_frame(day + 1, false, Some(&[(11, 1, 9 * 100_000, 9 * 100_000)]));
    retry.metrics.table_ns[TableId::CountryIntel.index()] = 1;
    let frames = vec![
        logistics_frame(day, true, Some(&[(11, 1, -300 * 100_000, 50 * 100_000)])),
        retry,
    ];
    let scanned = scan(&replay(frames), 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.logistics_reads, [day]);
    assert_eq!(
        scanned.balance.runs[&1]
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.amount_raw.clone()))
            .collect::<Vec<_>>(),
        [(day, day + 1, Some("-30000000".to_owned()))]
    );
}

#[test]
fn logistics_balances_are_known_for_player_reads_and_ai_reads_are_marked_apart() {
    use crate::products::{country_groups::HourSpan, equipment_stockpile::scan};
    let day = 1000 * replay::cadence::STAGGER_PERIOD;
    let frames = vec![
        // Germany (link 11) short 300 of token 1 with 50 in stock; Italy's row is not Germany's.
        logistics_frame(
            day,
            true,
            Some(&[(11, 1, -300 * 100_000, 50 * 100_000), (21, 1, 7, 7)]),
        ),
        logistics_frame(day + 1, false, Some(&[])),
        // The next read has Germany AI-controlled: no rows for it.
        logistics_frame(day + 2, true, Some(&[(21, 1, 7, 7)])),
        logistics_frame(day + 3, false, Some(&[])),
        // A missing table ends both.
        logistics_frame(day + 4, false, None),
        logistics_frame(day + 5, true, Some(&[(9001, 1, 0, 20 * 100_000)])),
    ];
    let replay = replay(frames);
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let span = |first_hour, last_hour| HourSpan {
        first_hour,
        last_hour,
    };
    assert_eq!(scanned.logistics_reads, [day, day + 5]);
    assert_eq!(
        scanned.balance.spans,
        [span(day, day + 1), span(day + 5, day + 5)]
    );
    assert_eq!(scanned.ai.spans, [span(day + 2, day + 3)]);
    let runs = |runs: &crate::read_runs::ReadRuns<u32>| {
        runs.runs[&1]
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.amount_raw.clone()))
            .collect::<Vec<_>>()
    };
    // A recorded zero balance is a value, told apart from an archetype never listed.
    assert_eq!(
        runs(&scanned.balance),
        [
            (day, day + 1, Some("-30000000".to_owned())),
            (day + 5, day + 5, Some("0".to_owned()))
        ]
    );
    assert_eq!(
        runs(&scanned.logistics_stock),
        [
            (day, day + 1, Some("5000000".to_owned())),
            (day + 5, day + 5, Some("2000000".to_owned()))
        ]
    );
    assert_eq!(scanned.logistics_tokens[&1], "infantry_equipment_1");
}

#[test]
fn the_stockpile_reply_carries_the_logistics_record_and_the_ai_hours() {
    use crate::products::{
        CountryParams, LoadContext, Product,
        country_groups::HourSpan,
        equipment_stockpile::{EquipmentStockpile, NEED_BOUND_RAW},
    };
    let day = 1000 * replay::cadence::STAGGER_PERIOD;
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![
                    logistics_frame(day, true, Some(&[(11, 1, -300 * 100_000, 50 * 100_000)])),
                    logistics_frame(day + 1, true, Some(&[])),
                ],
            )],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "logistics".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = EquipmentStockpile::load(
        &source,
        &CountryParams {
            branch_id: "1".into(),
            tag: "GER".into(),
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    // Without content the archetype keeps its recorded spelling, marked unresolved
    // with a mark settings can store.
    assert_eq!(reply.logistics.len(), 1);
    assert_eq!(reply.logistics[0].key, ":infantry_equipment_1");
    // Saved variant choices are scoped by the campaign the metadata names.
    assert_eq!(
        reply.campaign_key,
        crate::map_settings::campaign_key(&source.replay())
    );
    assert!(!reply.campaign_key.is_empty());
    assert_eq!(
        reply.logistics[0].balance[0].amount_raw.as_deref(),
        Some("-30000000")
    );
    assert_eq!(reply.logistics_reads, [day]);
    assert_eq!(
        reply.balance_hours,
        [HourSpan {
            first_hour: day,
            last_hour: day
        }]
    );
    assert_eq!(
        reply.ai_hours,
        [HourSpan {
            first_hour: day + 1,
            last_hour: day + 1
        }]
    );
    assert_eq!(reply.need_bound_raw, NEED_BOUND_RAW.to_string());
}

#[test]
fn stockpile_values_start_at_reads_carry_until_the_next_and_never_cross_gaps() {
    use crate::products::{country_groups::HourSpan, equipment_stockpile::scan};
    use engine::cequipment_production_line::{KIND_MILITARY, KIND_NAVAL};
    let slot = stockpile_slot(1000);
    let lines: &[(u64, i64, i64, u8)] = &[
        // 3.60045 IC a day at 22.5 IC a unit: 1.12014 a week.
        (10, 360_045, 2_250_000, KIND_MILITARY),
        (10, 360_045, 2_250_000, KIND_MILITARY),
        // A naval line's output is not established, so variant 30's weekly total is
        // unknown, never zero. Variant 20 has a line with a
        // rate and a costless one, whose rate the game does not compute: its weekly
        // total is unknown, not the other line's alone.
        (30, 500_000, 1_000_000, KIND_NAVAL),
        (20, 360_045, 2_250_000, KIND_MILITARY),
        (20, 100_000, 0, KIND_MILITARY),
    ];
    let frames = vec![
        // A variant listed twice holds the sum.
        stockpile_frame(
            slot,
            true,
            &[(10, 500_000), (20, 100_000), (10, 50_000)],
            Some(lines),
        ),
        // Carried: values seen off a read are not a read.
        stockpile_frame(slot + 1, false, &[(10, 1)], Some(&[])),
        // A read without the lines table: stock goes on, production is unknown.
        stockpile_frame(slot + 2, true, &[(10, 550_000)], None),
        stockpile_frame(slot + 3, false, &[], None),
        // After a gap, everything starts again.
        stockpile_frame(slot + 5, true, &[(20, 100_000)], Some(&[])),
    ];
    let replay = replay(frames);
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let runs = |runs: &crate::read_runs::ReadRuns<u64>, key| {
        runs.runs.get(&key).map_or_else(Vec::new, |runs| {
            runs.iter()
                .map(|run| (run.first_hour, run.last_hour, run.amount_raw.clone()))
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(scanned.reads, [slot, slot + 2, slot + 5]);
    assert_eq!(
        runs(&scanned.stock, 10),
        [(slot, slot + 3, Some("550000".to_owned()))]
    );
    assert_eq!(
        runs(&scanned.stock, 20),
        [
            (slot, slot + 1, Some("100000".to_owned())),
            (slot + 5, slot + 5, Some("100000".to_owned()))
        ]
    );
    let span = |first_hour, last_hour| HourSpan {
        first_hour,
        last_hour,
    };
    assert_eq!(
        scanned.stock.spans,
        [span(slot, slot + 3), span(slot + 5, slot + 5)]
    );
    // Two lines of one variant add their weekly output; production is known only
    // where the lines were read.
    assert_eq!(
        runs(&scanned.weekly, 10),
        [(slot, slot + 1, Some("224028".to_owned()))]
    );
    assert_eq!(runs(&scanned.weekly, 20), [(slot, slot + 1, None)]);
    assert_eq!(runs(&scanned.weekly, 30), [(slot, slot + 1, None)]);
    assert_eq!(scanned.other_kinds, 1);
    assert_eq!(
        scanned.weekly.spans,
        [span(slot, slot + 1), span(slot + 5, slot + 5)]
    );
    assert_eq!(scanned.costless, 1);
}

#[test]
fn the_stockpile_replies_without_content_by_variant_and_says_when_it_has_no_logistics() {
    use crate::products::{
        CountryParams, LoadContext, Product, country_groups::HourSpan,
        equipment_stockpile::EquipmentStockpile,
    };
    let slot = stockpile_slot(1000);
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![
                    stockpile_frame(slot, true, &[(10, 500_000), (20, 100_000)], Some(&[])),
                    stockpile_frame(slot + 1, false, &[], Some(&[])),
                ],
            )],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "stockpile".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = EquipmentStockpile::load(
        &source,
        &CountryParams {
            branch_id: "1".into(),
            tag: "GER".into(),
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.decimals, 5);
    // Without content no type is resolved; the variants keep their tokens, and a
    // variant with its own name is named without localisation.
    assert!(reply.types.is_empty());
    assert_eq!(
        reply
            .variants
            .iter()
            .map(|variant| (
                variant.id.as_str(),
                variant.token.as_str(),
                variant.archetype.is_none(),
                variant.names.last().and_then(|name| name.name.as_deref())
            ))
            .collect::<Vec<_>>(),
        [
            ("10", "infantry_equipment_1", true, None),
            ("20", "infantry_equipment_1", true, Some("Gewehr 43"))
        ]
    );
    assert_eq!(reply.variants[0].stock[0].last_hour, slot + 1);
    assert_eq!(
        reply.stock_hours,
        [HourSpan {
            first_hour: slot,
            last_hour: slot + 1
        }]
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("The surplus or shortage is not in this recording")),
        "{:?}",
        reply.warnings
    );
}

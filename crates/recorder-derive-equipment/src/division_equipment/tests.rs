use super::*;
use crate::stock::Stockpile;
use recorder_derive_framework::{
    framework::Definitions,
    testing::deployed::{Edits, Run, division, world},
    testing::noting::{self, Noted, Noting},
};
use content_reader_key_names::unit_leader::needs::SubUnitNeeds;
use replay_data::rows::{
    CountryLocalSubunitRow, DivisionEquipmentCountRow, DivisionEquipmentDesignRow, DivisionEquipmentRow, DivisionEquipmentTemplateRow, DivisionEquipmentTypeRow, DivisionLandInputsRow,
    DivisionManpowerContributor, DivisionPanelStatsRow, DivisionTemplateIconRow, LexicalTokenRow, LocalSubunitRequirement, Recorded, StockpileRow, TemplateSettings, TemplateSubunit,
    VariantDefinitionRow,
};
use std::sync::Arc;

const INPUTS: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Divisions,
    TableId::DivisionEquipment,
    TableId::NewDivisionEquipment,
    TableId::DivisionTemplateIcons,
    TableId::DivisionLandInputs,
    TableId::DivisionPanelStats,
    TableId::CountryLocalSubunits,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::Stockpiles,
];

fn make() -> Vec<Box<dyn Writer>> {
    vec![Box::new(Stockpile::default()), Box::new(DivisionEquipment::default())]
}

const INFANTRY: u32 = 1;
const ENGINEER: u32 = 2;
const RIFLES: u32 = 50;
const SUPPORT: u32 = 51;
const TEMPLATE: u64 = 7;
const UNIT: i64 = 100_000;

fn handle(id: u32) -> u64 {
    u64::from(id) << 32 | u64::from(engine_native::cequipment_variant::REFERENCE_TYPE)
}

fn needs() -> Definitions {
    Definitions::read_none().with_needs(Ok(SubUnitNeeds {
        needs: [
            ("infantry".to_owned(), vec![("infantry_equipment".to_owned(), 100 * UNIT)]),
            ("engineer".to_owned(), vec![("support_equipment".to_owned(), 10 * UNIT), ("Infantry_Equipment".to_owned(), UNIT)]),
        ]
        .into(),
        dropped: Vec::new(),
    }))
}

fn token(token: u32, literal: &str) -> LexicalTokenRow {
    LexicalTokenRow { token, literal: literal.as_bytes().to_vec() }
}

fn unit(token: u32, local: u8, count: i32) -> TemplateSubunit {
    TemplateSubunit { token, local, count }
}

fn template(handle: u64, units: Vec<TemplateSubunit>) -> DivisionTemplateIconRow {
    DivisionTemplateIconRow {
        template: handle,
        subunits: units,
        country: 10,
        settings: Recorded::Value(Some(Arc::new(TemplateSettings { total_manpower: 3000, ..Default::default() }))),
        ..Default::default()
    }
}

fn entry(division: u64, ordinal: u32, variant: u32, amount: i64) -> DivisionEquipmentRow {
    DivisionEquipmentRow { division, template: TEMPLATE, country: 10, ordinal, variant_handle: handle(variant), amount_raw: amount }
}

fn land(division: u64, manpower: i32) -> DivisionLandInputsRow {
    DivisionLandInputsRow { division, manpower: vec![DivisionManpowerContributor { country: 10, amount: manpower }], ..Default::default() }
}

fn panel(division: u64, maximum: i64) -> DivisionPanelStatsRow {
    DivisionPanelStatsRow { division, status_maxima: [Some(maximum), None], ..Default::default() }
}

fn hp(id: u64, hit_points: i64) -> replay_data::rows::DivisionRow {
    replay_data::rows::DivisionRow { hit_points, ..division(id, 10, 10, 0, TEMPLATE) }
}

fn begin(edits: &mut Edits) -> &mut Edits {
    world(edits)
        .put(TableId::LexicalTokens, token(INFANTRY, "infantry"))
        .put(TableId::LexicalTokens, token(ENGINEER, "engineer"))
        .put(TableId::LexicalTokens, token(RIFLES, "infantry_equipment"))
        .put(TableId::LexicalTokens, token(SUPPORT, "support_equipment"))
        .put(TableId::VariantDefinitions, VariantDefinitionRow { id: handle(1), archetype_token: RIFLES, ..Default::default() })
        .put(TableId::VariantDefinitions, VariantDefinitionRow { id: handle(2), archetype_token: SUPPORT, ..Default::default() })
        .put(TableId::DivisionTemplateIcons, template(TEMPLATE, vec![unit(INFANTRY, 0, 2), unit(0, 0, 9), unit(ENGINEER, 0, 1)]))
        .put(TableId::Divisions, hp(100, 900_000))
        .put(TableId::Divisions, hp(101, 800_000))
        .put(TableId::DivisionEquipment, entry(100, 0, 1, 250 * UNIT))
        .put(TableId::DivisionEquipment, entry(100, 1, 2, 5 * UNIT))
        .put(TableId::DivisionEquipment, entry(101, 0, 1, 100 * UNIT))
        .put(TableId::DivisionEquipment, entry(101, 3, 2, 20 * UNIT))
        .put(TableId::DivisionLandInputs, land(100, 1000))
        .put(TableId::DivisionLandInputs, land(101, 2000))
        .put(TableId::DivisionPanelStats, panel(100, 1_000_000))
        .put(TableId::DivisionPanelStats, panel(101, 1_000_000))
        .put(TableId::Stockpiles, StockpileRow { country: 10, ordinal: 0, variant_handle: handle(1), amount_raw: 70 * UNIT })
        .put(TableId::Stockpiles, StockpileRow { country: 11, ordinal: 0, variant_handle: handle(2), amount_raw: 10 * UNIT })
}

fn value(cell: &Recorded<i64>) -> Option<i64> {
    match cell {
        Recorded::Value(value) => Some(*value),
        Recorded::NotRecorded => None,
    }
}

fn counts(run: &Run) -> Vec<(u32, u8, [Option<i64>; 3], Option<i32>, Option<i32>)> {
    run.rows::<DivisionEquipmentCountRow>(TableId::DivisionEquipmentCounts)
        .into_iter()
        .map(|row| (row.slot, row.side, [value(&row.divisions), value(&row.short), value(&row.unknown)], row.pool_hour, row.stock_hour))
        .collect()
}

fn causes(run: &Run, slot: u32, side: u8) -> (Option<i64>, Option<i64>, i64) {
    let row = run.rows::<DivisionEquipmentCountRow>(TableId::DivisionEquipmentCounts).into_iter().find(|row| (row.slot, row.side) == (slot, side)).unwrap();
    (value(&row.unpooled), value(&row.unclassified), row.unresolved)
}

fn lacking(run: &Run, slot: u32, side: u8) -> Vec<(Option<i64>, Option<i64>)> {
    run.rows::<DivisionEquipmentTemplateRow>(TableId::DivisionEquipmentByTemplate)
        .into_iter()
        .filter(|row| (row.slot, row.side) == (slot, side))
        .map(|row| (value(&row.manpower_lacking), value(&row.hp_max_lacking)))
        .collect()
}

fn types(run: &Run, slot: u32, side: u8) -> Vec<(Option<u32>, [Option<i64>; 6])> {
    run.rows::<DivisionEquipmentTypeRow>(TableId::DivisionEquipmentByType)
        .into_iter()
        .filter(|row| (row.slot, row.side) == (slot, side))
        .map(|row| (row.archetype, [&row.held, &row.needed, &row.filled, &row.missing, &row.short, &row.stock].map(value)))
        .collect()
}

fn designs(run: &Run, slot: u32, side: u8) -> Vec<(Option<u32>, Option<u32>, Option<i64>)> {
    run.rows::<DivisionEquipmentDesignRow>(TableId::DivisionEquipmentByDesign)
        .into_iter()
        .filter(|row| (row.slot, row.side) == (slot, side))
        .map(|row| (row.archetype, row.variant, value(&row.held)))
        .collect()
}

fn templates(run: &Run, slot: u32, side: u8) -> Vec<(u64, [Option<i64>; 9], String)> {
    run.rows::<DivisionEquipmentTemplateRow>(TableId::DivisionEquipmentByTemplate)
        .into_iter()
        .filter(|row| (row.slot, row.side) == (slot, side))
        .map(|row| {
            let cells = [&row.divisions, &row.unknown, &row.types_short, &row.manpower, &row.required, &row.hp, &row.hp_max, &row.needed, &row.held].map(value);
            (row.template, cells, String::from_utf8(row.need_missing).unwrap())
        })
        .collect()
}

fn tables(run: &Run) -> String {
    format!(
        "{:?} {:?} {:?} {:?}",
        run.rows::<DivisionEquipmentCountRow>(TableId::DivisionEquipmentCounts),
        run.rows::<DivisionEquipmentTypeRow>(TableId::DivisionEquipmentByType),
        run.rows::<DivisionEquipmentDesignRow>(TableId::DivisionEquipmentByDesign),
        run.rows::<DivisionEquipmentTemplateRow>(TableId::DivisionEquipmentByTemplate)
    )
}

#[test]
fn each_division_is_capped_at_its_own_need_before_the_slot_sums_and_its_designs_and_stock_are_kept() {
    let mut run = Run::new(make, INPUTS);
    run.define(needs());
    let mut edits = Edits::default();
    begin(&mut edits);
    run.hour(&edits);
    let owner = DivisionEquipmentCountRow::OWNER;
    assert_eq!(counts(&run)[0], (1, owner, [Some(2), Some(2), Some(0)], Some(0), Some(0)));
    assert_eq!(
        types(&run, 1, owner),
        [
            (Some(RIFLES), [Some(350 * UNIT), Some(402 * UNIT), Some(301 * UNIT), Some(101 * UNIT), Some(1), Some(70 * UNIT)]),
            (Some(SUPPORT), [Some(25 * UNIT), Some(20 * UNIT), Some(15 * UNIT), Some(5 * UNIT), Some(1), Some(10 * UNIT)]),
        ],
        "filled is 201 + 100 rifles and 5 + 10 support, not the pools' 350 and 25"
    );
    assert_eq!(designs(&run, 1, owner), [(Some(RIFLES), Some(1), Some(350 * UNIT)), (Some(SUPPORT), Some(2), Some(25 * UNIT))]);
    assert_eq!(
        templates(&run, 1, owner),
        [(TEMPLATE, [Some(2), Some(0), Some(2), Some(3000), Some(6000), Some(1_700_000), Some(2_000_000), Some(422 * UNIT), Some(316 * UNIT)], String::new())]
    );
    assert!(types(&run, 2, owner).is_empty() && counts(&run).iter().any(|row| row.0 == 3), "a country with no division has its counts");
    assert_eq!(causes(&run, 1, owner), (Some(0), Some(0), 0));
    assert_eq!(lacking(&run, 1, owner), [(Some(0), Some(0))]);
    let mut edits = Edits::default();
    edits
        .put(TableId::DivisionTemplateIcons, template(TEMPLATE, vec![unit(INFANTRY, 0, 2), unit(ENGINEER, 1, 1)]))
        .put(TableId::CountryLocalSubunits, CountryLocalSubunitRow { country: 11, token: ENGINEER, requirements: vec![LocalSubunitRequirement { archetype_token: RIFLES, amount_raw: 4 * UNIT }] });
    run.hour(&edits);
    let rifles = types(&run, 1, owner)[0].1;
    assert_eq!(rifles[1..4], [Some(408 * UNIT), Some(304 * UNIT), Some(104 * UNIT)], "needed 2 x 204, filled 204 + 100");
    assert_eq!(types(&run, 1, owner)[1].1[1..4], [Some(0), Some(0), Some(0)], "support is held, no longer needed");
}

#[test]
fn without_the_needs_definition_the_comparison_is_not_recorded_by_name_and_held_stays_known() {
    let mut run = Run::new(make, INPUTS);
    let mut edits = Edits::default();
    begin(&mut edits);
    run.hour(&edits);
    let owner = DivisionEquipmentCountRow::OWNER;
    assert_eq!(counts(&run)[0].2, [Some(2), Some(0), Some(2)]);
    assert_eq!(types(&run, 1, owner)[0].1[..5], [Some(350 * UNIT), None, None, None, Some(0)]);
    let why = &templates(&run, 1, owner)[0].2;
    assert!(why.contains("sub-unit equipment needs are not held") && why.contains("not packed yet"), "{why}");
    run.define(needs());
    let mut edits = Edits::default();
    edits.put(TableId::Divisions, hp(100, 850_000));
    run.hour(&edits);
    assert_eq!(counts(&run)[0].2, [Some(2), Some(2), Some(0)]);
    assert_eq!(templates(&run, 1, owner)[0].1[5], Some(1_650_000), "hit points moved alone");
    let mut edits = Edits::default();
    edits.put(TableId::DivisionEquipment, entry(101, 4, 3, UNIT));
    run.hour(&edits);
    assert_eq!(counts(&run)[0].2, [Some(2), Some(1), Some(1)]);
    assert_eq!(causes(&run, 1, owner), (Some(0), Some(1), 0), "the unknown one holds an unclassified design");
    assert_eq!(types(&run, 1, owner)[0], (None, [Some(UNIT), None, None, None, Some(0), None]));
    assert_eq!(designs(&run, 1, owner)[0], (None, Some(3), Some(UNIT)));
    for hour in 3..24 {
        let mut edits = Edits::default();
        edits.put(TableId::Divisions, hp(100, 850_000 - hour));
        run.hour(&edits);
    }
    assert_eq!(counts(&run)[0].3, Some(2), "the read at hour 2 stands for a day");
    for hour in 24..27 {
        let mut edits = Edits::default();
        edits.put(TableId::Divisions, hp(100, 850_000 - hour));
        run.hour(&edits);
    }
    assert_eq!(counts(&run)[0], (1, owner, [Some(2), Some(0), Some(2)], None, None), "neither the pools nor the stockpile are read within a day");
    assert_eq!(types(&run, 1, owner)[0].1, [Some(0), None, None, None, Some(0), None], "no pool is read");
    assert_eq!(causes(&run, 1, owner), (Some(2), Some(0), 0));
}

#[test]
fn an_owner_no_link_resolves_makes_its_side_unknown_and_a_resume_writes_what_an_unbroken_run_writes() {
    let mut run = Run::new(make, INPUTS);
    run.define(needs());
    let mut edits = Edits::default();
    begin(&mut edits).put(TableId::Divisions, replay_data::rows::DivisionRow { hit_points: 1, ..division(102, 10, 10, 99, TEMPLATE) });
    run.hour(&edits);
    let (owner, host) = (DivisionEquipmentCountRow::OWNER, DivisionEquipmentCountRow::HOST);
    let rows = counts(&run);
    assert_eq!(rows.iter().find(|row| (row.0, row.1) == (1, owner)).unwrap().2, [None, None, None]);
    assert_eq!(rows.iter().find(|row| (row.0, row.1) == (1, host)).unwrap().2, [Some(3), Some(3), Some(0)], "102 has no daily rows and the minted pools read complete: an empty pool");
    assert!(types(&run, 1, owner).iter().all(|(_, cells)| cells.iter().all(Option::is_none)));
    assert_eq!(causes(&run, 1, owner), (None, None, 1), "how many no link resolves stays known");
    assert_eq!(causes(&run, 2, owner).2, 1, "in every slot's row");
    assert_eq!(lacking(&run, 1, host), [(Some(1), Some(1))], "102 has no land inputs or panel row");
    let mut resumed = run.resumed();
    let mut edits = Edits::default();
    edits.delete(TableId::Divisions, 102).delete(TableId::Divisions, 101).put(TableId::DivisionEquipment, entry(100, 1, 2, 15 * UNIT));
    run.hour(&edits);
    resumed.hour(&edits);
    assert_eq!(counts(&run)[0], (1, owner, [Some(1), Some(0), Some(0)], Some(1), Some(0)));
    assert_eq!(types(&run, 1, owner)[1].1[..5], [Some(15 * UNIT), Some(10 * UNIT), Some(10 * UNIT), Some(0), Some(0)]);
    assert_eq!(tables(&run), tables(&resumed));
    let mut edits = Edits::default();
    edits.put(TableId::NewDivisionEquipment, entry(103, 0, 1, 7 * UNIT)).put(TableId::Divisions, hp(103, 500_000));
    run.hour(&edits);
    resumed.hour(&edits);
    assert_eq!(counts(&run)[0].2, [Some(2), Some(1), Some(0)], "a minted division's pool is the hourly read's, 7 rifles of 201");
    assert_eq!(tables(&run), tables(&resumed));
}

#[test]
fn daily_reads_stand_for_a_day_from_their_sample_hours_and_outlive_a_resume() {
    let mut run = Run::new(make, INPUTS);
    run.define(needs());
    let mut edits = Edits::default();
    begin(&mut edits);
    run.hour(&edits);
    let owner = DivisionEquipmentCountRow::OWNER;
    assert_eq!(run.store.view(TableId::Stockpiles.index()).sampled_at(), Some(0));
    assert_eq!(counts(&run)[0], (1, owner, [Some(2), Some(2), Some(0)], Some(0), Some(0)));
    assert_eq!(types(&run, 1, owner)[0].1[5], Some(70 * UNIT));
    let mut resumed = run.resumed();
    let mut edits = Edits::default();
    edits.put(TableId::Stockpiles, StockpileRow { country: 10, ordinal: 0, variant_handle: handle(1), amount_raw: 80 * UNIT }).unstamped(TableId::DivisionEquipment);
    run.hour(&edits);
    resumed.hour(&edits);
    assert_eq!((counts(&run)[0].3, counts(&run)[0].4), (Some(0), Some(1)), "the stockpile read at 1, the pools' at 0 still standing");
    assert_eq!(types(&run, 1, owner)[0].1[5], Some(80 * UNIT));
    assert_eq!(tables(&run), tables(&resumed));
    for hour in 2..24 {
        let mut edits = Edits::default();
        edits.put(TableId::Divisions, hp(100, 900_000 - hour)).unstamped(TableId::DivisionEquipment).unstamped(TableId::Stockpiles);
        run.hour(&edits);
        resumed.hour(&edits);
    }
    assert_eq!((counts(&run)[0].3, counts(&run)[0].4), (Some(0), Some(1)), "the reads stand for a day");
    let mut edits = Edits::default();
    edits.put(TableId::Divisions, hp(100, 800_000)).unstamped(TableId::DivisionEquipment).unstamped(TableId::Stockpiles);
    run.hour(&edits);
    resumed.hour(&edits);
    assert_eq!((counts(&run)[0].3, counts(&run)[0].4), (None, Some(1)), "the pools' read of hour 0 no longer stands at 24");
    assert_eq!(tables(&run), tables(&resumed));
}

fn noted() -> Vec<Box<dyn Writer>> {
    vec![Box::new(Stockpile::default()), Box::new(Noting(DivisionEquipment::default()))]
}

const OTHER: u64 = 8;

fn crowd(edits: &mut Edits, count: u64) -> &mut Edits {
    edits.put(TableId::DivisionTemplateIcons, template(OTHER, vec![unit(INFANTRY, 0, 2)]));
    for at in 0..count {
        let (ger, ita) = (1000 + at, 100_000 + at);
        edits
            .put(TableId::Divisions, hp(ger, 900_000))
            .put(TableId::DivisionEquipment, entry(ger, 0, 1, 150 * UNIT))
            .put(TableId::Divisions, replay_data::rows::DivisionRow { hit_points: 900_000, ..division(ita, 20, 20, 0, OTHER) })
            .put(TableId::DivisionEquipment, DivisionEquipmentRow { division: ita, template: OTHER, country: 20, ordinal: 0, variant_handle: handle(1), amount_raw: 210 * UNIT });
    }
    edits
}

fn slot_of(table: TableId, key: u128) -> u32 {
    let shift = match table {
        TableId::DivisionEquipmentCounts => 8,
        TableId::DivisionEquipmentByType => 41,
        TableId::DivisionEquipmentByDesign => 74,
        _ => 72,
    };
    (key >> shift) as u32
}

#[test]
fn one_changed_division_among_many_sets_only_the_rows_it_touches() {
    let mut run = Run::new(noted, INPUTS);
    run.define(needs());
    let mut edits = Edits::default();
    crowd(begin(&mut edits), 50);
    run.hour(&edits);
    noting::take();
    let mut edits = Edits::default();
    edits.put(TableId::Divisions, hp(100, 700_000));
    run.hour(&edits);
    let (owner, host) = (DivisionEquipmentCountRow::OWNER, DivisionEquipmentCountRow::HOST);
    let hit_points: Noted = vec![(TableId::DivisionEquipmentByTemplate, vec![template_key(1, owner, TEMPLATE), template_key(1, host, TEMPLATE)], Vec::new())];
    assert_eq!(noting::take(), [hit_points]);
    assert_eq!(templates(&run, 1, owner)[0].1[5], Some(1_500_000 + 50 * 900_000));
    let mut edits = Edits::default();
    edits.put(TableId::DivisionEquipment, entry(101, 0, 1, 90 * UNIT));
    run.hour(&edits);
    let noted = noting::take();
    assert_eq!(noted.len(), 1);
    for (table, sets, deletes) in &noted[0] {
        if *table != TableId::DivisionEquipmentCounts {
            assert!(sets.iter().chain(deletes).all(|&key| slot_of(*table, key) == 1), "{table:?} sets {sets:?}");
        }
    }
    let templates_set = noted[0].iter().find(|(table, ..)| *table == TableId::DivisionEquipmentByTemplate).map(|(_, sets, _)| sets.clone());
    assert_eq!(templates_set, Some(vec![template_key(1, owner, TEMPLATE), template_key(1, host, TEMPLATE)]));
    run.hour(&Edits::default());
    assert_eq!(noting::take(), [Noted::new()]);
}

#[test]
fn tokens_definitions_and_clones_move_only_what_reads_them_as_a_rebuild_would() {
    let mut run = Run::new(noted, INPUTS);
    run.define(needs());
    let mut edits = Edits::default();
    crowd(begin(&mut edits), 3).put(TableId::DivisionEquipment, entry(101, 4, 3, UNIT));
    run.hour(&edits);
    let local = |requirements: Vec<LocalSubunitRequirement>| CountryLocalSubunitRow { country: 11, token: ENGINEER, requirements };
    let mut steps: Vec<(Edits, bool)> = Vec::new();
    let mut step = |quiet: bool, edit: &dyn Fn(&mut Edits)| {
        let mut edits = Edits::default();
        edit(&mut edits);
        steps.push((edits, quiet));
    };
    step(true, &|edits: &mut Edits| {
        edits.put(TableId::VariantDefinitions, VariantDefinitionRow { id: handle(9), archetype_token: RIFLES, ..Default::default() });
    });
    step(true, &|edits: &mut Edits| {
        edits.put(TableId::LexicalTokens, token(60, "artillery_equipment"));
    });
    step(false, &|edits: &mut Edits| {
        edits.put(TableId::VariantDefinitions, VariantDefinitionRow { id: handle(3), archetype_token: SUPPORT, ..Default::default() });
    });
    step(true, &|edits: &mut Edits| {
        edits.put(TableId::DivisionTemplateIcons, template(TEMPLATE, vec![unit(INFANTRY, 0, 2), unit(ENGINEER, 1, 1)]));
    });
    step(false, &|edits: &mut Edits| {
        edits.put(TableId::CountryLocalSubunits, local(vec![LocalSubunitRequirement { archetype_token: RIFLES, amount_raw: 4 * UNIT }]));
    });
    step(false, &|edits: &mut Edits| {
        edits.put(TableId::CountryLocalSubunits, local(vec![LocalSubunitRequirement { archetype_token: RIFLES, amount_raw: 4 * UNIT }, LocalSubunitRequirement { archetype_token: RIFLES, amount_raw: UNIT }]));
    });
    step(false, &|edits: &mut Edits| {
        edits.delete(TableId::CountryLocalSubunits, 11 << 32 | u128::from(ENGINEER));
    });
    step(false, &|edits: &mut Edits| {
        edits.delete(TableId::VariantDefinitions, u128::from(handle(3)));
    });
    for (at, (edits, quiet)) in steps.iter().enumerate() {
        let mut resumed = run.resumed();
        noting::take();
        run.hour(edits);
        resumed.hour(edits);
        assert_eq!(tables(&run), tables(&resumed), "step {at}");
        let noted = noting::take();
        assert_eq!(noted.len(), 2, "step {at}");
        assert_eq!(noted[0], noted[1], "step {at}: the rebuild sets what the unbroken run sets");
        assert_eq!(noted[0].is_empty(), *quiet, "step {at}: {noted:?}");
    }
    assert_eq!(templates(&run, 1, DivisionEquipmentCountRow::OWNER)[0].2, "", "the clone deleted, the census complete: the global engineer");
}

#[cfg(feature = "memory-labels")]
mod labelled {
    use super::*;
    use engine_allocation::labels;

}

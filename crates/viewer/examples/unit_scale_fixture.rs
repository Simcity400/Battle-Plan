use replay::campaign::{Replay, SeekResult};
use replay_capture_spool::spool::Record;
use replay_data::{
    rows::CountryLinkRow, rows::LandCombatRow, rows::LandCombatSideRow, rows::TableId,
    rows::TableRows,
};
use replay_frame::{branches::BranchReason, frame::FrameEncoder, frame::TableView};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs, io,
};

const COMBAT_SHARE_PERCENT: usize = 20;
const SIDE_DIVISIONS: usize = 4;

fn main() -> io::Result<()> {
    let args: Vec<_> = env::args().collect();
    let replay = Replay::open(fs::read(&args[1])?)?;
    let count: usize = args[3].parse().unwrap();
    let hours: i32 = args[4].parse().unwrap();
    let stress = args.get(5).is_some_and(|v| v == "stress");
    let division_count = if stress { 100_000 } else { 50_000 };
    let wing_count = division_count;
    let ship_count = if stress { 50_000 } else { 20_000 };
    let first = replay.index[0].first_hour;
    let mut frame = match replay.seek(1, first + 1)? {
        SeekResult::Frame(frame) => frame,
        SeekResult::Gap { reason, .. } => panic!("source gap: {reason}"),
    };
    let TableView::Complete(TableRows::Countries(countries)) =
        &mut *frame.tables[TableId::Countries.index()]
    else {
        panic!("countries missing")
    };
    let original = countries.clone();
    assert!(count >= original.len());
    let max_id = countries.iter().map(|r| r.id).max().unwrap();
    let max_slot = countries.iter().map(|r| r.canonical_slot).max().unwrap();
    let mut sources = BTreeMap::new();
    for i in 0..count - original.len() {
        let mut country = original[i % original.len()].clone();
        let source = country.id;
        country.id = max_id + 1 + i as u32;
        country.canonical_slot = max_slot + 1 + i as u32;
        country.tag = format!("SYN{i}").into_bytes();
        sources.insert(country.id, source);
        countries.push(country);
    }
    let country_rows = countries.clone();
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    else {
        panic!("links missing")
    };
    links.extend(
        country_rows[original.len()..]
            .iter()
            .map(|r| CountryLinkRow {
                id: r.id,
                canonical_slot: r.canonical_slot,
            }),
    );
    let TableView::Complete(TableRows::CountryAppearance(appearance)) =
        &mut *frame.tables[TableId::CountryAppearance.index()]
    else {
        panic!("appearance missing")
    };
    let templates: BTreeMap<_, _> = appearance.iter().map(|r| (r.country, r.clone())).collect();
    for (&id, source) in &sources {
        let mut row = templates.get(source).expect("source appearance").clone();
        row.country = id;
        row.overlord = 0;
        if row.name_tag.is_empty() {
            row.name_tag = original
                .iter()
                .find(|r| r.id == *source)
                .unwrap()
                .tag
                .clone();
        }
        appearance.push(row);
    }
    appearance.sort_by_key(|r| r.country);
    let TableView::Complete(TableRows::Provinces(provinces)) =
        &mut *frame.tables[TableId::Provinces.index()]
    else {
        panic!("provinces missing")
    };
    let land = provinces
        .iter()
        .filter(|p| p.state != 0 && p.owner != 0)
        .count();
    assert!(
        land >= count,
        "fixture needs at least one recorded land province per country"
    );
    for (at, province) in provinces
        .iter_mut()
        .filter(|p| p.state != 0 && p.owner != 0)
        .enumerate()
    {
        let country = &country_rows[at * count / land];
        province.owner = country.id;
        province.controller = country.id;
    }
    frame.tables[TableId::RaidTargets.index()] = TableView::Unavailable {
        reason: "raid inputs excluded from the unit-scale fixture (#63)".into(),
    }
    .into();

    use replay_data::rows::RowCodec;
    let mut division_copies = BTreeMap::<u64, Vec<u64>>::new();
    let mut ship_copies = BTreeMap::<u64, Vec<u64>>::new();
    let mut wing_copies = BTreeMap::<u64, Vec<u64>>::new();
    macro_rules! population {
        ($table:ident, $count:expr, $copies:ident) => {{
            let TableView::Complete(TableRows::$table(rows)) =
                &mut *frame.tables[TableId::$table.index()]
            else {
                panic!("missing population {}", stringify!($table))
            };
            let source = rows.clone();
            assert!(!source.is_empty() && source.len() <= $count);
            let first_id = source.iter().map(|row| row.id >> 32).max().unwrap() + 1;
            assert!(first_id + ($count - source.len()) as u64 <= u64::from(u32::MAX));
            for i in source.len()..$count {
                let mut row = source[i % source.len()].clone();
                let old = row.id;
                row.id = ((first_id + (i - source.len()) as u64) << 32) | (old & 0xffff_ffff);
                $copies.entry(old).or_default().push(row.id);
                rows.push(row);
            }
            rows.sort_by_key(RowCodec::key);
        }};
    }
    population!(Divisions, division_count, division_copies);
    population!(Ships, ship_count, ship_copies);
    population!(AirWings, wing_count, wing_copies);
    macro_rules! companions {
        ($table:ident, $field:ident, $copies:ident) => {
            if let TableView::Complete(TableRows::$table(rows)) =
                &mut *frame.tables[TableId::$table.index()]
            {
                let source = rows.clone();
                for row in &source {
                    for &id in $copies.get(&row.$field).into_iter().flatten() {
                        let mut row = row.clone();
                        row.$field = id;
                        rows.push(row);
                    }
                }
                rows.sort_by_key(RowCodec::key);
            }
        };
    }
    companions!(DivisionMovement, division, division_copies);
    companions!(DivisionStatus, division, division_copies);
    companions!(ArmyIconInputs, division, division_copies);
    companions!(UnitIntel, unit, division_copies);
    companions!(DivisionEquipment, division, division_copies);
    companions!(NewDivisionEquipment, division, division_copies);
    companions!(ShipModels, ship, ship_copies);
    companions!(WingEquipment, wing, wing_copies);
    companions!(WingCumulative, id, wing_copies);
    if let TableView::Complete(TableRows::ProvinceUnitOrder(rows)) =
        &mut *frame.tables[TableId::ProvinceUnitOrder.index()]
    {
        for row in rows {
            let old = row.mixed.clone();
            for unit in old {
                row.mixed
                    .extend(division_copies.get(&unit).into_iter().flatten());
            }
        }
    }
    if let TableView::Complete(TableRows::AirBaseRegistrations(rows)) =
        &mut *frame.tables[TableId::AirBaseRegistrations.index()]
    {
        for row in rows {
            let old = row.wings.clone();
            for wing in old {
                row.wings
                    .extend(wing_copies.get(&wing).into_iter().flatten());
            }
        }
    }
    if let TableView::Complete(TableRows::AirWings(rows)) =
        &mut *frame.tables[TableId::AirWings.index()]
    {
        for row in rows {
            row.planes = 10;
        }
    }
    let TableView::Complete(TableRows::Divisions(division_rows)) =
        &*frame.tables[TableId::Divisions.index()]
    else {
        panic!("divisions missing")
    };
    let owners: BTreeMap<u64, (u32, u32)> = division_rows
        .iter()
        .map(|row| (row.id, (row.logical_owner, row.roster_owner)))
        .collect();
    let TableView::Complete(TableRows::LandCombats(recorded)) =
        &*frame.tables[TableId::LandCombats.index()]
    else {
        panic!("land combats missing")
    };
    let mut combats = recorded.clone();
    let recorded_sides: BTreeMap<u64, LandCombatSideRow> =
        match &*frame.tables[TableId::LandCombatSides.index()] {
            TableView::Complete(TableRows::LandCombatSides(rows))
            | TableView::Truncated {
                rows: TableRows::LandCombatSides(rows),
                ..
            } => rows.iter().map(|row| (row.combat, row.clone())).collect(),
            _ => BTreeMap::new(),
        };
    let canonical: BTreeMap<u32, u32> = match &*frame.tables[TableId::CountryLinks.index()] {
        TableView::Complete(TableRows::CountryLinks(rows)) => rows
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect(),
        _ => panic!("links missing"),
    };
    let fighting: BTreeSet<u64> = combats
        .iter()
        .flat_map(|combat| combat.attackers.iter().chain(&combat.defenders).copied())
        .collect();
    let needed = (division_count * COMBAT_SHARE_PERCENT / 100).saturating_sub(fighting.len());
    let mut free: Vec<_> = division_rows
        .iter()
        .filter(|row| !fighting.contains(&row.id))
        .map(|row| (row.logical_owner, row.id, row.province))
        .collect();
    free.sort_unstable();
    assert!(free.len() >= needed);
    let chosen: Vec<_> = free
        .iter()
        .step_by((free.len() / needed.max(1)).max(1))
        .take(needed)
        .collect();
    let (attackers, defenders) = chosen.split_at(chosen.len() / 2);
    let shape = combats.first().cloned().unwrap_or_default();
    let first_id = combats.iter().map(|combat| combat.id).max().unwrap_or(0) + 1;
    let added: Vec<_> = attackers
        .chunks(SIDE_DIVISIONS)
        .zip(defenders.chunks(SIDE_DIVISIONS))
        .zip(first_id..)
        .map(|((attackers, defenders), id)| LandCombatRow {
            id,
            province: defenders[0].2,
            attacker_classes: vec![0; attackers.len()],
            attackers: attackers.iter().map(|division| division.1).collect(),
            defenders: defenders.iter().map(|division| division.1).collect(),
            ..shape.clone()
        })
        .collect();
    let added_count = added.len();
    combats.extend(added);
    let in_combat: usize = combats
        .iter()
        .map(|combat| combat.attackers.len() + combat.defenders.len())
        .sum();
    let same = |a: u32, b: u32| {
        a == b
            || (a != 0
                && b != 0
                && canonical
                    .get(&a)
                    .is_some_and(|slot| canonical.get(&b) == Some(slot)))
    };
    let lists = |units: &[u64]| {
        let (mut primary, mut secondary) = (Vec::<u32>::new(), Vec::<u32>::new());
        for &(logical, roster) in units.iter().filter_map(|unit| owners.get(unit)) {
            if !primary.iter().any(|&listed| same(listed, logical)) {
                primary.push(logical);
            }
            if !same(logical, roster)
                && (roster as i32) > 0
                && !secondary.iter().any(|&listed| same(listed, roster))
            {
                secondary.push(roster);
            }
        }
        (primary, secondary)
    };
    let sides = |combats: &[LandCombatRow], base: &[LandCombatRow]| -> Vec<LandCombatSideRow> {
        combats
            .iter()
            .zip(base)
            .map(|(combat, base)| {
                let recorded = recorded_sides.get(&combat.id);
                if let Some(row) = recorded.filter(|_| combat == base) {
                    return row.clone();
                }
                let (attacker_primary, attacker_secondary) = lists(&combat.attackers);
                let (defender_primary, defender_secondary) = lists(&combat.defenders);
                LandCombatSideRow {
                    combat: combat.id,
                    attacker_primary,
                    attacker_secondary,
                    defender_primary,
                    defender_secondary,
                    ..recorded.cloned().unwrap_or_default()
                }
            })
            .collect()
    };
    println!(
        "unit fixture divisions={division_count} ships={ship_count} wings={wing_count} planes={} countries={count}; clones preserve source owners, locations and activity; companion handles remapped; source order memberships unchanged",
        wing_count * 10
    );
    println!(
        "land combats={} in_combat={in_combat} share={:.3} recorded_combats={}; every tenth combat drops its last attacker on odd hours and regains it on even ones",
        combats.len(),
        in_combat as f64 / division_count as f64,
        recorded.len()
    );
    println!(
        "marker coverage incomplete: added_combats={added_count} without LandCombatDetails or LandCombatDivisions rows (a seen one draws no tint) and with the out-of-combat DivisionMovement operands their divisions were cloned with; recorded combats keep their recorded side operands"
    );
    let mut branch = replay.branches[0].clone();
    branch.first_hour = first;
    branch.reason = BranchReason::LoadSignal;
    branch.loaded_hour = Some(first);
    let mut records = vec![
        Record::SessionStart(replay.sessions[0].clone()),
        Record::BranchStart(branch),
    ];
    let mut encoder = FrameEncoder::new();
    for offset in 0..hours {
        let mut hour = (*frame).clone();
        hour.hour = first + offset;
        let TableView::Complete(TableRows::Clock(clock)) =
            &mut *hour.tables[TableId::Clock.index()]
        else {
            panic!("clock missing")
        };
        clock[0].hour = hour.hour;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *hour.tables[TableId::Countries.index()]
        {
            for country in rows {
                country.command_power_raw += i64::from(offset);
            }
        }
        if let TableView::Complete(TableRows::Divisions(rows)) =
            &mut *hour.tables[TableId::Divisions.index()]
        {
            for row in rows.iter_mut().step_by(10) {
                row.organisation += i64::from(offset);
            }
        }
        if let TableView::Complete(TableRows::Ships(rows)) =
            &mut *hour.tables[TableId::Ships.index()]
        {
            for row in rows.iter_mut().step_by(10) {
                row.organisation += i64::from(offset);
            }
        }
        let mut hour_combats = combats.clone();
        if offset % 2 == 1 {
            for combat in hour_combats
                .iter_mut()
                .skip(offset as usize % 10)
                .step_by(10)
            {
                if combat.attackers.len() > 1 {
                    combat.attackers.pop();
                }
            }
        }
        hour.tables[TableId::LandCombatSides.index()] =
            TableView::Complete(TableRows::LandCombatSides(sides(&hour_combats, &combats))).into();
        hour.tables[TableId::LandCombats.index()] =
            TableView::Complete(TableRows::LandCombats(hour_combats)).into();
        records.push(Record::Frame(encoder.encode(hour, offset == 0)?));
    }
    let output = Replay::from_records(recorder_derive::writers(), replay.campaign_id, &[records])?;
    fs::write(&args[2], output.bytes())?;
    println!(
        "synthetic countries={count} hours={hours} first_hour={first} land_provinces={land} bytes={} source_country_count={}",
        output.bytes().len(),
        original.len()
    );
    Ok(())
}

//! Synthetic performance input from a supplied recording; not a native fidelity oracle.
use replay::{
    container::{Replay, SeekResult},
    frame::{FrameEncoder, TableView},
    rows::{CountryLinkRow, TableId, TableRows},
    spool::Record,
};
use std::{collections::BTreeMap, env, fs, io};

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
    let SeekResult::Frame(mut frame) = replay.seek(1, first + 1)? else {
        panic!("source gap")
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
        // Appearance facts and localisation come from the recorded source country.
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
    // Exclude the old raid inputs from synthetic measurements, without changing viewer code.
    for table in [TableId::RaidProvinces, TableId::RaidTargets] {
        frame.tables[table.index()] = TableView::Unavailable {
            reason: "raid inputs excluded from the unit-scale fixture (#63)".into(),
        }
        .into();
    }

    use replay::rows::RowCodec;
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
    companions!(DivisionEtaInputs, division, division_copies);
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
            row.effective_planes = 10;
        }
    }
    println!(
        "unit fixture divisions={division_count} ships={ship_count} wings={wing_count} planes={} countries={count}; clones preserve source owners, locations and activity; companion handles remapped; source combat lists and order memberships unchanged",
        wing_count * 10
    );
    let mut branch = replay.branches[0].clone();
    branch.first_hour = first;
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
        // Hourly country changes invalidate keys without inventing extra assets or units.
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *hour.tables[TableId::Countries.index()]
        {
            for country in rows {
                country.command_power_raw += i64::from(offset);
            }
        }
        // Exercise invalidation without inventing movement geometry or combat
        // membership. This is value churn, not a simulated busy game tick.
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
        records.push(Record::Frame(encoder.encode(hour, offset == 0)?));
    }
    let output = Replay::from_records(replay.campaign_id.clone(), &[records])?;
    fs::write(&args[2], output.bytes())?;
    println!(
        "synthetic countries={count} hours={hours} first_hour={first} land_provinces={land} bytes={} source_country_count={}",
        output.bytes().len(),
        original.len()
    );
    Ok(())
}

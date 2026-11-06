//! The family's columnar preparation held to its row preparation ([`super::row`]) on the same
//! hours, written as the native recorder writes them: every country's totals and each holder's
//! records of its partners, the hours known, the tags and the refusals, across segment starts, a
//! gap, cuts and a resume, as the replies answer them.

use super::{
    LendLeaseBalanceParams,
    emit::{Emit, SCHEMA},
    facts::{self, Facts, Projector},
    query, row,
};
use crate::products::test_support::{fixture, frame as row_frame, set};
use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_series::{
    Blob, Budget, Cache, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        CountryLinkRow, CountryRow, RowCodec, TABLE_COUNT, TableId, TableRows,
        WarLendLeaseHistoryRow, WarOwnedRelationRow,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::atomic::AtomicBool;

/// One hour as the recorder writes it: the countries (id, slot, tag), none where unavailable; the
/// links (raw id, slot); the rosters (owner, ordinal, first, second) and the histories (country,
/// partner, industry sent, fuel received), none where unavailable.
#[derive(Clone, PartialEq)]
struct Tables {
    hour: i32,
    countries: Option<Vec<(u32, u32, &'static str)>>,
    links: Vec<(u32, u32)>,
    relations: Option<Vec<(u32, u32, u32, u32)>>,
    histories: Option<Vec<(u32, u32, i64, i64)>>,
}

/// Hours 100 to 140 of four countries, without 120 and 121. AAA records sending to BBB, more each
/// hour through 105, and BBB receiving from AAA. AAA records CCC from 104 to 106, dropped then
/// (zero while AAA is on the rosters); a second raw id of AAA's slot records BBB at 108 and 109; a
/// record no link owns at 110 and 111 (no zero is proven); AAA records a partner no link names at
/// 112 and 113; the histories are unavailable at 115 and the countries at 125; BBB is renamed at
/// 128; CCC records DDD from 130, whose id is linked only from 132; CCC's id is linked to an
/// empty slot from 134; the rosters are unavailable from 135 to 137, and AAA's war ends at 138.
fn scenario() -> Vec<Tables> {
    (100..=140)
        .filter(|hour| !(120..=121).contains(hour))
        .map(|hour| {
            let b = if hour >= 128 { "BBX" } else { "BBB" };
            let countries = (hour != 125)
                .then(|| vec![(1, 10, "AAA"), (2, 20, b), (3, 30, "CCC"), (4, 40, "DDD")]);
            let c = if hour >= 134 { 31 } else { 30 };
            let mut links = vec![(1, 10), (2, 20), (3, c), (11, 10)];
            if hour >= 132 {
                links.push((4, 40));
            }
            links.sort_unstable();
            let mut relations = vec![(2, 0, 2, 4)];
            if hour < 138 {
                relations.push((1, 0, 1, 3));
            }
            relations.sort_unstable();
            let mut histories = vec![(1, 2, i64::from(hour.min(105) - 99) * 100, 0), (2, 1, 0, 7)];
            if (104..=106).contains(&hour) {
                histories.push((1, 3, 50, 0));
            }
            if (108..=109).contains(&hour) {
                histories.push((11, 2, 9, 0));
            }
            if (110..=111).contains(&hour) {
                histories.push((9, 2, 5, 0));
            }
            if (112..=113).contains(&hour) {
                histories.push((1, 9, 4, 0));
            }
            if hour >= 130 {
                histories.push((3, 4, 6, 0));
            }
            histories.sort_unstable_by_key(|&(country, target, ..)| (country, target));
            Tables {
                hour,
                countries,
                links,
                relations: (!(135..=137).contains(&hour)).then_some(relations),
                histories: (hour != 115).then_some(histories),
            }
        })
        .collect()
}

fn country_rows(countries: &[(u32, u32, &str)]) -> Vec<CountryRow> {
    countries
        .iter()
        .map(|&(id, canonical_slot, tag)| CountryRow {
            id,
            canonical_slot,
            tag: tag.as_bytes().to_vec(),
            ..Default::default()
        })
        .collect()
}

fn link_rows(links: &[(u32, u32)]) -> Vec<CountryLinkRow> {
    links
        .iter()
        .map(|&(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
        .collect()
}

fn relation_rows(relations: &[(u32, u32, u32, u32)]) -> Vec<WarOwnedRelationRow> {
    relations
        .iter()
        .map(|&(owner, ordinal, first_country, second_country)| WarOwnedRelationRow {
            owner,
            ordinal,
            first_country,
            second_country,
            ..Default::default()
        })
        .collect()
}

fn history_rows(histories: &[(u32, u32, i64, i64)]) -> Vec<WarLendLeaseHistoryRow> {
    histories
        .iter()
        .map(|&(country, target, industry_sent, fuel_received)| WarLendLeaseHistoryRow {
            country,
            target,
            industry_sent,
            fuel_sent: 0,
            industry_received: 0,
            fuel_received,
        })
        .collect()
}

const REFUSAL: &str = "synthetic refusal";

/// The hour as the row recording holds it.
fn frame(tables: &Tables) -> DecodedFrame {
    let mut frame = row_frame(tables.hour);
    let unavailable = || TableView::Unavailable {
        reason: REFUSAL.into(),
    };
    match &tables.countries {
        Some(countries) => set(&mut frame, TableRows::Countries(country_rows(countries))),
        None => *frame.tables[TableId::Countries.index()] = unavailable(),
    }
    set(&mut frame, TableRows::CountryLinks(link_rows(&tables.links)));
    match &tables.relations {
        Some(relations) => set(&mut frame, TableRows::WarOwnedRelations(relation_rows(relations))),
        None => *frame.tables[TableId::WarOwnedRelations.index()] = unavailable(),
    }
    match &tables.histories {
        Some(histories) => set(
            &mut frame,
            TableRows::WarLendLeaseHistories(history_rows(histories)),
        ),
        None => *frame.tables[TableId::WarLendLeaseHistories.index()] = unavailable(),
    }
    frame
}

fn write<R: RowCodec>(walk: &mut Synthetic, table: TableId, rows: Option<&[R]>) {
    let (read, sample) = (true, Sample::FromReadBit);
    let input = match rows {
        Some(rows) => Input::Snapshot {
            rows,
            partial: None,
            read,
            sample,
        },
        None => Input::Unavailable {
            reason: REFUSAL,
            read,
            sample,
        },
    };
    walk.table(table, input).unwrap();
}

/// The projected hours of `hours`, a projector starting cold at each of `starts` and at each
/// walk's first hour (after a gap, the walk starts again).
fn projected(hours: &[Tables], starts: &[i32]) -> Vec<facts::Hour> {
    let mut out = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut last: Option<i32> = None;
    let mut projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
    for (ordinal, tables) in (0..).zip(hours) {
        let gap = last.is_some_and(|last| last + 1 != tables.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, tables.hour).unwrap());
        }
        last = Some(tables.hour);
        let walk = walk.as_mut().unwrap();
        let countries = tables.countries.as_deref().map(country_rows);
        write(walk, TableId::Countries, countries.as_deref());
        write(walk, TableId::CountryLinks, Some(&link_rows(&tables.links)[..]));
        let relations = tables.relations.as_deref().map(relation_rows);
        write(walk, TableId::WarOwnedRelations, relations.as_deref());
        let histories = tables.histories.as_deref().map(history_rows);
        write(walk, TableId::WarLendLeaseHistories, histories.as_deref());
        let cold = starts.contains(&tables.hour);
        if cold {
            projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
        }
        walk.end(|state, manifest, decoded, first| {
            let starts = first || cold;
            let world = dashboard_world::World::at(
                &mut world,
                starts,
                &facts::needs(),
                (FormatVersion::CURRENT, CaptureMode::Full),
                state,
                manifest,
            )
            .unwrap();
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: decoded.hour,
                        run: 0,
                        ordinal,
                        starts,
                        opens: starts,
                        state,
                        manifest,
                        world,
                    },
                    &mut out,
                )
                .unwrap();
        })
        .unwrap();
    }
    // As its emit is handed them: written to the spool and read back.
    replay_spool::respool(out).unwrap()
}

/// The row preparation of `hours`, reading the hours the columnar walk reads: those whose tables
/// changed, and each segment's first.
fn row_prepared(hours: &[Tables], starts: &[i32]) -> row::Prepared {
    let mut prepared = row::Prepared::default();
    let mut before: Option<&Tables> = None;
    for tables in hours {
        let opens = starts.contains(&tables.hour)
            || before.is_none_or(|before| before.hour + 1 != tables.hour);
        let mut changed = [opens; TABLE_COUNT];
        if let Some(before) = before.filter(|_| !opens) {
            changed[TableId::Countries.index()] = before.countries != tables.countries;
            changed[TableId::CountryLinks.index()] = before.links != tables.links;
            changed[TableId::WarOwnedRelations.index()] = before.relations != tables.relations;
            changed[TableId::WarLendLeaseHistories.index()] = before.histories != tables.histories;
        }
        let changes = FrameChanges::observed(changed, [false; TABLE_COUNT], [false; TABLE_COUNT], None);
        prepared.observe_changes(&frame(tables), &changes).unwrap();
        before = Some(tables);
    }
    prepared
}

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [7; 32],
        content: [8; 32],
        branch: [9; 32],
        schema: SCHEMA,
    }
}

/// A publication built from projected hours, cut after each of `cuts`' hours; when `resume`
/// names one of them, the emit is saved there and goes on from the publication.
struct Built {
    pages: Memory,
    root: u64,
}

fn build(hours: Vec<facts::Hour>, cuts: &[i32], resume: Option<i32>, budget: &Budget) -> Built {
    let cache = Cache::new(budget);
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(budget, identity(), first).unwrap();
    let mut emit = Emit::new();
    let mut root = 0;
    let mut generation = 0;
    let mut hours = hours.into_iter().peekable();
    for &cut in cuts {
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.hour(&mut writer, hour).unwrap();
        }
        emit.cut(&mut writer, cut).unwrap();
        generation += 1;
        root = publish(
            &mut writer,
            &mut pages,
            &cache,
            Publication {
                generation,
                segment: generation - 1,
                accepted: Span::new(first, cut).unwrap(),
                reached: cut,
                refusal: None,
            },
            Blob::EMPTY,
        )
        .unwrap();
        if resume == Some(cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, &cache, budget, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), &cache, root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::resume(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

/// Every holder the scenario names, and one it never does.
const HOLDERS: [Option<&str>; 7] = [
    None,
    Some("10"),
    Some("20"),
    Some("30"),
    Some("31"),
    Some("40"),
    Some("99"),
];

/// Holds the publication `built` to the row preparation's replies through `through`.
fn check(built: &Built, mut prepared: row::Prepared, through: i32, label: &str) {
    let budget = Budget::new(64 << 20);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let flag = AtomicBool::new(false);
    let fixture = fixture();
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    for holder in HOLDERS {
        let params = LendLeaseBalanceParams {
            branch_id: "1".into(),
            holder: holder.map(str::to_owned),
        };
        let expected = row::query(&fixture.store, &fixture.key, &params, Some(through), &flag).unwrap();
        let reply = query::reply(&mut snapshot, &directory, &params, Some(through), &flag).unwrap();
        assert_eq!(
            serde_json::to_value(&reply).unwrap(),
            serde_json::to_value(&expected).unwrap(),
            "{label}: holder {holder:?}"
        );
    }
}

/// Every placement of segment starts, cuts and a resume answers as the row preparation does.
#[test]
fn the_replies_are_the_row_preparations_across_segments_cuts_and_a_resume() {
    let budget = Budget::new(256 << 20);
    let hours = scenario();
    let walked: Vec<i32> = hours.iter().map(|tables| tables.hour).collect();
    for (starts, cuts, resume) in [
        (&[][..], &[140][..], None),
        (&[105, 116, 126, 133][..], &[104, 119, 130, 140][..], Some(119)),
        (&[107, 111, 136, 138][..], &[110, 125, 137, 140][..], Some(125)),
        (&walked[1..], &walked[..], Some(110)),
    ] {
        let label = format!("starts {starts:?}");
        let built = build(projected(&hours, starts), cuts, resume, &budget);
        check(&built, row_prepared(&hours, starts), 140, &label);
        check(&built, row_prepared(&hours, starts), 126, &format!("{label}, through 126"));
    }
}

/// The scenario holds what its replies are held to: totals, a zero proven by the rosters, a
/// rename and the refusals.
#[test]
fn the_scenarios_replies_hold_what_they_are_held_to() {
    let budget = Budget::new(256 << 20);
    let hours = scenario();
    let built = build(projected(&hours, &[]), &[140], None, &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let flag = AtomicBool::new(false);
    let ask = |snapshot: &mut Snapshot<'_, Memory>, holder: Option<&str>| {
        let params = LendLeaseBalanceParams {
            branch_id: "1".into(),
            holder: holder.map(str::to_owned),
        };
        query::reply(snapshot, &directory, &params, Some(140), &flag).unwrap()
    };
    let every = ask(&mut snapshot, None);
    let holders: Vec<&str> = every.histories.iter().map(|history| history.holder.as_str()).collect();
    assert_eq!(holders, ["10", "20", "30", "31", "40"]);
    let bbb = &every.histories[1];
    assert_eq!(
        bbb.tags.iter().map(|tag| (tag.hour, tag.tag.as_str())).collect::<Vec<_>>(),
        [(100, "BBB"), (128, "BBX")]
    );
    let partners = ask(&mut snapshot, Some("10"));
    let ccc = partners
        .histories
        .iter()
        .find(|history| history.holder == "30")
        .unwrap();
    // Dropped at 107 while AAA is on the rosters: zero, until a record no link owns.
    assert_eq!(
        ccc.values[0].iter().map(|run| (run.0, run.1, run.2.as_str())).collect::<Vec<_>>(),
        [(104, 106, "50"), (107, 109, "0")]
    );
    for refusal in [
        "a lend-lease history names a country with no identity",
        "a lend-lease history names a partner with no identity",
        "a war roster names a country with no identity",
        "hour 115 WarLendLeaseHistories is unavailable",
        "hour 125 Countries is unavailable",
        "hour 135 WarOwnedRelations is unavailable",
    ] {
        assert!(
            every.warnings.iter().any(|warning| warning.contains(refusal)),
            "{refusal}: {:?}",
            every.warnings
        );
    }
}

/// An hour that changes nothing the balance reads hands the emit nothing; one that changes one
/// record of 1,500 countries' hands only it.
#[test]
fn an_hour_hands_the_emit_only_what_changed() {
    let countries: Vec<(u32, u32, String)> = (0..1_500)
        .map(|slot| (slot + 1, slot, format!("T{slot:04}")))
        .collect();
    let country_rows: Vec<CountryRow> = countries
        .iter()
        .map(|(id, canonical_slot, tag)| CountryRow {
            id: *id,
            canonical_slot: *canonical_slot,
            tag: tag.as_bytes().to_vec(),
            ..Default::default()
        })
        .collect();
    let links: Vec<CountryLinkRow> = countries
        .iter()
        .map(|&(id, canonical_slot, _)| CountryLinkRow { id, canonical_slot })
        .collect();
    let relations: Vec<WarOwnedRelationRow> = (1..1_500)
        .step_by(2)
        .map(|owner| WarOwnedRelationRow {
            owner,
            ordinal: 0,
            first_country: owner,
            second_country: owner + 1,
            ..Default::default()
        })
        .collect();
    let histories = |moved: i64| -> Vec<WarLendLeaseHistoryRow> {
        (1..1_500)
            .map(|country| WarLendLeaseHistoryRow {
                country,
                target: country + 1,
                industry_sent: if country == 700 { moved } else { 100 },
                fuel_sent: 0,
                industry_received: 0,
                fuel_received: 0,
            })
            .collect()
    };
    let mut walk = Synthetic::new(facts::TABLES, 1, 100).unwrap();
    let mut world = None;
    let mut projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
    let mut out = Vec::new();
    for (ordinal, (hour, moved)) in (0..).zip([(100, 100), (101, 100), (102, 250)]) {
        write(&mut walk, TableId::Countries, Some(&country_rows[..]));
        write(&mut walk, TableId::CountryLinks, Some(&links[..]));
        write(&mut walk, TableId::WarOwnedRelations, Some(&relations[..]));
        write(&mut walk, TableId::WarLendLeaseHistories, Some(&histories(moved)[..]));
        walk.end(|state, manifest, _, first| {
            let world = dashboard_world::World::at(
                &mut world,
                first,
                &facts::needs(),
                (FormatVersion::CURRENT, CaptureMode::Full),
                state,
                manifest,
            )
            .unwrap();
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour,
                        run: 0,
                        ordinal,
                        starts: first,
                        opens: first,
                        state,
                        manifest,
                        world,
                    },
                    &mut out,
                )
                .unwrap();
        })
        .unwrap();
    }
    assert!(matches!(&out[0].facts, Some(Facts::Read(read)) if read.whole && read.records.len() == 1_499));
    assert!(out[1].facts.is_none());
    let Some(Facts::Read(read)) = &out[2].facts else {
        panic!("a changed record is read");
    };
    assert!(!read.whole);
    assert_eq!(read.records, [((699, 700), Some([250, 0, 0, 0]))]);
    assert_eq!(read.totals, [(699, Some([250, 0, 0, 0]))]);
    assert!(read.zeroable.is_empty() && read.tags.is_empty() && read.counts.is_none());
}

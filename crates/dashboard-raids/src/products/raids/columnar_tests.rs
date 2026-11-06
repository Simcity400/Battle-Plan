//! Raids' columnar preparation held to the row scan ([`super::scan`]) on the same hours, written
//! as the native recorder writes them: every view's reply (episodes, runs, preparation, tallies,
//! presence, read hours and warnings), across segment starts, a gap, cuts and a resume.

use super::{
    emit::{Emit, SCHEMA},
    query::{answer, scanned},
    segment::{Segments, TABLES},
    *,
};
use dashboard_core::test_fixtures::{CAMPAIGN, records, source};
use dashboard_keyed::segment::{self as keyed, Hour};
use dashboard_series::{
    Blob, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator as _},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        CountryAppearanceRow, CountryLinkRow, CountryRow, LexicalTokenRow, ProvinceRow,
        RaidCountryRow, RaidTargetRow, ReservedCountryNamingRow, RowCodec, StateRow, TABLE_COUNT,
        TableRows,
    },
    scope::CaptureMode,
};
use replay_frame::frame::TableView;

const REFUSAL: &str = "synthetic unavailable table";
const A: u32 = 100_000;
const RAID_TYPE: u32 = 77;

/// Hours 100 to 125 without 112 and 113.
fn hours() -> Vec<i32> {
    (100..=125)
        .filter(|hour| !(112..=113).contains(hour))
        .collect()
}

/// Three countries, the third from 102, the second renamed from 120; none read at 123.
fn countries(hour: i32) -> Option<Vec<CountryRow>> {
    (hour != 123).then(|| {
        (0..3)
            .filter(|&slot| slot != 2 || hour >= 102)
            .map(|slot| CountryRow {
                id: A + slot,
                canonical_slot: slot,
                tag: match slot {
                    0 => b"AAA".to_vec(),
                    1 if hour >= 120 => b"BBX".to_vec(),
                    1 => b"BBB".to_vec(),
                    _ => b"CCC".to_vec(),
                },
                ..CountryRow::default()
            })
            .collect()
    })
}

fn links(hour: i32) -> Vec<CountryLinkRow> {
    (0..3)
        .filter(|&slot| slot != 2 || hour >= 102)
        .map(|slot| CountryLinkRow {
            id: A + slot,
            canonical_slot: slot,
        })
        .collect()
}

/// Every slot's raid country, its used types moving every hour (which the scan does not read):
/// the third's before its country is, an unproved parent. None read at 121, where raid 2's target
/// changes controller, so the change is read at 122.
fn raid_countries(hour: i32) -> Option<Vec<RaidCountryRow>> {
    (hour != 121).then(|| raid_countries_read(hour))
}

fn raid_countries_read(hour: i32) -> Vec<RaidCountryRow> {
    (0..3)
        .map(|slot| RaidCountryRow {
            canonical_slot: slot,
            country: (A + slot) as i32,
            eligible_types: vec![RAID_TYPE],
            used_types: vec![hour as u32],
        })
        .collect()
}

fn target(id: u32, province: u32) -> RaidTargetRow {
    RaidTargetRow {
        id,
        reference_type: native::REFERENCE_TYPE_VALUE,
        scope: RaidScope {
            province,
            ..Default::default()
        },
        raid_type: RAID_TYPE,
        ..Default::default()
    }
}

fn raid_row(id: u32, parent_slot: u32, phase: i32, outcome: i32) -> RaidRow {
    RaidRow {
        id,
        reference_type: native::REFERENCE_TYPE_VALUE,
        parent_slot,
        phase,
        outcome,
        ..Default::default()
    }
}

/// Raid 1 (the first's, on province 500) prepares at 101 to 104 (its hours moving), flies at 105
/// to 107 (its distance moving) and has ended at 108 to 110, a success. Raid 2 (the third's, on
/// province 501) is live at 118 to 122, its preparation moving at 119 and stopping at 120 with
/// nothing the scan reads changing. Raid 3 (the second's, on the state 3) prepares from 124.
fn raids(hour: i32) -> (Vec<RaidRow>, Vec<RaidTargetRow>) {
    let mut live = Vec::new();
    let mut targets = Vec::new();
    if (101..=110).contains(&hour) {
        let mut raid = match hour {
            101..=104 => raid_row(1, 0, native::PHASE_PREPARING, native::OUTCOME_NONE),
            105..=107 => raid_row(1, 0, native::PHASE_IN_PROGRESS, native::OUTCOME_NONE),
            _ => raid_row(1, 0, native::PHASE_ENDED, native::OUTCOME_SUCCESS),
        };
        raid.preparation_hours = (hour - 100).min(4);
        raid.travelled = (hour >= 105).then(|| i64::from(hour.min(108) - 104) * 1000);
        live.push(raid);
        targets.push(target(1, 500));
    }
    if (118..=122).contains(&hour) {
        let mut raid = raid_row(2, 2, native::PHASE_PREPARING, native::OUTCOME_NONE);
        raid.preparation_hours = (hour - 117).min(2);
        live.push(raid);
        targets.push(target(2, 501));
    }
    if hour >= 124 {
        let mut raid = raid_row(3, 1, native::PHASE_PREPARING, native::OUTCOME_NONE);
        raid.preparation_hours = hour - 123;
        live.push(raid);
        targets.push(RaidTargetRow {
            scope: RaidScope {
                state: 3,
                ..Default::default()
            },
            ..target(3, 0)
        });
    }
    (live, targets)
}

/// Province 500's controller changes at 116 with no raid live, and 501's at 121 under raid 2;
/// province 999, which no raid reaches, changes controller every hour.
fn provinces(hour: i32) -> Vec<ProvinceRow> {
    vec![
        ProvinceRow {
            id: 500,
            state: 2,
            owner: A + 1,
            controller: if hour >= 116 { A } else { A + 1 },
            move_gate: (hour % 2) as u8,
            ..Default::default()
        },
        ProvinceRow {
            id: 501,
            state: 3,
            owner: A + 2,
            controller: if hour >= 121 { A } else { A + 2 },
            ..Default::default()
        },
        ProvinceRow {
            id: 999,
            state: 4,
            owner: A,
            controller: A + (hour as u32 % 2),
            ..Default::default()
        },
    ]
}

fn states() -> Vec<StateRow> {
    (2..=4)
        .map(|id| StateRow {
            id,
            owner: A + id % 3,
            controller: A + id % 3,
            custom_name: format!("State {id}").into_bytes(),
            ..Default::default()
        })
        .collect()
}

fn tokens() -> Vec<LexicalTokenRow> {
    vec![LexicalTokenRow {
        token: RAID_TYPE,
        literal: b"raid_airborne".to_vec(),
    }]
}

fn reserved() -> Vec<ReservedCountryNamingRow> {
    vec![ReservedCountryNamingRow::default()]
}

/// The hour as the row recording holds it.
fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    let mut put = |table: TableId, rows: Option<TableRows>| {
        *frame.tables[table.index()] = match rows {
            Some(rows) => TableView::Complete(rows),
            None => TableView::Unavailable {
                reason: REFUSAL.into(),
            },
        };
    };
    let (live, targets) = raids(hour);
    put(TableId::Countries, countries(hour).map(TableRows::Countries));
    put(TableId::CountryLinks, Some(TableRows::CountryLinks(links(hour))));
    put(
        TableId::RaidCountries,
        raid_countries(hour).map(TableRows::RaidCountries),
    );
    put(TableId::Raids, Some(TableRows::Raids(live)));
    put(TableId::RaidTargets, Some(TableRows::RaidTargets(targets)));
    put(TableId::LexicalTokens, Some(TableRows::LexicalTokens(tokens())));
    put(TableId::Provinces, Some(TableRows::Provinces(provinces(hour))));
    put(TableId::States, Some(TableRows::States(states())));
    put(
        TableId::CountryAppearance,
        Some(TableRows::CountryAppearance(Vec::<CountryAppearanceRow>::new())),
    );
    put(
        TableId::ReservedCountryNaming,
        Some(TableRows::ReservedCountryNaming(reserved())),
    );
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

/// The hours the segments' operators hand on, a fresh operator starting at each of `starts`
/// and at each walk's first hour, and the row frames of the same hours.
fn projected(starts: &[i32]) -> (Vec<Hour<Segments>>, Vec<DecodedFrame>) {
    let format = FormatVersion::CURRENT;
    let needs = keyed::needs::<Segments>().unwrap();
    let mut out = Vec::new();
    let mut frames = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut operator = keyed::Operator::<Segments>::new(format).unwrap();
    for (ordinal, hour) in (0..).zip(hours()) {
        let gap = frames
            .last()
            .is_some_and(|last: &DecodedFrame| last.hour + 1 != hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(TABLES, 1, hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        let (live, targets) = raids(hour);
        write(walk, TableId::Countries, countries(hour).as_deref());
        write(walk, TableId::CountryLinks, Some(&links(hour)[..]));
        write(walk, TableId::RaidCountries, raid_countries(hour).as_deref());
        write(walk, TableId::Raids, Some(&live[..]));
        write(walk, TableId::RaidTargets, Some(&targets[..]));
        write(walk, TableId::LexicalTokens, Some(&tokens()[..]));
        write(walk, TableId::Provinces, Some(&provinces(hour)[..]));
        write(walk, TableId::States, Some(&states()[..]));
        write::<CountryAppearanceRow>(walk, TableId::CountryAppearance, Some(&[][..]));
        write(walk, TableId::ReservedCountryNaming, Some(&reserved()[..]));
        let cold = starts.contains(&hour);
        let metrics = walk
            .end(|state, manifest, decoded, first| {
                let starts = first || cold;
                if starts {
                    operator = keyed::Operator::new(format).unwrap();
                }
                let world = dashboard_world::World::at(
                    &mut world,
                    starts,
                    &needs,
                    (format, CaptureMode::Full),
                    state,
                    manifest,
                )
                .unwrap();
                operator
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
                decoded.metrics.clone()
            })
            .unwrap();
        let mut observed = frame(hour);
        observed.metrics = metrics;
        frames.push(observed);
    }
    // As its emit is handed them: written to the spool and read back.
    (replay_spool::respool(out).unwrap(), frames)
}

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [4; 32],
        content: [5; 32],
        branch: [6; 32],
        schema: SCHEMA,
    }
}

/// A publication built from projected hours, cut after each of `cuts`' hours; when `resume`
/// names one of them, the emit is saved there and goes on from the publication at the next
/// segment's first hour, as a preparation of a grown recording does.
fn build(hours: Vec<Hour<Segments>>, cuts: &[i32], resume: Option<i32>) -> (Memory, u64) {
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let format = FormatVersion::CURRENT;
    let mut writer = Writer::new(identity(), first).unwrap();
    let mut emit = Emit::new(format, 1, None);
    let mut root = 0;
    let mut hours = hours.into_iter().peekable();
    for (generation, &cut) in (1..).zip(cuts) {
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.hour(&mut writer, hour).unwrap();
        }
        emit.cut(&mut writer, cut).unwrap();
        let publication = Publication {
            generation,
            segment: generation - 1,
            accepted: Span::new(first, cut).unwrap(),
            reached: cut,
            refusal: None,
        };
        root = publish(&mut writer, &mut pages, publication, Blob::EMPTY).unwrap();
        if resume == Some(cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::load(format, 1, None, &directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none(), "every hour is cut");
    (pages, root)
}

fn replay_source(frames: Vec<DecodedFrame>) -> Source {
    Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "raids-columnar".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// Every request the widgets make.
fn requests() -> Vec<RaidsParams> {
    let params = |view, tags: &[&str], top| RaidsParams {
        branch_id: "1".into(),
        view,
        tags: tags.iter().map(|&tag| tag.to_owned()).collect(),
        top,
    };
    let mut requests = vec![
        params(RaidsView::Catalogue, &[], None),
        params(RaidsView::Log, &[], None),
        params(RaidsView::Tally, &[], None),
        params(RaidsView::Tally, &["AAA", "BBX", "CCC"], None),
    ];
    for metric in [RaidMetric::Launched, RaidMetric::Succeeded, RaidMetric::Both] {
        requests.push(params(
            RaidsView::Tally,
            &[],
            Some(RaidTop { count: 2, metric }),
        ));
    }
    requests
}

/// Holds the publication to the row scan's replies over `frames`.
fn check((pages, root): &(Memory, u64), frames: &[DecodedFrame], label: &str) {
    let mut snapshot = Snapshot::open(pages.clone(), *root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let source = replay_source(frames.to_vec());
    let context = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let histories =
        country_history::branch_histories(&source, 1, false, &[], &cancelled).unwrap();
    for params in requests() {
        let expected =
            serde_json::to_value(Raids::load(&source, &params, &context, &cancelled).unwrap())
                .unwrap();
        let published = scanned(&mut snapshot, &directory, params.view, &cancelled).unwrap();
        let histories = (params.view == RaidsView::Tally).then_some(&*histories);
        let actual = serde_json::to_value(
            answer(&source, &context, &params, published, histories, &cancelled).unwrap(),
        )
        .unwrap();
        assert_eq!(actual, expected, "{label}: {params:?}");
    }
}

#[test]
fn the_publication_answers_as_the_row_scan_across_segments_cuts_and_a_resume() {
    let all = hours();
    for (starts, cuts, resume) in [
        (&[][..], &[125][..], None),
        (&[105, 116][..], &[104, 111, 119, 125][..], Some(111)),
        (
            &[101, 108, 118, 120, 122][..],
            &[103, 109, 114, 121, 125][..],
            Some(121),
        ),
        (&all[1..], &all[..], Some(117)),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let (projected, frames) = projected(starts);
        let built = build(projected, cuts, resume);
        check(&built, &frames, &label);
    }
}

/// The scenario reads as the row scan finds it: both raids' episodes, raid 1 counted launched and
/// succeeded for its country, raid 2's preparation ended where it stopped moving, and the hours
/// before the third country's row unread.
#[test]
fn the_scenario_reads_as_the_row_scan_finds_it() {
    let (_, frames) = projected(&[]);
    let source = replay_source(frames);
    let cancelled = AtomicBool::new(false);
    let scanned = scan(source.replay_ref(), 1, None, &cancelled).unwrap();
    assert_eq!(scanned.episodes.len(), 3);
    let tally = scanned.series["AAA"].last().unwrap();
    assert_eq!((tally.launched, tally.succeeded), (1, 1));
    let preparation = &scanned.episodes[1].preparation;
    assert!(preparation.len() >= 2, "{preparation:?}");
    assert!(!scanned.read_hours.contains(&100));
    assert!(scanned.read_hours.contains(&110));
}

/// Hours where only what the scan does not read moves (the raid countries' used types, the
/// movement gates, a province no raid reaches or reached) hand the emit no change; the province a
/// live raid's target reaches changing controller, and a raid's preparation moving, do.
#[test]
fn hours_where_nothing_the_scan_reads_moves_change_nothing() {
    let (out, _) = projected(&[]);
    let changed = |hour: i32| {
        let held = out.iter().find(|held| held.hour == hour).unwrap();
        held.read
            .as_ref()
            .map_or([false; TABLE_COUNT], |kept| kept.changes.rows_changed)
    };
    for hour in [115, 116, 117] {
        assert!(!changed(hour).contains(&true), "{hour}");
    }
    let mut rename = [false; TABLE_COUNT];
    rename[TableId::Countries.index()] = true;
    assert_eq!(changed(120), rename);
    assert!(changed(121)[TableId::Provinces.index()]);
    assert!(!changed(121)[TableId::Raids.index()]);
    assert!(changed(119)[TableId::Raids.index()]);
    assert!(!changed(119)[TableId::Provinces.index()]);
}

/// A reply answers through the publication it read: the gate's coverage cut back to its last
/// hour, or to no hour when none opened, and as the gate gives it when the publication reaches it.
#[test]
fn replies_answer_through_the_publication_they_read() {
    use dashboard_base::prepared::coverage::Coverage;
    let accepted = |last_hour, complete| Coverage {
        schema: 1,
        first_hour: 10,
        last_hour,
        complete,
    };
    let answered = super::query::answered;
    assert_eq!(answered(&accepted(149, false), Some(119)), Some(accepted(119, false)));
    assert_eq!(answered(&accepted(199, true), Some(119)), Some(accepted(119, false)));
    assert_eq!(answered(&accepted(149, false), None), Some(accepted(9, false)));
    assert_eq!(answered(&accepted(149, false), Some(149)), None);
    assert_eq!(answered(&accepted(199, true), Some(199)), None);
}

//! The ledger's columnar preparation held to its row oracle ([`super::scan`]) on the same hours,
//! written as the native recorder writes them: both families' replies (every leaf and point,
//! presence, read hours, unknown callers, counts and rankings), across segment starts, a gap,
//! cuts and a resume.

use super::{
    emit::{Emit, FAMILY, OBSERVED, SCHEMA},
    facts::{self, Projector},
    *,
};
use dashboard_core::test_fixtures::{CAMPAIGN, records, source};
use dashboard_series::{
    Blob, Knownness, Publication, Snapshot, Span, Writer,
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
        AirLossEventRow, BuildingBombingDamageRow, CountryLinkRow, CountryRow, LexicalTokenRow,
        RowCodec, TableRows,
    },
    scope::CaptureMode,
};
use replay_frame::frame::TableView;

const REFUSAL: &str = "synthetic unavailable table";

/// One hour as the recorder writes it: whether the countries are complete, and each event
/// table's rows, none where it is unavailable.
#[derive(Clone)]
struct Tables {
    hour: i32,
    countries: bool,
    air: Option<Vec<AirLossEventRow>>,
    bombing: Option<Vec<BuildingBombingDamageRow>>,
}

fn loss(
    id: u64,
    hour: i32,
    country: u32,
    opponent: Option<u32>,
    cause: u32,
    amount: i64,
) -> AirLossEventRow {
    AirLossEventRow {
        id,
        game_hour: hour,
        country,
        region: 40_000 + country,
        native_cause: cause,
        credited_enemy: country % 2 == 0,
        amount_raw: amount,
        opponent,
        ..Default::default()
    }
}

fn damage(
    id: u64,
    hour: i32,
    attacker: Option<u32>,
    victim: u32,
    cause: BuildingBombingDamageCause,
    building: u32,
) -> BuildingBombingDamageRow {
    BuildingBombingDamageRow {
        id,
        game_hour: hour,
        attacker,
        victim,
        state: victim % 3,
        building_type: building,
        cause,
        amount_raw: 1_000_000 + i64::from(victim),
        ..Default::default()
    }
}

/// Hours 10 to 45 without 30 and 31. Losses every hour but some, a loss dated before its read,
/// an unresolved country, an opponent, an unproved cause; the air table unread at 20 and 21,
/// after which its ordinals restart dated within the interval (a new epoch). Bombing by air
/// missions and air-naval strikes, with an unknown caller at 14, other callers, an unknown
/// attacker and an unspelled building type. The countries unread at 25; a tag that is not
/// UTF-8 from 35.
fn scenario() -> Vec<Tables> {
    use BuildingBombingDamageCause::*;
    let mut air_id = 0;
    let mut bombing_id = 0;
    (10..=45)
        .filter(|hour| !(30..=31).contains(hour))
        .map(|hour| {
            if hour == 22 {
                air_id = 0;
            }
            let mut air = Vec::new();
            if hour % 3 != 0 {
                for (country, opponent, cause) in [
                    (100_000, Some(100_001), 1),
                    (100_001, None, 2),
                    (999, None, 7),
                ] {
                    if cause == 7 && hour % 4 != 0 {
                        continue;
                    }
                    let dated = if hour == 22 {
                        20
                    } else {
                        hour - i32::from(hour % 5 == 0)
                    };
                    air.push(loss(
                        air_id,
                        dated,
                        country,
                        opponent,
                        cause,
                        100_000 * i64::from(hour),
                    ));
                    air_id += 1;
                }
            }
            let mut bombing = Vec::new();
            if hour % 2 == 0 {
                for (attacker, victim, cause, building) in [
                    (Some(100_002), 100_000, AirMission, 9001),
                    (None, 100_001, AirNaval, 9002),
                    (Some(100_000), 100_002, LandCombat, 9001),
                ] {
                    bombing.push(damage(bombing_id, hour, attacker, victim, cause, building));
                    bombing_id += 1;
                }
                if hour == 14 {
                    bombing.push(damage(bombing_id, hour, None, 100_003, Unknown, 9001));
                    bombing_id += 1;
                }
            }
            Tables {
                hour,
                countries: hour != 25,
                air: (!(20..=21).contains(&hour)).then_some(air),
                bombing: Some(bombing),
            }
        })
        .collect()
}

fn country_rows(hour: i32) -> Vec<CountryRow> {
    (0..4)
        .map(|index| CountryRow {
            id: 100_000 + index,
            canonical_slot: index,
            tag: if index == 3 && hour >= 35 {
                vec![0xff, 0xfe, 0xfd]
            } else {
                format!("T{index:02}").into_bytes()
            },
            ..CountryRow::default()
        })
        .collect()
}

fn link_rows() -> Vec<CountryLinkRow> {
    (0..4)
        .map(|index| CountryLinkRow {
            id: 100_000 + index,
            canonical_slot: index,
        })
        .collect()
}

fn token_rows() -> Vec<LexicalTokenRow> {
    vec![LexicalTokenRow {
        token: 9001,
        literal: b"air_base".to_vec(),
    }]
}

/// The hour as the row recording holds it.
fn frame(tables: &Tables) -> DecodedFrame {
    let mut frame = source(1, tables.hour, Some(1));
    let mut put = |table: TableId, view: TableView| {
        *frame.tables[table.index()] = view;
    };
    let unavailable = || TableView::Unavailable {
        reason: REFUSAL.into(),
    };
    put(
        TableId::Countries,
        if tables.countries {
            TableView::Complete(TableRows::Countries(country_rows(tables.hour)))
        } else {
            unavailable()
        },
    );
    put(
        TableId::CountryLinks,
        TableView::Complete(TableRows::CountryLinks(link_rows())),
    );
    put(
        TableId::LexicalTokens,
        TableView::Complete(TableRows::LexicalTokens(token_rows())),
    );
    put(
        TableId::AirLossEvents,
        match &tables.air {
            Some(rows) => TableView::Complete(TableRows::AirLossEvents(rows.clone())),
            None => unavailable(),
        },
    );
    put(
        TableId::BuildingBombingDamage,
        match &tables.bombing {
            Some(rows) => TableView::Complete(TableRows::BuildingBombingDamage(rows.clone())),
            None => unavailable(),
        },
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

/// The projected hours of `hours`, a projector starting cold at each of `starts` and at each
/// walk's first hour, and the row frames of the same hours.
fn projected(hours: &[Tables], starts: &[i32]) -> (Vec<facts::Hour>, Vec<DecodedFrame>) {
    let mut out = Vec::new();
    let mut frames = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(FormatVersion::CURRENT).unwrap();
    for (ordinal, tables) in (0..).zip(hours) {
        let gap = frames
            .last()
            .is_some_and(|last: &DecodedFrame| last.hour + 1 != tables.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, tables.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        let countries = tables.countries.then(|| country_rows(tables.hour));
        write(walk, TableId::Countries, countries.as_deref());
        write(walk, TableId::CountryLinks, Some(&link_rows()[..]));
        write(walk, TableId::LexicalTokens, Some(&token_rows()[..]));
        write(walk, TableId::AirLossEvents, tables.air.as_deref());
        write(
            walk,
            TableId::BuildingBombingDamage,
            tables.bombing.as_deref(),
        );
        let cold = starts.contains(&tables.hour);
        if cold {
            projector = Projector::new(FormatVersion::CURRENT).unwrap();
        }
        let metrics = walk
            .end(|state, manifest, decoded, first| {
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
                decoded.metrics.clone()
            })
            .unwrap();
        let mut observed = frame(tables);
        observed.metrics = metrics;
        frames.push(observed);
    }
    // As its emit is handed them: written to the spool and read back.
    (replay_spool::respool(out).unwrap(), frames)
}

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [7; 32],
        content: [8; 32],
        branch: [9; 32],
        schema: SCHEMA,
    }
}

struct Built {
    pages: Memory,
    root: u64,
}

/// A publication built from projected hours, cut after each of `cuts`' hours; when `resume`
/// names one of them, the emit is saved there and goes on from the publication.
fn build(hours: Vec<facts::Hour>, cuts: &[i32], resume: Option<i32>) -> Built {
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(identity(), first).unwrap();
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
            writer = Writer::resume(&mut pages, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::load(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

fn replay_source(frames: Vec<DecodedFrame>) -> Source {
    Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "ledger-columnar".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// Every request the widgets make of both families.
fn requests(middle: i32) -> Vec<serde_json::Value> {
    let mut requests = Vec::new();
    for family in ["airLosses", "bombing"] {
        for from in [None, Some(middle)] {
            requests.push(serde_json::json!({"branchId":"1","family":family,"fromHour":from}));
            for metric in ["dealt", "taken", "total"] {
                requests.push(
                    serde_json::json!({"branchId":"1","family":family,"fromHour":from,
                    "top":{"count":2,"metric":metric}}),
                );
            }
        }
        requests.push(
            serde_json::json!({"branchId":"1","family":family,"causes":[1, 7],"regions":[40000]}),
        );
    }
    requests
}

/// Holds the publication `built` to the row oracle's replies over `frames`.
fn check(built: &Built, frames: &[DecodedFrame], label: &str) {
    let mut snapshot = Snapshot::open(built.pages.clone(), built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let source = replay_source(frames.to_vec());
    let context = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let oracle = scan(source.replay_ref(), 1, &cancelled).unwrap();
    for request in requests(28) {
        let params: MilitaryAirParams = serde_json::from_value(request.clone()).unwrap();
        let expected = serde_json::to_value(
            oracle
                .reply(&source, &params, &context, &cancelled)
                .unwrap(),
        )
        .unwrap();
        let actual = super::query::reply(
            &source,
            &context,
            request.clone(),
            &mut snapshot,
            &directory,
            &cancelled,
        )
        .unwrap();
        assert_eq!(actual, expected, "{label}: {request}");
    }
    for family in ["airLosses", "bombing"] {
        let available = super::query::reply(
            &source,
            &context,
            serde_json::json!({"branchId":"1","family":family,"availability":true}),
            &mut snapshot,
            &directory,
            &cancelled,
        )
        .unwrap();
        assert_eq!(
            available,
            serde_json::json!({"available": true}),
            "{label}: {family}"
        );
    }
}

#[test]
fn the_publication_answers_as_the_row_ledger_across_segments_cuts_and_a_resume() {
    let hours = scenario();
    let all: Vec<i32> = hours.iter().map(|tables| tables.hour).collect();
    for (starts, cuts, resume) in [
        (&[][..], &[45][..], None),
        (&[15, 22, 32][..], &[19, 29, 37, 45][..], Some(29)),
        (&[11, 20, 26, 36, 40][..], &[13, 24, 35, 45][..], Some(35)),
        (&all[1..], &all[..], Some(21)),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let (projected, frames) = projected(&hours, starts);
        let built = build(projected, cuts, resume);
        check(&built, &frames, &label);
        known_only_where_recorded(&built, &all, &label);
    }
}

/// The scenario reads as the row ledger counts it: the new epoch after the unread interval is
/// accepted, every endpoint the countries cannot resolve stays unattributed, and the unknown
/// caller marks its read.
#[test]
fn the_scenario_reads_as_the_ledger_counts_it() {
    let (_, frames) = projected(&scenario(), &[]);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let oracle = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert!(oracle.air.leaves.keys().any(|leaf| leaf.country.is_none()));
    assert_eq!(
        oracle.bombing.incomplete,
        [HourSpan {
            first_hour: 14,
            last_hour: 14
        }]
    );
    assert!(
        oracle
            .air
            .warnings
            .keys()
            .any(|reason| reason.contains("cause 7"))
    );
    assert!(oracle.air.warnings.contains_key(UNNAMED_TAG));
    assert!(
        !oracle
            .air
            .observed
            .iter()
            .any(|span| span.first_hour <= 21 && 20 <= span.last_hour)
    );
}

/// An hour whose event tables hold no events hands the emit nothing but its reads.
#[test]
fn an_hour_without_events_hands_nothing_but_its_reads() {
    let quiet: Vec<Tables> = (10..=12)
        .map(|hour| Tables {
            hour,
            countries: true,
            air: Some(Vec::new()),
            bombing: Some(Vec::new()),
        })
        .collect();
    let (out, _) = projected(&quiet, &[]);
    assert!(out[0].identities.is_some());
    for hour in &out[1..] {
        assert!(hour.identities.is_none());
        assert!(
            hour.families
                .iter()
                .all(|events| matches!(events, facts::Events::Read(None)))
        );
    }
}

/// The publication's evidence of a family key is known exactly at the recorded `hours`, never
/// across the hours between two of them.
fn known_only_where_recorded(built: &Built, hours: &[i32], label: &str) {
    let mut snapshot = Snapshot::open(built.pages.clone(), built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let key = directory.key(FAMILY, OBSERVED, None).unwrap();
    let span = Span::new(hours[0], *hours.last().unwrap()).unwrap();
    let mut known = Vec::new();
    let mut cursor = None;
    loop {
        let page = snapshot
            .evidence_page(key, span, cursor, 1 << 16, |row| {
                if row.known == Knownness::Known {
                    known.extend(row.span.first..=row.span.last);
                }
                Ok(())
            })
            .unwrap();
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    known.sort_unstable();
    known.dedup();
    assert_eq!(known, hours, "{label}: known hours");
}

/// Events recorded again after an unread interval, their ordinals restarted and dated before
/// it, refuse the branch by the name the row ledger gives them.
#[test]
fn events_recorded_again_after_an_unread_interval_refuse_as_the_row_ledger_does() {
    let air = |hour: i32, ids: &[u64], dated: i32| Tables {
        hour,
        countries: true,
        air: Some(
            ids.iter()
                .map(|&id| loss(id, dated, 100_000, None, 1, 100_000))
                .collect(),
        ),
        bombing: Some(Vec::new()),
    };
    let unread = Tables {
        hour: 12,
        countries: true,
        air: None,
        bombing: Some(Vec::new()),
    };
    let hours = vec![air(10, &[0, 1], 10), air(11, &[2, 3], 11), unread, air(13, &[0, 1], 10)];
    let (projected, frames) = projected(&hours, &[]);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let expected = match scan(&replay, 1, &AtomicBool::new(false)) {
        Ok(_) => panic!("the row ledger accepted events recorded again"),
        Err(error) => error.to_string(),
    };
    assert_eq!(
        expected,
        "air loss events restart their ordinals after the unread interval from hour 12 but are dated from hour 10, before it: earlier events recorded again, not new ones"
    );
    let mut writer = Writer::new(identity(), 10).unwrap();
    let mut emit = Emit::new();
    let refused = projected
        .into_iter()
        .try_for_each(|hour| emit.hour(&mut writer, hour))
        .unwrap_err();
    assert_eq!(refused.to_string(), expected);
}

/// A read refused after an unrecorded hour, one family's events already read that hour, leaves
/// the publication cut through the hour before as a build of those hours alone publishes it.
#[test]
fn a_read_refused_after_a_gap_is_cut_through_the_hour_before() {
    use BuildingBombingDamageCause::AirMission;
    let air = |ids: &[u64], dated: i32| {
        Some(
            ids.iter()
                .map(|&id| loss(id, dated, 100_000, None, 1, 100_000))
                .collect(),
        )
    };
    let bombing = |ids: &[u64], hour: i32| {
        Some(
            ids.iter()
                .map(|&id| damage(id, hour, Some(100_001), 100_000, AirMission, 9001))
                .collect(),
        )
    };
    let tables = |hour: i32, air, bombing| Tables {
        hour,
        countries: true,
        air,
        bombing,
    };
    // Hour 13's air loss is new; its bombing repeats an ordinal.
    let hours = vec![
        tables(10, air(&[0, 1], 10), bombing(&[0], 10)),
        tables(11, None, bombing(&[1], 11)),
        tables(13, air(&[2], 13), bombing(&[1], 13)),
    ];
    let (projected, frames) = projected(&hours, &[]);
    let clean = build(self::projected(&hours[..2], &[]).0, &[11], None);
    let mut writer = Writer::new(identity(), 10).unwrap();
    let mut emit = Emit::new();
    let mut projected = projected.into_iter();
    for hour in projected.by_ref().take(2) {
        emit.hour(&mut writer, hour).unwrap();
    }
    emit.hour(&mut writer, projected.next().unwrap())
        .expect_err("a bombing ordinal repeated");
    emit.cut(&mut writer, 11).unwrap();
    let mut pages = Memory::default();
    let publication = Publication {
        generation: 1,
        segment: 0,
        accepted: Span::new(10, 11).unwrap(),
        reached: 11,
        refusal: None,
    };
    let root = publish(&mut writer, &mut pages, publication, Blob::EMPTY).unwrap();
    let refused = Built { pages, root };
    known_only_where_recorded(&refused, &[10, 11], "refused after a gap");
    let source = replay_source(frames[..2].to_vec());
    let context = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let replies = |built: &Built| {
        let mut snapshot =
            Snapshot::open(built.pages.clone(), built.root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        requests(10)
            .into_iter()
            .map(|request| {
                super::query::reply(
                    &source,
                    &context,
                    request,
                    &mut snapshot,
                    &directory,
                    &cancelled,
                )
                .unwrap()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(replies(&refused), replies(&clean));
}

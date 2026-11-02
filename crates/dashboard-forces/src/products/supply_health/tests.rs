use super::*;
use dashboard_base::products::country_history::{
    CountryIdentityRun, RecordedCountry, RecordedHistories,
};
use dashboard_series::{
    Blob, Budget, Cache, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use facts::{Country, Hour};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        CountryLinkRow, CountryRow, DivisionLandInputsRow, DivisionRow, DivisionStatusRow,
        Recorded, RowCodec, TableId,
    },
    scope::CaptureMode,
};
use serde_json::json;
use std::{sync::Arc, sync::atomic::AtomicBool};

fn defines() -> Defines {
    // Synthetic content, deliberately different from shipped thresholds.
    Defines {
        grace: 10,
        low: 60_000,
        very_low_icon: 15_000,
        low_icon: 40_000,
    }
}

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [1; 32],
        content: [2; 32],
        branch: [3; 32],
        schema: 4,
    }
}

/// A publication built hour by hour from facts, cut after each of `cuts`' hours.
struct Built {
    pages: Memory,
    root: u64,
}

fn build(
    configured: &Configured,
    hours: Vec<Hour>,
    cuts: &[i32],
    resume: Option<i32>,
    budget: &Budget,
) -> Built {
    let cache = Cache::new(budget);
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(budget, identity(), first).unwrap();
    let mut emit = configured.emit();
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
            let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
            emit = emit::Emit::load(configured, &directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

fn hour(at: i32, whole: bool, named: &[(u32, &str)], countries: &[Country]) -> Hour {
    Hour {
        branch: 1,
        hour: at,
        named: named
            .iter()
            .map(|&(index, tag)| (index, Arc::from(tag)))
            .collect(),
        whole,
        countries: countries.to_vec(),
        land_present: true,
    }
}

fn country(index: u32, holder: Option<(u32, u32)>, counts: Option<[u32; 3]>) -> Country {
    Country {
        index,
        holder,
        counts,
    }
}

fn history(countries: &[(&str, &[(i32, i32, u32, u32)])]) -> RecordedHistories {
    let mut history = RecordedHistories::default();
    for (tag, runs) in countries {
        history.countries.insert(
            (*tag).to_owned(),
            RecordedCountry::identified(
                runs.iter()
                    .map(
                        |&(first_hour, last_hour, slot, country_id)| CountryIdentityRun {
                            first_hour,
                            last_hour,
                            slot,
                            country_id,
                        },
                    )
                    .collect(),
            ),
        );
    }
    history
}

fn run(first: i32, last: i32, starts: bool, [no, strained, supplied]: [u32; 3]) -> SupplyHealthRun {
    SupplyHealthRun {
        first_hour: first,
        last_hour: last,
        starts_segment: starts,
        no_supply: no,
        strained,
        in_supply: supplied,
    }
}

/// GER's holder changes at 103, its counts are unread at 107, a new segment's projector
/// numbers the countries afresh at 106, and 109 is not recorded: each run and its segment
/// start as hand-built, pieced by the selected holders' hours, and the summaries' edges name
/// the same runs.
#[test]
fn replies_follow_holders_counts_cuts_and_gaps() {
    let configured = Configured::with(Ok(defines()), vec!["A standing note".into()]);
    let (ger, zzz) = (Some((1, 10)), Some((5, 5)));
    let hours = vec![
        hour(
            100,
            true,
            &[(0, "GER"), (1, "ZZZ")],
            &[
                country(0, ger, Some([0, 1, 3])),
                country(1, zzz, Some([0, 0, 0])),
            ],
        ),
        hour(101, false, &[], &[]),
        hour(102, false, &[], &[country(0, ger, Some([1, 1, 2]))]),
        hour(
            103,
            false,
            &[],
            &[country(0, Some((1, 11)), Some([1, 1, 2]))],
        ),
        hour(104, false, &[], &[]),
        hour(105, false, &[], &[]),
        hour(
            106,
            true,
            &[(0, "ZZZ"), (1, "GER")],
            &[
                country(0, zzz, Some([0, 0, 0])),
                country(1, Some((1, 11)), Some([1, 1, 2])),
            ],
        ),
        hour(107, false, &[], &[country(1, Some((1, 11)), None)]),
        hour(
            108,
            false,
            &[],
            &[country(1, Some((1, 11)), Some([1, 1, 2]))],
        ),
        hour(
            110,
            true,
            &[],
            &[
                country(0, zzz, Some([0, 0, 0])),
                country(1, Some((1, 11)), Some([1, 1, 2])),
            ],
        ),
        hour(111, false, &[], &[]),
    ];
    let budget = Budget::new(64 << 20);
    let built = build(&configured, hours, &[105, 111], None, &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages, &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
    let history = history(&[
        ("GER", &[(100, 102, 1, 10), (103, 111, 1, 11)]),
        ("ZZZ", &[(100, 111, 5, 5)]),
    ]);
    let cancelled = AtomicBool::new(false);
    let expected = vec![
        run(100, 101, true, [0, 1, 3]),
        run(102, 102, false, [1, 1, 2]),
        run(103, 106, true, [1, 1, 2]),
        run(108, 108, true, [1, 1, 2]),
        run(110, 111, true, [1, 1, 2]),
    ];
    let params = json!({ "branchId": "1", "tags": ["GER"], "from": 100, "to": 111 });
    let reply = query::reply_to(&mut snapshot, &directory, &params, &history, &cancelled).unwrap();
    assert!(reply.available);
    assert_eq!(reply.warnings, vec!["A standing note".to_owned()]);
    assert_eq!(reply.series.len(), 1);
    assert_eq!(reply.series[0].tag, "GER");
    assert_eq!(reply.series[0].runs, expected);
    assert_eq!(reply.series[0].identities.as_ref().unwrap().len(), 2);
    assert!(reply.summaries.is_none());

    let params = json!({ "branchId": "1", "tags": ["GER"], "from": 100, "to": 111, "width": 4 });
    let reply = query::reply_to(&mut snapshot, &directory, &params, &history, &cancelled).unwrap();
    assert_eq!(reply.series[0].runs, expected);
    let summaries = reply.summaries.unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].values.len(), 3);
    // Twelve hours over four pixels: bins of two hours.
    for summary in &summaries[0].values {
        assert_eq!(summary.buckets.len(), 6);
        assert!(summary.runs.is_none());
    }
    // In supply is three in four of GER's divisions over hours 100 and 101.
    let first = summaries[0].values[2].buckets[0].unwrap();
    assert_eq!((first.first_hour, first.max_value), (100, 75.0));

    let params = json!({ "branchId": "1", "tags": ["GER"], "from": 103, "to": 103 });
    let reply = query::reply_to(&mut snapshot, &directory, &params, &history, &cancelled).unwrap();
    assert_eq!(reply.series[0].runs, vec![run(103, 103, true, [1, 1, 2])]);

    let navigation = |key: &str, from: Option<f64>| {
        json!({
            "branchId": "1", "tags": ["GER"], "key": key, "from": from,
            "firstHour": 100, "lastHour": 111,
        })
    };
    let step = |snapshot: &mut Snapshot<'_, Memory>, key: &str, from: Option<f64>| {
        query::next_change(
            snapshot,
            &directory,
            &navigation(key, from),
            &history,
            &cancelled,
        )
        .unwrap()["hour"]
            .as_f64()
    };
    assert_eq!(step(&mut snapshot, "ArrowRight", Some(100.5)), Some(102.0));
    assert_eq!(step(&mut snapshot, "ArrowRight", Some(106.0)), Some(107.0));
    assert_eq!(step(&mut snapshot, "ArrowLeft", Some(110.0)), Some(109.0));
    assert_eq!(step(&mut snapshot, "Home", None), Some(100.0));
}

#[test]
fn shares_restart_after_zero_divisions_through_cuts_and_a_resume() {
    let configured = Configured::with(Ok(defines()), vec![]);
    for (cuts, resume) in [
        (&[104][..], None),
        (&[100, 101, 102, 104][..], None),
        (&[100, 101, 102, 104][..], Some(101)),
        (&[100, 101, 102, 104][..], Some(102)),
    ] {
        let holder = Some((1, 10));
        let hours = vec![
            hour(
                100,
                true,
                &[(0, "GER")],
                &[country(0, holder, Some([0, 0, 1]))],
            ),
            hour(101, false, &[], &[country(0, holder, Some([0, 0, 0]))]),
            hour(102, false, &[], &[]),
            hour(
                103,
                true,
                &[(0, "GER")],
                &[country(0, holder, Some([0, 0, 1]))],
            ),
            hour(104, false, &[], &[country(0, holder, Some([1, 1, 2]))]),
        ];
        let budget = Budget::new(32 << 20);
        let built = build(&configured, hours, cuts, resume, &budget);
        let cache = Cache::new(&budget);
        let mut snapshot = Snapshot::open(built.pages, &cache, built.root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
        let actor = directory.actor(b"GER").unwrap();
        for (channel, measure) in emit::SHARES.into_iter().enumerate() {
            let key = directory.key(actor, measure, None).unwrap();
            let runs: Vec<_> = dashboard_base::prepared::engine::runs(&mut snapshot, key, 100, 104)
                .unwrap()
                .into_iter()
                .map(|run| (run.span.first, run.span.last, run.break_before, run.value))
                .collect();
            let before = if channel == 2 { 100_000_000_000_000 } else { 0 };
            let after = if channel == 2 {
                50_000_000_000_000
            } else {
                25_000_000_000_000
            };
            assert_eq!(
                runs,
                vec![
                    (100, 100, true, before),
                    (103, 103, true, before),
                    (104, 104, false, after)
                ],
                "share {channel}, cuts {cuts:?}, resume {resume:?}"
            );
        }
        let history = history(&[("GER", &[(100, 104, 1, 10)])]);
        let params = json!({ "branchId": "1", "tags": ["GER"], "from": 100, "to": 104 });
        let reply = query::reply_to(
            &mut snapshot,
            &directory,
            &params,
            &history,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            reply.series[0].runs,
            vec![
                run(100, 100, true, [0, 0, 1]),
                run(101, 102, false, [0, 0, 0]),
                run(103, 103, false, [0, 0, 1]),
                run(104, 104, false, [1, 1, 2]),
            ]
        );
    }
}

/// Without land inputs the family is unavailable, whatever its content says; a refused
/// content is the reply's refusal once inputs exist.
#[test]
fn unavailable_and_refused_content_reply_as_before() {
    let budget = Budget::new(16 << 20);
    let cache = Cache::new(&budget);
    let history = history(&[("GER", &[(0, 3, 1, 10)])]);
    let cancelled = AtomicBool::new(false);
    let params = json!({ "branchId": "1", "tags": ["GER", "GER"], "from": 0, "to": 3 });
    let configured = Configured::with(Ok(defines()), vec!["Unverified".into()]);
    let mut hours: Vec<Hour> = (0..4)
        .map(|at| hour(at, at == 0, &[(0, "GER")], &[]))
        .collect();
    for hour in &mut hours {
        hour.land_present = false;
    }
    let built = build(&configured, hours, &[3], None, &budget);
    let mut snapshot = Snapshot::open(built.pages, &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
    let reply = query::reply_to(&mut snapshot, &directory, &params, &history, &cancelled).unwrap();
    assert!(!reply.available);
    assert_eq!(reply.missing_inputs, vec!["DivisionLandInputs".to_owned()]);
    assert!(reply.warnings.is_empty());
    assert_eq!(reply.series.len(), 1);
    assert!(reply.series[0].identities.is_none());

    let refused = Configured::with(Err("Supply health refused 1 define(s): x".into()), vec![]);
    let hours = (0..4).map(|at| hour(at, at == 0, &[], &[])).collect();
    let built = build(&refused, hours, &[1, 3], None, &budget);
    let mut snapshot = Snapshot::open(built.pages, &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
    let error =
        query::reply_to(&mut snapshot, &directory, &params, &history, &cancelled).unwrap_err();
    assert_eq!(error.to_string(), "Supply health refused 1 define(s): x");
}

/// 1,500 countries over 200 hours, 25 changing each hour and every 24th hour a cut: the emit
/// keeps only open edges, and every country's runs read back equal to the model.
#[test]
fn fifteen_hundred_countries_keep_their_runs_through_cuts() {
    let configured = Configured::with(Ok(defines()), vec![]);
    let count = 1500_u32;
    let mut model: Vec<Vec<SupplyHealthRun>> = vec![Vec::new(); count as usize];
    let mut held = vec![[0_u32, 0, 1]; count as usize];
    let mut hours = Vec::new();
    let named: Vec<(u32, String)> = (0..count)
        .map(|index| (index, format!("C{index:04}")))
        .collect();
    for at in 0..200 {
        let mut countries = Vec::new();
        if at == 0 {
            for index in 0..count {
                countries.push(country(
                    index,
                    Some((index, index)),
                    Some(held[index as usize]),
                ));
            }
        } else {
            for change in 0..25 {
                let index = (at as u32 * 61 + change * 7) % count;
                let counts = &mut held[index as usize];
                counts[(at as usize + change as usize) % 3] += 1;
                countries.push(country(index, Some((index, index)), Some(*counts)));
            }
            countries.sort_by_key(|country| country.index);
        }
        for country in &countries {
            let runs = &mut model[country.index as usize];
            let counts = country.counts.unwrap();
            match runs.last_mut() {
                Some(last) if [last.no_supply, last.strained, last.in_supply] == counts => {}
                last => {
                    let starts = last.is_none();
                    runs.push(run(at, at, starts, counts));
                }
            }
        }
        hours.push(Hour {
            branch: 1,
            hour: at,
            named: if at == 0 {
                named
                    .iter()
                    .map(|(index, tag)| (*index, Arc::from(tag.as_str())))
                    .collect()
            } else {
                Vec::new()
            },
            whole: at == 0,
            countries,
            land_present: true,
        });
    }
    for runs in &mut model {
        for at in 0..runs.len() {
            runs[at].last_hour = runs.get(at + 1).map_or(199, |next| next.first_hour - 1);
        }
    }
    let cuts: Vec<i32> = (23..200).step_by(24).chain([199]).collect();
    let budget = Budget::new(512 << 20);
    let built = build(&configured, hours, &cuts, None, &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages, &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
    assert_eq!(directory.actors.len(), count as usize);
    let tags: Vec<String> = (0..count)
        .step_by(23)
        .map(|index| format!("C{index:04}"))
        .collect();
    let countries: Vec<(&str, Vec<(i32, i32, u32, u32)>)> = tags
        .iter()
        .zip((0..count).step_by(23))
        .map(|(tag, index)| (tag.as_str(), vec![(0, 199, index, index)]))
        .collect();
    let history = history(
        &countries
            .iter()
            .map(|(tag, runs)| (*tag, runs.as_slice()))
            .collect::<Vec<_>>(),
    );
    let params = json!({ "branchId": "1", "tags": tags, "from": 0, "to": 199 });
    let reply = query::reply_to(
        &mut snapshot,
        &directory,
        &params,
        &history,
        &AtomicBool::new(false),
    )
    .unwrap();
    for series in &reply.series {
        let index: usize = series.tag[1..].parse().unwrap();
        assert_eq!(series.runs, model[index], "{}", series.tag);
    }
}

/// The projector on a synthetic walk: the first hour hands every country, then only the
/// countries a division change reached; an hour without DivisionStatus unknows them all; a
/// segment's first hour hands every country again.
#[test]
fn the_projector_hands_only_what_changed() {
    let tables = [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Divisions,
        TableId::DivisionLandInputs,
        TableId::DivisionStatus,
    ];
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
                reason: "synthetic refusal",
                read,
                sample,
            },
        };
        walk.table(table, input).unwrap();
    }
    let countries = [("GER", 1_u32), ("ZZZ", 5)].map(|(tag, id)| CountryRow {
        id,
        canonical_slot: id,
        tag: tag.as_bytes().to_vec(),
        ..Default::default()
    });
    let links = [1_u32, 5].map(|id| CountryLinkRow {
        id,
        canonical_slot: id,
    });
    let division = |id: u64, expeditionary: u32| DivisionRow {
        id,
        roster_owner: 1,
        expeditionary_owner: expeditionary,
        ..Default::default()
    };
    let land = |division: u64, naval_transfer: bool| DivisionLandInputsRow {
        division,
        naval_transfer,
        ..Default::default()
    };
    let status = |division: u64, received: i64, remaining: i64| DivisionStatusRow {
        division,
        supply_ratio: received,
        supply_grace_maximum: Recorded::Value(1_000_000),
        supply_grace_remaining: Recorded::Value(remaining),
        ..Default::default()
    };
    // In supply, no supply, strained (frame 2), strained (frame 1).
    let statuses = |second: i64| {
        vec![
            status(1, 0, 1_000_000),
            status(2, second, 0),
            status(3, 15_000, 0),
            status(4, 40_000, 0),
        ]
    };
    let lands: Vec<_> = (1..=4).map(|id| land(id, false)).collect();
    let script: Vec<(Vec<DivisionRow>, Option<Vec<DivisionStatusRow>>)> = vec![
        (
            (1..=4).map(|id| division(id, 0)).collect(),
            Some(statuses(14_999)),
        ),
        (
            (1..=4).map(|id| division(id, 0)).collect(),
            Some(statuses(100_000)),
        ),
        (
            [
                division(1, 5),
                division(2, 0),
                division(3, 0),
                division(4, 0),
            ]
            .to_vec(),
            Some(statuses(100_000)),
        ),
        (
            [
                division(1, 5),
                division(2, 0),
                division(3, 0),
                division(4, 0),
            ]
            .to_vec(),
            None,
        ),
        (
            [
                division(1, 5),
                division(2, 0),
                division(3, 0),
                division(4, 0),
            ]
            .to_vec(),
            Some(statuses(100_000)),
        ),
        (
            [
                division(1, 5),
                division(2, 0),
                division(3, 0),
                division(4, 0),
            ]
            .to_vec(),
            Some(statuses(100_000)),
        ),
    ];
    let configured = Configured::with(Ok(defines()), vec![]);
    let mut walk = Synthetic::new(&tables, 1, 100).unwrap();
    let mut held: Option<dashboard_world::World> = None;
    let mut projector = configured.projector(FormatVersion::CURRENT).unwrap();
    let mut out: Vec<Hour> = Vec::new();
    for (ordinal, (divisions, statuses)) in (0..).zip(script) {
        write(&mut walk, TableId::Countries, Some(&countries[..]));
        write(&mut walk, TableId::CountryLinks, Some(&links[..]));
        write(&mut walk, TableId::Divisions, Some(&divisions[..]));
        write(&mut walk, TableId::DivisionLandInputs, Some(&lands[..]));
        write(&mut walk, TableId::DivisionStatus, statuses.as_deref());
        // The last hour starts a segment: a new projector over a new world.
        let segment = ordinal == 5;
        if segment {
            projector = configured.projector(FormatVersion::CURRENT).unwrap();
            held = None;
        }
        walk.end(|state, manifest, frame, starts| {
            let starts = starts || segment;
            let world = dashboard_world::World::at(
                &mut held,
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
                        hour: frame.hour,
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
    let (ger, zzz) = (Some((1, 1)), Some((5, 5)));
    let handed: Vec<(bool, Vec<Country>)> = out
        .iter()
        .map(|hour| (hour.whole, hour.countries.clone()))
        .collect();
    assert_eq!(
        handed,
        vec![
            (
                true,
                vec![
                    country(0, ger, Some([1, 2, 1])),
                    country(1, zzz, Some([0, 0, 0]))
                ]
            ),
            (false, vec![country(0, ger, Some([0, 2, 2]))]),
            (
                false,
                vec![
                    country(0, ger, Some([0, 2, 1])),
                    country(1, zzz, Some([0, 0, 1]))
                ]
            ),
            (false, vec![country(0, ger, None), country(1, zzz, None)]),
            (
                false,
                vec![
                    country(0, ger, Some([0, 2, 1])),
                    country(1, zzz, Some([0, 0, 1]))
                ]
            ),
            (
                true,
                vec![
                    country(0, ger, Some([0, 2, 1])),
                    country(1, zzz, Some([0, 0, 1]))
                ]
            ),
        ]
    );
    assert_eq!(out[0].named.len(), 2);
    assert!(out[1..5].iter().all(|hour| hour.named.is_empty()));
    assert_eq!(out[5].named.len(), 2);
}

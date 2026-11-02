use super::{
    aid::{self, Party, Sender},
    emit::{Emit, FAMILY, POSITIONS, RANKS, SCHEMA},
    facts::{self, Country, Hour, Projector},
    query, *,
};
use crate::products::volunteers_expeditionaries::{
    self as volunteers, MilitaryAidKind, VolunteersExpeditionariesParams,
};
use dashboard_base::{
    prepared::{coverage::Coverage, prefix::Prefix},
    products::country_history::{CountryIdentityRun, RecordedCountry, RecordedHistories},
};
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
        CountryLinkRow, CountryRow, DivisionCensusRow, DivisionLandInputsRow,
        DivisionManpowerContributor, DivisionRow, DivisionTemplateIconRow, LexicalTokenRow,
        RowCodec, TemplateSubunit,
    },
    scope::CaptureMode,
};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [4; 32],
        content: [5; 32],
        branch: [6; 32],
        schema: 2,
    }
}

/// A publication built from projected hours, cut after each of `cuts`' hours; when `resume`
/// names one of them, the emit is saved there and goes on from the publication.
struct Built {
    pages: Memory,
    root: u64,
}

fn build(hours: Vec<Hour>, cuts: &[i32], resume: Option<i32>, budget: &Budget) -> Built {
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
            emit = Emit::load(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

fn hour(at: i32, named: &[(u32, &str)], live: Option<&[u32]>, countries: Vec<Country>) -> Hour {
    Hour {
        branch: 1,
        hour: at,
        named: named
            .iter()
            .map(|&(index, tag)| (index, Arc::from(tag)))
            .collect(),
        live: live.map(<[u32]>::to_vec),
        countries,
        divisions: true,
        strength: true,
        manpower: true,
        battalions: true,
        fallback: false,
        unreadable: 0,
        composition_unreadable: 0,
        invalid: Vec::new(),
        aid: aid::Projection::default(),
    }
}

/// Every channel of a country from its divisions, HP, battalions and manpower.
fn totals(divisions: i128) -> Totals {
    [
        Some(divisions),
        Some(divisions * 100_000),
        Some(divisions),
        Some(divisions * 100_000),
        Some(divisions * 9),
        Some(divisions * 9),
        Some(divisions * 1_000),
        Some(divisions * 1_000),
    ]
}

fn country(index: u32, holder: (u32, u32), divisions: Option<i128>) -> Country {
    Country {
        index,
        holder,
        totals: divisions.map(totals),
        mix: None,
    }
}

fn history(countries: &[(&str, (i32, i32, u32, u32))]) -> RecordedHistories {
    let mut history = RecordedHistories::default();
    for &(tag, (first_hour, last_hour, slot, country_id)) in countries {
        history.countries.insert(
            tag.to_owned(),
            RecordedCountry::identified(vec![CountryIdentityRun {
                first_hour,
                last_hour,
                slot,
                country_id,
            }]),
        );
    }
    history
}

fn raw(first: i32, last: i32, starts: bool, value: i128) -> (i32, i32, bool, i128) {
    (first, last, starts, value)
}

fn runs_of(
    snapshot: &mut Snapshot<'_, Memory>,
    directory: &Directory,
    tag: &str,
    measure: u64,
    range: (i32, i32),
) -> Vec<(i32, i32, bool, i128)> {
    let actor = directory.actor(tag.as_bytes()).unwrap();
    let key = directory.key(actor, measure, None).unwrap();
    dashboard_base::prepared::engine::runs(snapshot, key, range.0, range.1)
        .unwrap()
        .into_iter()
        .map(|run| (run.span.first, run.span.last, run.break_before, run.value))
        .collect()
}

/// GER is projected only where it changes; FRA's totals turn unknown at 2 and its holder
/// changes at 8; nothing is read at hour 6. Runs go on through unprojected hours and across
/// segments, end at the unknown and the unread hour, and break at the holder change; ranks
/// follow. The publication is cut at 3 and resumed from its saved emit at the segment
/// starting at 4.
#[test]
fn runs_ranks_and_cells_follow_holders_unknowns_gaps_cuts_and_a_resume() {
    let budget = Budget::new(256 << 20);
    let named = [(0, "GER"), (1, "FRA")];
    let (ger, fra) = ((1, 1), (2, 2));
    let hours = vec![
        hour(
            0,
            &named,
            Some(&[0, 1]),
            vec![country(0, ger, Some(10)), country(1, fra, Some(5))],
        ),
        hour(1, &[], None, vec![country(1, fra, Some(5))]),
        hour(2, &[], None, vec![country(1, fra, None)]),
        hour(
            3,
            &named,
            Some(&[0, 1]),
            vec![country(0, ger, Some(12)), country(1, fra, Some(20))],
        ),
        hour(
            4,
            &named,
            Some(&[0, 1]),
            vec![country(0, ger, Some(12)), country(1, fra, Some(20))],
        ),
        hour(5, &[], None, vec![country(0, ger, Some(12))]),
        hour(
            7,
            &[],
            Some(&[0, 1]),
            vec![country(0, ger, Some(12)), country(1, fra, Some(20))],
        ),
        hour(8, &[], None, vec![country(1, (2, 3), Some(20))]),
        hour(9, &[], None, Vec::new()),
    ];
    let built = build(hours, &[3, 9], Some(3), &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages, &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let divisions = channel(ArmyMeasure::Divisions, ArmyAttribution::Owner) as u64;
    assert_eq!(
        runs_of(&mut snapshot, &directory, "GER", divisions, (0, 9)),
        vec![
            raw(0, 2, true, 10),
            raw(3, 5, false, 12),
            raw(7, 9, true, 12)
        ]
    );
    assert_eq!(
        runs_of(&mut snapshot, &directory, "FRA", divisions, (0, 9)),
        vec![
            raw(0, 1, true, 5),
            raw(3, 5, true, 20),
            raw(7, 7, true, 20),
            raw(8, 9, true, 20)
        ]
    );
    let strength = channel(ArmyMeasure::Strength, ArmyAttribution::Host) as u64;
    assert_eq!(
        runs_of(&mut snapshot, &directory, "GER", strength, (0, 9)),
        vec![
            raw(0, 2, true, 1_000_000),
            raw(3, 5, false, 1_200_000),
            raw(7, 9, true, 1_200_000)
        ]
    );
    // The first rank by owner divisions, and each country's rank.
    assert_eq!(
        runs_of_family(&mut snapshot, &directory, POSITIONS + divisions, 0),
        vec![
            raw(0, 2, false, 0),
            raw(3, 5, false, 1),
            raw(7, 9, false, 1)
        ]
    );
    assert_eq!(
        runs_of(&mut snapshot, &directory, "GER", RANKS + divisions, (0, 9)),
        vec![
            raw(0, 2, false, 1),
            raw(3, 5, false, 2),
            raw(7, 9, false, 2)
        ]
    );
    assert_eq!(
        runs_of(&mut snapshot, &directory, "FRA", RANKS + divisions, (0, 9)),
        vec![
            raw(0, 1, false, 2),
            raw(3, 5, false, 1),
            raw(7, 9, false, 1)
        ]
    );
    let cells = query::cells(&mut snapshot, &directory, &["GER", "FRA", "ITA"], 2).unwrap();
    assert_eq!(
        cells,
        vec![(Some(10_000), true), (None, false), (None, false)]
    );
    assert_eq!(
        query::cells(&mut snapshot, &directory, &["GER"], 6).unwrap(),
        vec![(None, false)],
        "an unread hour is unknown, never zero"
    );
    let history = history(&[("GER", (0, 9, 1, 1)), ("FRA", (0, 9, 2, 2))]);
    let cancelled = AtomicBool::new(false);
    let ranked = query::summary(
        &serde_json::json!({
            "branchId": "1", "tags": [], "from": 0, "to": 9, "width": 10,
            "family": {"kind": "army-size", "measure": "divisions", "countFor": "owner", "top": 1},
        }),
        &mut snapshot,
        &directory,
        &history,
        (0, 9),
        &cancelled,
    )
    .unwrap();
    let tags: Vec<&str> = ranked["series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line["tag"].as_str().unwrap())
        .collect();
    assert_eq!(tags, ["FRA", "GER"]);
    let next = query::next_change(
        &serde_json::json!({
            "branchId": "1", "tags": ["GER"], "key": "ArrowRight", "from": 0,
            "firstHour": 0, "lastHour": 9,
            "family": {"kind": "army-size", "measure": "divisions", "countFor": "owner"},
        }),
        &mut snapshot,
        &directory,
        &history,
        &cancelled,
    )
    .unwrap();
    assert_eq!(next["hour"], 3.0);
}

fn runs_of_family(
    snapshot: &mut Snapshot<'_, Memory>,
    directory: &Directory,
    measure: u64,
    position: u64,
) -> Vec<(i32, i32, bool, i128)> {
    let key = directory.key(FAMILY, measure, Some(position)).unwrap();
    dashboard_base::prepared::engine::runs(snapshot, key, 0, 9)
        .unwrap()
        .into_iter()
        .map(|run| (run.span.first, run.span.last, run.break_before, run.value))
        .collect()
}

fn next(seed: &mut u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *seed >> 33
}

/// 1,500 countries over 80 hours, some changing each hour, some unknown, cut four times and
/// resumed from the saved emit once: each rank's country and each country's rank read back
/// as a fresh sort of that hour's known totals (larger first, ties by tag) gives them.
#[test]
fn fifteen_hundred_countries_rank_as_a_fresh_sort_through_cuts_and_a_resume() {
    const COUNT: u32 = 1500;
    let budget = Budget::new(1 << 30);
    let names: Vec<String> = (0..COUNT)
        .map(|at| format!("C{:04}", at % 1000 * 7 % 1000 + at / 1000 * 1000))
        .collect();
    let mut seed = 0x2545_f491_4f6c_dd1d;
    let mut held: Vec<Option<i128>> = vec![None; COUNT as usize];
    let mut models: BTreeMap<i32, Vec<Option<i128>>> = BTreeMap::new();
    let mut hours = Vec::new();
    let cuts = [19, 39, 40, 79];
    for at in 0..80 {
        let starts = at == 0 || at == 40;
        let named: Vec<(u32, &str)> = if starts {
            names
                .iter()
                .enumerate()
                .map(|(index, name)| (index as u32, name.as_str()))
                .collect()
        } else {
            Vec::new()
        };
        let mut countries = Vec::new();
        for index in 0..COUNT {
            let roll = next(&mut seed) % 20;
            let value = match roll {
                0 => None,
                1..=4 => Some((next(&mut seed) % 50) as i128),
                _ if starts || held[index as usize].is_none() && roll < 8 => {
                    Some((next(&mut seed) % 50) as i128)
                }
                _ => continue,
            };
            held[index as usize] = value;
            countries.push(country(index, (index, index), value));
        }
        models.insert(at, held.clone());
        let live: Vec<u32> = (0..COUNT).collect();
        hours.push(hour(at, &named, starts.then_some(&live[..]), countries));
    }
    let built = build(hours, &cuts, Some(39), &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages, &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let channel = channel(ArmyMeasure::Battalions, ArmyAttribution::Host);
    let positions = directory
        .dimensions(FAMILY, POSITIONS + channel as u64)
        .to_vec();
    for at in [0, 7, 19, 20, 39, 40, 41, 63, 79] {
        let mut known: Vec<(i128, &str, u32)> = models[&at]
            .iter()
            .enumerate()
            .filter_map(|(index, value)| {
                Some((
                    value.map(|value| value * 9)?,
                    names[index].as_str(),
                    index as u32,
                ))
            })
            .collect();
        known.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(right.1)));
        for (position, &(_, name, _)) in known.iter().enumerate() {
            let (_, key) = positions[position];
            let held = dashboard_base::prepared::engine::run_at(&mut snapshot, key, at)
                .unwrap()
                .map(|(run, _)| run.value);
            let actor = directory.actor(name.as_bytes()).unwrap();
            assert_eq!(
                held,
                Some(i128::from(actor)),
                "hour {at} position {position}"
            );
            let rank = directory.key(actor, RANKS + channel as u64, None).unwrap();
            assert_eq!(
                dashboard_base::prepared::engine::run_at(&mut snapshot, rank, at)
                    .unwrap()
                    .map(|(run, _)| run.value),
                Some(position as i128 + 1),
                "hour {at} {name}"
            );
        }
        for &(_, key) in &positions[known.len()..] {
            assert!(
                dashboard_base::prepared::engine::run_at(&mut snapshot, key, at)
                    .unwrap()
                    .is_none(),
                "hour {at}: a rank past the known countries holds one"
            );
        }
    }
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
            reason: "synthetic refusal",
            read,
            sample,
        },
    };
    walk.table(table, input).unwrap();
}

/// The census counts owner divisions, the inventory the rest; an expeditionary counts for its
/// owner and its host's roster, manpower wraps each division's signed sum, and template
/// battalions are named by their sub-units. An HP edit hands only its country, unread land
/// inputs leave manpower unknown, and a segment's first hour hands every country again.
#[test]
fn the_projector_counts_the_army_as_the_game_does_and_hands_only_what_changed() {
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
    let census = [
        DivisionCensusRow {
            country: 1,
            grouped: 2,
            ungrouped: 1,
        },
        DivisionCensusRow {
            country: 5,
            grouped: 0,
            ungrouped: 0,
        },
    ];
    let divisions = |hp: i64| {
        vec![
            DivisionRow {
                id: 1,
                roster_owner: 1,
                logical_owner: 1,
                template: 7,
                hit_points: hp,
                ..Default::default()
            },
            DivisionRow {
                id: 2,
                roster_owner: 1,
                logical_owner: 1,
                expeditionary_owner: 5,
                template: 7,
                hit_points: 50_000,
                ..Default::default()
            },
        ]
    };
    let lands = [
        DivisionLandInputsRow {
            division: 1,
            manpower: vec![
                DivisionManpowerContributor {
                    country: 1,
                    amount: i32::MAX,
                },
                DivisionManpowerContributor {
                    country: 1,
                    amount: 1,
                },
            ],
            ..Default::default()
        },
        DivisionLandInputsRow {
            division: 2,
            manpower: vec![DivisionManpowerContributor {
                country: 5,
                amount: 1_000,
            }],
            ..Default::default()
        },
    ];
    let unit = |token, count| TemplateSubunit {
        token,
        local: 0,
        count,
    };
    let templates = [DivisionTemplateIconRow {
        template: 7,
        subunits: vec![unit(1, 2), unit(0, 1), unit(2, 1)],
        ..Default::default()
    }];
    let tokens = [(1, "infantry"), (2, "artillery")].map(|(token, name)| LexicalTokenRow {
        token,
        literal: name.as_bytes().to_vec(),
    });
    // HP 1.0, 1.2, then land inputs unread, then a new segment.
    let script = [
        (100_000, true),
        (120_000, true),
        (120_000, false),
        (120_000, true),
    ];
    let mut walk = Synthetic::new(facts::TABLES, 1, 100).unwrap();
    let mut held: Option<dashboard_world::World> = None;
    let mut projector = Projector::with(FormatVersion::CURRENT, Vec::new());
    let mut out: Vec<Hour> = Vec::new();
    for (ordinal, (hp, land)) in (0..).zip(script) {
        write(&mut walk, TableId::Countries, Some(&countries[..]));
        write(&mut walk, TableId::CountryLinks, Some(&links[..]));
        write(&mut walk, TableId::DivisionCensus, Some(&census[..]));
        write(&mut walk, TableId::Divisions, Some(&divisions(hp)[..]));
        write(
            &mut walk,
            TableId::DivisionTemplateIcons,
            Some(&templates[..]),
        );
        write(
            &mut walk,
            TableId::DivisionLandInputs,
            land.then_some(&lands[..]),
        );
        write(&mut walk, TableId::LexicalTokens, Some(&tokens[..]));
        for table in [
            TableId::SentVolunteers,
            TableId::VolunteerHosts,
            TableId::CountryCounterInputs,
        ] {
            write::<CountryLinkRow>(&mut walk, table, None);
        }
        let segment = ordinal == 3;
        if segment {
            projector = Projector::with(FormatVersion::CURRENT, Vec::new());
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
    let ger = |hp: i128, manpower: bool| -> Totals {
        [
            Some(3),
            Some(hp),
            Some(2),
            Some(hp + 50_000),
            Some(3),
            Some(6),
            manpower.then_some(i128::from(i32::MIN)),
            manpower.then_some(i128::from(i32::MIN) + 1_000),
        ]
    };
    let zzz = |manpower: bool| -> Totals {
        [
            Some(0),
            Some(50_000),
            Some(0),
            Some(0),
            Some(3),
            Some(0),
            manpower.then_some(1_000),
            manpower.then_some(0),
        ]
    };
    let handed: Vec<Vec<(u32, (u32, u32), Option<Totals>)>> = out
        .iter()
        .map(|hour| {
            hour.countries
                .iter()
                .map(|country| (country.index, country.holder, country.totals))
                .collect()
        })
        .collect();
    assert_eq!(
        handed,
        vec![
            vec![
                (0, (1, 1), Some(ger(100_000, true))),
                (1, (5, 5), Some(zzz(true)))
            ],
            vec![(0, (1, 1), Some(ger(120_000, true)))],
            vec![
                (0, (1, 1), Some(ger(120_000, false))),
                (1, (5, 5), Some(zzz(false)))
            ],
            vec![
                (0, (1, 1), Some(ger(120_000, true))),
                (1, (5, 5), Some(zzz(true)))
            ],
        ]
    );
    let lives: Vec<Option<Vec<u32>>> = out.iter().map(|hour| hour.live.clone()).collect();
    assert_eq!(lives, vec![Some(vec![0, 1]), None, None, Some(vec![0, 1])]);
    let named: Vec<Vec<(u32, String)>> = out
        .iter()
        .map(|hour| {
            hour.named
                .iter()
                .map(|(index, tag)| (*index, tag.to_string()))
                .collect()
        })
        .collect();
    let both = vec![(0, "GER".to_owned()), (1, "ZZZ".to_owned())];
    assert_eq!(named, vec![both.clone(), Vec::new(), Vec::new(), both]);
    assert!(out.iter().all(|hour| hour.divisions && hour.battalions));
    assert_eq!(
        out.iter().map(|hour| hour.manpower).collect::<Vec<_>>(),
        [true, true, false, true]
    );
    let parts = |pairs: &[(&str, i64)]| -> Option<BTreeMap<String, i64>> {
        Some(
            pairs
                .iter()
                .map(|&(name, count)| (name.to_owned(), count))
                .collect(),
        )
    };
    let first = &out[0].countries;
    assert_eq!(
        first[0].mix.as_deref(),
        Some(&[
            parts(&[("artillery", 1), ("infantry", 2)]),
            parts(&[("artillery", 2), ("infantry", 4)])
        ])
    );
    assert_eq!(
        first[1].mix.as_deref(),
        Some(&[parts(&[("artillery", 1), ("infantry", 2)]), parts(&[])])
    );
    // An HP edit keeps the composition the projection handed before.
    assert!(Arc::ptr_eq(
        first[0].mix.as_ref().unwrap(),
        out[1].countries[0].mix.as_ref().unwrap()
    ));
}

fn party(slot: u32, tag: &str) -> Party {
    Party {
        slot,
        id: slot,
        tag: Arc::from(tag),
    }
}

fn channel_of(complete: bool, values: &[(&Party, i64)], reasons: &[&str]) -> aid::Channel {
    aid::Channel {
        complete,
        values: values
            .iter()
            .map(|&(party, count)| (party.clone(), count))
            .collect(),
        reasons: reasons
            .iter()
            .map(|&reason| (reason.to_owned(), 1))
            .collect(),
    }
}

/// GER sends ITA two volunteers, then three; its host tally stays at two, then goes unread:
/// the land link holds 2 then 3, the tally 2 then nothing, the unread hour is unobserved, and
/// the tally's disagreement and refusal are warned once each.
#[test]
fn volunteer_links_read_back_with_their_tallies_unobserved_hours_and_warnings() {
    let budget = Budget::new(64 << 20);
    let (ger, ita) = (party(1, "GER"), party(3, "ITA"));
    let sender = |land: i64, tally: Option<i64>| Sender {
        party: ger.clone(),
        channels: [
            channel_of(true, &[(&ita, land)], &[]),
            channel_of(true, &[], &[]),
            match tally {
                Some(tally) => channel_of(true, &[(&ita, tally)], &[]),
                None => channel_of(false, &[], &["VolunteerHosts unavailable"]),
            },
        ],
        hosts: Vec::new(),
        volunteers: Vec::new(),
        expeditionaries: None,
    };
    let aided = |at: i32, parties: bool, senders: Vec<Sender>, reads: [bool; 3]| {
        let mut hour = hour(at, &[], None, Vec::new());
        hour.aid = aid::Projection {
            parties: parties.then(|| vec![ger.clone(), ita.clone()]),
            senders,
            reads,
            reasons: Default::default(),
        };
        hour
    };
    let hours = vec![
        aided(0, true, vec![sender(2, Some(2))], [true; 3]),
        aided(1, false, vec![sender(3, Some(2))], [true; 3]),
        aided(2, false, vec![sender(3, None)], [true, true, false]),
    ];
    let built = build(hours, &[2], None, &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages, &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let prefix = Prefix {
        coverage: Coverage {
            schema: dashboard_base::prepared::coverage::SCHEMA,
            first_hour: 0,
            last_hour: 2,
            complete: true,
        },
        pending: Vec::new(),
    };
    let params = VolunteersExpeditionariesParams {
        branch_id: "1".into(),
        kind: MilitaryAidKind::Land,
        tags: None,
    };
    let reply = volunteers::read(
        &mut snapshot,
        &directory,
        &params,
        prefix.clone(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let runs = |link: &volunteers::MilitaryAidLink| -> Vec<(i32, i32, Option<f64>, bool)> {
        link.runs
            .iter()
            .map(|run| {
                (
                    run.first_hour,
                    run.last_hour,
                    run.divisions,
                    run.starts_segment,
                )
            })
            .collect()
    };
    assert_eq!(reply.links.len(), 1);
    assert_eq!(
        (
            reply.links[0].key.as_str(),
            reply.links[0].sender.as_str(),
            reply.links[0].host_slot
        ),
        ("army-aid:1:0:0", "GER", Some(3))
    );
    assert_eq!(
        runs(&reply.links[0]),
        vec![(0, 0, Some(2.0), true), (1, 2, Some(3.0), false)]
    );
    assert_eq!(
        runs(&reply.tallies[0]),
        vec![(0, 1, Some(2.0), true), (2, 2, None, false)]
    );
    assert_eq!(
        reply.unobserved,
        vec![dashboard_base::products::country_groups::HourSpan {
            first_hour: 2,
            last_hour: 2
        }]
    );
    assert_eq!(
        reply.warnings,
        vec![
            "1 VolunteerHosts tally differs from resolved volunteer list".to_owned(),
            "1 VolunteerHosts unavailable".to_owned(),
        ]
    );
    let expeditionary = volunteers::read(
        &mut snapshot,
        &directory,
        &VolunteersExpeditionariesParams {
            kind: MilitaryAidKind::Expeditionary,
            ..params
        },
        prefix,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(expeditionary.links.is_empty() && expeditionary.tallies.is_empty());
    assert!(expeditionary.unobserved.is_empty());
}

/// A window's reply names the selected country's runs within its identity, and nothing
/// before the family read any hour stays unknown while preparing.
#[test]
fn a_window_reply_reads_the_selected_country_within_its_identity() {
    let budget = Budget::new(64 << 20);
    let named = [(0, "GER"), (1, "FRA")];
    let hours = vec![
        hour(
            0,
            &named,
            Some(&[0, 1]),
            vec![country(0, (1, 1), Some(10)), country(1, (2, 2), Some(5))],
        ),
        hour(1, &[], None, vec![country(0, (1, 1), Some(11))]),
        hour(2, &[], None, Vec::new()),
        hour(3, &[], None, Vec::new()),
    ];
    let built = build(hours, &[3], None, &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages, &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let fixture = crate::products::force_fixtures::Fixture::recording_only();
    let params = ArmySizeParams {
        template_at: None,
        branch_id: "1".into(),
        tags: vec!["GER".into()],
        top: None,
        measure: ArmyMeasure::Divisions,
        count_for: ArmyAttribution::Owner,
        by_type: false,
        window: Some(0),
    };
    let reply = query::reply(
        &fixture.source,
        &LoadContext { content: None },
        &params,
        &mut snapshot,
        &directory,
        &history(&[("GER", (0, 3, 1, 1)), ("FRA", (0, 3, 2, 2))]),
        true,
        &AtomicBool::new(false),
    )
    .unwrap();
    // The fixture's first block holds hours 0 and 1.
    assert_eq!(
        reply.interval,
        Some(dashboard_base::products::country_groups::HourSpan {
            first_hour: 0,
            last_hour: 1
        })
    );
    assert!(reply.available);
    assert!(reply.unknown_measures.is_empty());
    assert_eq!(reply.series.len(), 1);
    assert_eq!(reply.series[0].tag, "GER");
    assert_eq!(
        reply.series[0]
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.raw.as_str()))
            .collect::<Vec<_>>(),
        [(0, 0, "10"), (1, 1, "11")]
    );
}

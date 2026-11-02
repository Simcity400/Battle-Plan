//! The family's columnar preparation held to the row preparation ([`super::row`]) on the same
//! hours, written as the native recorder writes them: every view's history and ledger page,
//! their cursors included, across segment starts, a gap, cuts and a resume; and an hour's work
//! bounded by what it changed.

use super::{
    Field,
    emit::{Emit, SCHEMA},
    facts::{self, Events, Facts, Occurrence, Projector, Row, Table},
    query::{self, Catalogue},
    row, *,
};
use crate::products::test_support::{countries, frame, set, unavailable};
use dashboard_series::{
    Blob, Budget, Cache, Identity, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{tables, *},
    scope::CaptureMode,
};
use dashboard_base::products::equipment::{RawVariant, VariantHour};
use replay_frame::frame::{DecodedFrame, Frame, FrameDecoder, TableUpdate, TableView};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};

fn pool(items: &[(u64, i64)]) -> SharedMarketPool {
    MarketPool {
        allow_zero: false,
        items: items
            .iter()
            .map(|&(variant, amount_raw)| MarketEquipment { variant, amount_raw })
            .collect(),
    }
    .into()
}

fn levels(levels: &[(u64, i32)]) -> SharedMarketPriceLevels {
    levels
        .iter()
        .map(|&(variant, level)| MarketPriceLevel { variant, level })
        .collect::<Vec<_>>()
        .into()
}

fn sorted<R: RowCodec>(mut rows: Vec<R>) -> Vec<R> {
    rows.sort_by_key(RowCodec::key);
    rows
}

fn occurrence(id: u64, kind: u8, hour: i32, seller: u32, buyer: u32) -> MarketEventRow {
    MarketEventRow {
        id,
        occurrence: id + 1000,
        hour,
        agreement: 9,
        country: seller,
        receiver: buyer,
        kind,
        variant: 0xa1,
        amount_raw: 100_000 + i64::from(hour),
        total_raw: 7,
        ..Default::default()
    }
}

/// Hours 100 to 140, without 125 and 126. SEL (slot 10, also linked from raw id 11) lists two
/// equipment entries, one amount changing at 105, raw id 11 lists the same entry from 108 to
/// 110, and a row no link resolves stands at 112 and 113; SEL sells BUY a contract whose speed
/// changes at 104, and requests from an unknown buyer from 106 to 114; the terms are unavailable
/// at 120 and 121; BUY lends OTH from 103, revised at 107; a purchase progresses every few hours;
/// the countries are unavailable at 118; OTH is renamed OTX at 130. The occurrences drain hourly:
/// one is carried twice, one names an unknown buyer, one lags its read, one has no amount, one is
/// drained while the identities are unavailable, and they are unavailable at 135.
fn scenario() -> Vec<DecodedFrame> {
    (100..=140)
        .filter(|hour| !(125..=126).contains(hour))
        .map(|hour| {
            let mut at = frame(hour);
            let other = if hour >= 130 { "OTX" } else { "OTH" };
            countries(&mut at, &[(1, 10, "SEL"), (2, 20, "BUY"), (3, 30, other)]);
            let mut links = vec![
                CountryLinkRow { id: 1, canonical_slot: 10 },
                CountryLinkRow { id: 2, canonical_slot: 20 },
                CountryLinkRow { id: 3, canonical_slot: 30 },
                CountryLinkRow { id: 11, canonical_slot: 10 },
            ];
            links.sort_by_key(|link| link.id);
            set(&mut at, TableRows::CountryLinks(links));
            if hour == 118 {
                unavailable(&mut at, TableId::Countries);
            }
            let amount = if hour < 105 { 500_000 } else { 600_000 };
            let mut market = vec![CountryMarketRow {
                country: 1,
                inventory: pool(&[(0xa1, amount), (0xa2, 100)]),
                prices: levels(&[(0xa1, 2)]),
                ..Default::default()
            }];
            if (108..=110).contains(&hour) {
                market.push(CountryMarketRow {
                    country: 11,
                    inventory: pool(&[(0xa1, 9)]),
                    ..Default::default()
                });
            }
            if (112..=113).contains(&hour) {
                market.push(CountryMarketRow {
                    country: 99,
                    inventory: pool(&[(0xa3, 1)]),
                    ..Default::default()
                });
            }
            set(&mut at, TableRows::CountryMarket(sorted(market)));
            let mut terms = vec![MarketContractTermsRow {
                country: 1,
                buyer: 2,
                id: 9,
                kind: 1,
                speed: if hour < 104 { 3 } else { 4 },
                equipment: pool(&[(0xa1, 500_000)]),
                prices: pool(&[(0xa1, 2_000_000)]),
                price_levels: levels(&[(0xa1, 1)]),
                runtime: MarketContractRuntime {
                    route_length: hour,
                    ..Default::default()
                },
                ..Default::default()
            }];
            if (106..=114).contains(&hour) {
                terms.push(MarketContractTermsRow {
                    country: 1,
                    buyer: 77,
                    id: 8,
                    kind: 0,
                    speed: 1,
                    equipment: pool(&[(0xa2, 50)]),
                    ..Default::default()
                });
            }
            set(&mut at, TableRows::MarketContractTerms(sorted(terms)));
            if (120..=121).contains(&hour) {
                unavailable(&mut at, TableId::MarketContractTerms);
            }
            let leases = if hour >= 103 {
                vec![LendLeaseAgreementRow {
                    country: 2,
                    receiver: 3,
                    id: 5,
                    revision: if hour < 107 { 1 } else { 2 },
                    active_since: 103,
                    last_delivery: hour - hour % 4,
                    monthly: pool(&[(0xa2, 1_000)]),
                    fuel_daily_raw: 32_768,
                    ..Default::default()
                }]
            } else {
                Vec::new()
            };
            set(&mut at, TableRows::LendLeaseAgreements(leases));
            set(
                &mut at,
                TableRows::PurchaseContracts(vec![PurchaseContractRow {
                    id: 9,
                    seller: 1,
                    buyer: 2,
                    convoys_assigned: 2,
                    convoys_total: 3,
                    collected_raw: i64::from(hour / 3),
                    completed_deliveries: (hour - 100) / 6,
                    route_days: hour,
                    ..Default::default()
                }]),
            );
            let events = match hour {
                101 => vec![occurrence(1, 10, 101, 1, 77)],
                103 => vec![occurrence(2, 11, 103, 1, 2)],
                105 | 106 => vec![occurrence(3, 20, 105, 1, 2)],
                110 => vec![occurrence(4, 21, 110, 2, 1), occurrence(5, 20, 110, 1, 2)],
                114 => vec![MarketEventRow {
                    mode: 20,
                    ordinal: 2,
                    ..occurrence(6, 34, 114, 2, 1)
                }],
                118 => vec![occurrence(7, 20, 118, 1, 2)],
                122 => vec![occurrence(8, 12, 122, 1, 2)],
                128 => vec![occurrence(9, 20, 127, 2, 3), occurrence(10, 21, 128, 3, 2)],
                132 => vec![occurrence(11, 20, 132, 2, 3)],
                138 => vec![occurrence(12, 20, 138, 1, 2)],
                _ => Vec::new(),
            };
            set(&mut at, TableRows::MarketEvents(sorted(events)));
            if hour == 135 {
                unavailable(&mut at, TableId::MarketEvents);
            }
            at
        })
        .collect()
}

fn write<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame) {
    let (read, sample) = (true, Sample::FromReadBit);
    let input = match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        TableView::Truncated { rows, missing } => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: Some(*missing),
            read,
            sample,
        },
        TableView::Unavailable { reason } => Input::Unavailable {
            reason,
            read,
            sample,
        },
        TableView::NotRecorded { .. } => Input::NotRecorded { read, sample },
    };
    walk.table(T::ID, input).unwrap();
}

fn write_hour(walk: &mut Synthetic, frame: &DecodedFrame) {
    write::<tables::Campaign>(walk, frame);
    write::<tables::Countries>(walk, frame);
    write::<tables::CountryLinks>(walk, frame);
    write::<tables::CountryAppearance>(walk, frame);
    write::<tables::LexicalTokens>(walk, frame);
    write::<tables::VariantDefinitions>(walk, frame);
    write::<tables::VariantModules>(walk, frame);
    write::<tables::VariantUpgrades>(walk, frame);
    write::<tables::VariantChanges>(walk, frame);
    write::<tables::CountryMarket>(walk, frame);
    write::<tables::MarketContractTerms>(walk, frame);
    write::<tables::LendLeaseAgreements>(walk, frame);
    write::<tables::PurchaseContracts>(walk, frame);
    write::<tables::MarketEvents>(walk, frame);
}

/// The projected hours of `frames`, a projector starting cold at each of `starts`, from the
/// state of the hour before, and at each walk's first hour (after a gap, the walk starts
/// again).
fn projected(frames: &[DecodedFrame], starts: &[i32]) -> Vec<facts::Hour> {
    let blocks: Vec<_> = frames
        .iter()
        .enumerate()
        .filter(|&(index, frame)| index == 0 || frames[index - 1].hour + 1 != frame.hour)
        .map(|(_, frame)| frame.hour)
        .collect();
    projected_blocks(frames, starts, &blocks)
}

fn projected_blocks(frames: &[DecodedFrame], starts: &[i32], blocks: &[i32]) -> Vec<facts::Hour> {
    let format = FormatVersion::CURRENT;
    let needs = facts::needs().unwrap();
    let blocks: Arc<BTreeMap<u64, Arc<[i32]>>> = Arc::new([(1, Arc::from(blocks))].into());
    let mut out = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(format, CaptureMode::Full, Arc::clone(&blocks)).unwrap();
    let mut before: Option<i32> = None;
    for (ordinal, frame) in (0..).zip(frames) {
        let gap = before.is_some_and(|last| last + 1 != frame.hour);
        before = Some(frame.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::tables(), 1, frame.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        write_hour(walk, frame);
        let cold = starts.contains(&frame.hour);
        if cold && (ordinal == 0 || gap) {
            projector = Projector::new(format, CaptureMode::Full, Arc::clone(&blocks)).unwrap();
        }
        if ordinal == 0 || gap {
            projector
                .start(segments::Start::Run { continues: gap }, 1, frame.hour)
                .unwrap();
        }
        let restarts = frames
            .get(ordinal as usize + 1)
            .filter(|next| next.hour == frame.hour + 1 && starts.contains(&next.hour))
            .map(|next| next.hour);
        walk.end(|state, manifest, decoded, first| {
            let starts = first || cold;
            let world = dashboard_world::World::at(
                &mut world,
                starts,
                &needs,
                (format, CaptureMode::Full),
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
            if let Some(next) = restarts {
                projector = Projector::new(format, CaptureMode::Full, Arc::clone(&blocks)).unwrap();
                projector
                    .start(segments::Start::Restart(state), 1, next)
                    .unwrap();
            }
        })
        .unwrap();
    }
    out
}

fn identity() -> Identity {
    Identity {
        source: [4; 32],
        content: [5; 32],
        branch: [6; 32],
        schema: SCHEMA,
    }
}

/// A publication of `hours`, cut after each of `cuts`' hours; when `resume` names one of them,
/// the emit is saved there and goes on from the publication.
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
    let mut hours = hours.into_iter().peekable();
    for (generation, &cut) in (1..).zip(cuts) {
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.hour(&mut writer, hour).unwrap();
        }
        emit.cut(&mut writer, cut).unwrap();
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

/// The row preparation's publication of `frames`, each observed whole.
fn rows(frames: &[DecodedFrame]) -> dashboard_base::prepared::series::tests::Fixture {
    let mut state = row::Prepared::default();
    for frame in frames {
        state.step(frame);
    }
    let fixture = crate::products::test_support::fixture();
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
    fixture
}

/// Every reply the scenario's checks ask, of the row publication and of `built`'s, equal.
fn check(built: &Built, frames: &[DecodedFrame], label: &str) {
    let fixture = rows(frames);
    let budget = Budget::new(64 << 20);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let catalogue = Catalogue::read(&directory).unwrap();
    let cancelled = AtomicBool::new(false);
    let through = frames.last().unwrap().hour;
    let selections: Vec<Vec<String>> = [&[][..], &["SEL"], &["BUY"], &["OTH"], &["OTX"], &["BUY", "OTH"]]
        .iter()
        .map(|tags| tags.iter().map(|tag| tag.to_string()).collect())
        .collect();
    for view in [
        MarketView::Contracts,
        MarketView::Requests,
        MarketView::Inventory,
        MarketView::Agreements,
    ] {
        for tags in &selections {
            let params = MarketHistoryParams {
                branch_id: "1".into(),
                view,
                tags: tags.clone(),
            };
            let expected = row::history(
                &fixture.store,
                &fixture.key,
                &params,
                Some(through),
                None,
                Vec::new(),
                &cancelled,
            )
            .unwrap();
            let actual = query::history(
                Some((&mut snapshot, &catalogue)),
                &params,
                Some(through),
                None,
                Vec::new(),
                &cancelled,
            )
            .unwrap();
            assert_eq!(
                serde_json::to_value(&actual).unwrap(),
                serde_json::to_value(&expected).unwrap(),
                "{label}: {view:?} history of {tags:?}"
            );
        }
    }
    let members = vec![
        LedgerMember {
            tag: "BUY".into(),
            first_hour: 100,
            last_hour: 112,
        },
        LedgerMember {
            tag: "OTX".into(),
            first_hour: 120,
            last_hour: 140,
        },
    ];
    let mut asks = Vec::new();
    for tags in &selections {
        for last_hour in [104, 117, 124, 128, 140] {
            for kinds in [Vec::new(), vec![20], vec![10, 34]] {
                for first_hour in [None, Some(110)] {
                    asks.push((tags.clone(), None, kinds.clone(), first_hour, last_hour));
                }
            }
        }
    }
    asks.push((Vec::new(), Some(members), Vec::new(), None, 140));
    for (tags, members, kinds, first_hour, last_hour) in asks {
        let mut before = None;
        for page in 0.. {
            assert!(page < 64, "{label}: a ledger never ends");
            let params = MarketLedgerParams {
                branch_id: "1".into(),
                tags: tags.clone(),
                members: members.clone(),
                kinds: kinds.clone(),
                first_hour,
                last_hour,
                before: before.clone(),
                page_size: 2,
            };
            let expected = row::ledger(
                &fixture.store,
                &fixture.key,
                &params,
                Some(through),
                None,
                Vec::new(),
                &cancelled,
            )
            .unwrap();
            let actual = query::ledger(
                Some((&mut snapshot, &catalogue)),
                &params,
                Some(through),
                None,
                Vec::new(),
                &cancelled,
            )
            .unwrap();
            assert_eq!(
                serde_json::to_value(&actual).unwrap(),
                serde_json::to_value(&expected).unwrap(),
                "{label}: ledger of {tags:?} {kinds:?} from {first_hour:?} to {last_hour} page {page}"
            );
            match expected.next {
                Some(next) => before = Some(next),
                None => break,
            }
        }
    }
}

#[test]
fn the_publication_answers_as_the_row_preparation_across_segments_cuts_and_a_resume() {
    let frames = scenario();
    let budget = Budget::new(256 << 20);
    for (starts, cuts, resume, label) in [
        (&[][..], &[140][..], None, "one segment"),
        (&[111, 128, 137][..], &[107, 117, 124, 134, 140][..], Some(117), "segments and cuts"),
        (&[104, 105, 106, 119][..], &[104, 105, 118, 119, 140][..], Some(105), "segment every hour"),
        (&[110, 118][..], &[110, 127, 140][..], Some(127), "resumed after the gap"),
    ] {
        let hours = projected(&frames, starts);
        let built = build(hours, cuts, resume, &budget);
        check(&built, &frames, label);
    }
}

#[test]
fn refusals_match_the_row_preparations_observed_hours_and_decoder_text() {
    let mut first = frame(100);
    countries(&mut first, &[(1, 10, "SEL"), (2, 20, "BUY")]);
    let terms = MarketContractTermsRow {
        country: 1,
        buyer: 77,
        id: 9,
        kind: 1,
        ..Default::default()
    };
    set(&mut first, TableRows::MarketContractTerms(vec![terms.clone()]));
    unavailable(&mut first, TableId::CountryMarket);
    let frames = decoded(
        first,
        &[
            (101, Vec::new()),
            (
                102,
                vec![(
                    TableId::CountryMarket,
                    TableUpdate::Snapshot(TableRows::CountryMarket(Vec::new())),
                )],
            ),
            (
                103,
                vec![(
                    TableId::VariantDefinitions,
                    TableUpdate::Snapshot(TableRows::VariantDefinitions(vec![
                        VariantDefinitionRow { id: 0xa1, creator_link: 1, ..Default::default() },
                    ])),
                )],
            ),
            (
                104,
                vec![(
                    TableId::MarketContractTerms,
                    TableUpdate::Snapshot(TableRows::MarketContractTerms(vec![MarketContractTermsRow {
                        runtime: MarketContractRuntime { route_length: 5, ..Default::default() },
                        ..terms
                    }])),
                )],
            ),
            (
                105,
                vec![(TableId::MarketEvents, TableUpdate::NotRecorded { mode: CaptureMode::Full })],
            ),
            (106, Vec::new()),
            (
                107,
                vec![(
                    TableId::MarketEvents,
                    TableUpdate::Snapshot(TableRows::MarketEvents(vec![occurrence(1, 20, 107, 1, 77)])),
                )],
            ),
            (110, Vec::new()),
        ],
    );
    let fixture = changed_rows(&frames);
    let params = MarketHistoryParams {
        branch_id: "1".into(),
        view: MarketView::Contracts,
        tags: Vec::new(),
    };
    let cancelled = AtomicBool::new(false);
    let expected = row::history(
        &fixture.store,
        &fixture.key,
        &params,
        Some(110),
        None,
        Vec::new(),
        &cancelled,
    )
    .unwrap();
    assert_eq!(
        expected.warnings,
        [
            "Market: a market country has no identity (8 observed hours)",
            "Market: a market occurrence's party has no identity (1 observed hours)",
            "Market: market inventory unavailable: hour 100 CountryMarket is unavailable: synthetic writer refusal (1 observed hours)",
            "Market: market inventory unavailable: hour 101 CountryMarket is unavailable: synthetic writer refusal (1 observed hours)",
            "Market: market occurrences unavailable: hour 105 MarketEvents is not recorded in a full recording (1 observed hours)",
            "Market: market occurrences unavailable: hour 106 MarketEvents is not recorded in a full recording (1 observed hours)",
        ]
    );
    let budget = Budget::new(64 << 20);
    for starts in [&[][..], &[103, 105, 107], &[104, 110], &[101, 102, 106]] {
        let hours = projected(&frames, starts);
        assert_eq!(
            hours.iter().filter(|hour| hour.woke).map(|hour| hour.hour).collect::<Vec<_>>(),
            vec![100, 101, 102, 104, 105, 106, 107, 110],
        );
        let built = build(hours, &[102, 105, 110], Some(105), &budget);
        let cache = Cache::new(&budget);
        let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        let catalogue = Catalogue::read(&directory).unwrap();
        let actual = query::history(
            Some((&mut snapshot, &catalogue)),
            &params,
            Some(110),
            None,
            Vec::new(),
            &cancelled,
        )
        .unwrap();
        assert_eq!(actual.warnings, expected.warnings, "segments at {starts:?}");
    }
}

fn check_adjacent_runs(first: DecodedFrame, second: DecodedFrame, warnings: &[&str]) {
    let quiet = second.hour + 1;
    let runs = [decoded(first, &[]), decoded(second, &[(quiet, Vec::new())])];
    let blocks: Arc<BTreeMap<u64, Arc<[i32]>>> =
        Arc::new([(1, Arc::from([runs[0][0].hour, runs[1][0].hour]))].into());
    let format = FormatVersion::CURRENT;
    let needs = facts::needs().unwrap();
    let mut hours = Vec::new();
    let mut frames = Vec::new();
    let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    for (run, frames_in_run) in runs.iter().enumerate() {
        let first = frames_in_run[0].hour;
        let mut walk = Synthetic::new(facts::tables(), 1, first).unwrap();
        let mut world = None;
        let mut projector = Projector::new(format, CaptureMode::Full, Arc::clone(&blocks)).unwrap();
        projector
            .start(segments::Start::Run { continues: run != 0 }, 1, first)
            .unwrap();
        for (ordinal, frame) in frames_in_run.iter().enumerate() {
            write_hour(&mut walk, frame);
            walk.end(|state, manifest, decoded, starts| {
                let changes = tracker.observe(frame, format);
                assert!(
                    [TableId::Countries, TableId::CountryLinks]
                        .into_iter()
                        .chain(STATE)
                        .all(|table| {
                            changes.rows_changed[table.index()]
                                == (ordinal == 0
                                    || !matches!(*frame.tables[table.index()], TableView::Complete(_)))
                        }),
                    "run {run}, ordinal {ordinal}",
                );
                frames.push(frame.clone());
                let world = dashboard_world::World::at(
                    &mut world,
                    starts,
                    &needs,
                    (format, CaptureMode::Full),
                    state,
                    manifest,
                )
                .unwrap();
                projector
                    .hour(
                        &segments::Hour {
                            branch: 1,
                            hour: decoded.hour,
                            run: run as u32,
                            ordinal: ordinal as u32,
                            starts,
                            opens: starts,
                            state,
                            manifest,
                            world,
                        },
                        &mut hours,
                    )
                    .unwrap();
            })
            .unwrap();
        }
    }
    let fixture = changed_rows(&frames);
    let params = MarketHistoryParams {
        branch_id: "1".into(),
        view: MarketView::Contracts,
        tags: Vec::new(),
    };
    let cancelled = AtomicBool::new(false);
    let through = frames.last().unwrap().hour;
    let expected = row::history(
        &fixture.store,
        &fixture.key,
        &params,
        Some(through),
        None,
        Vec::new(),
        &cancelled,
    )
    .unwrap();
    assert_eq!(expected.warnings, warnings);
    let budget = Budget::new(64 << 20);
    let built = build(hours, &[100, through], Some(100), &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let catalogue = Catalogue::read(&directory).unwrap();
    let actual = query::history(
        Some((&mut snapshot, &catalogue)),
        &params,
        Some(through),
        None,
        Vec::new(),
        &cancelled,
    )
    .unwrap();
    assert_eq!(actual.warnings, expected.warnings);
}

#[test]
fn adjacent_run_start_counts_identity_and_occurrence_refusals() {
    let mut first = frame(100);
    countries(&mut first, &[(1, 10, "SEL"), (2, 20, "BUY")]);
    let mut second = first.clone();
    second.hour = 101;
    for table in [TableId::Countries, TableId::CountryLinks].into_iter().chain(STATE) {
        unavailable(&mut second, table);
    }
    check_adjacent_runs(first, second, &[
        "Market: country identities unavailable: hour 101 Countries is unavailable: synthetic writer refusal (1 observed hours)",
        "Market: country identities unavailable: hour 102 Countries is unavailable: synthetic writer refusal (1 observed hours)",
        "Market: market occurrences unavailable: hour 101 MarketEvents is unavailable: synthetic writer refusal (1 observed hours)",
        "Market: market occurrences unavailable: hour 102 MarketEvents is unavailable: synthetic writer refusal (1 observed hours)",
    ]);
}

#[test]
fn adjacent_run_start_observes_unchanged_watched_tables() {
    let mut first = frame(100);
    countries(&mut first, &[(1, 10, "SEL"), (2, 20, "BUY")]);
    set(&mut first, TableRows::MarketContractTerms(vec![MarketContractTermsRow {
        country: 1,
        buyer: 77,
        id: 9,
        kind: 1,
        ..Default::default()
    }]));
    let mut second = first.clone();
    second.hour = 101;
    check_adjacent_runs(first, second, &[
        "Market: a market country has no identity (2 observed hours)",
    ]);
}

#[test]
fn refusals_match_the_row_preparation_at_row_block_starts_inside_a_native_run() {
    let mut first = frame(100);
    countries(&mut first, &[(1, 10, "SEL"), (2, 20, "BUY")]);
    let terms = MarketContractTermsRow {
        country: 1,
        buyer: 77,
        id: 9,
        kind: 1,
        ..Default::default()
    };
    set(&mut first, TableRows::MarketContractTerms(vec![terms.clone()]));
    let frames = decoded(
        first,
        &[
            (101, Vec::new()),
            (102, Vec::new()),
            (103, Vec::new()),
            (104, Vec::new()),
            (
                105,
                vec![(
                    TableId::MarketContractTerms,
                    TableUpdate::Snapshot(TableRows::MarketContractTerms(vec![
                        MarketContractTermsRow { speed: 1, ..terms },
                    ])),
                )],
            ),
            (106, Vec::new()),
        ],
    );
    let blocks = [100, 102, 104];
    let fixture = changed_rows_in_blocks(&frames, &blocks);
    let params = MarketHistoryParams {
        branch_id: "1".into(),
        view: MarketView::Contracts,
        tags: Vec::new(),
    };
    let cancelled = AtomicBool::new(false);
    let expected = row::history(
        &fixture.store,
        &fixture.key,
        &params,
        Some(106),
        None,
        Vec::new(),
        &cancelled,
    )
    .unwrap();
    assert_eq!(expected.warnings, [
        "Market: a market country has no identity (4 observed hours)",
    ]);
    let budget = Budget::new(64 << 20);
    for starts in [&[][..], &[101, 103, 106], &[102, 104]] {
        let hours = projected_blocks(&frames, starts, &blocks);
        assert_eq!(
            hours.iter().filter(|hour| hour.woke).map(|hour| hour.hour).collect::<Vec<_>>(),
            vec![100, 102, 104, 105],
            "segments at {starts:?}",
        );
        let built = build(hours, &[101, 103, 106], Some(103), &budget);
        let cache = Cache::new(&budget);
        let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        let catalogue = Catalogue::read(&directory).unwrap();
        let actual = query::history(
            Some((&mut snapshot, &catalogue)),
            &params,
            Some(106),
            None,
            Vec::new(),
            &cancelled,
        )
        .unwrap();
        assert_eq!(actual.warnings, expected.warnings, "segments at {starts:?}");
    }
}

/// An hour that writes one contract's speed among 1,500 countries' hands on that row alone; an
/// hour writing only fields the market does not read hands on nothing.
#[test]
fn an_hour_hands_on_only_the_rows_it_wrote_in_fields_the_market_reads() {
    let frames: Vec<DecodedFrame> = (0..4)
        .map(|offset| {
            let hour = 200 + offset;
            let mut at = frame(hour);
            crate::products::test_support::many_countries(&mut at, 1_500);
            let terms = (1..=1_500_u32)
                .map(|country| MarketContractTermsRow {
                    country,
                    buyer: country % 1_500 + 1,
                    id: u64::from(country),
                    kind: 1,
                    speed: if offset >= 2 && country == 700 { 9 } else { 1 },
                    runtime: MarketContractRuntime {
                        route_length: if offset == 1 { 5 } else { 0 },
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .collect();
            set(&mut at, TableRows::MarketContractTerms(sorted(terms)));
            at
        })
        .collect();
    let hours = projected(&frames, &[]);
    let rows = |hour: &facts::Hour| match hour.facts.as_ref().and_then(|facts| facts.tables[1].as_ref()) {
        Some(Table::Rows { whole, rows }) => Some((*whole, rows.len())),
        _ => None,
    };
    assert_eq!(rows(&hours[0]), Some((true, 1_500)));
    assert!(hours[1].woke, "every field of a watched table wakes the row preparation");
    assert!(
        hours[1].facts.as_ref().unwrap().tables.iter().all(Option::is_none),
        "a route length is not read"
    );
    assert_eq!(rows(&hours[2]), Some((false, 1)));
    assert!(hours[3].facts.is_none(), "nothing changed");
}

/// A publication of `hours` emitted directly, cut after the last.
fn emitted(hours: Vec<facts::Hour>, budget: &Budget) -> Built {
    let last = hours.last().unwrap().hour;
    build(hours, &[last], None, budget)
}

/// The history of `view` and a ledger page of `tags` through `through`, from `built`.
fn replies(built: &Built, tags: &[&str], through: i32, budget: &Budget) -> (MarketHistoryReply, MarketLedgerReply) {
    let cache = Cache::new(budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let catalogue = Catalogue::read(&directory).unwrap();
    let cancelled = AtomicBool::new(false);
    let tags: Vec<String> = tags.iter().map(|tag| tag.to_string()).collect();
    let history = query::history(
        Some((&mut snapshot, &catalogue)),
        &MarketHistoryParams {
            branch_id: "1".into(),
            view: MarketView::Inventory,
            tags: tags.clone(),
        },
        Some(through),
        None,
        Vec::new(),
        &cancelled,
    )
    .unwrap();
    let ledger = query::ledger(
        Some((&mut snapshot, &catalogue)),
        &MarketLedgerParams {
            branch_id: "1".into(),
            tags,
            members: None,
            kinds: Vec::new(),
            first_hour: None,
            last_hour: through,
            before: None,
            page_size: 16,
        },
        Some(through),
        None,
        Vec::new(),
        &cancelled,
    )
    .unwrap();
    (history, ledger)
}

/// A variant no read names, named before one a read names, leaves the later one's names known.
#[test]
fn a_variant_without_names_hides_no_later_variants_names() {
    let budget = Budget::new(64 << 20);
    let raw = RawVariant {
        id: 0xa2,
        first_hour: 100,
        display_name: "Tank".into(),
        ..RawVariant::default()
    };
    let row = Row {
        values: vec![
            (Field::Inventory(10, 0, 0xa1), 5),
            (Field::Inventory(10, 1, 0xa2), 7),
        ],
        variants: vec![0xa1, 0xa2],
        ..Row::default()
    };
    let facts = Facts {
        identities: Some(Ok(())),
        tags: Some(vec![(10, "SEL".into())]),
        tables: [
            Some(Table::Rows {
                whole: true,
                rows: vec![(1, Some(row))],
            }),
            None,
            None,
            None,
        ],
        events: None,
    };
    let hours = vec![facts::Hour::of(
        100,
        true,
        (VariantHour::Read, vec![(0xa2, raw)]),
        Some(facts),
    )];
    let built = emitted(hours, &budget);
    let (history, _) = replies(&built, &[], 100, &budget);
    assert!(history.variants["a1"].is_empty());
    assert!(history.variants["a2"].first().is_some_and(|name| name.known));
}

/// An occurrence without its buyer at the first hour leaves the ledger of chosen countries
/// unproved through the hours the row preparation did not observe, and proved from the next
/// it did, though nothing the market reads changed then.
#[test]
fn an_unproved_first_hour_is_proved_from_the_next_observed_hour() {
    let budget = Budget::new(64 << 20);
    let facts = Facts {
        identities: Some(Ok(())),
        tags: Some(vec![(10, "SEL".into()), (20, "BUY".into())]),
        tables: [None, None, None, None],
        events: Some(Events::Rows {
            ids: vec![1],
            kept: vec![Occurrence {
                row: occurrence(1, 20, 100, 1, 77),
                seller: Some(10),
                buyer: None,
            }],
        }),
    };
    let carried = (VariantHour::Carried { complete: true }, Vec::new());
    let hours = vec![
        facts::Hour::of(100, true, carried.clone(), Some(facts)),
        facts::Hour::of(101, true, carried.clone(), None),
        facts::Hour::of(102, true, carried, None).woken(),
    ];
    let built = emitted(hours, &budget);
    let (_, ledger) = replies(&built, &["SEL"], 102, &budget);
    assert_eq!(ledger.known, vec![crate::products::keyed::EconomyRun(102, 102, "1".into())]);
    assert_eq!(ledger.rows.len(), 1);
}

// Recordings restate non-complete tables rather than sharing their previous views.
fn decoded(first: DecodedFrame, hours: &[(i32, Vec<(TableId, TableUpdate)>)]) -> Vec<DecodedFrame> {
    let restated = |view: &TableView, hour| match view {
        TableView::Complete(TableRows::Clock(rows)) => {
            TableUpdate::Snapshot(TableRows::Clock(vec![ClockRow { hour, ..rows[0].clone() }]))
        }
        TableView::Complete(rows) => TableUpdate::Snapshot(rows.clone()),
        TableView::Truncated { rows, missing } => TableUpdate::Truncated {
            rows: rows.clone(),
            missing: *missing,
        },
        TableView::Unavailable { reason } => TableUpdate::Unavailable { reason: reason.clone() },
        TableView::NotRecorded { mode } => TableUpdate::NotRecorded { mode: *mode },
    };
    let mut decoder = FrameDecoder::new();
    let mut frames = vec![
        decoder
            .apply(&Frame {
                branch: first.branch,
                hour: first.hour,
                keyframe: false,
                metrics: first.metrics.clone(),
                tables: std::array::from_fn(|index| restated(&first.tables[index], first.hour)),
            })
            .unwrap(),
    ];
    for (hour, changed) in hours {
        let previous = frames.last().unwrap();
        let mut next = Frame {
            branch: previous.branch,
            hour: *hour,
            keyframe: false,
            metrics: previous.metrics.clone(),
            tables: std::array::from_fn(|index| match &*previous.tables[index] {
                TableView::Complete(rows) if rows.table() != TableId::Clock => {
                    TableUpdate::Unchanged { baseline: previous.hour }
                }
                view => restated(view, *hour),
            }),
        };
        for (table, update) in changed {
            next.tables[table.index()] = update.clone();
        }
        frames.push(decoder.apply(&next).unwrap());
    }
    frames
}

/// The row preparation's publication of `frames`, observing only the hours that changed.
fn changed_rows(frames: &[DecodedFrame]) -> dashboard_base::prepared::series::tests::Fixture {
    changed_rows_in_blocks(frames, &[])
}

fn changed_rows_in_blocks(
    frames: &[DecodedFrame],
    blocks: &[i32],
) -> dashboard_base::prepared::series::tests::Fixture {
    let mut state = row::Prepared::default();
    let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    for frame in frames {
        if blocks.contains(&frame.hour) {
            tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
        }
        let changes = tracker.observe(frame, FormatVersion::CURRENT);
        state.observe_changes(FormatVersion::CURRENT, frame, &changes).unwrap();
    }
    let fixture = crate::products::test_support::fixture();
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
    fixture
}

/// An unproved first hour, a quiet hour and a new kind of occurrence give the keys, and so the
/// ledger's cursors, the row preparation numbered them by.
#[test]
fn a_quiet_hour_after_an_unproved_one_keeps_the_rows_cursors() {
    let mut first = frame(100);
    countries(&mut first, &[(1, 10, "SEL"), (2, 20, "BUY")]);
    set(&mut first, TableRows::MarketEvents(vec![occurrence(1, 20, 100, 1, 77)]));
    let frames = decoded(
        first,
        &[
            (101, Vec::new()),
            (
                102,
                vec![(
                    TableId::MarketEvents,
                    TableUpdate::Snapshot(TableRows::MarketEvents(vec![
                        occurrence(2, 21, 102, 1, 2),
                        occurrence(3, 21, 102, 2, 1),
                    ])),
                )],
            ),
            (103, Vec::new()),
        ],
    );
    let fixture = changed_rows(&frames);
    let budget = Budget::new(64 << 20);
    // A segment starting at the quiet hour restates it, which the row preparation never saw.
    for starts in [&[][..], &[101], &[102], &[101, 103]] {
        check_cursors(&fixture, &frames, starts, &budget);
    }
}

/// A status alone moving at a segment's first hour is an hour the row preparation observed.
#[test]
fn a_status_moving_at_a_segment_start_keeps_the_rows_cursors() {
    let mut first = frame(100);
    countries(&mut first, &[(1, 10, "SEL"), (2, 20, "BUY")]);
    set(&mut first, TableRows::MarketEvents(vec![occurrence(1, 20, 100, 1, 77)]));
    let frames = decoded(
        first,
        &[
            (
                101,
                vec![(
                    TableId::CountryMarket,
                    TableUpdate::Unavailable { reason: "synthetic writer refusal".into() },
                )],
            ),
            (
                102,
                vec![(
                    TableId::MarketEvents,
                    TableUpdate::Snapshot(TableRows::MarketEvents(vec![
                        occurrence(2, 21, 102, 1, 2),
                        occurrence(3, 21, 102, 2, 1),
                    ])),
                )],
            ),
            (103, Vec::new()),
        ],
    );
    let fixture = changed_rows(&frames);
    let budget = Budget::new(64 << 20);
    for starts in [&[][..], &[101], &[102]] {
        check_cursors(&fixture, &frames, starts, &budget);
    }
}

/// The ledger pages of every country and of SEL, one row each, from `frames` projected with
/// segments starting at `starts`, held to the row preparation's in `fixture`.
fn check_cursors(
    fixture: &dashboard_base::prepared::series::tests::Fixture,
    frames: &[DecodedFrame],
    starts: &[i32],
    budget: &Budget,
) {
    let built = build(projected(frames, starts), &[103], None, budget);
    let cache = Cache::new(budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let catalogue = Catalogue::read(&directory).unwrap();
    let cancelled = AtomicBool::new(false);
    for tags in [Vec::new(), vec!["SEL".to_owned()]] {
        let mut before = None;
        loop {
            let params = MarketLedgerParams {
                branch_id: "1".into(),
                tags: tags.clone(),
                members: None,
                kinds: Vec::new(),
                first_hour: None,
                last_hour: 103,
                before: before.clone(),
                page_size: 1,
            };
            let expected = row::ledger(
                &fixture.store,
                &fixture.key,
                &params,
                Some(103),
                None,
                Vec::new(),
                &cancelled,
            )
            .unwrap();
            let actual = query::ledger(
                Some((&mut snapshot, &catalogue)),
                &params,
                Some(103),
                None,
                Vec::new(),
                &cancelled,
            )
            .unwrap();
            assert_eq!(
                serde_json::to_value(&actual).unwrap(),
                serde_json::to_value(&expected).unwrap(),
                "ledger of {tags:?} with segments from {starts:?}"
            );
            match expected.next {
                Some(next) => before = Some(next),
                None => break,
            }
        }
    }
}

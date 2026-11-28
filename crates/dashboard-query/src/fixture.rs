
use crate::{
    Query,
    identity::{self, Holders},
    timeline::{Join, Stretch, Timeline, Unheld},
};
use replay::campaign::Replay;
use replay_journal::synthetic::{Input, Sample, Synthetic};
use replay_capture_spool::spool::Record;
use replay_columnar::envelope::{SPAN_BLOCKS, UNIT_ORDINALS};
use replay_data::{
    identity::{CaptureMode, Identity, Roster, SampleBoundary, SessionStart, Verification},
    rows::{
        CampaignRow, ClockRow, CountryFactoriesRow, CountryGameScoreRow, CountryLinkRow, CountryResourceRow, CountryRow, FactionMemberRow, FactionRow, MarketEventRow, Recorded,
        ProvinceRow, RowCodec, StateRow, TableId, TableRows, WorldTensionRow,
    },
};
use replay_frame::{
    branches::{BranchReason, BranchStart, Parentage},
    frame::{DecodedFrame, FrameEncoder, FrameInput, Metrics, TableView},
};
use std::{io, ops::RangeInclusive, sync::Arc};

pub const FIRST: i32 = 10_003;
pub const UNIT_HOURS: i32 = UNIT_ORDINALS as i32;
pub const FIRST_HOURS: i32 = 9 * UNIT_HOURS;
pub const GAP: i32 = 400;
pub const SECOND: i32 = FIRST + FIRST_HOURS + GAP;
pub const SECOND_HOURS: i32 = UNIT_HOURS;
pub const LAST: i32 = SECOND + SECOND_HOURS - 1;
pub const ITA_FROM: i32 = FIRST + 5_000;
pub const COMMAND_FROM: i32 = ITA_FROM + UNIT_HOURS;
pub const COMMAND_TIE: i32 = (ITA_FROM / 96 + 30) * 96 - 1;
pub const GAP_REASON: &str = "The game was not recorded here";
pub const TAGS: [&str; 2] = ["GER", "ITA"];

pub fn exists(id: u32, hour: i32) -> bool {
    id == 9001 || hour >= ITA_FROM
}

pub fn power(id: u32, hour: i32) -> Option<i64> {
    match hour.rem_euclid(97) {
        40..=42 if id == 9001 => None,
        at => Some(i64::from(at) * 1_234 - 50_000 + i64::from(hour / 1_000) * 7 + i64::from(id - 9000) * 100_000),
    }
}

pub const FACTION: u64 = 401;

pub fn states(id: u32, hour: i32) -> u32 {
    match id {
        _ if !exists(id, hour) => 0,
        9001 => 3 + (hour / 24 % 7) as u32,
        _ => 1 + (hour / 24 % 4) as u32,
    }
}

const STATES: u32 = 16;
const UNLINKED: u32 = 9099;

pub fn unresolved(hour: i32) -> bool {
    matches!(hour.rem_euclid(97), 40..=42)
}

pub fn faction_sum(hour: i32) -> Option<i64> {
    (!unresolved(hour)).then(|| i64::from(states(9001, hour) + states(9002, hour)))
}

pub fn tension(hour: i32) -> i64 {
    i64::from(hour / 24) * 10
}

pub fn dropped(id: u32, hour: i32) -> bool {
    exists(id, hour - 1) && matches!((power(id, hour - 1), power(id, hour)), (Some(before), Some(after)) if after < before)
}

pub fn resources(id: u32, hour: i32) -> Vec<u32> {
    match id {
        9001 if (FIRST + 2_000..FIRST + 6_000).contains(&hour) => vec![1, 2, 3, 4],
        9001 => vec![1, 2, 3],
        _ if exists(id, hour) => vec![1, 2],
        _ => Vec::new(),
    }
}

pub fn resource(id: u32, resource: u32, hour: i32) -> CountryResourceRow {
    let step = i64::from(hour / (5 + resource as i32));
    let base = i64::from(id - 9000) * 1_000_000 + i64::from(resource) * 10_000;
    CountryResourceRow {
        country: id,
        resource,
        produced: base + (step % 40) * 100,
        adjustment: 0,
        imported: (step % 3) * 50,
        used: base / 2,
        special_total: base - (step % 11) * 700,
        production_demand: -(step % 17) * 900,
        to_export: 0,
        base_export: 0,
        exported: step % 2,
    }
}

pub fn market(hour: i32) -> Vec<(u64, u8, u32)> {
    if hour.rem_euclid(13) != 0 {
        return Vec::new();
    }
    let turn = hour.div_euclid(13);
    let country = if turn % 2 == 1 && exists(9002, hour) { 9002 } else { 9001 };
    let mut out = vec![(hour as u64 * 4, (turn % 3) as u8, country)];
    if hour.rem_euclid(39) == 0 {
        out.push((hour as u64 * 4 + 1, 7, 9001));
    }
    out
}

pub fn receiver(country: u32, kind: u8, hour: i32) -> u32 {
    match country {
        9002 => 9001,
        9001 if kind == 0 && exists(9002, hour) => 9002,
        _ => 0,
    }
}

pub enum Data {
    Power,
    RankedPower,
    GameScores,
    Resources,
    Market,
    DenseMarket(i32, u64),
    Tension,
    Faction,
}

fn sample<R: RowCodec>(frame: &mut Synthetic, table: TableId, rows: &[R]) -> io::Result<()> {
    frame.table(table, Input::Snapshot { rows, partial: None, read: true, sample: Sample::FromReadBit })
}

fn country_rows(frame: &mut Synthetic, hour: i32, data: &Data) -> io::Result<()> {
    let countries: Vec<_> = [(9001, 7), (9002, 8)]
        .into_iter()
        .filter(|&(id, _)| exists(id, hour))
        .map(|(id, canonical_slot)| CountryRow {
            id,
            canonical_slot,
            tag: tag_of(id).as_bytes().to_vec(),
            political_power_raw: if matches!(data, Data::Power | Data::RankedPower) { power(id, hour) } else { None },
            command_power_raw: match (id, hour) {
                _ if !matches!(data, Data::RankedPower) || hour < COMMAND_FROM => 0,
                (9001, _) | (_, COMMAND_TIE) => 1,
                _ if hour < COMMAND_TIE => 2,
                _ => 0,
            },
            ..CountryRow::default()
        })
        .collect();
    let links: Vec<_> = countries.iter().map(|row| CountryLinkRow { id: row.id, canonical_slot: row.canonical_slot }).collect();
    sample(frame, TableId::Countries, &countries)?;
    sample(frame, TableId::CountryLinks, &links)
}

fn resource_rows(frame: &mut Synthetic, hour: i32) -> io::Result<()> {
    let rows: Vec<_> = [9001, 9002].into_iter().flat_map(|id| resources(id, hour).into_iter().map(move |held| resource(id, held, hour))).collect();
    sample(frame, TableId::CountryResources, &rows)
}

fn market_rows(frame: &mut Synthetic, hour: i32) -> io::Result<()> {
    let events: Vec<_> = market(hour)
        .into_iter()
        .map(|(id, kind, country)| MarketEventRow { id, occurrence: id, hour, country, receiver: receiver(country, kind, hour), kind, amount_raw: i64::from(hour), ..MarketEventRow::default() })
        .collect();
    sample(frame, TableId::MarketEvents, &events)
}

fn faction_rows(frame: &mut Synthetic, hour: i32) -> io::Result<()> {
    let (ger, ita) = (states(9001, hour), states(9002, hour));
    let held: Vec<_> = (1..=STATES)
        .map(|id| {
            let country = match id {
                _ if id <= ger => 9001,
                _ if id <= ger + ita => 9002,
                STATES if unresolved(hour) => UNLINKED,
                _ => 0,
            };
            StateRow { id, owner: country, controller: country, ..StateRow::default() }
        })
        .collect();
    let members: Vec<_> = [9001, 9002].into_iter().filter(|&id| exists(id, hour)).zip(0..).map(|(country, ordinal)| FactionMemberRow { country, faction: FACTION, ordinal }).collect();
    sample(frame, TableId::States, &held)?;
    sample::<ProvinceRow>(frame, TableId::Provinces, &[])?;
    sample(frame, TableId::Factions, &[FactionRow { id: FACTION, ..FactionRow::default() }])?;
    sample(frame, TableId::FactionMembers, &members)
}

fn session(branch: u64) -> SessionStart {
    SessionStart {
        id: branch,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 10_000_000,
        sample_boundary: SampleBoundary::EndOfHour,
        capture_mode: CaptureMode::Full,
        identity: Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: vec![],
            mods: vec![],
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified { launcher: Roster::default(), memory: Roster::default() },
        },
    }
}

fn branch_start(branch: u64, first: i32) -> BranchStart {
    BranchStart { id: branch, session: branch, loaded_hour: None, first_hour: first, parent: Parentage::Unknown, reason: BranchReason::SessionStart }
}

pub(crate) fn bare(first: i32, last: i32, write: impl FnMut(&mut Synthetic, i32) -> io::Result<()>) -> io::Result<Replay> {
    bare_runs(&[first..=last], write)
}

pub(crate) fn bare_runs(runs: &[RangeInclusive<i32>], write: impl FnMut(&mut Synthetic, i32) -> io::Result<()>) -> io::Result<Replay> {
    Replay::from_synthetic_runs(recorder_derive_framework::framework::Writers::new(Vec::new()), b"bare".to_vec(), session(1), branch_start(1, *runs[0].start()), runs, write)
}

pub(crate) fn segment(branch: u64, first: i32, hours: i32) -> Vec<Record> {
    let mut out = vec![Record::SessionStart(session(branch)), Record::BranchStart(branch_start(branch, first))];
    let mut encoder = FrameEncoder::new();
    for hour in first..first + hours {
        let mut tables = std::array::from_fn(|_| TableView::Unavailable { reason: "outside the fixture".into() });
        tables[TableId::Campaign.index()] = TableView::Complete(TableRows::Campaign(vec![CampaignRow { id: b"tiles".to_vec() }]));
        let mut metrics = Metrics::default();
        metrics.table_ns[TableId::Campaign.index()] = 1;
        let mut input = FrameInput::from(DecodedFrame { branch, hour, tables: tables.map(Into::into), metrics });
        input.scope_to(CaptureMode::Full);
        out.push(Record::Frame(encoder.encode_input(input, false).unwrap()));
    }
    out
}

pub struct Fixture {
    replay: Replay,
    timeline: Arc<Timeline>,
    holders: Arc<Holders>,
}

impl Fixture {
    pub fn new(data: Data, hours: RangeInclusive<i32>) -> Self {
        Self::record(data, &[hours], None, false)
    }

    pub fn with_not_recorded_factories(hours: RangeInclusive<i32>) -> Self {
        Self::record(Data::Power, &[hours], None, true)
    }

    pub fn with_gap(data: Data, hours: RangeInclusive<i32>, gap: RangeInclusive<i32>) -> Self {
        Self::record(data, &[hours], Some(gap), false)
    }

    pub fn with_runs(data: Data, runs: &[RangeInclusive<i32>]) -> Self {
        Self::record(data, runs, None, false)
    }

    pub fn campaign(data: Data) -> Self {
        Self::with_gap(data, FIRST..=LAST, FIRST + FIRST_HOURS..=SECOND - 1)
    }

    pub fn campaign_with_sealed_span(data: Data) -> Self {
        let gap = FIRST + (SPAN_BLOCKS as i32 + 1) * UNIT_HOURS;
        let second = gap + GAP;
        let fixture = Self::with_gap(data, FIRST..=second + 240, gap..=second - 1);
        assert_eq!(fixture.replay.recording().run(0).unwrap().directory().spans.len(), 1);
        fixture
    }

    fn record(data: Data, runs: &[RangeInclusive<i32>], gap: Option<RangeInclusive<i32>>, not_recorded_factories: bool) -> Self {
        let (first, last) = (*runs[0].start(), *runs[runs.len() - 1].end());
        let campaign = CampaignRow { id: b"tiles".to_vec() };
        let writers = if matches!(data, Data::GameScores) {
            let current = replay_recording::schema::columns_at(replay_data::FORMAT_VERSION).unwrap();
            let mut given = current.clone();
            given[TableId::CountryGameScores.index()] = 0;
            recorder_derive::writers_within(&given, &current)
        } else {
            recorder_derive::writers()
        };
        let replay = Replay::from_synthetic_runs(writers, campaign.id.clone(), session(1), branch_start(1, first), runs, |frame, hour| {
            sample(frame, TableId::Clock, &[ClockRow { hour, ..ClockRow::default() }])?;
            sample(frame, TableId::Campaign, std::slice::from_ref(&campaign))?;
            country_rows(frame, hour, &data)?;
            if not_recorded_factories {
                frame.table::<CountryFactoriesRow>(TableId::CountryFactories, Input::NotRecorded { read: false, sample: Sample::FromReadBit })?;
            }
            match data {
                Data::Power | Data::RankedPower => Ok(()),
                Data::GameScores => sample(frame, TableId::CountryGameScores, &[9001, 9002].map(|country| CountryGameScoreRow {
                    country,
                    army: match (hour % 13, hour % 7, hour % 91, country) {
                        (0, _, _, 9002) => Recorded::NotRecorded,
                        (_, 0, _, _) => Recorded::Value(30),
                        (_, _, 5..=6, 9002) => Recorded::Value(60),
                        (_, _, _, 9001) => Recorded::Value(i64::from(hour % 11) + 31),
                        _ => Recorded::Value(20),
                    },
                    navy: if country == 9001 { Recorded::Value(20) } else { Recorded::NotRecorded },
                    ..CountryGameScoreRow::default()
                })),
                Data::Resources => resource_rows(frame, hour),
                Data::Market => market_rows(frame, hour),
                Data::DenseMarket(dense_hour, dense_count) => {
                    let events: Vec<MarketEventRow> = if hour == dense_hour {
                        (0..dense_count)
                            .map(|idx| MarketEventRow {
                                id: 10_000 + idx,
                                occurrence: 10_000 + idx,
                                hour,
                                country: 9001,
                                kind: 20,
                                agreement: 10_000 + idx,
                                amount_raw: (idx as i64 + 1) * 1000,
                                ..MarketEventRow::default()
                            })
                            .collect()
                    } else if hour % 6 == 0 {
                        vec![MarketEventRow {
                            id: hour as u64,
                            occurrence: hour as u64,
                            hour,
                            country: 9001,
                            kind: 20,
                            agreement: hour as u64,
                            amount_raw: 1000,
                            ..MarketEventRow::default()
                        }]
                    } else {
                        Vec::new()
                    };
                    sample(frame, TableId::MarketEvents, &events)
                }
                Data::Tension => sample(frame, TableId::WorldTension, &[WorldTensionRow { total_raw: tension(hour), ..WorldTensionRow::default() }]),
                Data::Faction => faction_rows(frame, hour),
            }
        }).unwrap();
        assert_eq!(replay.recording().run_count() as usize, runs.len());
        let mut stretches: Vec<Stretch> = runs
            .iter()
            .enumerate()
            .map(|(run, hours)| Stretch { run: run as u32, first: *hours.start(), last: *hours.end(), join: if run == 0 { Join::Start } else { Join::Break } })
            .collect();
        let mut unheld = Vec::new();
        if let Some(gap) = gap {
            let (gap_first, gap_last) = gap.into_inner();
            assert!(runs.len() == 1 && first < gap_first && gap_first <= gap_last && gap_last < last);
            stretches[0].last = gap_first - 1;
            stretches.push(Stretch { run: 0, first: gap_last + 1, last, join: Join::Break });
            unheld.push(Unheld { first: gap_first, last: gap_last, reason: GAP_REASON.into() });
        }
        let timeline = Arc::new(Timeline { id: 1, first, last, stretches, unheld });
        let holders = Arc::new(identity::holders(replay.recording(), &timeline).unwrap());
        Self { replay, timeline, holders }
    }

    pub fn query(&self) -> Query<'_> {
        Query { replay: &self.replay, timeline: Arc::clone(&self.timeline), holders: Arc::clone(&self.holders) }
    }
}

pub fn tag_of(id: u32) -> &'static str {
    if id == 9001 { "GER" } else { "ITA" }
}

pub fn held(query: &Query<'_>, tag: &str, hour: i32) -> bool {
    query.holders.at(tag, hour).is_some() && query.timeline.at(hour).is_some()
}

pub fn unbase64(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let value = |byte: u8| ALPHABET.iter().position(|&held| held == byte).unwrap() as u32;
    let mut out = Vec::new();
    for chunk in text.as_bytes().chunks(4) {
        let pad = chunk.iter().filter(|&&byte| byte == b'=').count();
        let word = chunk.iter().take(4 - pad).enumerate().fold(0u32, |word, (at, &byte)| word | value(byte) << (18 - 6 * at));
        out.extend_from_slice(&word.to_be_bytes()[1..4 - pad]);
    }
    out
}

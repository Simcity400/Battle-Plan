
use crate::{
    Query,
    identity::{Holder, HolderRun, Holders},
    timeline::{Join, Stretch, Timeline, Unheld},
};
use replay::{campaign::Replay, spool::Record};
use replay_columnar::envelope::UNIT_ORDINALS;
use replay_data::{
    identity::{CaptureMode, Identity, Roster, SampleBoundary, SessionStart, Verification},
    rows::{CampaignRow, CountryLinkRow, CountryResourceRow, CountryRow, FactionMemberRow, FactionRow, MarketEventRow, StateRow, TableId, TableRows, WorldTensionRow},
};
use replay_frame::{
    branches::{BranchReason, BranchStart, Parentage},
    frame::{DecodedFrame, FrameEncoder, FrameInput, Metrics, TableView},
};
use std::sync::{Arc, OnceLock};

pub const FIRST: i32 = 10_003;
pub const FIRST_HOURS: i32 = 17 * UNIT_ORDINALS as i32;
pub const GAP: i32 = 400;
pub const SECOND: i32 = FIRST + FIRST_HOURS + GAP;
pub const SECOND_HOURS: i32 = 2 * UNIT_ORDINALS as i32;
pub const LAST: i32 = SECOND + SECOND_HOURS - 1;
pub const ITA_FROM: i32 = FIRST + 5_000;
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

fn frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable { reason: "outside the fixture".into() });
    tables[TableId::Campaign.index()] = TableView::Complete(TableRows::Campaign(vec![CampaignRow { id: b"tiles".to_vec() }]));
    let mut countries = vec![CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), political_power_raw: power(9001, hour), ..CountryRow::default() }];
    let mut links = vec![CountryLinkRow { id: 9001, canonical_slot: 7 }];
    if exists(9002, hour) {
        countries.push(CountryRow { id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), political_power_raw: power(9002, hour), ..CountryRow::default() });
        links.push(CountryLinkRow { id: 9002, canonical_slot: 8 });
    }
    let rows = [9001, 9002].into_iter().flat_map(|id| resources(id, hour).into_iter().map(move |held| resource(id, held, hour))).collect();
    let events = market(hour)
        .into_iter()
        .map(|(id, kind, country)| MarketEventRow { id, occurrence: id, hour, country, receiver: receiver(country, kind, hour), kind, amount_raw: i64::from(hour), ..MarketEventRow::default() })
        .collect();
    tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(countries));
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links));
    tables[TableId::CountryResources.index()] = TableView::Complete(TableRows::CountryResources(rows));
    tables[TableId::MarketEvents.index()] = TableView::Complete(TableRows::MarketEvents(events));
    let (ger, ita) = (states(9001, hour), states(9002, hour));
    let held = (1..=STATES)
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
    let members = [9001, 9002].into_iter().filter(|&id| exists(id, hour)).zip(0..).map(|(country, ordinal)| FactionMemberRow { country, faction: FACTION, ordinal }).collect();
    tables[TableId::States.index()] = TableView::Complete(TableRows::States(held));
    tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(Vec::new()));
    tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(vec![FactionRow { id: FACTION, ..FactionRow::default() }]));
    tables[TableId::FactionMembers.index()] = TableView::Complete(TableRows::FactionMembers(members));
    tables[TableId::WorldTension.index()] = TableView::Complete(TableRows::WorldTension(vec![WorldTensionRow { total_raw: tension(hour), ..WorldTensionRow::default() }]));
    let mut metrics = Metrics::default();
    for (index, view) in tables.iter().enumerate() {
        if matches!(view, TableView::Complete(_)) {
            metrics.table_ns[index] = 1;
        }
    }
    DecodedFrame { branch, hour, tables: tables.map(Into::into), metrics }
}

pub(crate) fn segment(branch: u64, first: i32, hours: i32) -> Vec<Record> {
    let session = SessionStart {
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
    };
    let mut out = vec![
        Record::SessionStart(session),
        Record::BranchStart(BranchStart { id: branch, session: branch, loaded_hour: None, first_hour: first, parent: Parentage::Unknown, reason: BranchReason::SessionStart }),
    ];
    let mut encoder = FrameEncoder::new();
    for hour in first..first + hours {
        let mut input = FrameInput::from(frame(branch, hour));
        input.scope_to(CaptureMode::Full);
        out.push(Record::Frame(encoder.encode_input(input, false).unwrap()));
    }
    out
}

pub fn replay() -> &'static Replay {
    static HELD: OnceLock<Replay> = OnceLock::new();
    HELD.get_or_init(|| Replay::from_records(b"tiles".to_vec(), &[segment(1, FIRST, LAST - FIRST + 1)]).unwrap())
}

pub fn query() -> Query<'static> {
    let replay = replay();
    assert_eq!(replay.recording().run_count(), 1);
    let timeline = Timeline {
        id: 1,
        first: FIRST,
        last: LAST,
        stretches: vec![
            Stretch { run: 0, first: FIRST, last: FIRST + FIRST_HOURS - 1, join: Join::Start },
            Stretch { run: 0, first: SECOND, last: LAST, join: Join::Break },
        ],
        unheld: vec![Unheld { first: FIRST + FIRST_HOURS, last: SECOND - 1, reason: GAP_REASON.into() }],
    };
    let run = |first: i32, last: i32, slot: u32, id: u32| HolderRun { first, last, holder: Holder::Found { slot, id }, starts_segment: true };
    let holders = Holders::new(
        [
            ("GER".to_owned(), vec![run(FIRST, FIRST + FIRST_HOURS - 1, 7, 9001), run(SECOND, LAST, 7, 9001)]),
            ("ITA".to_owned(), vec![run(ITA_FROM, FIRST + FIRST_HOURS - 1, 8, 9002), run(SECOND, LAST, 8, 9002)]),
        ]
        .into(),
        Vec::new(),
    );
    Query { replay, timeline: Arc::new(timeline), holders: Arc::new(holders) }
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

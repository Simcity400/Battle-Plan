
use dashboard_base::products::Country;
use dashboard_base::{products::{LoadContext, Product}, queries::country_ids::Names};
use dashboard_base::Source;
use dashboard_base::products::battles::{BattleBound, BattleEnd, BattleMoment, BattleWarning, BattleWindow, BattleWinner};
use dashboard_query::Query;
use replay_lanes::lanes::{self, Cell, Keys};
use replay_data::{
    codec::Decoder,
    rows::{LandBattleEndRow, LandEquipmentLoss, TableId},
    value::Value as _,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

fn branch(text: &str) -> io::Result<u64> {
    text.parse().map_err(|_| invalid("Invalid battle branch"))
}

const NOT_RECORDED: &str = "Not in this recording";
const OPENED_EARLIER: &str = "The battle began before this part of the recording";
const NO_OPENING: &str = "No opening was recorded; the game starts border wars outside the recorded combat start";
const CASUALTIES_UNREAD: &str = "Casualties not read";
const MEMBER_UNREAD: &str = "Manpower or casualties not read";
const UNATTRIBUTED: &str = "An edge in this hour could not be tied to its combat";
const ROSTER_UNREAD: &str = "Combat census not read";

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LandBattlesParams {
    pub branch_id: String,
    pub window: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandBattlePeak {
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
    pub value: u64,
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandBattleRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub counts: [u32; 2],
    pub roster_unknown: bool,
    pub countries: [Vec<String>; 2],
    pub countries_unknown: [bool; 2],
    #[doc = " The native counter at `first_hour`."]
    pub duration: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(untagged)]
pub enum LandManpower {
    Known(i32),
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(untagged)]
pub enum LandLossTotal {
    Known { manpower: LandManpower, equipment: String },
    Unknown { manpower: LandManpower, unknown: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(untagged)]
pub enum LandLossTotals {
    Read([LandLossTotal; 2]),
    Unread(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
pub struct LandBattleLossRead {
    pub hour: i32,
    pub terminal: bool,
    pub sides: LandLossTotals,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
pub struct LandBattleResultRead {
    pub hour: Option<i32>,
    pub duration: i32,
    pub winner: BattleWinner,
    pub countries: [Vec<String>; 2],
    #[serde(rename = "countriesUnknown")]
    pub countries_unknown: [bool; 2],
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandBattleHistory {
    pub id: String,
    pub province_id: Option<u32>,
    pub start: BattleMoment,
    pub end: Option<BattleEnd>,
    pub end_unknown_hour: Option<i32>,
    pub counter_base: i32,
    #[doc = " The battle's most divisions at once as of the reply's first hour."]
    pub prior_peak: Option<LandBattlePeak>,
    #[doc = " Each side's, attacker then defender."]
    pub prior_side_peaks: [Option<LandBattlePeak>; 2],
    pub runs: Vec<LandBattleRun>,
    pub losses: Vec<LandBattleLossRead>,
    pub result: Option<LandBattleResultRead>,
    #[doc = " The hour its end row was written: its end and result are known from then, dated by their own hours."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub settled_hour: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandBattlesManifestReply {
    #[doc = " The replay's blocks, cut to the hours the timeline holds."]
    pub windows: Vec<BattleWindow>,
    pub available: bool,
    pub unavailable: Option<String>,
    pub content_warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandBattlesReply {
    pub window: usize,
    pub interval: BattleWindow,
    #[doc = " Hours the combat census was read."]
    pub observed: Vec<(i32, i32)>,
    #[doc = " Warning reasons, which `warnings` name by index."]
    pub reasons: Vec<String>,
    #[doc = " (first hour, reason, hours)."]
    pub warnings: Vec<(i32, usize, usize)>,
    pub rows: Vec<LandBattleHistory>,
    pub content_warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Default, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BattleCountryFilter {
    pub countries: Option<Vec<Country>>,
    pub membership: Option<Vec<dashboard_world::products::groups::MembershipRun>>,
    pub unobserved: Vec<dashboard_base::span::HourSpan>,
}

impl BattleCountryFilter {
    pub fn matches(&self, holders: &dashboard_query::identity::Holders, sides: [(&[String], bool); 2], hour: i32) -> bool {
        if self.countries.is_none() && self.membership.is_none() {
            return true;
        }
        if sides.iter().any(|(_, unknown)| *unknown) {
            return true;
        }
        let unobserved = self.unobserved.iter().any(|span| span.first_hour <= hour && hour <= span.last_hour);
        sides.iter().any(|(countries, _)| {
            countries.iter().any(|tag| {
                self.countries.as_ref().is_none_or(|countries| countries.iter().any(|country| holders.goes_by(country, tag, hour)))
                    && self.membership.as_ref().is_none_or(|runs| unobserved || runs.iter().any(|run| run.tag == *tag && run.first_hour <= hour && hour <= run.last_hour))
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LandBattlesRankingParams {
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub top_n: f64,
    pub measure: String,
    pub countries: BattleCountryFilter,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandBattlesRankingReply {
    pub rows: Vec<LandBattleHistory>,
    #[doc = " The last hour at or before the range's end the combat census was read."]
    pub census_hour: Option<i32>,
    #[doc = " What it ranked by: casualties fall back to peak size where no battle's were recorded."]
    pub effective_measure: String,
    pub warnings: Vec<BattleWarning>,
    pub content_warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LandBattleCourseParams {
    pub branch_id: String,
    #[doc = " A battle id of this branch."]
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
pub struct LandEquipmentDesign {
    pub name: Option<String>,
    pub variant: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(untagged)]
pub enum LandLossSide {
    Variants { manpower: LandManpower, variants: Vec<(usize, String)> },
    Changed { manpower: LandManpower, changed: Vec<(usize, String)> },
    Unknown { manpower: LandManpower, unknown: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(untagged)]
pub enum LandLossSides {
    Read([LandLossSide; 2]),
    Unread(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
pub struct LandBattleCourseRead {
    pub hour: i32,
    pub terminal: bool,
    pub sides: LandLossSides,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandEdgeMember {
    pub division: Option<String>,
    pub side: u8,
    pub manpower: Option<i32>,
    pub casualties: Option<i32>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandBattleOpening {
    pub hour: i32,
    pub members: Vec<LandEdgeMember>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandDivisionExit {
    pub hour: i32,
    pub division: Option<String>,
    pub side: Option<u8>,
    pub losses: Option<i32>,
    pub manpower: Option<i32>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandEdgeGap {
    pub first_hour: i32,
    pub last_hour: i32,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandBattleEdges {
    pub opening: Option<LandBattleOpening>,
    pub opening_missing: Option<String>,
    pub exits: Vec<LandDivisionExit>,
    pub unrecorded: Vec<LandEdgeGap>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandBattleCourse {
    pub id: String,
    pub province_id: Option<u32>,
    pub start: BattleMoment,
    pub end: Option<BattleEnd>,
    pub end_unknown_hour: Option<i32>,
    pub counter_base: i32,
    pub runs: Vec<LandBattleRun>,
    pub losses: Vec<LandBattleCourseRead>,
    pub result: Option<LandBattleResultRead>,
    pub edges: LandBattleEdges,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandBattleCourseReply {
    pub battle: LandBattleCourse,
    #[doc = " The designs the course's reads name by index."]
    pub designs: Vec<LandEquipmentDesign>,
    pub content_warnings: Vec<String>,
}

macro_rules! content_warnings {
    ($($reply:ty),+) => {$(
        impl dashboard_base::products::ReplyWarnings for $reply {
            fn warnings_mut(&mut self) -> &mut Vec<String> {
                &mut self.content_warnings
            }
        }
    )+};
}
content_warnings!(LandBattlesManifestReply, LandBattlesReply, LandBattlesRankingReply, LandBattleCourseReply);

struct Columns {
    battles: [usize; 9],
    ends: [usize; 27],
    combat: [usize; 4],
    sides: [usize; 4],
    progress: [usize; 4],
}

impl Columns {
    fn new() -> io::Result<Self> {
        let field = dashboard_declare::field;
        let battles = ["start_hour", "start_exact", "counter_base", "peak_total", "peak_total_hour", "peak_attackers", "peak_attackers_hour", "peak_defenders", "peak_defenders_hour"];
        let mut ends = [0; 27];
        for (at, slot) in ends.iter_mut().enumerate() {
            *slot = at;
        }
        Ok(Self {
            battles: try_map(battles, |name| field(TableId::LandBattles, name))?,
            ends,
            combat: try_map(["province", "duration", "attackers", "defenders"], |name| field(TableId::LandCombats, name))?,
            sides: try_map(["attacker_primary", "attacker_secondary", "defender_primary", "defender_secondary"], |name| field(TableId::LandCombatSides, name))?,
            progress: try_map(["attacker_manpower_loss", "defender_manpower_loss", "attacker_equipment_losses", "defender_equipment_losses"], |name| field(TableId::LandCombatProgress, name))?,
        })
    }
}

fn try_map<const N: usize>(names: [&str; N], field: impl Fn(&str) -> io::Result<usize>) -> io::Result<[usize; N]> {
    let mut out = [0; N];
    for (slot, name) in out.iter_mut().zip(names) {
        *slot = field(name)?;
    }
    Ok(out)
}

fn cell(history: &lanes::History, cells: &[Cell], column: usize) -> Option<Cell> {
    history.columns.binary_search(&column).ok().map(|at| cells[at])
}

fn number(history: &lanes::History, cells: &[Cell], column: usize) -> Option<i64> {
    match cell(history, cells, column) {
        Some(Cell::Number(value)) => Some(value),
        _ => None,
    }
}

fn peak(value: Option<i64>, hour: Option<i64>) -> Option<LandBattlePeak> {
    Some(LandBattlePeak { value: u64::try_from(value?).ok()?, hour: hour? as i32 })
}

pub fn distinct(bytes: &[u8]) -> Option<u32> {
    if bytes.len() % 8 != 0 {
        return None;
    }
    let mut handles: Vec<u64> = bytes.chunks_exact(8).map(|item| u64::from_le_bytes(item.try_into().expect("8 bytes"))).collect();
    handles.sort_unstable();
    handles.dedup();
    u32::try_from(handles.len()).ok()
}

fn u32s(bytes: &[u8]) -> Option<Vec<u32>> {
    (bytes.len() % 4 == 0).then(|| bytes.chunks_exact(4).map(|item| u32::from_le_bytes(item.try_into().expect("4 bytes"))).collect())
}

fn tags_of(names: &Names, slots: &[u32], hour: i32) -> (Vec<String>, bool) {
    let mut unknown = slots.is_empty();
    let mut tags: Vec<String> = Vec::with_capacity(slots.len());
    for &slot in slots {
        match names.slot_tag(slot, hour) {
            Some(tag) => tags.push(tag.to_owned()),
            None => unknown = true,
        }
    }
    tags.sort_unstable();
    tags.dedup();
    (tags, unknown)
}

type Key = (u64, i32, Option<i32>);

fn end_key(key: u128, hour: i32) -> Key {
    ((key >> 33) as u64, key as u32 as i32, ((key >> 32) & 1 == 1).then_some(hour))
}

fn written_ends(ended: &lanes::History) -> impl Iterator<Item = (Key, u128, i32, &[Cell])> + '_ {
    ended.keys.iter().flat_map(|(key, steps)| steps.iter().filter_map(move |step| step.cells.as_deref().map(|cells| (end_key(*key, step.hour), *key, step.hour, cells))))
}

#[derive(Clone, Debug, Default)]
struct Battle {
    combat: u64,
    start: i32,
    result_only: Option<i32>,
    exact: bool,
    counter_base: i32,
    province: Option<u32>,
    live: Option<(i32, i32)>,
    peaks: [Option<LandBattlePeak>; 3],
    end: Option<Ended>,
}

#[derive(Clone, Debug, Default)]
struct Ended {
    kind: Option<u8>,
    end_hour: Option<i32>,
    written: i32,
    province: Option<u32>,
    last_read: Option<i32>,
    last_duration: Option<i32>,
    peaks: [Option<LandBattlePeak>; 3],
    manpower: [Option<i32>; 2],
    sides: [Option<Vec<u32>>; 2],
    countries_unknown: u8,
    result: Option<u64>,
    winners: Option<u8>,
    result_duration: Option<i32>,
    terminal: [Option<i32>; 2],
    terminal_equipment: [Option<Vec<LandEquipmentLoss>>; 2],
    result_sides: [Option<Vec<u32>>; 2],
    result_unknown: u8,
}

fn live_battles(query: &Query<'_>, columns: &Columns, run: u32, from: i32, to: i32, combat: Option<u64>, out: &mut BTreeMap<Key, Battle>) -> io::Result<()> {
    let one: Vec<u128> = combat.map(u128::from).into_iter().collect();
    let keys = if one.is_empty() { Keys::All } else { Keys::These(&one) };
    let live = dashboard_query::cost::storage(|| lanes::history(query.replay.recording(), run, TableId::LandBattles, &sorted(&columns.battles), keys, from, to))?;
    for (key, steps) in &live.keys {
        let combat = *key as u64;
        for (at, step) in steps.iter().enumerate() {
            let Some(cells) = &step.cells else { continue };
            let Some(start) = number(&live, cells, columns.battles[0]).map(|start| start as i32) else { continue };
            let last = steps.get(at + 1).map_or(live.to, |next| next.hour - 1);
            let battle = out.entry((combat, start, None)).or_insert_with(|| Battle {
                combat,
                start,
                exact: number(&live, cells, columns.battles[1]) == Some(1),
                counter_base: number(&live, cells, columns.battles[2]).unwrap_or(0) as i32,
                peaks: [3, 5, 7].map(|at| peak(number(&live, cells, columns.battles[at]), number(&live, cells, columns.battles[at + 1]))),
                ..Battle::default()
            });
            battle.live = Some(battle.live.map_or((step.hour, last), |(first, _)| (first, last)));
        }
    }
    Ok(())
}

fn decode_ended_rows(ended: &lanes::History, out: &mut BTreeMap<Key, Battle>) {
    for (key, _, settled, cells) in written_ends(ended) {
        if out.get(&key).is_some_and(|battle| battle.end.is_some()) {
            continue;
        }
        let n = |column: usize| number(ended, cells, column);
        let (combat, start, result_only) = key;
        let end = Ended {
            kind: n(0).and_then(|value| u8::try_from(value).ok()),
            end_hour: n(1).and_then(|value| i32::try_from(value).ok()),
            written: settled,
            province: n(4).map(|value| value as u32),
            last_read: n(5).map(|hour| hour as i32),
            last_duration: n(6).map(|value| value as i32),
            peaks: [(7, 8), (9, 10), (11, 12)].map(|(value, hour)| peak(n(value), n(hour))),
            manpower: [n(13).map(|value| value as i32), n(14).map(|value| value as i32)],
            countries_unknown: n(17).unwrap_or(3) as u8,
            result: n(18).map(|id| id as u64),
            winners: n(20).map(|mask| mask as u8),
            result_duration: n(21).map(|value| value as i32),
            terminal: [n(22).map(|value| value as i32), n(23).map(|value| value as i32)],
            result_unknown: n(26).unwrap_or(3) as u8,
            ..Ended::default()
        };
        let battle = out.entry(key).or_insert_with(|| Battle {
            combat,
            start,
            result_only,
            exact: n(2) == Some(1),
            counter_base: n(3).unwrap_or(0) as i32,
            ..Battle::default()
        });
        if let Some(province) = end.province {
            battle.province = Some(province);
        }
        battle.end = Some(end);
    }
}

fn ended_battles(query: &Query<'_>, columns: &Columns, run: u32, from: i32, to: i32, combat: Option<u64>, out: &mut BTreeMap<Key, Battle>) -> io::Result<()> {
    let recording = query.replay.recording();
    let range = combat.map(|combat| (u128::from(combat) << 33, u128::from(combat) << 33 | 0x1_FFFF_FFFF));
    let ended = dashboard_query::cost::storage(|| {
        lanes::history(recording, run, TableId::LandBattleEnds, &columns.ends, range.map_or(Keys::All, |(low, high)| Keys::Range(low, high)), from, to)
    })?;
    decode_ended_rows(&ended, out);
    let asks: Vec<(i32, u128)> = written_ends(&ended).map(|(_, key, hour, _)| (hour, key)).collect();
    if !asks.is_empty() {
        let lists = [15, 16, 24, 25];
        dashboard_query::cost::storage(|| {
            lanes::at_each(recording, run, TableId::LandBattleEnds, Some(&lists), &asks, &mut |hour, key, row| {
                let Some((view, slot)) = row else { return Ok(()) };
                let read = |column: usize| view.list(column, slot).and_then(u32s);
                if let Some(end) = end_written(out, key, hour) {
                    end.sides = [read(15), read(16)];
                    end.result_sides = [read(24), read(25)];
                }
                Ok(())
            })
        })?;
        let pools = [27, 28];
        let read = dashboard_query::cost::storage(|| {
            lanes::at_each(recording, run, TableId::LandBattleEnds, Some(&pools), &asks, &mut |hour, key, row| {
                let Some((view, slot)) = row else { return Ok(()) };
                let pool = |column: usize| view.list(column, slot).and_then(|bytes| Vec::<LandEquipmentLoss>::decode(&mut Decoder::new(bytes)).ok());
                if let Some(end) = end_written(out, key, hour) {
                    end.terminal_equipment = [pool(27), pool(28)];
                }
                Ok(())
            })
        });
        if read.is_err() {
            for battle in out.values_mut() {
                if let Some(end) = &mut battle.end {
                    end.terminal_equipment = [None, None];
                }
            }
        }
    }
    Ok(())
}

fn end_written(out: &mut BTreeMap<Key, Battle>, key: u128, hour: i32) -> Option<&mut Ended> {
    out.get_mut(&end_key(key, hour))?.end.as_mut().filter(|end| end.written == hour)
}

fn battles(query: &Query<'_>, columns: &Columns, run: u32, from: i32, to: i32, combat: Option<u64>) -> io::Result<BTreeMap<Key, Battle>> {
    let mut out = BTreeMap::new();
    live_battles(query, columns, run, from, to, combat, &mut out)?;
    ended_battles(query, columns, run, from, to, combat, &mut out)?;
    Ok(out)
}

#[derive(Clone, Debug, Default)]
struct CensusRead {
    province: Option<u32>,
    duration: Option<i32>,
    counts: Option<[u32; 2]>,
}

#[derive(Default)]
struct Combats {
    from: i32,
    census: HashMap<u64, Vec<(i32, Option<CensusRead>)>>,
    sides: HashMap<u64, Vec<(i32, Option<[Option<Vec<u32>>; 2]>)>>,
    losses: HashMap<u64, Vec<(i32, Option<Losses>)>>,
    since: HashMap<u64, i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Losses {
    manpower: [Option<i32>; 2],
    equipment: [Option<Vec<LandEquipmentLoss>>; 2],
}

fn combats(query: &Query<'_>, columns: &Columns, run: u32, from: i32, to: i32, combats: &[u64]) -> io::Result<Combats> {
    let recording = query.replay.recording();
    let keys: Vec<u128> = combats.iter().map(|&combat| u128::from(combat)).collect();
    let mut out = Combats { from, ..Combats::default() };
    if keys.is_empty() {
        return Ok(out);
    }
    let shape = [columns.combat[0], columns.combat[2], columns.combat[3]];
    let census = dashboard_query::cost::storage(|| lanes::history(recording, run, TableId::LandCombats, &sorted(&shape), Keys::These(&keys), from, to))?;
    let asks: Vec<(i32, u128)> = census.keys.iter().flat_map(|(key, steps)| steps.iter().filter(|step| step.cells.is_some()).map(move |step| (step.hour, *key))).collect();
    let mut read: HashMap<(i32, u64), CensusRead> = HashMap::new();
    dashboard_query::cost::storage(|| {
        lanes::at_each(recording, run, TableId::LandCombats, Some(&sorted(&columns.combat)), &asks, &mut |hour, key, row| {
            if let Some((view, slot)) = row {
                let counts = match (view.list(columns.combat[2], slot), view.list(columns.combat[3], slot)) {
                    (Some(attackers), Some(defenders)) => distinct(attackers).zip(distinct(defenders)).map(|(a, d)| [a, d]),
                    _ => None,
                };
                read.insert((hour, key as u64), CensusRead { province: view.number(columns.combat[0], slot).map(|value| value as u32), duration: view.number(columns.combat[1], slot).map(|value| value as i32), counts });
            }
            Ok(())
        })
    })?;
    for (key, steps) in &census.keys {
        let combat = *key as u64;
        out.census.insert(combat, steps.iter().map(|step| (step.hour, step.cells.as_ref().and_then(|_| read.get(&(step.hour, combat)).cloned()))).collect());
    }
    let sides = dashboard_query::cost::storage(|| lanes::history(recording, run, TableId::LandCombatSides, &sorted(&columns.sides), Keys::These(&keys), from, to))?;
    let asks: Vec<(i32, u128)> = sides.keys.iter().flat_map(|(key, steps)| steps.iter().filter(|step| step.cells.is_some()).map(move |step| (step.hour, *key))).collect();
    let mut references: HashMap<(i32, u64), [Option<Vec<u32>>; 2]> = HashMap::new();
    dashboard_query::cost::storage(|| {
        lanes::at_each(recording, run, TableId::LandCombatSides, Some(&sorted(&columns.sides)), &asks, &mut |hour, key, row| {
            if let Some((view, slot)) = row {
                let side = |first: usize, second: usize| -> Option<Vec<u32>> {
                    let mut list = view.list(first, slot).and_then(u32s)?;
                    list.extend(view.list(second, slot).and_then(u32s)?);
                    Some(list)
                };
                references.insert((hour, key as u64), [side(columns.sides[0], columns.sides[1]), side(columns.sides[2], columns.sides[3])]);
            }
            Ok(())
        })
    })?;
    for (key, steps) in &sides.keys {
        let combat = *key as u64;
        out.sides.insert(combat, steps.iter().map(|step| (step.hour, step.cells.as_ref().and_then(|_| references.get(&(step.hour, combat)).cloned()))).collect());
    }
    let progress = dashboard_query::cost::storage(|| lanes::history(recording, run, TableId::LandCombatProgress, &sorted(&columns.progress), Keys::These(&keys), from, to))?;
    let mut pools: Vec<(i32, u128)> = Vec::new();
    for (key, steps) in &progress.keys {
        for step in steps {
            let Some(cells) = &step.cells else { continue };
            for side in 2..4 {
                if let Some(Cell::Written(hour)) = cell(&progress, cells, columns.progress[side]) {
                    pools.push((hour.max(from), *key));
                }
            }
        }
    }
    let mut equipment: HashMap<(i32, u64), [Option<Vec<LandEquipmentLoss>>; 2]> = HashMap::new();
    dashboard_query::cost::storage(|| {
        lanes::at_each(recording, run, TableId::LandCombatProgress, Some(&sorted(&columns.progress[2..])), &pools, &mut |hour, key, row| {
            if let Some((view, slot)) = row {
                let pool = |column: usize| view.list(column, slot).and_then(|bytes| Vec::<LandEquipmentLoss>::decode(&mut Decoder::new(bytes)).ok());
                equipment.insert((hour, key as u64), [pool(columns.progress[2]), pool(columns.progress[3])]);
            }
            Ok(())
        })
    })?;
    for (key, steps) in &progress.keys {
        let combat = *key as u64;
        let mut reads = Vec::with_capacity(steps.len());
        for step in steps {
            let Some(cells) = &step.cells else {
                reads.push((step.hour, None));
                continue;
            };
            let mut losses = Losses { manpower: [0, 1].map(|side| number(&progress, cells, columns.progress[side]).map(|value| value as i32)), ..Losses::default() };
            for side in 0..2 {
                losses.equipment[side] = match cell(&progress, cells, columns.progress[side + 2]) {
                    Some(Cell::Written(hour)) => equipment.get(&(hour.max(from), combat)).and_then(|pools| pools[side].clone()),
                    _ => None,
                };
            }
            reads.push((step.hour, Some(losses)));
        }
        out.losses.insert(combat, reads);
    }
    Ok(out)
}

fn changed_at(query: &Query<'_>, run: u32, table: TableId, columns: &[usize], asks: &[(u128, i32, i32)]) -> io::Result<HashMap<(u128, i32), i32>> {
    let recording = query.replay.recording();
    let mut by_hour: BTreeMap<i32, Vec<(u128, i32)>> = BTreeMap::new();
    for &(key, hour, floor) in asks {
        by_hour.entry(hour).or_default().push((key, floor.min(hour)));
    }
    let mut out = HashMap::new();
    for (hour, mut open) in by_hour {
        let (mut to, mut back) = (hour, 24);
        while let Some(floor) = open.iter().map(|&(_, floor)| floor).min() {
            let from = to.saturating_sub(back).max(floor);
            let keys: Vec<u128> = open.iter().map(|&(key, _)| key).collect();
            let read = dashboard_query::cost::storage(|| lanes::history(recording, run, table, columns, Keys::These(&keys), from, to))?;
            let last: HashMap<u128, i32> = read.keys.iter().filter_map(|(key, steps)| steps.last().map(|step| (*key, step.hour))).collect();
            open.retain(|&(key, own)| {
                let found = last.get(&key).copied().unwrap_or(from);
                let settled = found > from || from <= own;
                if settled {
                    out.insert((key, hour), found.max(own));
                }
                !settled
            });
            to = from;
            back = back.saturating_mul(16);
        }
    }
    Ok(out)
}

fn settle(query: &Query<'_>, columns: &Columns, run: u32, data: &mut Combats, floors: &HashMap<u64, i32>) -> io::Result<()> {
    let asks: Vec<(u128, i32, i32)> = data
        .losses
        .iter()
        .filter(|(_, reads)| reads.first().is_some_and(|(hour, losses)| *hour == data.from && losses.is_some()))
        .filter_map(|(&combat, _)| floors.get(&combat).filter(|&&floor| floor < data.from).map(|&floor| (u128::from(combat), data.from, floor)))
        .collect();
    if asks.is_empty() {
        return Ok(());
    }
    for ((key, _), hour) in changed_at(query, run, TableId::LandCombatProgress, &sorted(&columns.progress), &asks)? {
        data.since.insert(key as u64, hour);
    }
    Ok(())
}

fn carried(hours: &[i32], first: i32, last: i32) -> std::ops::Range<usize> {
    let end = hours.partition_point(|&hour| hour <= last);
    hours.partition_point(|&hour| hour <= first).saturating_sub(1).min(end)..end
}

fn clipped(run: &LandBattleRun, first: i32, last: i32) -> LandBattleRun {
    LandBattleRun { first_hour: first, last_hour: last, duration: (i64::from(run.duration) + i64::from(first) - i64::from(run.first_hour)) as i32, ..run.clone() }
}

fn window_runs(runs: &[LandBattleRun], from: i32, to: i32, ends_within: bool) -> Vec<LandBattleRun> {
    let mut out: Vec<LandBattleRun> = runs.iter().filter(|run| run.first_hour <= to && run.last_hour >= from).map(|run| clipped(run, run.first_hour.max(from), run.last_hour.min(to))).collect();
    if out.is_empty()
        && ends_within
        && let Some(last) = runs.last()
    {
        out.push(clipped(last, last.last_hour, last.last_hour));
    }
    out
}

fn sorted(columns: &[usize]) -> Vec<usize> {
    let mut columns = columns.to_vec();
    columns.sort_unstable();
    columns.dedup();
    columns
}

fn at<T>(changes: &[(i32, T)], hour: i32) -> Option<&T> {
    let index = changes.partition_point(|(changed, _)| *changed <= hour);
    index.checked_sub(1).map(|index| &changes[index].1)
}

fn manpower(value: Option<i32>, reason: &str) -> LandManpower {
    value.map_or_else(|| LandManpower::Unknown(reason.into()), LandManpower::Known)
}

fn equipment_total(pool: &[LandEquipmentLoss]) -> String {
    pool.iter().map(|loss| i128::from(loss.amount_raw)).sum::<i128>().to_string()
}

fn total(losses: &Losses, side: usize) -> LandLossTotal {
    let manpower = manpower(losses.manpower[side], NOT_RECORDED);
    match &losses.equipment[side] {
        Some(pool) => LandLossTotal::Known { manpower, equipment: equipment_total(pool) },
        None => LandLossTotal::Unknown { manpower, unknown: NOT_RECORDED.into() },
    }
}

fn winner(mask: Option<u8>) -> BattleWinner {
    match mask {
        Some(1) => BattleWinner::Side { side: 0 },
        Some(2) => BattleWinner::Side { side: 1 },
        Some(_) => BattleWinner::Unknown { unknown: "No single winner recorded".into() },
        None => BattleWinner::Unknown { unknown: "Result not recorded".into() },
    }
}

struct Shaped {
    history: LandBattleHistory,
    course: Vec<(i32, bool, Option<Losses>)>,
}

fn shape(battle: &Battle, data: &Combats, names: &Names, links: &dashboard_base::queries::country_ids::RunLinks, branch: u64, from: i32, to: i32, peaks_through: i32) -> Shaped {
    let combat = battle.combat;
    let end = battle.end.as_ref();
    let (live_first, live_last) = battle.live.unwrap_or_else(|| {
        let last = end.and_then(|end| end.last_read).unwrap_or(battle.start);
        (battle.start.max(data.from), last.min(to))
    });
    let empty: Vec<(i32, Option<CensusRead>)> = Vec::new();
    let census = data.census.get(&combat).unwrap_or(&empty);
    let no_sides: Vec<(i32, Option<[Option<Vec<u32>>; 2]>)> = Vec::new();
    let sides = data.sides.get(&combat).unwrap_or(&no_sides);
    let countries_at = |hour: i32| -> ([Vec<String>; 2], [bool; 2]) {
        match at(sides, hour).and_then(Option::as_ref) {
            Some(raw) => {
                let resolve = |side: Option<&[u32]>| match side {
                    Some(side) => {
                        let slots: Vec<u32> = side.iter().filter_map(|&reference| links.slot_at(reference, hour)).collect();
                        let (tags, unknown) = tags_of(names, &slots, hour);
                        (tags, unknown || slots.len() < side.len())
                    }
                    None => (Vec::new(), true),
                };
                let (a, b) = (resolve(raw[0].as_deref()), resolve(raw[1].as_deref()));
                ([a.0, b.0], [a.1, b.1])
            }
            None => ([Vec::new(), Vec::new()], [true, true]),
        }
    };
    let mut changes: Vec<i32> = census.iter().map(|(hour, _)| *hour).chain(sides.iter().map(|(hour, _)| *hour)).filter(|hour| (live_first..=live_last).contains(hour)).collect();
    changes.push(live_first);
    changes.sort_unstable();
    changes.dedup();
    let mut runs: Vec<LandBattleRun> = Vec::new();
    let mut province = battle.province;
    for (index, &first) in changes.iter().enumerate() {
        if live_first > live_last {
            break;
        }
        let last = changes.get(index + 1).map_or(live_last, |next| next - 1);
        let read = census.iter().rev().find(|(hour, _)| *hour <= first).and_then(|(hour, read)| read.as_ref().map(|read| (*hour, read)));
        let (counts, roster_unknown, duration) = match read {
            Some((hour, read)) => {
                if battle.result_only.is_none() {
                    province = province.or(read.province);
                }
                (read.counts.unwrap_or([0, 0]), read.counts.is_none(), read.duration.map_or(0, |duration| duration + (first - hour)))
            }
            None => ([0, 0], true, 0),
        };
        let (countries, countries_unknown) = countries_at(first);
        let run = LandBattleRun { first_hour: first, last_hour: last, counts, roster_unknown, countries, countries_unknown, duration };
        match runs.last_mut() {
            Some(held) if held.last_hour + 1 == first && held.counts == run.counts && held.roster_unknown == run.roster_unknown && held.countries == run.countries && held.countries_unknown == run.countries_unknown && i64::from(held.duration) + i64::from(first - held.first_hour) == i64::from(run.duration) => {
                held.last_hour = last;
            }
            _ => runs.push(run),
        }
    }
    let no_losses: Vec<(i32, Option<Losses>)> = Vec::new();
    let reads = data.losses.get(&combat).unwrap_or(&no_losses);
    let mut course: Vec<(i32, bool, Option<Losses>)> = Vec::new();
    for (hour, losses) in reads.iter().filter(|(hour, _)| (live_first..=live_last).contains(hour)) {
        if course.last().is_some_and(|(_, _, held)| held == losses) {
            continue;
        }
        course.push((*hour, false, losses.clone()));
    }
    if let Some(first) = course.first_mut()
        && first.0 == data.from
        && let Some(&since) = data.since.get(&combat)
    {
        first.0 = since;
    }
    let result = end.filter(|end| end.result.is_some() && end.written <= to);
    if let Some(end) = result
        && (end.terminal[0].is_some() || end.terminal[1].is_some())
    {
        course.push((end.written.max(course.last().map_or(i32::MIN, |read| read.0)), true, Some(Losses { manpower: end.terminal, equipment: end.terminal_equipment.clone() })));
    }
    let ends_within = end.is_some_and(|end| end.kind != Some(LandBattleEndRow::RESULT_ONLY) && (from..=to).contains(&end.written));
    let runs = window_runs(&runs, from, to, ends_within);
    let hours: Vec<i32> = course.iter().map(|(hour, ..)| *hour).collect();
    let losses = course[carried(&hours, runs.first().map_or(from, |run| run.first_hour), to)]
        .iter()
        .map(|(hour, terminal, losses)| LandBattleLossRead {
            hour: *hour,
            terminal: *terminal,
            sides: match losses {
                Some(losses) => LandLossTotals::Read([total(losses, 0), total(losses, 1)]),
                None => LandLossTotals::Unread(NOT_RECORDED.into()),
            },
        })
        .collect();
    let (end_moment, end_unknown_hour) = match end.filter(|end| end.kind != Some(LandBattleEndRow::RESULT_ONLY)) {
        Some(end) => match (end.kind, end.end_hour) {
            (None, _) => (Some(BattleEnd::Unknown { unknown: "Ending kind not recorded".into() }), Some(end.written)),
            (Some(_), None) => (Some(BattleEnd::Unknown { unknown: "Ending hour not recorded".into() }), Some(end.written)),
            (Some(LandBattleEndRow::UNKNOWN), Some(hour)) => (Some(BattleEnd::Unknown { unknown: "Ending not recorded".into() }), Some(hour)),
            (Some(_), Some(hour)) => (Some(BattleEnd::Moment(BattleMoment { hour, bound: BattleBound::Exact })), None),
        },
        None => (None, None),
    };
    let result = result.map(|end| {
        let lookup_hour = end.end_hour.unwrap_or(end.written);
        let resolve = |slots: Option<&[u32]>, unknown: bool| match slots {
            Some(slots) => {
                let (tags, missing) = tags_of(names, slots, lookup_hour);
                (tags, unknown || missing)
            }
            None => (Vec::new(), true),
        };
        let (a, b) = (
            resolve(end.result_sides[0].as_deref(), end.result_unknown & LandBattleEndRow::ATTACKERS_UNKNOWN != 0),
            resolve(end.result_sides[1].as_deref(), end.result_unknown & LandBattleEndRow::DEFENDERS_UNKNOWN != 0),
        );
        LandBattleResultRead {
            hour: end.end_hour,
            duration: end.result_duration.unwrap_or(0),
            winner: winner(end.winners),
            countries: [a.0, b.0],
            countries_unknown: [a.1, b.1],
        }
    });
    let peaks = if battle.live.is_some() { battle.peaks } else { end.map_or([None; 3], |end| end.peaks) }.map(|peak| peak.filter(|peak| peak.hour <= peaks_through));
    let history = LandBattleHistory {
        id: match battle.result_only {
            Some(written) => format!("{branch}:{combat}:{}:{written}", battle.start),
            None => format!("{branch}:{combat}:{}", battle.start),
        },
        province_id: province.or_else(|| end.and_then(|end| end.province)),
        start: BattleMoment { hour: battle.start, bound: if battle.exact { BattleBound::Exact } else { BattleBound::By } },
        end: end_moment,
        end_unknown_hour,
        counter_base: battle.counter_base,
        prior_peak: peaks[0],
        prior_side_peaks: [peaks[1], peaks[2]],
        runs,
        losses,
        result,
        settled_hour: end.map(|end| end.written),
    };
    Shaped { history, course }
}

pub fn last_read(query: &Query<'_>, table: TableId, to: i32) -> io::Result<Option<i32>> {
    for stretch in query.timeline.stretches.iter().rev().filter(|stretch| stretch.first <= to) {
        let reads = dashboard_query::cost::storage(|| lanes::reads(query.replay.recording(), stretch.run, table, stretch.first, stretch.last.min(to)))?;
        if let Some(&(_, last)) = reads.last() {
            return Ok(Some(last));
        }
    }
    Ok(None)
}

fn observed(query: &Query<'_>, run: u32, from: i32, to: i32) -> io::Result<Vec<(i32, i32)>> {
    dashboard_query::cost::storage(|| lanes::reads(query.replay.recording(), run, TableId::LandCombats, from, to))
}

fn window_rows(source: &Source, query: &Query<'_>, run: u32, first: i32, from: i32, to: i32, cancelled: &AtomicBool) -> io::Result<Vec<LandBattleHistory>> {
    let columns = Columns::new()?;
    let read_from = (from - 1).max(first);
    let found = battles(query, &columns, run, read_from, to, None)?;
    dashboard_text::metadata::check_cancel(cancelled)?;
    let combats_read: Vec<u64> = found.keys().map(|(combat, ..)| *combat).collect::<HashSet<_>>().into_iter().collect();
    let mut data = combats(query, &columns, run, read_from, to, &combats_read)?;
    let floors: HashMap<u64, i32> = found.values().filter(|battle| battle.live.is_some_and(|(live, _)| live == read_from)).map(|battle| (battle.combat, battle.start.max(first))).collect();
    settle(query, &columns, run, &mut data, &floors)?;
    let names = Names::new(&query.holders);
    let links = dashboard_base::queries::links(source, run)?;
    let mut rows: Vec<LandBattleHistory> = found.values().filter_map(|battle| listed(shape(battle, &data, &names, &links, query.timeline.id, from, to, from - 1).history, from)).collect();
    rows.sort_by(|a, b| a.start.hour.cmp(&b.start.hour).then_with(|| a.id.cmp(&b.id)));
    Ok(rows)
}

fn listed(mut history: LandBattleHistory, from: i32) -> Option<LandBattleHistory> {
    if history.runs.is_empty() {
        let settled = history.settled_hour.filter(|&hour| hour >= from && history.result.is_some())?;
        history.runs.push(LandBattleRun { first_hour: settled, last_hour: settled, counts: [0, 0], roster_unknown: true, countries: [Vec::new(), Vec::new()], countries_unknown: [true, true], duration: 0 });
    }
    Some(history)
}

fn unread_warnings(read: &[(i32, i32)], from: i32, to: i32) -> Vec<(i32, usize, usize)> {
    super::division_log::unread(read, from, to).into_iter().map(|(first, last)| (first, 0, (last - first + 1) as usize)).collect()
}

pub struct LandBattles;
pub struct LandBattlesManifest;
pub struct LandBattlesRanking;
pub struct LandBattleCourses;

impl Product for LandBattlesManifest {
    const KEY: &'static str = "land-battles-manifest";
    const CONTENT_DEPENDENT: bool = false;
    type Params = dashboard_base::products::BranchParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = LandBattlesManifestReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LandBattlesManifestReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid battle branch")
    }

    fn load(source: &Source, params: &Self::Params, _: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let windows = dashboard_world::products::army_windows::windows(&query)?.into_iter().map(|span| BattleWindow { first_hour: span.first_hour, last_hour: span.last_hour }).collect();
        let mut available = false;
        for stretch in &query.timeline.stretches {
            available |= !observed(&query, stretch.run, stretch.first, stretch.last)?.is_empty();
        }
        Ok(LandBattlesManifestReply { windows, available, unavailable: (!available).then(|| "The combat census was never read".into()), content_warnings: Vec::new() })
    }
}

impl Product for LandBattles {
    const KEY: &'static str = "land-battles";
    const CONTENT_DEPENDENT: bool = false;
    type Params = LandBattlesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, window];
    type Reply = LandBattlesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        let mut out = dashboard_base::products::battles::types();
        out.extend([
            LandBattlesParams::decl(),
            LandBattlePeak::decl(),
            LandBattleRun::decl(),
            LandManpower::decl(),
            LandLossTotal::decl(),
            LandLossTotals::decl(),
            LandBattleLossRead::decl(),
            LandBattleResultRead::decl(),
            LandBattleHistory::decl(),
            LandBattlesReply::decl(),
        ]);
        out
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid battle branch")
    }

    fn load(source: &Source, params: &Self::Params, _: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let interval = dashboard_world::products::army_windows::windows(&query)?.get(params.window).cloned().ok_or_else(|| invalid("Battle window not in recording"))?;
        let (from, to) = (interval.first_hour, interval.last_hour);
        let stretches: Vec<_> = query.timeline.within(from, to).collect();
        if stretches.is_empty() {
            return Err(invalid("Battle window not in recording"));
        }
        let mut obs = Vec::new();
        let mut by_id: HashMap<String, usize> = HashMap::new();
        let mut rows: Vec<LandBattleHistory> = Vec::new();
        for stretch in stretches {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let full_first = query.timeline.at(stretch.first).map_or(stretch.first, |held| held.first);
            obs.extend(observed(&query, stretch.run, stretch.first, stretch.last)?);
            for row in window_rows(source, &query, stretch.run, full_first, stretch.first, stretch.last, cancelled)? {
                if let Some(&at) = by_id.get(&row.id) {
                    let held = &mut rows[at];
                    if held.province_id.is_none() {
                        held.province_id = row.province_id;
                    }
                    held.runs.extend(row.runs);
                    held.losses.extend(row.losses);
                    if row.end.is_some() {
                        held.end = row.end;
                        held.end_unknown_hour = row.end_unknown_hour;
                    }
                    if row.result.is_some() {
                        held.result = row.result;
                    }
                    if row.settled_hour.is_some() {
                        held.settled_hour = row.settled_hour;
                    }
                } else {
                    by_id.insert(row.id.clone(), rows.len());
                    rows.push(row);
                }
            }
        }
        rows.sort_by(|a, b| a.start.hour.cmp(&b.start.hour).then_with(|| a.id.cmp(&b.id)));
        let warnings = unread_warnings(&obs, from, to);
        Ok(LandBattlesReply {
            window: params.window,
            interval: BattleWindow { first_hour: from, last_hour: to },
            reasons: if warnings.is_empty() { Vec::new() } else { vec![ROSTER_UNREAD.into()] },
            warnings,
            observed: obs,
            rows,
            content_warnings: Vec::new(),
        })
    }
}

pub const MEASURES: [&str; 3] = ["Casualties", "Peak size", "Length"];

#[derive(Clone, Debug)]
pub struct Ranked {
    pub casualties: Option<f64>,
    pub length: u64,
    pub peak: Option<u64>,
    pub id: String,
}

fn ranked_hour(shaped: &LandBattleHistory, read: i32) -> i32 {
    shaped.result.as_ref().and_then(|result| result.hour.or(shaped.settled_hour)).unwrap_or(read)
}

fn battle_side<'a>(shaped: &'a LandBattleHistory, at: usize) -> (&'a [String], bool) {
    match &shaped.result {
        Some(result) => (&result.countries[at][..], result.countries_unknown[at]),
        None => shaped.runs.last().map_or((&[][..], true), |run| (&run.countries[at][..], run.countries_unknown[at])),
    }
}

pub fn rank(entries: &mut [Ranked], measure: &str) -> &'static str {
    let effective = match measure {
        "Casualties" if entries.iter().any(|entry| entry.casualties.is_some()) => "Casualties",
        "Length" => "Length",
        _ => "Peak size",
    };
    let value = |entry: &Ranked| match effective {
        "Casualties" => entry.casualties,
        "Length" => Some(entry.length as f64),
        _ => entry.peak.map(|peak| peak as f64),
    };
    entries.sort_by(|a, b| value(b).partial_cmp(&value(a)).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.id.cmp(&b.id)));
    effective
}

#[derive(Clone, Copy, Debug)]
struct Chosen {
    run: u32,
    combat: u64,
    floor: i32,
    read: i32,
    live: bool,
    to: i32,
}

fn last_reads(query: &Query<'_>, columns: &Columns, rows: &mut [(LandBattleHistory, Chosen)]) -> io::Result<()> {
    let recording = query.replay.recording();
    let mut runs: Vec<u32> = rows.iter().map(|(_, chosen)| chosen.run).collect();
    runs.sort_unstable();
    runs.dedup();
    for run in runs {
        let ended: Vec<(i32, u128)> = rows.iter().filter(|(_, chosen)| chosen.run == run && !chosen.live).map(|(_, chosen)| (chosen.read, u128::from(chosen.combat))).collect();
        let mut counts: HashMap<(i32, u64), [u32; 2]> = HashMap::new();
        let mut losses: HashMap<(i32, u64), Losses> = HashMap::new();
        if !ended.is_empty() {
            let rosters = sorted(&columns.combat[2..]);
            dashboard_query::cost::storage(|| {
                lanes::at_each(recording, run, TableId::LandCombats, Some(&rosters), &ended, &mut |hour, key, row| {
                    if let Some((view, slot)) = row
                        && let (Some(attackers), Some(defenders)) = (view.list(columns.combat[2], slot), view.list(columns.combat[3], slot))
                        && let (Some(attackers), Some(defenders)) = (distinct(attackers), distinct(defenders))
                    {
                        counts.insert((hour, key as u64), [attackers, defenders]);
                    }
                    Ok(())
                })
            })?;
            dashboard_query::cost::storage(|| {
                lanes::at_each(recording, run, TableId::LandCombatProgress, Some(&sorted(&columns.progress)), &ended, &mut |hour, key, row| {
                    if let Some((view, slot)) = row {
                        let pool = |column: usize| view.list(column, slot).and_then(|bytes| Vec::<LandEquipmentLoss>::decode(&mut Decoder::new(bytes)).ok());
                        let manpower = |column: usize| view.number(column, slot).map(|value| value as i32);
                        let read = Losses { manpower: [manpower(columns.progress[0]), manpower(columns.progress[1])], equipment: [pool(columns.progress[2]), pool(columns.progress[3])] };
                        losses.insert((hour, key as u64), read);
                    }
                    Ok(())
                })
            })?;
        }
        let asks: Vec<(u128, i32, i32)> = rows
            .iter()
            .filter(|(history, chosen)| chosen.run == run && (if chosen.live { history.losses.iter().any(|loss| !loss.terminal && loss.hour == chosen.to) } else { losses.contains_key(&(chosen.read, chosen.combat)) }))
            .map(|(_, chosen)| (u128::from(chosen.combat), if chosen.live { chosen.to } else { chosen.read }, chosen.floor))
            .collect();
        let changed = changed_at(query, run, TableId::LandCombatProgress, &sorted(&columns.progress), &asks)?;
        for (history, chosen) in rows.iter_mut().filter(|(_, chosen)| chosen.run == run) {
            if chosen.live {
                if let Some(&hour) = changed.get(&(u128::from(chosen.combat), chosen.to)) {
                    for loss in history.losses.iter_mut().filter(|loss| !loss.terminal && loss.hour == chosen.to) {
                        loss.hour = hour;
                    }
                }
                continue;
            }
            if let Some(&sizes) = counts.get(&(chosen.read, chosen.combat))
                && let Some(shown) = history.runs.first_mut()
            {
                shown.counts = sizes;
                shown.roster_unknown = false;
            }
            if let Some(read) = losses.get(&(chosen.read, chosen.combat)) {
                let hour = changed.get(&(u128::from(chosen.combat), chosen.read)).copied().unwrap_or(chosen.read);
                let hourly = LandBattleLossRead { hour, terminal: false, sides: LandLossTotals::Read([total(read, 0), total(read, 1)]) };
                history.losses = std::iter::once(hourly).chain(history.losses.drain(..).filter(|loss| loss.terminal)).collect();
            }
        }
    }
    Ok(())
}

impl Product for LandBattlesRanking {
    const KEY: &'static str = "land-battles-ranking";
    const CONTENT_DEPENDENT: bool = false;
    type Params = LandBattlesRankingParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, firstHour: Start, lastHour: End, topN, measure, "countries.membership[].firstHour": Start, "countries.membership[].lastHour": End, "countries.unobserved[].firstHour": Start, "countries.unobserved[].lastHour": End, "countries.countries[]"];
    type Reply = LandBattlesRankingReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![BattleCountryFilter::decl(), LandBattlesRankingParams::decl(), LandBattlesRankingReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid battle branch")?;
        if params.first_hour > params.last_hour || !params.top_n.is_finite() || params.top_n < 1.0 || params.top_n.fract() != 0.0 {
            return Err("Invalid battle ranking range or top N");
        }
        if !MEASURES.contains(&params.measure.as_str()) {
            return Err("Unknown battle measure");
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, _: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let columns = Columns::new()?;
        let names = Names::new(&query.holders);
        let mut entries: Vec<(Ranked, LandBattleHistory, Chosen)> = Vec::new();
        let mut warnings = Vec::new();
        for stretch in query.timeline.within(params.first_hour, params.last_hour) {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let (from, to) = (stretch.first, stretch.last);
            let first = query.timeline.at(to).map_or(from, |whole| whole.first);
            let read = observed(&query, stretch.run, from, to)?;
            warnings.extend(unread_warnings(&read, from, to).into_iter().map(|(hour, _, count)| BattleWarning { hour, reason: ROSTER_UNREAD.into(), count }));
            let mut all = BTreeMap::new();
            ended_battles(&query, &columns, stretch.run, from, to, None, &mut all)?;
            live_battles(&query, &columns, stretch.run, to, to, None, &mut all)?;
            let live: Vec<u64> = all.values().filter(|battle| battle.live.is_some()).map(|battle| battle.combat).collect::<HashSet<_>>().into_iter().collect();
            let data = combats(&query, &columns, stretch.run, to, to, &live)?;
            let links = dashboard_base::queries::links(source, stretch.run)?;
            for battle in all.values() {
                let mut shaped = shape(battle, &data, &names, &links, query.timeline.id, from, to, to).history;
                let read = battle.live.map(|(_, last)| last).or_else(|| battle.end.as_ref().and_then(|end| end.last_read)).unwrap_or(battle.start).min(to);
                if battle.live.is_none()
                    && let Some(end) = &battle.end
                {
                    let resolve = |slots: Option<&[u32]>, unknown: bool| match slots {
                        Some(slots) => {
                            let (tags, missing) = tags_of(&names, slots, read);
                            (tags, unknown || missing)
                        }
                        None => (Vec::new(), true),
                    };
                    let (attackers, a_unknown) = resolve(end.sides[0].as_deref(), end.countries_unknown & 1 != 0);
                    let (defenders, d_unknown) = resolve(end.sides[1].as_deref(), end.countries_unknown & 2 != 0);
                    shaped.runs = vec![LandBattleRun {
                        first_hour: read,
                        last_hour: read,
                        counts: [0, 0],
                        roster_unknown: true,
                        countries: [attackers, defenders],
                        countries_unknown: [a_unknown, d_unknown],
                        duration: end.last_duration.unwrap_or(0),
                    }];
                    if end.manpower.iter().any(Option::is_some) && !shaped.losses.iter().any(|loss| loss.terminal) {
                        shaped.losses.insert(0, LandBattleLossRead { hour: read, terminal: false, sides: LandLossTotals::Read([0, 1].map(|side| LandLossTotal::Unknown { manpower: manpower(end.manpower[side], NOT_RECORDED), unknown: "Equipment losses are not in the battle's end".into() })) });
                    }
                }
                let hour = ranked_hour(&shaped, read);
                if !params.countries.matches(&query.holders, [battle_side(&shaped, 0), battle_side(&shaped, 1)], hour) {
                    continue;
                }
                let casualties = shaped.losses.last().and_then(|loss| match &loss.sides {
                    LandLossTotals::Read(sides) => sides.iter().try_fold(0.0, |sum, side| match side {
                        LandLossTotal::Known { manpower: LandManpower::Known(value), .. } | LandLossTotal::Unknown { manpower: LandManpower::Known(value), .. } => Some(sum + f64::from(*value)),
                        _ => None,
                    }),
                    LandLossTotals::Unread(_) => None,
                });
                let run = shaped.runs.last();
                let duration = shaped.result.as_ref().map_or_else(|| run.map_or(0, |run| i64::from(run.duration) + i64::from(read - run.first_hour)), |result| i64::from(result.duration));
                let length = (i64::from(hour) - i64::from(battle.start)).max(duration - i64::from(battle.counter_base)).max(0) as u64;
                let peak = shaped.prior_peak.map(|peak| peak.value).max(run.filter(|run| !run.roster_unknown).map(|run| u64::from(run.counts[0] + run.counts[1])));
                let last_run = shaped.runs.last().cloned();
                shaped.runs = last_run
                    .map(|run| vec![LandBattleRun { first_hour: read, last_hour: read, duration: (i64::from(run.duration) + i64::from(read - run.first_hour)) as i32, ..run }])
                    .unwrap_or_default();
                shaped.losses = shaped.losses.split_off(shaped.losses.len().saturating_sub(1));
                let chosen = Chosen { run: stretch.run, combat: battle.combat, floor: battle.start.max(first), read, live: battle.live.is_some(), to };
                entries.push((Ranked { casualties, length, peak, id: shaped.id.clone() }, shaped, chosen));
            }
        }
        let mut ranked: Vec<Ranked> = entries.iter().map(|(ranked, ..)| ranked.clone()).collect();
        let effective = rank(&mut ranked, &params.measure);
        let mut by_id: HashMap<String, (LandBattleHistory, Chosen)> = entries.into_iter().map(|(ranked, history, chosen)| (ranked.id, (history, chosen))).collect();
        let mut rows: Vec<(LandBattleHistory, Chosen)> = ranked.into_iter().take(params.top_n as usize).filter_map(|entry| by_id.remove(&entry.id)).collect();
        last_reads(&query, &columns, &mut rows)?;
        let rows = rows.into_iter().map(|(history, _)| history).collect();
        let census_hour = last_read(&query, TableId::LandCombats, params.last_hour)?;
        Ok(LandBattlesRankingReply { rows, census_hour, effective_measure: effective.into(), warnings, content_warnings: Vec::new() })
    }
}

pub fn parse_id(id: &str, branch: u64) -> Option<Key> {
    let mut parts = id.split(':');
    let (held, combat, start) = (parts.next()?, parts.next()?, parts.next()?);
    let written = parts.next().map(str::parse).transpose().ok()?;
    (parts.next().is_none() && held.parse::<u64>().ok()? == branch).then_some(())?;
    Some((combat.parse().ok()?, start.parse().ok()?, written))
}

fn course_side(previous: Option<&Vec<(usize, i64)>>, pool: Option<&Vec<LandEquipmentLoss>>, manpower_value: LandManpower, designs: &mut Vec<u64>) -> (LandLossSide, Option<Vec<(usize, i64)>>) {
    let Some(pool) = pool else { return (LandLossSide::Unknown { manpower: manpower_value, unknown: NOT_RECORDED.into() }, None) };
    let indexed: Vec<(usize, i64)> = pool
        .iter()
        .map(|loss| {
            let at = designs.iter().position(|&variant| variant == loss.variant).unwrap_or_else(|| {
                designs.push(loss.variant);
                designs.len() - 1
            });
            (at, loss.amount_raw)
        })
        .collect();
    let side = match previous {
        Some(held) if held.len() == indexed.len() && held.iter().zip(&indexed).all(|(a, b)| a.0 == b.0) => LandLossSide::Changed {
            manpower: manpower_value,
            changed: indexed.iter().enumerate().filter(|(at, now)| held[*at].1 != now.1).map(|(at, now)| (at, now.1.to_string())).collect(),
        },
        _ => LandLossSide::Variants { manpower: manpower_value, variants: indexed.iter().map(|(at, amount)| (*at, amount.to_string())).collect() },
    };
    (side, Some(indexed))
}

impl Product for LandBattleCourses {
    const KEY: &'static str = "land-battle-course";
    const CONTENT_DEPENDENT: bool = true;
    type Params = LandBattleCourseParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, id];
    type Reply = LandBattleCourseReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LandBattleCourseParams::decl(), LandEquipmentDesign::decl(), LandLossSide::decl(), LandLossSides::decl(), LandBattleCourseRead::decl(), LandEdgeMember::decl(), LandBattleOpening::decl(), LandDivisionExit::decl(), LandEdgeGap::decl(), LandBattleEdges::decl(), LandBattleCourse::decl(), LandBattleCourseReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid battle branch")?;
        if !(3..=4).contains(&params.id.split(':').count()) {
            return Err("Invalid battle id");
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let branch = branch(&params.branch_id)?;
        let query = dashboard_base::queries::query(source, branch)?;
        let key @ (combat, start, _) = parse_id(&params.id, branch).ok_or_else(|| invalid("Invalid battle id"))?;
        let stretch = query.timeline.at(start).ok_or_else(|| dashboard_core::refused("This battle is not in the recording"))?;
        let columns = Columns::new()?;
        let found = battles(&query, &columns, stretch.run, start, stretch.last, Some(combat))?;
        let battle = found.get(&key).ok_or_else(|| dashboard_core::refused("This battle is not in the recording"))?;
        let last = battle.end.as_ref().map_or_else(|| battle.live.map_or(start, |(_, last)| last), |end| end.written).min(stretch.last);
        let data = combats(&query, &columns, stretch.run, start, last, &[combat])?;
        let names = Names::new(&query.holders);
        let links = dashboard_base::queries::links(source, stretch.run)?;
        let shaped = shape(battle, &data, &names, &links, branch, start, last, last);
        let mut designs: Vec<u64> = Vec::new();
        let mut previous: [Option<Vec<(usize, i64)>>; 2] = [None, None];
        let losses = shaped
            .course
            .iter()
            .map(|(hour, terminal, losses)| LandBattleCourseRead {
                hour: *hour,
                terminal: *terminal,
                sides: match losses {
                    Some(losses) => LandLossSides::Read([0, 1].map(|side| {
                        let (made, held) = course_side(previous[side].as_ref(), losses.equipment[side].as_ref(), manpower(losses.manpower[side], NOT_RECORDED), &mut designs);
                        previous[side] = held;
                        made
                    })),
                    None => {
                        previous = [None, None];
                        LandLossSides::Unread(NOT_RECORDED.into())
                    }
                },
            })
            .collect();
        let history = shaped.history;
        let edges = battle_edges(&query, stretch.run, battle, stretch.first, last)?;
        let (designs, content_warnings) = design_names(&query, &super::divisions::Content::load(source, context), stretch.run, last, &designs)?;
        Ok(LandBattleCourseReply {
            battle: LandBattleCourse { id: history.id, province_id: history.province_id, start: history.start, end: history.end, end_unknown_hour: history.end_unknown_hour, counter_base: history.counter_base, runs: history.runs, losses, result: history.result, edges },
            designs,
            content_warnings,
        })
    }
}

fn battle_edges(query: &Query<'_>, run: u32, battle: &Battle, first: i32, last: i32) -> io::Result<LandBattleEdges> {
    use replay_data::rows::LandCombatEdgesRow as Edge;
    let from = (battle.start - 1).max(first);
    let recording = query.replay.recording();
    if recording.predates(TableId::LandCombatEdges.index()).is_some() {
        return Ok(shape_edges(&[], &[], &[], battle, from, last));
    }
    let names = ["kind", "hour", "division", "side", "manpower", "casualties_before", "casualties_after", "withheld"];
    let columns = try_map(names, |name| dashboard_declare::field(TableId::LandCombatEdges, name))?;
    let read_combat = |combat: u64| -> io::Result<Vec<Edge>> {
        let low = u128::from(combat) << 64;
        let read = dashboard_query::cost::storage(|| lanes::history(recording, run, TableId::LandCombatEdges, &sorted(&columns), Keys::Range(low, low | u128::from(u64::MAX)), from, last))?;
        Ok(read
            .keys
            .iter()
            .filter_map(|(key, steps)| {
                let cells = steps.iter().find_map(|step| step.cells.as_deref())?;
                let n = |at: usize| number(&read, cells, columns[at]);
                Some(Edge {
                    id: *key as u64,
                    kind: n(0)? as u8,
                    hour: n(1)? as i32,
                    combat,
                    division: n(2).map(|value| value as u64),
                    side: n(3)? as u8,
                    manpower: n(4).map(|value| value as i32),
                    casualties_before: n(5).map(|value| value as i32),
                    casualties_after: n(6).map(|value| value as i32),
                    withheld: n(7)? as u8,
                })
            })
            .collect())
    };
    let rows = read_combat(battle.combat)?;
    let unattributed: Vec<i32> = read_combat(0)?.iter().map(|row| row.hour).collect();
    let complete = dashboard_query::cost::storage(|| lanes::complete_reads(recording, run, TableId::LandCombatEdges, from, last))?;
    Ok(shape_edges(&rows, &complete, &unattributed, battle, from, last))
}

fn occurrence(rows: &[replay_data::rows::LandCombatEdgesRow], battle: &Battle) -> Vec<replay_data::rows::LandCombatEdgesRow> {
    use replay_data::rows::LandCombatEdgesRow as Edge;
    let mut rows = rows.to_vec();
    rows.sort_unstable_by_key(|row| row.id);
    let opened = |at: usize| rows[at].kind == Edge::OPENED;
    let groups: Vec<usize> = (0..rows.len()).filter(|&at| opened(at) && (at == 0 || !opened(at - 1))).collect();
    let (begin, end) = match groups.iter().position(|&at| rows[at].hour >= battle.start - 1) {
        Some(own) => (groups[own], groups.get(own + 1).copied().unwrap_or(rows.len())),
        None => (rows.iter().rposition(|row| row.kind == Edge::OPENED).map_or(0, |at| at + 1), rows.len()),
    };
    let mut kept = rows[begin..end].to_vec();
    kept.sort_unstable_by_key(|row| (row.hour, row.id));
    kept
}

fn shape_edges(rows: &[replay_data::rows::LandCombatEdgesRow], complete: &[(i32, i32)], unattributed: &[i32], battle: &Battle, from: i32, last: i32) -> LandBattleEdges {
    use replay_data::rows::LandCombatEdgesRow as Edge;
    let rows = &occurrence(rows, battle)[..];
    let mut unrecorded = Vec::new();
    let mut next = from;
    for &(first, through) in complete.iter().filter(|&&(first, through)| through >= from && first <= last) {
        if first > next {
            unrecorded.push(LandEdgeGap { first_hour: next, last_hour: first - 1, reason: NOT_RECORDED.into() });
        }
        next = next.max(through + 1);
    }
    if next <= last {
        unrecorded.push(LandEdgeGap { first_hour: next, last_hour: last, reason: NOT_RECORDED.into() });
    }
    let mut lost: Vec<i32> = unattributed.iter().copied().filter(|hour| (from..=last).contains(hour)).collect();
    lost.sort_unstable();
    lost.dedup();
    unrecorded.extend(lost.into_iter().map(|hour| LandEdgeGap { first_hour: hour, last_hour: hour, reason: UNATTRIBUTED.into() }));
    let division = |row: &Edge| row.division.map(|division| division.to_string());
    let opened: Vec<&Edge> = rows.iter().filter(|row| row.kind == Edge::OPENED).collect();
    let opening = opened.first().map(|row| LandBattleOpening {
        hour: row.hour,
        members: opened
            .iter()
            .filter(|row| row.side <= 1)
            .map(|row| LandEdgeMember {
                division: division(row),
                side: row.side,
                manpower: row.manpower,
                casualties: row.casualties_before,
                missing: row
                    .withheld_reason()
                    .or_else(|| (row.manpower.is_none() || row.casualties_before.is_none()).then(|| MEMBER_UNREAD.into())),
            })
            .collect(),
    });
    let opening_missing = match (&opening, opened.first()) {
        (Some(_), Some(row)) if row.side > 1 => row.withheld_reason(),
        (Some(_), _) => None,
        _ if !battle.exact && battle.result_only.is_none() => Some(OPENED_EARLIER.into()),
        _ if unrecorded.iter().any(|gap| gap.first_hour <= battle.start && gap.last_hour >= from) => Some(NOT_RECORDED.into()),
        _ => Some(NO_OPENING.into()),
    };
    let exits = rows
        .iter()
        .filter(|row| row.kind == Edge::EXITED)
        .map(|row| {
            let losses = row.casualties_after.zip(row.casualties_before).map(|(after, before)| after - before);
            LandDivisionExit {
                hour: row.hour,
                division: division(row),
                side: (row.side <= 1).then_some(row.side),
                losses,
                manpower: row.manpower,
                missing: row.withheld_reason().or_else(|| losses.is_none().then(|| CASUALTIES_UNREAD.into())),
            }
        })
        .collect();
    LandBattleEdges { opening, opening_missing, exits, unrecorded }
}

pub(super) fn design_names(query: &Query<'_>, content: &super::divisions::Content, run: u32, hour: i32, designs: &[u64]) -> io::Result<(Vec<LandEquipmentDesign>, Vec<String>)> {
    use super::divisions::{DESIGN_TABLES, Refusals, find};
    use replay_lanes::at::Read;
    use replay_data::rows::tables;
    if designs.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut keys: Vec<u128> = designs.iter().map(|&variant| u128::from(variant)).collect();
    keys.sort_unstable();
    keys.dedup();
    let reads: Vec<Read<'_>> = DESIGN_TABLES
        .iter()
        .map(|&table| if table == TableId::VariantDefinitions { Read { table, columns: None, keys: Keys::These(&keys) } } else { Read::all(table) })
        .collect();
    let frame = dashboard_query::cost::storage(|| replay_lanes::at::frame(query.replay.recording(), run, hour, &reads))?;
    let mut refusals = Refusals::default();
    let definitions = frame.complete::<tables::VariantDefinitions>().map_err(|why| refusals.note(format!("Design names unavailable: {why}"))).ok();
    let named = designs
        .iter()
        .map(|&variant| {
            let name = definitions.and_then(|rows| match find(Some(rows), variant, |row| row.id) {
                Some(definition) => refusals.keep(content.design_name(&frame, definition)).0,
                None => {
                    refusals.note("An equipment design has no recorded definition at the battle's last hour");
                    None
                }
            });
            LandEquipmentDesign { name, variant: variant.to_string() }
        })
        .collect();
    Ok((named, refusals.lines()))
}

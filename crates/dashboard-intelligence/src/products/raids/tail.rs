use super::{Content, RaidEndAt, RaidEpisode, RaidVictimAt, TYPE_NOT_RECORDED, episode_at, episodes, missing, native};
use dashboard_base::{
    Source,
    catalogue::Catalogue,
    products::{
        LoadContext, Product,
        occurrences::{self, OccurrenceCount, OccurrenceReply, OccurrenceSort},
    },
};
use dashboard_follow::panel::Panel;
use dashboard_query::{
    Query,
    identity::{Country, Holders},
    log::occurrence::{Ask, Chip, Index, Occurrence, PlacedGap, Role, Selection},
    series::ranked::GroupSelection,
    timeline::Timeline,
};
use replay::campaign::Replay;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum RaidStatus {
    Assembling,
    Preparing,
    Prepared,
    InProgress,
    Failed,
    LimitedSuccess,
    Succeeded,
    CriticalSuccess,
    Cancelled,
    OutcomeNotRecorded,
    PhaseNotRecorded,
    NotRead,
}

const STATUSES: [RaidStatus; 12] = [
    RaidStatus::Assembling,
    RaidStatus::Preparing,
    RaidStatus::Prepared,
    RaidStatus::InProgress,
    RaidStatus::Failed,
    RaidStatus::LimitedSuccess,
    RaidStatus::Succeeded,
    RaidStatus::CriticalSuccess,
    RaidStatus::Cancelled,
    RaidStatus::OutcomeNotRecorded,
    RaidStatus::PhaseNotRecorded,
    RaidStatus::NotRead,
];

const TYPED: u8 = 16;

const LIVE: [u8; 8] = [0, 1, 2, 3, TYPED, TYPED + 1, TYPED + 2, TYPED + 3];

const IN_PREPARATION: [u8; 6] = [0, 1, 2, TYPED, TYPED + 1, TYPED + 2];

impl RaidStatus {
    fn code(self) -> u8 {
        STATUSES.iter().position(|held| *held == self).unwrap_or(0) as u8
    }

    fn of_code(code: u8) -> Option<Self> {
        STATUSES.get(usize::from(code & !TYPED)).copied()
    }

    fn at(episode: &RaidEpisode, hour: i32) -> Self {
        let read = &episode.runs[..episode.runs.partition_point(|run| run.first_hour <= hour)];
        let Some(latest) = read.last() else { return Self::PhaseNotRecorded };
        if latest.phase == native::PHASE_ENDED {
            return match read.iter().rev().find(|run| run.outcome != native::OUTCOME_NONE).map(|run| run.outcome) {
                Some(native::OUTCOME_FAILURE) => Self::Failed,
                Some(native::OUTCOME_LIMITED_SUCCESS) => Self::LimitedSuccess,
                Some(native::OUTCOME_SUCCESS) => Self::Succeeded,
                Some(native::OUTCOME_CRITICAL_SUCCESS) => Self::CriticalSuccess,
                Some(native::OUTCOME_CANCELED) => Self::Cancelled,
                _ => Self::OutcomeNotRecorded,
            };
        }
        if episode.disappeared_hour.is_some_and(|gone| gone <= hour) {
            return Self::OutcomeNotRecorded;
        }
        if latest.last_hour < hour {
            return Self::NotRead;
        }
        match latest.phase {
            native::PHASE_ASSEMBLING => Self::Assembling,
            native::PHASE_PREPARING => Self::Preparing,
            native::PHASE_PREPARED => Self::Prepared,
            native::PHASE_IN_PROGRESS => Self::InProgress,
            _ => Self::PhaseNotRecorded,
        }
    }
}

fn state(episode: &RaidEpisode, hour: i32) -> u8 {
    RaidStatus::at(episode, hour).code() | if episode.raid_type_hour.is_some_and(|known| known <= hour) { TYPED } else { 0 }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidRaider {
    #[doc = " The raider slot's tag at the row's hour; null with `missing` when it holds none."]
    pub tag: Option<String>,
    pub slot: Option<u32>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidPlaceAt {
    #[doc = " The target's province and state, each from the hour it was first read."]
    pub province: Option<u32>,
    pub province_hour: Option<i32>,
    pub state: Option<u32>,
    pub state_hour: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidPreparationAt {
    #[doc = " The recorded preparation in force at the hour asked: `hours` at `since`, then `perHour` each hour"]
    #[doc = " (0 until a second read gives the rate), through `through` when that is already past."]
    pub since: i32,
    pub hours: i32,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
    pub per_hour: i64,
    pub through: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidPhaseAt {
    pub first_hour: i32,
    pub first_exact: bool,
    pub after_hour: Option<i32>,
    pub phase: i32,
    pub phase_name: Option<String>,
    pub outcome: i32,
    pub outcome_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidLogRow {
    #[doc = " The raid episode's id: the row key and what a click hands to the detail."]
    pub key: String,
    #[doc = " The episode's start: its first read, `exact` when the read before it proves it, else after `afterHour`."]
    pub hour: i32,
    pub exact: bool,
    pub after_hour: Option<i32>,
    pub observed: i32,
    pub ordinal: String,
    pub live: bool,
    pub status: RaidStatus,
    pub phase_name: Option<String>,
    pub outcome_name: Option<String>,
    pub outcome_missing: Option<String>,
    pub raider: RaidRaider,
    #[doc = " The victim as last read by the hour asked, named at that read."]
    pub victim: RaidVictimAt,
    #[doc = " The type's recorded spelling once read by the hour asked; null with `typeMissing` before."]
    pub raid_type: Option<String>,
    pub raid_type_name: Option<String>,
    pub type_missing: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
    pub place: Option<RaidPlaceAt>,
    pub place_missing: Option<String>,
    pub preparation: Option<RaidPreparationAt>,
    pub phases: Vec<RaidPhaseAt>,
    pub launched_hour: Option<i32>,
    pub launched_exact: bool,
    #[doc = " The end read by the hour asked, naming the raider at that end."]
    pub end: Option<RaidEndAt>,
    pub disappeared_hour: Option<i32>,
    pub disappeared_exact: bool,
    #[doc = " The raid's last read, once the hour asked is after it with no end or disappearance read."]
    pub last_seen: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidTypeChip {
    pub spelling: String,
    #[doc = " The hour a raid of this type was first read with its type spelled."]
    pub first_hour: i32,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RaidLogTailParams {
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub to: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub countries: Option<Vec<Country>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub sort: Option<OccurrenceSort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub offset: Option<String>,
    pub limit: u32,
    #[doc = " Raids of these recorded type spellings, each only once its type was read; every raid when none."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub raid_types: Option<Vec<String>>,
    #[doc = " The statuses listed, each at the hour asked; every one when none."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub statuses: Option<Vec<RaidStatus>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidLogTailReply {
    pub branch_id: String,
    pub total: String,
    pub offset: String,
    pub live: String,
    pub counts: Vec<OccurrenceCount>,
    #[doc = " The types first read by the hour asked, in spelling order."]
    pub raid_types: Vec<RaidTypeChip>,
    pub rows: Vec<RaidLogRow>,
    pub missing: Vec<PlacedGap>,
    pub next_change: Option<i32>,
    #[doc = " Why no raid can exist: the content declares no raid type."]
    pub unavailable: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(RaidLogTailReply);

impl OccurrenceReply for RaidLogTailReply {
    fn next_change(&self) -> Option<i32> {
        self.next_change
    }

    fn row_count(&self) -> usize {
        self.rows.len()
    }
}

struct Raid {
    episode: RaidEpisode,
    changes: Box<[i32]>,
}

struct Tail {
    index: Index<Raid>,
    spellings: Vec<String>,
    chips: Vec<RaidTypeChip>,
    icons: BTreeMap<String, Option<String>>,
    unavailable: Option<String>,
    warnings: Vec<String>,
}

#[derive(Clone)]
enum Made {
    Catalogue(Arc<Catalogue>),
    Refused(String),
}

impl Made {
    fn of(content: &Content) -> Self {
        match &content.catalogue {
            Ok(catalogue) => Self::Catalogue(Arc::clone(catalogue)),
            Err(reason) => Self::Refused(reason.clone()),
        }
    }

    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Catalogue(a), Self::Catalogue(b)) => Arc::ptr_eq(a, b),
            (Self::Refused(a), Self::Refused(b)) => a == b,
            _ => false,
        }
    }
}

#[derive(Default)]
struct Kept(HashMap<u64, (Made, Arc<Tail>)>);

pub fn forget(source: &Source) {
    *source.state::<Kept>().lock().unwrap_or_else(PoisonError::into_inner) = Kept::default();
}

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

fn id_of(key: &str) -> io::Result<u128> {
    let (raw, nth) = key.split_once('#').map_or((key, None), |(raw, nth)| (raw, Some(nth)));
    let raw: u128 = raw.parse().map_err(|_| invalid(format!("Invalid raid episode id {key}")))?;
    match nth.map(str::parse::<u128>) {
        None => Ok(raw),
        Some(Ok(nth)) => Ok(raw ^ (nth << 112)),
        Some(Err(_)) => Err(invalid(format!("Invalid raid episode id {key}"))),
    }
}

fn changes_of(episode: &RaidEpisode, last: i32) -> Vec<i32> {
    let mut hours: Vec<i32> = [Some(episode.first_hour), episode.raid_type_hour, episode.icon_hour, episode.place_hour, episode.state_hour, episode.launched_hour, episode.disappeared_hour]
        .into_iter()
        .flatten()
        .chain(episode.runs.iter().flat_map(|run| [run.first_hour, run.last_hour.saturating_add(1)]))
        .chain(episode.victims.iter().map(|victim| victim.hour))
        .chain(episode.ends.iter().map(|end| end.hour))
        .chain(episode.preparation.iter().flat_map(|run| [run.0, run.0.saturating_add(1), run.1.saturating_add(1)]))
        .filter(|&hour| hour >= episode.first_hour && hour <= last)
        .collect();
    hours.sort_unstable();
    hours.dedup();
    hours
}

fn scan(source: &Source, query: &Query<'_>, content: &Content) -> io::Result<Tail> {
    let (first, last) = (query.timeline.first, query.timeline.last);
    let mut scratch = BTreeSet::new();
    let (found, artwork) = episodes(source, query, (first, last), &[], content, &mut scratch)?;
    let mut first_spelled: BTreeMap<String, i32> = BTreeMap::new();
    for episode in &found {
        if let (Some(spelling), Some(hour)) = (&episode.raid_type, episode.raid_type_hour) {
            let held = first_spelled.entry(spelling.clone()).or_insert(hour);
            *held = (*held).min(hour);
        }
    }
    let spellings: Vec<String> = first_spelled.keys().cloned().collect();
    let chips = first_spelled
        .iter()
        .map(|(spelling, &first_hour)| {
            let (icon, icon_missing) = match content.icon(spelling, None) {
                Ok((_, image)) => (Some(image), None),
                Err(reason) => (None, Some(reason)),
            };
            RaidTypeChip { spelling: spelling.clone(), first_hour, name: content.label(&format!("raid_type_{spelling}")), icon, icon_missing }
        })
        .collect();
    let mut occurrences = Vec::with_capacity(found.len());
    for episode in found {
        let changes = changes_of(&episode, last);
        let mut states: Vec<(i32, u8)> = Vec::new();
        for &hour in &changes {
            let code = state(&episode, hour);
            if states.last().is_none_or(|&(_, held)| held != code) {
                states.push((hour, code));
            }
        }
        let date = episode.first_hour;
        let mut roles = vec![Role { slot: episode.raider_slot, at: date }];
        roles.extend(episode.victims.iter().filter_map(|victim| victim.tag.as_ref().map(|tag| Role { slot: query.holders.slot_at(&Country::Tag(tag.clone()), victim.hour), at: victim.hour })));
        let mut text: Vec<&str> = episode.raider_slot.and_then(|slot| query.holders.slot_tag(slot, date)).into_iter().collect();
        text.extend(episode.victims.iter().filter(|victim| victim.hour == date).filter_map(|victim| victim.tag.as_deref()));
        if episode.raid_type_hour == Some(date) {
            text.extend(episode.raid_type.as_deref());
            text.extend(episode.raid_type_name.as_deref());
        }
        let kind = episode.raid_type.as_ref().and_then(|spelling| spellings.binary_search(spelling).ok()).map_or(0, |at| at as u16 + 1);
        occurrences.push(Occurrence {
            id: id_of(&episode.id)?,
            date,
            observed: date,
            kind,
            roles: roles.into_boxed_slice(),
            subjects: Box::new([]),
            states: states.into_boxed_slice(),
            sort_keys: Box::new([]),
            text: text.join(" ").into_boxed_str(),
            payload: Raid { episode, changes: changes.into_boxed_slice() },
        });
    }
    let none = content.raids.as_ref().is_ok_and(|raids| raids.types.is_empty() && raids.uncertain.is_empty());
    Ok(Tail {
        index: Index::new(occurrences, missing(query, first, last)?, &LIVE)?,
        spellings,
        chips,
        icons: artwork.into_iter().map(|(key, art)| (key, art.image)).collect(),
        unavailable: none.then(|| "The content declares no raid type.".to_owned()),
        warnings: content.warnings(),
    })
}

fn tail(source: &Source, query: &Query<'_>, branch: u64, load: &LoadContext) -> io::Result<Arc<Tail>> {
    let content = Content::load(source, load);
    let made = Made::of(&content);
    let kept = source.state::<Kept>();
    if let Some((held, found)) = kept.lock().unwrap_or_else(PoisonError::into_inner).0.get(&branch)
        && held.same(&made)
    {
        return Ok(Arc::clone(found));
    }
    let built = Arc::new(scan(source, query, &content)?);
    kept.lock().unwrap_or_else(PoisonError::into_inner).0.insert(branch, (made, Arc::clone(&built)));
    Ok(built)
}

struct Prepared {
    tail: Arc<Tail>,
    selection: Option<Selection>,
}

fn prepare(source: &Source, query: &Query<'_>, branch: u64, load: &LoadContext, params: &RaidLogTailParams, cancelled: &AtomicBool) -> io::Result<Prepared> {
    let selection = dashboard_world::products::group_members::dated(source, query, load, params.countries.as_deref(), params.group.as_ref(), cancelled)?;
    Ok(Prepared { tail: tail(source, query, branch, load)?, selection })
}

fn preparation_at(episode: &RaidEpisode, hour: i32) -> Option<RaidPreparationAt> {
    let run = episode.preparation.iter().rev().find(|run| run.0 <= hour)?;
    Some(RaidPreparationAt { since: run.0, hours: run.2, per_hour: if run.1 > run.0 { run.3 } else { 0 }, through: (run.1 < hour).then_some(run.1) })
}

fn row(tail: &Tail, holders: &Holders, raid: &Raid, shown_ordinal: u64, live: bool, status: RaidStatus, hour: i32) -> Option<RaidLogRow> {
    let at = episode_at(&raid.episode, hour)?;
    let date = at.first_hour;
    let raider = match at.raider_slot {
        None => RaidRaider { tag: None, slot: None, missing: Some("The raid's raider is not recorded".to_owned()) },
        Some(slot) => match holders.slot_tag(slot, date) {
            Some(tag) => RaidRaider { tag: Some(tag.to_owned()), slot: Some(slot), missing: None },
            None => RaidRaider { tag: None, slot: Some(slot), missing: Some("The raider's country holds no tag at the raid's hour".to_owned()) },
        },
    };
    let victim = at.victims.last().cloned().unwrap_or(RaidVictimAt { hour: date, tag: None, missing: Some("The raid's target is not recorded".to_owned()), source: None });
    let place = (at.place_hour.is_some() || at.state_hour.is_some()).then(|| RaidPlaceAt { province: at.province, province_hour: at.place_hour, state: at.state, state_hour: at.state_hour });
    let icon = at.icon.as_ref().and_then(|key| tail.icons.get(key).cloned().flatten());
    let icon_missing = at.icon_missing.clone().or_else(|| at.icon.as_ref().filter(|_| icon.is_none()).map(|key| format!("raid sprite {key} is unavailable")));
    Some(RaidLogRow {
        key: at.id.clone(),
        hour: date,
        exact: at.first_hour_exact,
        after_hour: at.runs.first().and_then(|run| run.after_hour),
        observed: date,
        ordinal: shown_ordinal.to_string(),
        live,
        status,
        phase_name: at.phase_name.clone(),
        outcome_name: at.outcome_name.clone(),
        outcome_missing: at.outcome_missing.clone(),
        raider,
        victim,
        type_missing: at.raid_type.is_none().then(|| TYPE_NOT_RECORDED.to_owned()),
        raid_type: at.raid_type.clone(),
        raid_type_name: at.raid_type_name.clone(),
        icon,
        icon_missing,
        place_missing: place.is_none().then(|| "The raid's target place is not recorded".to_owned()),
        place,
        preparation: matches!(status, RaidStatus::Preparing | RaidStatus::Prepared).then(|| preparation_at(&at, hour)).flatten(),
        phases: at
            .runs
            .iter()
            .map(|run| RaidPhaseAt {
                first_hour: run.first_hour,
                first_exact: run.first_exact,
                after_hour: run.after_hour,
                phase: run.phase,
                phase_name: run.phase_name.clone(),
                outcome: run.outcome,
                outcome_name: run.outcome_name.clone(),
            })
            .collect(),
        launched_hour: at.launched_hour,
        launched_exact: at.launched_exact,
        end: at.ends.last().cloned(),
        disappeared_hour: at.disappeared_hour,
        disappeared_exact: at.disappeared_exact,
        last_seen: (status == RaidStatus::NotRead).then_some(at.last_hour),
    })
}

fn load_tail_at(prepared: &Prepared, query: &Query<'_>, params: &RaidLogTailParams, hour: i32, cancelled: &AtomicBool) -> io::Result<RaidLogTailReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let tail = &prepared.tail;
    let index = &tail.index;
    let (order, newest) = occurrences::order(params.sort.as_ref(), &[])?;
    let (offset, limit) = occurrences::window(params.offset.as_deref(), params.limit)?;
    let kinds: Option<Vec<u16>> = params.raid_types.as_ref().map(|asked| asked.iter().filter_map(|spelling| tail.spellings.binary_search(spelling).ok()).map(|at| at as u16 + 1).collect());
    let statuses: Vec<RaidStatus> = params.statuses.clone().unwrap_or_else(|| STATUSES.to_vec());
    let typed: &[u8] = if kinds.is_some() { &[TYPED] } else { &[0, TYPED] };
    let states: Option<Vec<u8>> = (params.statuses.is_some() || kinds.is_some()).then(|| statuses.iter().flat_map(|status| typed.iter().map(move |bit| status.code() | bit)).collect());
    let chips = [Chip { kinds: None, states: Some(IN_PREPARATION.to_vec()) }];
    let answer = index.ask(
        &query.holders,
        &Ask {
            hour,
            from: params.from,
            to: params.to,
            selection: prepared.selection.as_ref(),
            subject: None,
            kinds: kinds.as_deref(),
            states: states.as_deref(),
            search: params.search.as_deref(),
            order,
            live_first: newest,
            offset,
            limit,
            chips: &chips,
        },
    )?;
    let mut next_change = answer.next_change;
    let mut rows = Vec::with_capacity(answer.rows.len());
    for shown in &answer.rows {
        let raid = &index.get(shown.at).payload;
        let status = shown.state.and_then(RaidStatus::of_code).unwrap_or(RaidStatus::PhaseNotRecorded);
        rows.push(row(tail, &query.holders, raid, shown.ordinal, shown.live, status, hour).ok_or_else(|| invalid(format!("Raid {} is listed before its first read", raid.episode.id)))?);
        if let Some(&later) = raid.changes.get(raid.changes.partition_point(|&at| at <= hour)) {
            next_change = Some(next_change.map_or(later, |held| held.min(later)));
        }
    }
    let first_chip = tail.chips.iter().filter(|chip| chip.first_hour > hour).map(|chip| chip.first_hour).min();
    if let Some(later) = first_chip {
        next_change = Some(next_change.map_or(later, |held| held.min(later)));
    }
    let reply = RaidLogTailReply {
        branch_id: params.branch_id.clone(),
        total: answer.total.to_string(),
        offset: answer.offset.to_string(),
        live: answer.live.to_string(),
        counts: occurrences::counts(&answer, &["active"]),
        raid_types: tail.chips.iter().filter(|chip| chip.first_hour <= hour).cloned().collect(),
        rows,
        missing: answer.gaps,
        next_change,
        unavailable: tail.unavailable.clone(),
        warnings: tail.warnings.clone(),
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

pub struct RaidLogTail;

impl Product for RaidLogTail {
    const KEY: &'static str = "raid-log-tail";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = RaidLogTailParams;
    const FIELDS: dashboard_base::products::Fields =
        dashboard_base::fields![branchId, hour: Read, from: Start, to: End, countries, "countries[]", group, search, sort, offset, limit, raidTypes, "raidTypes[]", statuses];
    type Reply = RaidLogTailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            RaidLogTailParams::decl(),
            RaidStatus::decl(),
            RaidRaider::decl(),
            RaidPlaceAt::decl(),
            RaidPreparationAt::decl(),
            RaidPhaseAt::decl(),
            RaidLogRow::decl(),
            RaidTypeChip::decl(),
            RaidLogTailReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        crate::products::intelligence::branch(&params.branch_id)?;
        params.countries.iter().flatten().try_for_each(dashboard_base::products::validate_country)
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().map_err(|_| invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let hour = occurrences::cutoff(&query, params.hour)?;
        let prepared = prepare(source, &query, branch, load, params, cancelled)?;
        load_tail_at(&prepared, &query, params, hour, cancelled)
    }
}

struct Owned {
    replay: Arc<Replay>,
    timeline: Arc<Timeline>,
    holders: Arc<Holders>,
}

pub fn dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let mut request = asked.clone();
    request.as_object_mut().ok_or("Invalid raid-log-tail panel")?.remove("panel");
    let params: RaidLogTailParams = dashboard_query::named::named(&request).map_err(|error| format!("Invalid raid-log-tail panel: {error}"))?;
    RaidLogTail::validate(&params)?;
    let branch = params.branch_id.parse().map_err(|_| "Invalid branch")?;
    let query = dashboard_base::queries::query(source, branch).map_err(|error| error.to_string())?;
    let prepared = prepare(source, &query, branch, load, &params, &AtomicBool::new(false)).map_err(|error| error.to_string())?;
    let owned = Owned { replay: source.replay(), timeline: Arc::clone(&query.timeline), holders: Arc::clone(&query.holders) };
    Ok(occurrences::occurrence_panel(source.tables(), move |hour, cancelled| {
        let query = Query { replay: &owned.replay, timeline: Arc::clone(&owned.timeline), holders: Arc::clone(&owned.holders) };
        let hour = occurrences::cutoff(&query, Some(hour))?;
        load_tail_at(&prepared, &query, &params, hour, cancelled)
    }))
}


use dashboard_base::{
    Source,
    products::{
        Country as RequestedCountry, LoadContext, Product,
        at_hour::{At, AtHour},
        validate_country,
    },
    queries::country_ids::Names,
};
use dashboard_intelligence::products::intelligence::{Country, IDENTITY, Tags, branch, missing};
use dashboard_query::{coverage::population, identity::Holder, series::ranked::GroupSelection};
use dashboard_world::products::group_members::GroupsAt;
use replay_columnar::manifest::Status;
use replay_data::rows::{CountryOccupationReserveManpowerRow, TableId, tables};
use replay_frame::frame::{DecodedFrame, Keys};
use replay_lanes::{lanes, points::Cell as Read};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const TABLES: [TableId; 5] = [
    TableId::OccupationSettings,
    TableId::CountryOccupationReserveManpower,
    TableId::CountryCounterInputs,
    IDENTITY[0],
    IDENTITY[1],
];

const fn entry_key(holder: u32, ordinal: u32) -> u128 {
    ((holder as u128) << 32) | ordinal as u128
}

pub struct OccupationReserve;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OccupationReserveParams {
    pub branch_id: String,
    #[serde(default)]
    pub countries: Vec<RequestedCountry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[doc = " The group whose members at the hour are the rows; never with countries or a top."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ReserveEntry {
    pub ordinal: u32,
    pub source: Country,
    pub amount: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ReserveHolder {
    pub country: Country,
    #[doc = " The recorded slot holding the country at the hour."]
    pub slot: Option<u32>,
    #[doc = " Its occupation status was read: no entry is a recorded zero."]
    pub admitted: bool,
    #[doc = " Sum of its positive entries at this hour."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
    pub total: i64,
    #[doc = " Its entries in list order."]
    pub entries: Vec<ReserveEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OccupationReserveReply {
    #[doc = " Ranked descending by positive reserve total, then tag."]
    pub holders: Vec<ReserveHolder>,
    #[doc = " The reserve's read hour the rows hold."]
    pub read_hour: Option<i32>,
    #[doc = " Holders whose recording named no tag at this hour."]
    pub unnamed: u32,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(OccupationReserveReply);

pub fn holders(
    admitted: &[u32],
    entries: &[CountryOccupationReserveManpowerRow],
    owned: Option<&HashMap<u32, i32>>,
    country: impl Fn(u32) -> Country,
) -> Vec<ReserveHolder> {
    let mut held: BTreeMap<u32, ReserveHolder> = admitted
        .iter()
        .map(|&raw| (raw, ReserveHolder { country: country(raw), slot: None, admitted: true, total: 0, entries: Vec::new() }))
        .collect();
    let mut sorted: Vec<&CountryOccupationReserveManpowerRow> = entries.iter().collect();
    sorted.sort_by_key(|entry| (entry.country, entry.ordinal));
    for entry in sorted {
        let holder = held.entry(entry.country).or_insert_with(|| ReserveHolder { country: country(entry.country), slot: None, admitted: false, total: 0, entries: Vec::new() });
        if entry.amount > 0 {
            holder.total += i64::from(entry.amount);
        }
        holder.entries.push(ReserveEntry { ordinal: entry.ordinal, source: country(entry.source_country), amount: entry.amount });
    }
    held.into_iter()
        .filter(|(raw, _)| owned.is_none_or(|owned| owned.get(raw).is_none_or(|&states| states > 0)))
        .map(|(_, holder)| holder)
        .collect()
}

fn select_holders(
    all: Vec<ReserveHolder>,
    params: &OccupationReserveParams,
    at: &At<'_, GroupsAt>,
    hour: i32,
) -> Result<(Vec<ReserveHolder>, u32), String> {
    let slots: HashMap<u32, u32> = at
        .holders
        .tags
        .values()
        .flatten()
        .filter(|run| run.first <= hour && hour <= run.last)
        .filter_map(|run| match run.holder {
            Holder::Found { slot, id } => Some((id, slot)),
            Holder::Unresolved => None,
        })
        .collect();
    let all: Vec<ReserveHolder> = all.into_iter().map(|holder| ReserveHolder { slot: slots.get(&holder.country.id).copied(), ..holder }).collect();
    let chosen = at.context.chosen_slots(params.group.as_ref(), &params.countries, at.holders, hour).map_err(|error| error.to_string())??;
    let unnamed = all.iter().filter(|holder| holder.country.tag.is_none() && (holder.admitted || !holder.entries.is_empty())).count() as u32;
    let unresolved = at.holders.unknown.iter().any(|&(first, last, _)| first <= hour && hour <= last)
        || at.holders.tags.values().flatten().any(|run| run.first <= hour && hour <= run.last && run.holder == Holder::Unresolved);
    let mut unplaced: Vec<i64> = Vec::new();
    let mut named: Vec<ReserveHolder> = Vec::new();
    for holder in all {
        if !holder.admitted && holder.entries.is_empty() {
            continue;
        }
        if holder.country.tag.is_some() {
            named.push(holder);
        } else if holder.total > 0 {
            unplaced.push(holder.total);
        }
    }
    if let Some(chosen) = chosen {
        named.retain(|holder| holder.slot.is_some_and(|slot| chosen.contains(&slot)));
        if !named.iter().any(|holder| !holder.entries.is_empty()) {
            return Ok((Vec::new(), unnamed));
        }
        named.sort_unstable_by(|a, b| b.total.cmp(&a.total).then_with(|| a.country.tag.cmp(&b.country.tag)));
        return Ok((named, unnamed));
    }
    if let Some(limit) = params.top {
        named.retain(|holder| holder.total > 0);
        named.sort_unstable_by(|a, b| b.total.cmp(&a.total).then_with(|| a.country.tag.cmp(&b.country.tag)));
        let count = usize::try_from(limit).unwrap_or(usize::MAX);
        named.truncate(count);
        if (unresolved || !unplaced.is_empty()) && !unplaced.is_empty() {
            let full = named.len() >= count;
            let floor = named.last().map(|holder| holder.total);
            let contending = unplaced.iter().filter(|&&value| !full || floor.is_none_or(|floor| value >= floor)).count();
            if contending > 0 {
                return Err(format!("The top {limit} occupation reserve holders are not known: {contending} countries could not be matched to a tag at this hour and could be among them"));
            }
        }
        return Ok((named, unnamed));
    }
    if !named.iter().any(|holder| !holder.entries.is_empty()) {
        return Ok((Vec::new(), unnamed));
    }
    named.sort_unstable_by(|a, b| b.total.cmp(&a.total).then_with(|| a.country.tag.cmp(&b.country.tag)));
    Ok((named, unnamed))
}

impl AtHour for OccupationReserve {
    const KEY: &'static str = "occupation-reserve-manpower";
    const TITLE: &'static str = "Occupation reserve";
    const CONTENT_DEPENDENT: bool = false;
    type Params = OccupationReserveParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, top, group, "countries[]"];
    type Reply = OccupationReserveReply;
    type Context = GroupsAt;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![OccupationReserveParams::decl(), Country::decl(), ReserveEntry::decl(), ReserveHolder::decl(), OccupationReserveReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        if params.top.is_some_and(|top| top == 0 || !params.countries.is_empty()) {
            return Err("Invalid country selection");
        }
        if params.group.is_some() && (params.top.is_some() || !params.countries.is_empty()) {
            return Err("A selection names a group, a top or countries, one of them");
        }
        for country in &params.countries {
            validate_country(country)?;
        }
        Ok(())
    }

    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        at.context.valid_through(params.group.as_ref(), at.hour)
    }

    fn projection(_params: &Self::Params, table: TableId, _plan: &[(Vec<dashboard_base::products::at_hour::Read>, i32)], _context: &Self::Context) -> Option<Vec<usize>> {
        match table {
            TableId::OccupationSettings => Some(Vec::new()),
            TableId::CountryCounterInputs => Some(dashboard_base::products::at_hour::field_columns(table, &["owned_state_count"])),
            TableId::Countries => Some(dashboard_base::products::at_hour::field_columns(table, &["canonical_slot", "tag"])),
            _ => None,
        }
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(GroupsAt::default())
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        GroupsAt::load(source, load, &params.branch_id, params.group.as_ref())
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut warnings = Vec::new();
        let read_hour = frame.tables[TableId::CountryOccupationReserveManpower.index()].sampled_at();
        let made = (|| -> Result<(Vec<ReserveHolder>, u32), String> {
            let tags = Tags::read(frame).map_err(|error| error.to_string())?;
            let admitted: Vec<u32> = population::<tables::OccupationSettings>(frame, &Keys::All)?.iter().map(|row| row.country).collect();
            let entries = population::<tables::CountryOccupationReserveManpower>(frame, &Keys::All)?;
            let owned: Option<HashMap<u32, i32>> = match population::<tables::CountryCounterInputs>(frame, &Keys::All) {
                Ok(rows) => Some(rows.iter().map(|row| (row.country, row.owned_state_count)).collect()),
                Err(_) => {
                    warnings.extend(missing(frame, TableId::CountryCounterInputs).map(|reason| format!("Which holders exist is not known, so none is left out: {reason}")));
                    None
                }
            };
            let built = holders(&admitted, entries, owned.as_ref(), |raw| Country::of(&tags, raw));
            select_holders(built, params, at, frame.hour)
        })();
        Ok(match made {
            Ok((holders, unnamed)) => OccupationReserveReply { holders, read_hour, unnamed, missing: None, warnings },
            Err(reason) => OccupationReserveReply { holders: Vec::new(), read_hour, unnamed: 0, missing: Some(reason), warnings },
        })
    }
}

pub struct ReserveHistory;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReserveHistoryParams {
    pub branch_id: String,
    #[doc = " The holder's raw country id."]
    pub holder: u32,
    #[doc = " The hour answered: nothing recorded after it is read into the reply."]
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ReserveAmount {
    pub source: u32,
    pub amount: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ReserveSegment {
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " Its occupation status was read: no entry is a recorded zero."]
    pub admitted: bool,
    #[doc = " Its entries in list order; none where the reserve was not known, with why."]
    pub entries: Option<Vec<ReserveAmount>>,
    pub unknown: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SourceTag {
    pub tag: Option<String>,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ReserveSource {
    pub id: u32,
    pub tags: Vec<SourceTag>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ReserveHistoryReply {
    #[doc = " The hour answered, never after the branch's last hour."]
    pub hour: i32,
    #[doc = " The first hour after `hour` at which an entry, admission, table coverage, source tag or timeline stretch changes; null when none does."]
    pub next_change: Option<i32>,
    #[doc = " In hour order through `hour`; hours the timeline does not hold are not in it."]
    pub segments: Vec<ReserveSegment>,
    #[doc = " Every source the segments by `hour` name, by raw id, with tags through `hour`."]
    pub sources: Vec<ReserveSource>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ReserveHistoryReply);

fn status_reason(status: Status) -> Option<String> {
    match status {
        Status::Complete => None,
        Status::NotRecorded => Some("The reserve was not recorded at these hours".to_owned()),
        Status::Truncated { missing } => Some(format!("The reserve was read in part at these hours ({missing} rows not read)")),
        Status::Unavailable { .. } => Some("The reserve was not available at these hours".to_owned()),
    }
}

fn admission_status_reason(status: Status) -> Option<String> {
    match status {
        Status::Complete => None,
        Status::NotRecorded => Some("Occupation settings were not recorded at these hours".to_owned()),
        Status::Truncated { missing } => Some(format!("Occupation settings were read in part at these hours ({missing} rows not read)")),
        Status::Unavailable { .. } => Some("Occupation settings were not available at these hours".to_owned()),
    }
}

fn number(cell: Read) -> Option<i64> {
    match cell {
        Read::Number(value) => Some(value),
        _ => None,
    }
}

fn cells_at<'a>(steps: &'a [lanes::Step], at: &mut usize, hour: i32) -> Option<&'a [Read]> {
    while *at + 1 < steps.len() && steps[*at + 1].hour <= hour {
        *at += 1;
    }
    steps.get(*at).filter(|step| step.hour <= hour).and_then(|step| step.cells.as_deref())
}

fn stretch_segments(
    entries: &lanes::History,
    admission: &lanes::History,
    statuses: &[lanes::StatusRun],
    admission_statuses: &[lanes::StatusRun],
    first: i32,
    last: i32,
) -> Vec<ReserveSegment> {
    let mut cuts: BTreeSet<i32> = BTreeSet::from([first]);
    cuts.extend(entries.keys.iter().flat_map(|(_, steps)| steps.iter().map(|step| step.hour)));
    cuts.extend(admission.keys.iter().flat_map(|(_, steps)| steps.iter().map(|step| step.hour)));
    cuts.extend(statuses.iter().flat_map(|run| [run.first, run.last.saturating_add(1)]));
    cuts.extend(admission_statuses.iter().flat_map(|run| [run.first, run.last.saturating_add(1)]));
    let cuts: Vec<i32> = cuts.into_iter().filter(|&hour| first <= hour && hour <= last).collect();
    let mut places = vec![0usize; entries.keys.len()];
    let mut admitted_at = 0usize;
    let mut out: Vec<ReserveSegment> = Vec::new();
    for (index, &hour) in cuts.iter().enumerate() {
        let end = cuts.get(index + 1).map_or(last, |next| next - 1);
        let status = statuses.iter().find(|run| run.first <= hour && hour <= run.last).map_or(Status::NotRecorded, |run| run.status);
        let admission_status = admission_statuses.iter().find(|run| run.first <= hour && hour <= run.last).map_or(Status::NotRecorded, |run| run.status);
        let admitted = admission_status == Status::Complete && admission.keys.first().is_some_and(|(_, steps)| cells_at(steps, &mut admitted_at, hour).is_some());
        let mut held = Vec::new();
        for ((_, steps), place) in entries.keys.iter().zip(places.iter_mut()) {
            if let Some(cells) = cells_at(steps, place, hour)
                && let (Some(source), Some(amount)) = (number(cells[0]), number(cells[1]))
            {
                held.push(ReserveAmount { source: source as u32, amount: amount as i32 });
            }
        }
        let unknown = status_reason(status).or_else(|| held.is_empty().then(|| admission_status_reason(admission_status)).flatten());
        let segment = ReserveSegment { first_hour: hour, last_hour: end, admitted, entries: unknown.is_none().then_some(held), unknown };
        match out.last_mut() {
            Some(previous) if previous.last_hour + 1 == hour && (previous.admitted, &previous.entries, &previous.unknown) == (segment.admitted, &segment.entries, &segment.unknown) => {
                previous.last_hour = end;
            }
            _ => out.push(segment),
        }
    }
    out
}

fn next_history_change(
    entries: &lanes::History,
    admission: &lanes::History,
    statuses: &[lanes::StatusRun],
    admission_statuses: &[lanes::StatusRun],
    hour: i32,
    limit: i32,
) -> Option<i32> {
    let mut next = i32::MAX;
    let mut after = |at: i32| {
        if at > hour && at <= limit && at < next {
            next = at;
        }
    };
    for (_, steps) in &entries.keys {
        for step in steps {
            after(step.hour);
        }
    }
    for (_, steps) in &admission.keys {
        for step in steps {
            after(step.hour);
        }
    }
    for run in statuses.iter().chain(admission_statuses.iter()) {
        after(run.first);
        after(run.last.saturating_add(1));
    }
    (next != i32::MAX).then_some(next)
}

struct HolderRunHistory {
    entries: lanes::History,
    admitted: lanes::History,
    statuses: Vec<lanes::StatusRun>,
    admission_statuses: Vec<lanes::StatusRun>,
}

#[derive(Default)]
struct HistoryCache {
    by_stretch_holder: HashMap<(u32, i32, i32, u32), Arc<HolderRunHistory>>,
}

fn cached_holder_history(source: &Source, run: u32, first: i32, last: i32, holder: u32) -> io::Result<Arc<HolderRunHistory>> {
    let cache = source.state::<HistoryCache>();
    let key = (run, first, last, holder);
    if let Some(held) = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).by_stretch_holder.get(&key) {
        return Ok(Arc::clone(held));
    }
    let recording = source.replay_ref().recording();
    let table = TableId::CountryOccupationReserveManpower;
    let columns = [dashboard_declare::field(table, "source_country")?, dashboard_declare::field(table, "amount")?];
    let admission = [dashboard_declare::field(TableId::OccupationSettings, "default_law_present")?];
    let holder_key = [u128::from(holder)];
    let range = lanes::Keys::Range(entry_key(holder, 0), entry_key(holder, u32::MAX));
    let (entries, admitted, statuses, admission_statuses) = dashboard_query::cost::storage(|| {
        io::Result::Ok((
            lanes::history(recording, run, table, &columns, range, first, last)?,
            lanes::history(recording, run, TableId::OccupationSettings, &admission, lanes::Keys::These(&holder_key), first, last)?,
            lanes::statuses(recording, run, table, first, last)?,
            lanes::statuses(recording, run, TableId::OccupationSettings, first, last)?,
        ))
    })?;
    let built = Arc::new(HolderRunHistory { entries, admitted, statuses, admission_statuses });
    let mut guard = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(Arc::clone(guard.by_stretch_holder.entry(key).or_insert(built)))
}

impl Product for ReserveHistory {
    const KEY: &'static str = "occupation-reserve-history";
    const CONTENT_DEPENDENT: bool = false;
    const INTERACTIVE: bool = true;
    type Params = ReserveHistoryParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, holder, hour: Read];
    type Reply = ReserveHistoryReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ReserveHistoryParams::decl(), ReserveAmount::decl(), ReserveSegment::decl(), SourceTag::decl(), ReserveSource::decl(), ReserveHistoryReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?)?;
        let names = Names::new(&query.holders);
        let branch_last = query.timeline.last;
        let hour = params.hour.min(branch_last);
        let mut next = i32::MAX;
        let mut after = |at: i32| {
            if at > hour && at <= branch_last && at < next {
                next = at;
            }
        };
        let mut segments = Vec::new();
        let mut sources: BTreeMap<u32, Vec<SourceTag>> = BTreeMap::new();
        for stretch in query.timeline.stretches.iter() {
            dashboard_text::metadata::check_cancel(cancelled)?;
            after(stretch.first);
            after(stretch.last.saturating_add(1));
            if stretch.first > hour {
                continue;
            }
            let (first, stretch_last) = (stretch.first, stretch.last);
            let upto = stretch_last.min(hour);
            let held = cached_holder_history(source, stretch.run, first, stretch_last, params.holder)?;
            let made = stretch_segments(&held.entries, &held.admitted, &held.statuses, &held.admission_statuses, first, upto);
            if stretch_last > hour
                && let Some(at) = next_history_change(&held.entries, &held.admitted, &held.statuses, &held.admission_statuses, hour, stretch_last)
            {
                after(at);
            }
            let links = dashboard_base::queries::links(source, stretch.run)?;
            let named: BTreeSet<u32> = made.iter().flat_map(|segment| segment.entries.iter().flatten().map(|entry| entry.source)).collect();
            for raw in named {
                let tags = sources.entry(raw).or_default();
                for (tag, a, b) in names.tags_over(&links, raw, first, stretch_last) {
                    after(a);
                    after(b.saturating_add(1));
                    if a <= hour {
                        tags.push(SourceTag { tag: tag.map(str::to_owned), first_hour: a, last_hour: b.min(hour) });
                    }
                }
            }
            segments.extend(made);
        }
        Ok(ReserveHistoryReply {
            hour,
            next_change: (next != i32::MAX).then_some(next),
            segments,
            sources: sources.into_iter().map(|(id, tags)| ReserveSource { id, tags }).collect(),
            warnings: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
    use replay::campaign::Replay;
    use replay_data::rows::{CountryLinkRow, CountryRow, OccupationSettingsRow, TableRows};
    use replay_frame::frame::TableView;

    fn entry(country: u32, ordinal: u32, source_country: u32, amount: i32) -> CountryOccupationReserveManpowerRow {
        CountryOccupationReserveManpowerRow { country, ordinal, source_country, amount }
    }

    fn named(raw: u32) -> Country {
        Country { id: raw, tag: Some(format!("T{raw}")) }
    }

    #[test]
    fn an_admitted_holder_without_entries_holds_a_recorded_zero_and_each_entry_carries_its_amount_in_list_order() {
        let entries = [entry(2, 1, 9, 300), entry(2, 0, 8, -50), entry(3, 0, 9, 500)];
        let held = holders(&[1, 2], &entries, None, named);
        assert_eq!(held.iter().map(|holder| (holder.country.id, holder.admitted, holder.total)).collect::<Vec<_>>(), vec![(1, true, 0), (2, true, 300), (3, false, 500)]);
        assert_eq!(held[1].entries, vec![ReserveEntry { ordinal: 0, source: named(8), amount: -50 }, ReserveEntry { ordinal: 1, source: named(9), amount: 300 }]);
        assert!(held[0].entries.is_empty());
    }

    #[test]
    fn a_holder_owning_no_state_does_not_exist_and_one_whose_states_are_not_recorded_stays() {
        let owned = HashMap::from([(1, 0), (2, 4)]);
        let held = holders(&[1, 2, 3], &[], Some(&owned), named);
        assert_eq!(held.iter().map(|holder| holder.country.id).collect::<Vec<_>>(), vec![2, 3]);
    }

    fn step(hour: i32, cells: Option<&[i64]>) -> lanes::Step {
        lanes::Step { hour, cells: cells.map(|cells| cells.iter().map(|&value| Read::Number(value)).collect()) }
    }

    fn history(keys: Vec<(u128, Vec<lanes::Step>)>, to: i32) -> lanes::History {
        lanes::History { columns: vec![0, 1], to, texts: Vec::new(), keys, ..Default::default() }
    }

    #[test]
    fn a_holders_history_cuts_where_an_entry_its_admission_or_the_tables_status_changes_and_an_unread_stretch_is_unknown() {
        let entries = history(
            vec![(entry_key(2, 0), vec![step(10, Some(&[9, 100])), step(20, Some(&[9, 150]))]), (entry_key(2, 1), vec![step(15, Some(&[8, 70])), step(25, None)])],
            40,
        );
        let admission = history(vec![(2, vec![step(10, Some(&[1, 0]))])], 40);
        let statuses = [lanes::StatusRun { first: 10, last: 29, status: Status::Complete }, lanes::StatusRun { first: 30, last: 40, status: Status::NotRecorded }];
        let admission_statuses = [lanes::StatusRun { first: 10, last: 40, status: Status::Complete }];
        let segments = stretch_segments(&entries, &admission, &statuses, &admission_statuses, 10, 40);
        let amounts = |segment: &ReserveSegment| segment.entries.as_ref().map(|entries| entries.iter().map(|entry| (entry.source, entry.amount)).collect::<Vec<_>>());
        assert_eq!(segments.iter().map(|segment| (segment.first_hour, segment.last_hour)).collect::<Vec<_>>(), vec![(10, 14), (15, 19), (20, 24), (25, 29), (30, 40)]);
        assert_eq!(amounts(&segments[0]), Some(vec![(9, 100)]));
        assert_eq!(amounts(&segments[1]), Some(vec![(9, 100), (8, 70)]));
        assert_eq!(amounts(&segments[2]), Some(vec![(9, 150), (8, 70)]));
        assert_eq!(amounts(&segments[3]), Some(vec![(9, 150)]));
        assert!(segments.iter().take(4).all(|segment| segment.admitted && segment.unknown.is_none()));
        assert_eq!(segments[4].entries, None);
        assert!(segments[4].unknown.is_some());
    }

    #[test]
    fn unread_occupation_settings_withholds_zero_while_keeping_witnessed_entries() {
        let entries = history(vec![(entry_key(2, 0), vec![step(10, Some(&[9, 100])), step(15, None)])], 20);
        let admission = history(vec![], 20);
        let statuses = [lanes::StatusRun { first: 10, last: 20, status: Status::Complete }];
        let admission_statuses = [lanes::StatusRun { first: 10, last: 20, status: Status::NotRecorded }];
        let segments = stretch_segments(&entries, &admission, &statuses, &admission_statuses, 10, 20);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].entries, Some(vec![ReserveAmount { source: 9, amount: 100 }]));
        assert!(!segments[0].admitted);
        assert_eq!(segments[1].entries, None);
        assert!(segments[1].unknown.as_deref().is_some_and(|reason| reason.contains("Occupation settings")));
    }

    #[test]
    fn a_reply_asked_at_an_hour_holds_no_entry_or_source_first_seen_after_it_and_reports_next_change() {
        let frames: Vec<DecodedFrame> = (10..=20)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let third_tag = if hour >= 18 { b"DDD".to_vec() } else { b"CCC".to_vec() };
                *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
                    CountryRow { id: 9001, canonical_slot: 1, tag: b"AAA".to_vec(), ..CountryRow::default() },
                    CountryRow { id: 9002, canonical_slot: 2, tag: b"BBB".to_vec(), ..CountryRow::default() },
                    CountryRow { id: 9003, canonical_slot: 3, tag: third_tag, ..CountryRow::default() },
                ]));
                *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
                    CountryLinkRow { id: 9001, canonical_slot: 1 },
                    CountryLinkRow { id: 9002, canonical_slot: 2 },
                    CountryLinkRow { id: 9003, canonical_slot: 3 },
                ]));
                *frame.tables[TableId::OccupationSettings.index()] = TableView::Complete(TableRows::OccupationSettings(vec![
                    OccupationSettingsRow { country: 9001, default_law_present: 1, ..Default::default() },
                ]));
                let mut rows = vec![entry(9001, 0, 9002, 100)];
                if hour >= 15 {
                    rows.push(entry(9001, 1, 9003, 250));
                }
                *frame.tables[TableId::CountryOccupationReserveManpower.index()] = TableView::Complete(TableRows::CountryOccupationReserveManpower(rows));
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        let src = Source::from_replay(dashboard_base::SourceKey { source_id: "reserve".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
        let ctx = LoadContext { content: None };

        let at14 = ReserveHistory::load(&src, &ReserveHistoryParams { branch_id: "1".into(), holder: 9001, hour: 14 }, &ctx, &AtomicBool::new(false)).unwrap();
        assert_eq!(at14.hour, 14);
        assert_eq!(at14.next_change, Some(15));
        assert_eq!(at14.segments.len(), 1);
        assert_eq!((at14.segments[0].first_hour, at14.segments[0].last_hour), (10, 14));
        assert_eq!(at14.segments[0].entries, Some(vec![ReserveAmount { source: 9002, amount: 100 }]));
        assert_eq!(at14.sources.iter().map(|s| s.id).collect::<Vec<_>>(), vec![9002], "source 9003 first seen at 15 must be absent at 14");

        let at15 = ReserveHistory::load(&src, &ReserveHistoryParams { branch_id: "1".into(), holder: 9001, hour: 15 }, &ctx, &AtomicBool::new(false)).unwrap();
        assert_eq!(at15.hour, 15);
        assert_eq!(at15.next_change, Some(18));
        assert_eq!(at15.sources.iter().map(|s| s.id).collect::<Vec<_>>(), vec![9002, 9003]);
        assert_eq!(
            at15.sources.iter().find(|s| s.id == 9003).unwrap().tags,
            vec![SourceTag { tag: Some("CCC".into()), first_hour: 10, last_hour: 15 }],
            "future tag DDD at hour 18 must be absent at 15"
        );

        let at20 = ReserveHistory::load(&src, &ReserveHistoryParams { branch_id: "1".into(), holder: 9001, hour: 20 }, &ctx, &AtomicBool::new(false)).unwrap();
        assert_eq!(at20.hour, 20);
        assert_eq!(at20.next_change, None);
    }

    #[test]
    fn a_group_chooses_its_holders_at_the_hour_by_recorded_slot() {
        use dashboard_base::span::HourSpan;
        use dashboard_follow::live::Cells;
        use dashboard_query::{
            identity::{HolderRun, Holders},
            series::ranked::{GroupKind, GroupMembership},
        };
        use dashboard_world::products::groups::{CountryGroupReply, FactionGroup, MembershipRun, UnobservedHours};
        let run = |tag: &str, first_hour, last_hour| MembershipRun { tag: tag.into(), first_hour, last_hour };
        let groups = GroupsAt::new(
            Arc::new(CountryGroupReply {
                branch_id: "1".into(),
                humans: Vec::new(),
                factions: vec![FactionGroup {
                    id: "42".into(),
                    name: None,
                    name_reason: None,
                    names: Vec::new(),
                    recorded_name: String::new(),
                    first_hour: 0,
                    last_hour: 100,
                    members: vec![run("T1", 0, 100), run("T2", 50, 100)],
                    leaders: Vec::new(),
                    incomplete: vec![HourSpan { first_hour: 70, last_hour: 80 }],
                }],
                wars: Vec::new(),
                war_starts: Vec::new(),
                capitulations: Vec::new(),
                identity_changes: Vec::new(),
                country_identities: Vec::new(),
                unobserved: UnobservedHours::default(),
                warnings: Vec::new(),
            }),
            0,
        );
        let held = |slot: u32, id: u32| vec![HolderRun { first: 0, last: 100, holder: Holder::Found { slot, id }, starts_segment: true }];
        let tagged = Holders::new([("T1".to_owned(), held(5, 1)), ("T2".to_owned(), held(6, 2)), ("T3".to_owned(), held(7, 3))].into_iter().collect(), Vec::new());
        let faction = GroupSelection { kind: GroupKind::Faction, id: Some("42".into()), tag: None, membership: GroupMembership::Hour };
        let params = OccupationReserveParams { branch_id: "1".into(), countries: Vec::new(), top: None, group: Some(faction) };
        let built = holders(&[1, 2, 3], &[entry(1, 0, 9, 300), entry(2, 0, 9, 100), entry(3, 0, 9, 900)], None, named);
        let live = Cells::default();
        let cancelled = AtomicBool::new(false);
        let chosen = |hour: i32| {
            let at = At { owner: None, holders: &tagged, hour, context: &groups, cancelled: &cancelled, live: &live };
            select_holders(built.clone(), &params, &at, hour).map(|(rows, _)| rows.iter().map(|holder| (holder.country.id, holder.slot)).collect::<Vec<_>>())
        };
        assert_eq!(chosen(10), Ok(vec![(1, Some(5))]), "T2 joins at 50; T3 is no member");
        assert_eq!(chosen(60), Ok(vec![(1, Some(5)), (2, Some(6))]));
        assert_eq!(chosen(75), Err("Membership of this group is not recorded at this hour".to_owned()), "unknown membership is refused, never guessed");
    }

    #[test]
    fn the_schema_names_the_columns_the_reserve_reads() {
        for (table, field) in [(TableId::CountryOccupationReserveManpower, "source_country"), (TableId::CountryOccupationReserveManpower, "amount"), (TableId::OccupationSettings, "default_law_present")] {
            assert!(dashboard_declare::field(table, field).is_ok(), "{field}");
        }
    }
}

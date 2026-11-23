
use dashboard_base::{Source, products::{LoadContext, Product, at_hour::{At, AtHour}}, queries::country_ids::Names};
use dashboard_intelligence::products::intelligence::{Country, IDENTITY, Tags, branch, missing};
use dashboard_follow::live::{Cells, Source as Cell};
use replay_lanes::{lanes, points::Cell as Read};
use replay_columnar::manifest::Status;
use replay_data::rows::{CountryOccupationReserveManpowerRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ReserveEntry {
    pub ordinal: u32,
    pub source: Country,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ReserveHolder {
    pub country: Country,
    #[doc = " Its occupation status was read: no entry is a recorded zero."]
    pub admitted: bool,
    #[doc = " Its entries in list order."]
    pub entries: Vec<ReserveEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OccupationReserveReply {
    #[doc = " By country id: the race ranks them by their entries' live amounts."]
    pub holders: Vec<ReserveHolder>,
    #[doc = " The reserve's read hour the rows hold; it moves with the reads, so a widget renders nothing"]
    #[doc = " for it."]
    pub read_hour: Option<i32>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(OccupationReserveReply);

pub fn holders(
    admitted: &[u32],
    entries: &[CountryOccupationReserveManpowerRow],
    owned: Option<&HashMap<u32, i32>>,
    country: impl Fn(u32) -> Country,
    live: &Cells,
    amount: usize,
) -> Vec<ReserveHolder> {
    let mut held: BTreeMap<u32, ReserveHolder> = admitted
        .iter()
        .map(|&raw| (raw, ReserveHolder { country: country(raw), admitted: true, entries: Vec::new() }))
        .collect();
    let mut sorted: Vec<&CountryOccupationReserveManpowerRow> = entries.iter().collect();
    sorted.sort_by_key(|entry| (entry.country, entry.ordinal));
    for entry in sorted {
        let holder = held.entry(entry.country).or_insert_with(|| ReserveHolder { country: country(entry.country), admitted: false, entries: Vec::new() });
        holder.entries.push(ReserveEntry { ordinal: entry.ordinal, source: country(entry.source_country) });
    }
    held.into_iter()
        .filter(|(raw, _)| owned.is_none_or(|owned| owned.get(raw).is_none_or(|&states| states > 0)))
        .map(|(raw, holder)| {
            for entry in &holder.entries {
                let key = entry_key(raw, entry.ordinal);
                live.value(&format!("{raw}:{}", entry.ordinal), "amount", Cell { table: TableId::CountryOccupationReserveManpower, column: amount, key }, 1.0);
            }
            holder
        })
        .collect()
}

impl AtHour for OccupationReserve {
    const KEY: &'static str = "occupation-reserve-manpower";
    const TITLE: &'static str = "Occupation reserve";
    const CONTENT_DEPENDENT: bool = false;
    type Params = OccupationReserveParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = OccupationReserveReply;
    type Context = ();

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![OccupationReserveParams::decl(), Country::decl(), ReserveEntry::decl(), ReserveHolder::decl(), OccupationReserveReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }


    const CLOCK_DEPENDENT: bool = false;

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
        Ok(())
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut warnings = Vec::new();
        let read_hour = frame.tables[TableId::CountryOccupationReserveManpower.index()].sampled_at();
        let amount = dashboard_declare::field(TableId::CountryOccupationReserveManpower, "amount")?;
        let made = (|| -> io::Result<Vec<ReserveHolder>> {
            let tags = Tags::read(frame)?;
            let admitted: Vec<u32> = frame.complete::<tables::OccupationSettings>()?.iter().map(|row| row.country).collect();
            let entries = frame.complete::<tables::CountryOccupationReserveManpower>()?;
            let owned: Option<HashMap<u32, i32>> = match frame.complete::<tables::CountryCounterInputs>() {
                Ok(rows) => Some(rows.iter().map(|row| (row.country, row.owned_state_count)).collect()),
                Err(_) => {
                    warnings.extend(missing(frame, TableId::CountryCounterInputs).map(|reason| format!("Which holders exist is not known, so none is left out: {reason}")));
                    None
                }
            };
            Ok(holders(&admitted, entries, owned.as_ref(), |raw| Country::of(&tags, raw), at.live, amount))
        })();
        Ok(match made {
            Ok(holders) => OccupationReserveReply { holders, read_hour, missing: None, warnings },
            Err(error) => OccupationReserveReply { holders: Vec::new(), read_hour, missing: Some(error.to_string()), warnings },
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
    #[doc = " In hour order; hours the timeline does not hold are not in it."]
    pub segments: Vec<ReserveSegment>,
    #[doc = " Every source the segments name, by raw id."]
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

fn stretch_segments(entries: &lanes::History, admission: &lanes::History, statuses: &[lanes::StatusRun], first: i32, last: i32) -> Vec<ReserveSegment> {
    let mut cuts: BTreeSet<i32> = BTreeSet::from([first]);
    cuts.extend(entries.keys.iter().flat_map(|(_, steps)| steps.iter().map(|step| step.hour)));
    cuts.extend(admission.keys.iter().flat_map(|(_, steps)| steps.iter().map(|step| step.hour)));
    cuts.extend(statuses.iter().map(|run| run.first));
    let cuts: Vec<i32> = cuts.into_iter().filter(|&hour| first <= hour && hour <= last).collect();
    let mut places = vec![0usize; entries.keys.len()];
    let mut admitted_at = 0usize;
    let mut out: Vec<ReserveSegment> = Vec::new();
    for (index, &hour) in cuts.iter().enumerate() {
        let end = cuts.get(index + 1).map_or(last, |next| next - 1);
        let status = statuses.iter().find(|run| run.first <= hour && hour <= run.last).map_or(Status::NotRecorded, |run| run.status);
        let admitted = admission.keys.first().is_some_and(|(_, steps)| cells_at(steps, &mut admitted_at, hour).is_some());
        let unknown = status_reason(status);
        let mut held = Vec::new();
        for ((_, steps), place) in entries.keys.iter().zip(places.iter_mut()) {
            if let Some(cells) = cells_at(steps, place, hour)
                && let (Some(source), Some(amount)) = (number(cells[0]), number(cells[1]))
            {
                held.push(ReserveAmount { source: source as u32, amount: amount as i32 });
            }
        }
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

impl Product for ReserveHistory {
    const KEY: &'static str = "occupation-reserve-history";
    const CONTENT_DEPENDENT: bool = false;
    const INTERACTIVE: bool = true;
    type Params = ReserveHistoryParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, holder];
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
        let recording = query.replay.recording();
        let table = TableId::CountryOccupationReserveManpower;
        let columns = [dashboard_declare::field(table, "source_country")?, dashboard_declare::field(table, "amount")?];
        let admission = [dashboard_declare::field(TableId::OccupationSettings, "default_law_present")?];
        let holder = [u128::from(params.holder)];
        let range = lanes::Keys::Range(entry_key(params.holder, 0), entry_key(params.holder, u32::MAX));
        let names = Names::new(&query.holders);
        let mut segments = Vec::new();
        let mut sources: BTreeMap<u32, Vec<SourceTag>> = BTreeMap::new();
        for stretch in query.timeline.stretches.iter() {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let (first, last) = (stretch.first, stretch.last);
            let (entries, admitted, statuses) = dashboard_query::cost::storage(|| {
                io::Result::Ok((
                    lanes::history(recording, stretch.run, table, &columns, range, first, last)?,
                    lanes::history(recording, stretch.run, TableId::OccupationSettings, &admission, lanes::Keys::These(&holder), first, last)?,
                    lanes::statuses(recording, stretch.run, table, first, last)?,
                ))
            })?;
            let made = stretch_segments(&entries, &admitted, &statuses, first, last);
            let links = dashboard_base::queries::links(source, stretch.run)?;
            let named: BTreeSet<u32> = made.iter().flat_map(|segment| segment.entries.iter().flatten().map(|entry| entry.source)).collect();
            for raw in named {
                let tags = sources.entry(raw).or_default();
                for (tag, a, b) in names.tags_over(&links, raw, first, last) {
                    tags.push(SourceTag { tag: tag.map(str::to_owned), first_hour: a, last_hour: b });
                }
            }
            segments.extend(made);
        }
        Ok(ReserveHistoryReply { segments, sources: sources.into_iter().map(|(id, tags)| ReserveSource { id, tags }).collect(), warnings: Vec::new() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(country: u32, ordinal: u32, source_country: u32, amount: i32) -> CountryOccupationReserveManpowerRow {
        CountryOccupationReserveManpowerRow { country, ordinal, source_country, amount }
    }

    fn named(raw: u32) -> Country {
        Country { id: raw, tag: Some(format!("T{raw}")) }
    }

    #[test]
    fn an_admitted_holder_without_entries_holds_a_recorded_zero_and_each_entry_is_a_live_amount_in_list_order() {
        let live = Cells::default();
        let entries = [entry(2, 1, 9, 300), entry(2, 0, 8, -50), entry(3, 0, 9, 500)];
        let held = holders(&[1, 2], &entries, None, named, &live, 1);
        assert_eq!(held.iter().map(|holder| (holder.country.id, holder.admitted)).collect::<Vec<_>>(), vec![(1, true), (2, true), (3, false)]);
        assert_eq!(held[1].entries, vec![ReserveEntry { ordinal: 0, source: named(8) }, ReserveEntry { ordinal: 1, source: named(9) }]);
        assert!(held[0].entries.is_empty());
        let live = live.finish();
        assert_eq!(live.groups.len(), 1);
        assert_eq!(live.groups[0].keys, vec![entry_key(2, 0), entry_key(2, 1), entry_key(3, 0)]);
    }

    #[test]
    fn a_holder_owning_no_state_does_not_exist_and_one_whose_states_are_not_recorded_stays() {
        let owned = HashMap::from([(1, 0), (2, 4)]);
        let held = holders(&[1, 2, 3], &[], Some(&owned), named, &Cells::default(), 1);
        assert_eq!(held.iter().map(|holder| holder.country.id).collect::<Vec<_>>(), vec![2, 3]);
    }

    fn step(hour: i32, cells: Option<&[i64]>) -> lanes::Step {
        lanes::Step { hour, cells: cells.map(|cells| cells.iter().map(|&value| Read::Number(value)).collect()) }
    }

    fn history(keys: Vec<(u128, Vec<lanes::Step>)>, to: i32) -> lanes::History {
        lanes::History { columns: vec![0, 1], to, texts: Vec::new(), keys }
    }

    #[test]
    fn a_holders_history_cuts_where_an_entry_its_admission_or_the_tables_status_changes_and_an_unread_stretch_is_unknown() {
        let entries = history(
            vec![(entry_key(2, 0), vec![step(10, Some(&[9, 100])), step(20, Some(&[9, 150]))]), (entry_key(2, 1), vec![step(15, Some(&[8, 70])), step(25, None)])],
            40,
        );
        let admission = history(vec![(2, vec![step(10, Some(&[1, 0]))])], 40);
        let statuses = [lanes::StatusRun { first: 10, last: 29, status: Status::Complete }, lanes::StatusRun { first: 30, last: 40, status: Status::NotRecorded }];
        let segments = stretch_segments(&entries, &admission, &statuses, 10, 40);
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
    fn the_schema_names_the_columns_the_reserve_reads() {
        for (table, field) in [(TableId::CountryOccupationReserveManpower, "source_country"), (TableId::CountryOccupationReserveManpower, "amount"), (TableId::OccupationSettings, "default_law_present")] {
            assert!(dashboard_declare::field(table, field).is_ok(), "{field}");
        }
    }
}

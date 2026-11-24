
use dashboard_base::products::{Country, LoadContext, Product, validate_country};
use dashboard_base::{Source, products::at_hour::{At, AtHour, Clock, Owner, Read, ReadsAt}};
use dashboard_world::products::{group_members::GroupsAt, scalars::factory_channel_value, territory::{slot_tags, territory_value}};
use dashboard_declare::{Measure, Series};
use dashboard_query::{
    chart::{ChartBucket, ChartSummary},
    coverage::{Witness, witnessed},
    identity::Holder,
    series::{Ask, Line, MissingSpan, grid, ranked::GroupSelection},
};
use replay_data::rows::{CountryFactoriesRow, GroupSpanRow as Span, TableId, TerritoryTotalRow, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const KINDS: [&str; 2] = ["territory", "country-factories"];

fn measure_of(kind: &str, measure: &str) -> Result<(&'static Series, &'static Measure), &'static str> {
    if !KINDS.contains(&kind) {
        return Err("Faction totals sum territory or factory counts");
    }
    let series = dashboard_declare::series(kind).ok_or("Undeclared faction series")?;
    Ok((series, series.measure(measure).ok_or("Unknown faction measure")?))
}

pub struct FactionRosters;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactionRostersParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionRoster {
    #[doc = " The faction's recorded id."]
    pub id: String,
    #[doc = " Its members at the hour, by tag, ascending."]
    pub members: Vec<String>,
    #[doc = " Why its roster is not known whole at the hour: its total is not known then."]
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionRostersReply {
    pub factions: Vec<FactionRoster>,
    #[doc = " Why no roster is known at this hour."]
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionLine {
    pub id: String,
    #[doc = " Its bins: first and last the sums of its members' first and last values; least and greatest the sums of"]
    #[doc = " theirs, so bounds of the faction's own wherever `bounds` says so."]
    pub summary: ChartSummary,
    pub missing: Vec<MissingSpan>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionLinesReply {
    pub branch_id: String,
    pub measure: String,
    pub decimals: u8,
    #[doc = " A bin holds more than one hour: its least and greatest are bounds, not the sum's own extremes."]
    pub bounds: bool,
    pub lines: Vec<FactionLine>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(FactionRostersReply, FactionLinesReply);

struct Rosters {
    members: BTreeMap<u64, BTreeSet<u32>>,
    incomplete: BTreeSet<u64>,
    unread: bool,
}

fn rosters(frame: &DecodedFrame) -> Result<Rosters, String> {
    let spans = frame.complete::<tables::GroupSpans>().map_err(|error| error.to_string())?;
    let groups = frame.complete::<tables::Groups>().map_err(|error| error.to_string())?;
    let numbers: HashMap<u32, u64> = groups.iter().map(|group| (group.number, group.faction)).collect();
    let mut out = Rosters { members: BTreeMap::new(), incomplete: BTreeSet::new(), unread: false };
    for span in spans {
        match span.kind {
            Span::UNOBSERVED if span.group == Span::FACTIONS => out.unread = true,
            Span::MEMBER if span.group == Span::HUMANS => {}
            Span::MEMBER | Span::PRESENT | Span::INCOMPLETE => {
                let Some(&id) = numbers.get(&span.group) else { continue };
                let members = out.members.entry(id).or_default();
                match span.kind {
                    Span::MEMBER => {
                        members.insert(span.value);
                    }
                    Span::INCOMPLETE => {
                        out.incomplete.insert(id);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

pub fn rosters_at(frame: &DecodedFrame, tags: &HashMap<u32, (&str, u32)>) -> FactionRostersReply {
    let empty = |missing: String| FactionRostersReply { factions: Vec::new(), missing: Some(missing), warnings: Vec::new() };
    let rosters = match rosters(frame) {
        Ok(rosters) if rosters.unread => return empty("Faction memberships were not read at this hour".to_owned()),
        Ok(rosters) => rosters,
        Err(reason) => return empty(format!("Faction memberships are not known at this hour: {reason}")),
    };
    let factions = rosters
        .members
        .iter()
        .map(|(id, slots)| {
            let mut missing = rosters.incomplete.contains(id).then(|| "Its roster was not read whole at this hour".to_owned());
            let mut members: Vec<String> = Vec::with_capacity(slots.len());
            for slot in slots {
                match tags.get(slot) {
                    Some(&(tag, _)) => members.push(tag.to_owned()),
                    None if missing.is_none() => missing = Some("A member's country is not known at this hour".to_owned()),
                    None => {}
                }
            }
            members.sort_unstable();
            FactionRoster { id: id.to_string(), members, missing }
        })
        .collect();
    FactionRostersReply { factions, missing: None, warnings: Vec::new() }
}

impl AtHour for FactionRosters {
    const KEY: &'static str = "faction-rosters";
    const TITLE: &'static str = "Faction rosters";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;
    type Params = FactionRostersParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = FactionRostersReply;
    type Context = ();

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FactionRostersParams::decl(), FactionRoster::decl(), FactionRostersReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::GroupSpans, TableId::Groups]
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<()> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, ()>) -> io::Result<Self::Reply> {
        Ok(rosters_at(frame, &slot_tags(at.holders, frame.hour)))
    }
}

pub struct FactionLines;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactionLinesParams {
    pub branch_id: String,
    pub kind: String,
    pub measure: String,
    pub from: i32,
    pub to: i32,
    pub width: u32,
    #[doc = " Whose membership a faction sums: `hour` (each member over its own membership; the default) or `ever`"]
    #[doc = " (anyone a member within the view, over the whole view)."]
    #[serde(default)]
    pub membership: Option<String>,
    #[doc = " A group's members summed as one line, `group`, instead of the factions: each by tag over its spans (the"]
    #[doc = " whole view where none), the hours their membership was not read in `unread`."]
    #[serde(default)]
    pub members: Option<Vec<GroupMember>>,
    #[serde(default)]
    pub unread: Option<Vec<(i32, i32)>>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroupMember {
    pub tag: String,
    #[serde(default)]
    pub spans: Option<Vec<(i32, i32)>>,
}

pub const GROUP_LINE: &str = "group";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Membership {
    Hour,
    Ever,
}

impl Membership {
    fn of(text: Option<&str>) -> Result<Self, &'static str> {
        match text {
            None | Some("hour") => Ok(Self::Hour),
            Some("ever") => Ok(Self::Ever),
            Some(_) => Err("A faction's membership is hour or ever"),
        }
    }
}

fn roster_spans(runs: Vec<(String, i32, i32)>, membership: Membership, from: i32, to: i32) -> Vec<(String, Vec<(i32, i32)>)> {
    let mut by_tag: BTreeMap<String, Vec<(i32, i32)>> = BTreeMap::new();
    for (tag, first, last) in runs {
        by_tag.entry(tag).or_default().push((first, last));
    }
    by_tag
        .into_iter()
        .filter_map(|(tag, spans)| {
            let counted = match membership {
                Membership::Hour => merged(spans, from, to),
                Membership::Ever => spans.iter().any(|&(first, last)| first <= to && from <= last).then(|| vec![(from, to)]).unwrap_or_default(),
            };
            (!counted.is_empty()).then_some((tag, counted))
        })
        .collect()
}

fn merged(mut spans: Vec<(i32, i32)>, from: i32, to: i32) -> Vec<(i32, i32)> {
    spans.retain_mut(|span| {
        *span = (span.0.max(from), span.1.min(to));
        span.0 <= span.1
    });
    spans.sort_unstable();
    let mut out: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
    for span in spans {
        match out.last_mut() {
            Some(held) if i64::from(held.1) + 1 >= i64::from(span.0) => held.1 = held.1.max(span.1),
            _ => out.push(span),
        }
    }
    out
}

fn within(missing: &[MissingSpan], spans: &[(i32, i32)]) -> Vec<MissingSpan> {
    let mut out = Vec::new();
    for gap in missing {
        for &(first, last) in spans {
            let (a, b) = (gap.first_hour.max(first), gap.last_hour.min(last));
            if a <= b {
                out.push(MissingSpan { first_hour: a, last_hour: b, reason: gap.reason.clone() });
            }
        }
    }
    out
}

pub fn summed(charts: &[ChartSummary], missing: &[MissingSpan], from: i32, to: i32, width: u32) -> ChartSummary {
    let hours = i64::from(to) - i64::from(from) + 1;
    let (first_bin, bin_hours) = if hours > i64::from(width) {
        let (_, bin_hours) = grid(hours, width);
        (i64::from(from).div_euclid(bin_hours) * bin_hours, bin_hours)
    } else {
        (i64::from(from), (hours + i64::from(width) - 1) / i64::from(width))
    };
    let bins = charts.iter().map(|chart| chart.buckets.len()).max().unwrap_or(0);
    let mut buckets: Vec<Option<ChartBucket>> = vec![None; bins];
    for chart in charts {
        for (bin, bucket) in chart.buckets.iter().enumerate() {
            let Some(bucket) = bucket else { continue };
            buckets[bin] = Some(match buckets[bin].take() {
                None => *bucket,
                Some(held) => ChartBucket {
                    absolute_total: 0,
                    first_hour: held.first_hour.min(bucket.first_hour),
                    last_hour: held.last_hour.max(bucket.last_hour),
                    first_value: held.first_value + bucket.first_value,
                    last_value: held.last_value + bucket.last_value,
                    last_value_hour: held.last_value_hour.max(bucket.last_value_hour),
                    min_hour: held.min_hour.min(bucket.min_hour),
                    min_value: held.min_value + bucket.min_value,
                    max_hour: held.max_hour.max(bucket.max_hour),
                    max_value: held.max_value + bucket.max_value,
                    first_break: held.first_break && bucket.first_break,
                    has_break: held.has_break || bucket.has_break,
                },
            });
        }
    }
    for (bin, bucket) in buckets.iter_mut().enumerate() {
        let start = first_bin + bin as i64 * bin_hours;
        let end = start + bin_hours - 1;
        if missing.iter().any(|gap| i64::from(gap.first_hour) <= end && start <= i64::from(gap.last_hour)) {
            *bucket = None;
        }
    }
    for bin in 0..buckets.len() {
        let joined = bin > 0 && buckets[bin - 1].is_some();
        if let Some(bucket) = &mut buckets[bin] {
            bucket.first_break = !joined;
        }
    }
    ChartSummary { from: f64::from(from), to: f64::from(to), width, buckets, runs: None }
}

fn ids_of(members: &BTreeMap<u64, Vec<(String, i32, i32)>>, from: i32, to: i32) -> Vec<String> {
    members.iter().filter(|(_, runs)| runs.iter().any(|&(_, first, last)| first <= to && from <= last)).map(|(id, _)| id.to_string()).collect()
}

fn summed_line(line: &Line<'_, '_>, id: String, drawn: &[(Ask, Vec<(i32, i32)>)], unread: &[MissingSpan], from: i32, to: i32, width: u32) -> io::Result<Option<FactionLine>> {
    if drawn.is_empty() {
        return Ok(None);
    }
    let charts = line.summaries_within(drawn, from, to, width)?;
    let mut missing = unread.to_vec();
    for ((_, spans), (_, gaps)) in drawn.iter().zip(&charts) {
        missing.extend(within(gaps, spans));
    }
    missing.sort_unstable_by_key(|gap| (gap.first_hour, gap.last_hour));
    let summaries: Vec<ChartSummary> = charts.into_iter().map(|(summary, _)| summary).collect();
    Ok(Some(FactionLine { id, summary: summed(&summaries, &missing, from, to, width), missing }))
}

fn group_drawn(members: &[GroupMember], from: i32, to: i32) -> Vec<(String, Vec<(i32, i32)>)> {
    members
        .iter()
        .filter_map(|member| {
            let spans = merged(member.spans.clone().unwrap_or_else(|| vec![(from, to)]), from, to);
            (!spans.is_empty()).then(|| (member.tag.clone(), spans))
        })
        .collect()
}

fn lines(source: &Source, params: &FactionLinesParams, cancelled: &AtomicBool) -> io::Result<FactionLinesReply> {
    let (series, measure) = measure_of(&params.kind, &params.measure).map_err(dashboard_core::invalid)?;
    if params.from > params.to || !(1..=8192).contains(&params.width) {
        return Err(dashboard_core::invalid("Invalid faction view"));
    }
    let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let query = dashboard_base::queries::query(source, branch)?;
    let line = Line::new(&query, series, measure)?;
    let (from, to, width) = (params.from, params.to, params.width);
    let mut out = Vec::new();
    let membership = Membership::of(params.membership.as_deref()).map_err(dashboard_core::invalid)?;
    let exact = (params.members.is_none() && membership == Membership::Hour).then(|| dashboard_declare::faction_measure(&params.kind, &params.measure)).flatten();
    if let Some(name) = exact {
        let totals = dashboard_declare::series("faction-totals").ok_or_else(|| dashboard_core::invalid("Faction totals are not declared"))?;
        let line = Line::new(&query, totals, totals.measure(name).ok_or_else(|| dashboard_core::invalid("Unknown faction measure"))?)?;
        let (members, _) = dashboard_world::products::groups::faction_members(source, &query)?;
        let ids = ids_of(&members, from, to);
        dashboard_text::metadata::check_cancel(cancelled)?;
        let charts = line.summaries(&ids, from, to, width)?;
        for (id, (summary, missing)) in ids.into_iter().zip(charts) {
            out.push(FactionLine { id, summary, missing });
        }
    } else if let Some(members) = &params.members {
        let unread: Vec<MissingSpan> = merged(params.unread.clone().unwrap_or_default(), from, to)
            .into_iter()
            .map(|(first_hour, last_hour)| MissingSpan { first_hour, last_hour, reason: "The group's members were not read whole here".to_owned() })
            .collect();
        let drawn: Vec<(Ask, Vec<(i32, i32)>)> = group_drawn(members, from, to).into_iter().map(|(tag, spans)| (Ask::whole(&tag), spans)).collect();
        out.extend(summed_line(&line, GROUP_LINE.to_owned(), &drawn, &unread, from, to, width)?);
    } else {
        let (members, unknown) = dashboard_world::products::groups::faction_members(source, &query)?;
        let unread_spans = merged(unknown.iter().map(|span| (span.first_hour, span.last_hour)).collect(), from, to);
        let unread_spans = match membership {
            Membership::Hour => unread_spans,
            Membership::Ever if !unread_spans.is_empty() => vec![(from, to)],
            Membership::Ever => Vec::new(),
        };
        let unread: Vec<MissingSpan> = unread_spans
            .into_iter()
            .map(|(first_hour, last_hour)| MissingSpan { first_hour, last_hour, reason: "Faction memberships were not read whole here".to_owned() })
            .collect();
        for (id, runs) in members {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let drawn: Vec<(Ask, Vec<(i32, i32)>)> =
                roster_spans(runs, membership, from, to).into_iter().map(|(tag, spans)| (Ask::whole(&tag), spans)).collect();
            out.extend(summed_line(&line, id.to_string(), &drawn, &unread, from, to, width)?);
        }
    }
    let hours = i64::from(to) - i64::from(from) + 1;
    Ok(FactionLinesReply {
        branch_id: params.branch_id.clone(),
        measure: measure.name.to_owned(),
        decimals: measure.decimals,
        bounds: exact.is_none() && hours > i64::from(width),
        lines: out,
        warnings: Vec::new(),
    })
}

impl Product for FactionLines {
    const KEY: &'static str = "faction-lines";
    const CONTENT_DEPENDENT: bool = false;
    type Params = FactionLinesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, kind, measure, from: Start, to: End, width, membership, "members[].spans[].0": Start, "members[].spans[].1": End, "unread[].0": Start, "unread[].1": End];
    type Reply = FactionLinesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FactionLinesParams::decl(), GroupMember::decl(), FactionLine::decl(), FactionLinesReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        Membership::of(params.membership.as_deref())?;
        measure_of(&params.kind, &params.measure).map(|_| ())
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        lines(source, params, cancelled)
    }
}

pub struct FactionTotalsAt;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactionTotalsAtParams {
    pub branch_id: String,
    pub kind: String,
    pub measure: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub members: Option<Vec<Country>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionTotalRow {
    pub id: String,
    pub members: Vec<String>,
    pub member_countries: Vec<Country>,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number | null"))]
    pub total: Option<i64>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionTotalsAtReply {
    pub hour: i32,
    pub kind: String,
    pub measure: String,
    pub rows: Vec<FactionTotalRow>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(FactionTotalsAtReply);

enum MeasureTable<'f> {
    Territory(HashMap<u32, &'f TerritoryTotalRow>),
    Factories(HashMap<u32, &'f CountryFactoriesRow>),
}

impl<'f> MeasureTable<'f> {
    fn read(frame: &'f DecodedFrame, kind: &str) -> Result<Self, String> {
        if kind == "territory" {
            let rows = frame.complete::<tables::TerritoryTotals>().map_err(|error| format!("Territory totals are not known at this hour: {error}"))?;
            Ok(Self::Territory(rows.iter().map(|row| (row.slot, row)).collect()))
        } else {
            let rows = frame.complete::<tables::CountryFactories>().map_err(|error| format!("Factories are not known at this hour: {error}"))?;
            Ok(Self::Factories(rows.iter().map(|row| (row.country, row)).collect()))
        }
    }

    fn value(&self, slot: u32, id: u32, measure: &str) -> Option<i64> {
        match self {
            Self::Territory(rows) => rows.get(&slot).and_then(|row| territory_value(row, measure)),
            Self::Factories(rows) => rows.get(&id).and_then(|row| factory_channel_value(row, measure)),
        }
    }
}

fn witnessed_value(frame: &DecodedFrame, kind: &str, slot: u32, id: u32, measure: &str, tag: &str) -> Result<i64, String> {
    if kind == "territory" {
        match witnessed::<tables::TerritoryTotals>(frame, u128::from(slot)) {
            Witness::Row(row) => territory_value(row, measure).ok_or_else(|| format!("A member's value ({tag}) is not recorded at this hour")),
            Witness::Absent => Err(format!("A member's value ({tag}) is not recorded at this hour")),
            Witness::NotRecorded(reason) => Err(reason),
        }
    } else {
        match witnessed::<tables::CountryFactories>(frame, u128::from(id)) {
            Witness::Row(row) => factory_channel_value(row, measure).ok_or_else(|| format!("A member's value ({tag}) is not recorded at this hour")),
            Witness::Absent => Err(format!("A member's value ({tag}) is not recorded at this hour")),
            Witness::NotRecorded(reason) => Err(reason),
        }
    }
}

impl AtHour for FactionTotalsAt {
    const KEY: &'static str = "faction-totals-at";
    const TITLE: &'static str = "Faction totals";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: Clock = Clock::Boundaries;
    type Params = FactionTotalsAtParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, kind, measure, top, members, group, "members[]"];
    type Reply = FactionTotalsAtReply;
    type Context = GroupsAt;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FactionTotalsAtParams::decl(), FactionTotalRow::decl(), FactionTotalsAtReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")?;
        measure_of(&params.kind, &params.measure)?;
        if params.top == Some(0) {
            return Err("top must be at least 1");
        }
        if params.group.is_some() && params.members.is_some() {
            return Err("Choose a group or explicit members, not both");
        }
        if params.top.is_some() && (params.group.is_some() || params.members.is_some()) {
            return Err("top applies to factions, not an aggregated group");
        }
        if let Some(members) = &params.members {
            for country in members {
                validate_country(country)?;
            }
        }
        Ok(())
    }

    fn tables(params: &Self::Params) -> Vec<TableId> {
        let target = if params.kind == "territory" { TableId::TerritoryTotals } else { TableId::CountryFactories };
        if params.members.is_some() || params.group.is_some() {
            vec![target]
        } else {
            vec![TableId::GroupSpans, TableId::Groups, target]
        }
    }

    fn retained_reads(params: &Self::Params, _owner: Option<Owner>, stage: usize, _held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> (Vec<Read>, i32) {
        if stage > 0 {
            return (Vec::new(), i32::MAX);
        }
        let target = if params.kind == "territory" { TableId::TerritoryTotals } else { TableId::CountryFactories };
        let key_of = |slot: u32, id: u32| if params.kind == "territory" { u128::from(slot) } else { u128::from(id) };
        let valid = at.context.valid_through(params.group.as_ref(), at.hour);
        if let Some(group) = &params.group {
            let Ok(members) = at.context.members(group, at.hour) else {
                return (Vec::new(), valid);
            };
            if members.refused.is_some() || members.tags.is_empty() {
                return (Vec::new(), valid);
            }
            let keys: Vec<u128> = members
                .tags
                .iter()
                .filter_map(|tag| match at.holders.at(&Country::Tag(tag.clone()), at.hour).map(|run| run.holder) {
                    Some(Holder::Found { slot, id }) => Some(key_of(slot, id)),
                    _ => None,
                })
                .collect();
            return (vec![Read::these(target, keys)], valid);
        }
        if let Some(members) = &params.members {
            let keys: Vec<u128> = members
                .iter()
                .filter_map(|country| match at.holders.at(country, at.hour).map(|run| run.holder) {
                    Some(Holder::Found { slot, id }) => Some(key_of(slot, id)),
                    _ => None,
                })
                .collect();
            return (vec![Read::these(target, keys)], i32::MAX);
        }
        (vec![Read::all(TableId::GroupSpans), Read::all(TableId::Groups), Read::all(target)], i32::MAX)
    }

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        at.context.valid_through(params.group.as_ref(), at.hour)
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(GroupsAt::default())
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        GroupsAt::load(source, load, &params.branch_id, params.group.as_ref())
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let empty = |missing: String| FactionTotalsAtReply {
            hour: frame.hour,
            kind: params.kind.clone(),
            measure: params.measure.clone(),
            rows: Vec::new(),
            missing: Some(missing),
            warnings: Vec::new(),
        };
        let aggregate_members: Option<Vec<Country>> = if let Some(group) = &params.group {
            let members = at.context.members(group, frame.hour)?;
            if let Some(refused) = members.refused {
                return Ok(empty(refused));
            }
            if members.tags.is_empty() {
                return Ok(FactionTotalsAtReply {
                    hour: frame.hour,
                    kind: params.kind.clone(),
                    measure: params.measure.clone(),
                    rows: Vec::new(),
                    missing: None,
                    warnings: Vec::new(),
                });
            }
            Some(members.tags.into_iter().map(Country::Tag).collect())
        } else {
            params.members.clone()
        };

        if let Some(members) = aggregate_members {
            let mut sum = 0i64;
            let mut missing: Option<String> = None;
            let mut tags = Vec::with_capacity(members.len());
            let mut member_countries = Vec::with_capacity(members.len());
            for country in &members {
                let tag = at.holders.name(country, frame.hour);
                tags.push(tag.clone());
                match at.holders.at(country, frame.hour).map(|run| run.holder) {
                    Some(Holder::Found { slot, id }) => {
                        member_countries.push(Country::Slot(slot));
                        match witnessed_value(frame, &params.kind, slot, id, &params.measure, &tag) {
                            Ok(value) => match sum.checked_add(value) {
                                Some(next) => sum = next,
                                None if missing.is_none() => missing = Some("Group total overflowed at this hour".to_owned()),
                                None => {}
                            },
                            Err(reason) if missing.is_none() => missing = Some(reason),
                            Err(_) => {}
                        }
                    }
                    _ => {
                        member_countries.push(country.clone());
                        if missing.is_none() {
                            missing = Some(format!("A member ({tag}) is not present at this hour"));
                        }
                    }
                }
            }
            tags.sort_unstable();
            tags.dedup();
            member_countries.sort_unstable();
            member_countries.dedup();
            let row = FactionTotalRow {
                id: GROUP_LINE.to_owned(),
                members: tags,
                member_countries,
                total: missing.is_none().then_some(sum),
                missing,
            };
            return Ok(FactionTotalsAtReply {
                hour: frame.hour,
                kind: params.kind.clone(),
                measure: params.measure.clone(),
                rows: vec![row],
                missing: None,
                warnings: Vec::new(),
            });
        }

        let table = match MeasureTable::read(frame, &params.kind) {
            Ok(table) => table,
            Err(reason) => return Ok(empty(reason)),
        };
        let rosters = match rosters(frame) {
            Ok(rosters) if rosters.unread => return Ok(empty("Faction memberships were not read at this hour".to_owned())),
            Ok(rosters) => rosters,
            Err(reason) => return Ok(empty(format!("Faction memberships are not known at this hour: {reason}"))),
        };
        let tags = slot_tags(at.holders, frame.hour);
        let mut rows_out: Vec<FactionTotalRow> = rosters
            .members
            .iter()
            .filter(|(_, slots)| !slots.is_empty())
            .map(|(faction_id, slots)| {
                let mut missing = rosters.incomplete.contains(faction_id).then(|| "Its roster was not read whole at this hour".to_owned());
                let mut sum = 0i64;
                let mut member_tags = Vec::with_capacity(slots.len());
                let mut member_countries = Vec::with_capacity(slots.len());
                for &slot in slots {
                    member_countries.push(Country::Slot(slot));
                    match tags.get(&slot) {
                        Some(&(tag, country_id)) => {
                            member_tags.push(tag.to_owned());
                            if missing.is_none() {
                                match table.value(slot, country_id, &params.measure) {
                                    Some(value) => match sum.checked_add(value) {
                                        Some(next) => sum = next,
                                        None => missing = Some("Faction total overflowed at this hour".to_owned()),
                                    },
                                    None => missing = Some(format!("A member's value ({tag}) is not recorded at this hour")),
                                }
                            }
                        }
                        None if missing.is_none() => {
                            missing = Some("A member's country is not known at this hour".to_owned());
                        }
                        None => {}
                    }
                }
                member_tags.sort_unstable();
                member_countries.sort_unstable();
                member_countries.dedup();
                FactionTotalRow {
                    id: faction_id.to_string(),
                    members: member_tags,
                    member_countries,
                    total: missing.is_none().then_some(sum),
                    missing,
                }
            })
            .collect();
        rows_out.sort_unstable_by(|a, b| match (a.total, b.total) {
            (Some(av), Some(bv)) => bv.cmp(&av).then_with(|| a.id.cmp(&b.id)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.id.cmp(&b.id),
        });
        if let Some(limit) = params.top {
            let limit = usize::try_from(limit).unwrap_or(usize::MAX);
            if rows_out.iter().any(|row| row.missing.is_some()) {
                return Ok(empty(format!(
                    "The top {limit} factions cannot be proved: a faction's total is not known at this hour"
                )));
            }
            rows_out.truncate(limit);
        }
        Ok(FactionTotalsAtReply {
            hour: frame.hour,
            kind: params.kind.clone(),
            measure: params.measure.clone(),
            rows: rows_out,
            missing: None,
            warnings: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bucket(first_hour: i32, last_hour: i32, first: f64, last: f64, least: f64, most: f64) -> Option<ChartBucket> {
        Some(ChartBucket {
            absolute_total: 0,
            first_hour,
            last_hour,
            first_value: first,
            last_value: last,
            last_value_hour: last_hour,
            min_hour: first_hour,
            min_value: least,
            max_hour: last_hour,
            max_value: most,
            first_break: false,
            has_break: false,
        })
    }

    fn chart(buckets: Vec<Option<ChartBucket>>) -> ChartSummary {
        ChartSummary { from: 0.0, to: 23.0, width: 4, buckets, runs: None }
    }

    #[test]
    fn a_factions_bins_sum_its_members_and_a_member_not_known_leaves_the_bin_unknown() {
        let one = chart(vec![bucket(0, 5, 1.0, 2.0, 1.0, 2.0), bucket(6, 11, 2.0, 2.0, 2.0, 2.0), None, bucket(18, 23, 4.0, 4.0, 4.0, 4.0)]);
        let two = chart(vec![bucket(0, 5, 10.0, 10.0, 10.0, 10.0), None, bucket(12, 17, 5.0, 6.0, 5.0, 6.0), bucket(18, 23, 1.0, 0.0, 0.0, 1.0)]);
        let gap = MissingSpan { first_hour: 20, last_hour: 21, reason: "not recorded".into() };
        let sum = summed(&[one, two], &[gap], 0, 23, 4);
        let first = sum.buckets[0].unwrap();
        assert_eq!((first.first_value, first.last_value, first.min_value, first.max_value), (11.0, 12.0, 11.0, 12.0));
        assert!(first.first_break, "a line starts at its first bin");
        assert_eq!(sum.buckets[1].unwrap().last_value, 2.0, "a member outside its membership adds nothing");
        assert!(!sum.buckets[2].unwrap().first_break, "the line goes on from the bin before");
        assert_eq!(sum.buckets[3], None, "a member's unknown hours leave the faction's bin unknown, never smaller");
    }

    #[test]
    fn spans_are_cut_to_the_view_and_merged_where_they_meet() {
        assert_eq!(merged(vec![(5, 9), (-3, 2), (3, 4), (20, 30)], 0, 25), vec![(0, 9), (20, 25)]);
        let gaps = [MissingSpan { first_hour: 0, last_hour: 10, reason: "why".into() }];
        let cut = within(&gaps, &[(3, 4), (8, 12)]);
        assert_eq!(cut.iter().map(|gap| (gap.first_hour, gap.last_hour)).collect::<Vec<_>>(), [(3, 4), (8, 10)]);
    }

    #[test]
    fn a_roster_of_anyone_ever_a_member_sums_its_members_over_the_whole_view_and_members_at_the_end_are_not_a_choice() {
        let runs = || vec![("GER".to_owned(), 0, 99), ("ITA".to_owned(), 20, 40), ("ITA".to_owned(), 60, 200), ("HUN".to_owned(), 150, 300), ("JAP".to_owned(), 0, 5)];
        let shown = |membership| roster_spans(runs(), membership, 10, 100);
        assert_eq!(shown(Membership::Hour), [("GER".to_owned(), vec![(10, 99)]), ("ITA".to_owned(), vec![(20, 40), (60, 100)])]);
        assert_eq!(shown(Membership::Ever), [("GER".to_owned(), vec![(10, 100)]), ("ITA".to_owned(), vec![(10, 100)])]);
        assert_eq!(Membership::of(Some("later")), Err("A faction's membership is hour or ever"));
        assert_eq!(Membership::of(Some("end")), Err("A faction's membership is hour or ever"), "members at the view's end are read after its hours");
        assert_eq!(Membership::of(None), Ok(Membership::Hour));
    }

    #[test]
    fn a_groups_members_are_summed_over_their_own_spans_cut_to_the_view_or_all_of_it() {
        let member = |tag: &str, spans: Option<Vec<(i32, i32)>>| GroupMember { tag: tag.to_owned(), spans };
        let members = [member("GER", None), member("ITA", Some(vec![(60, 200), (20, 40), (35, 50)])), member("JAP", Some(vec![(0, 5)]))];
        assert_eq!(
            group_drawn(&members, 10, 100),
            [("GER".to_owned(), vec![(10, 100)]), ("ITA".to_owned(), vec![(20, 50), (60, 100)])],
            "a member outside the view adds nothing; overlapping spans count once"
        );
    }

    #[test]
    fn truncated_default_faction_lines_leave_both_factions_missing_on_unstored_measure() {
        use dashboard_base::test_fixtures::{CAMPAIGN, records};
        use replay::campaign::Replay;
        use replay_data::rows::{CampaignRow, CountryFactoriesRow, CountryLinkRow, CountryRow, FactionMemberRow, FactionRow, Recorded, TableRows};
        use replay_frame::frame::{Metrics, TableView};
        use std::sync::Arc;

        let countries = vec![
            CountryRow { border_land_doctrine_level: Recorded::Value(None), id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..CountryRow::default() },
            CountryRow { border_land_doctrine_level: Recorded::Value(None), id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), ..CountryRow::default() },
        ];
        let links = vec![
            CountryLinkRow { id: 9001, canonical_slot: 7 },
            CountryLinkRow { id: 9002, canonical_slot: 8 },
        ];
        let factions = vec![
            FactionRow { id: 100, name: b"Axis".to_vec(), ..FactionRow::default() },
            FactionRow { id: 200, name: b"Allies".to_vec(), ..FactionRow::default() },
        ];
        let members = vec![
            FactionMemberRow { country: 9001, faction: 100, ordinal: 0 },
            FactionMemberRow { country: 9002, faction: 200, ordinal: 0 },
        ];
        let fac_ger = CountryFactoriesRow { country: 9001, military_damaged_raw: 1_000_000, ..CountryFactoriesRow::default() };
        let fac_ita = CountryFactoriesRow { country: 9002, military_damaged_raw: 2_000_000, ..CountryFactoriesRow::default() };
        let frames: Vec<DecodedFrame> = (120..=130)
            .map(|hour| {
                let mut tables = std::array::from_fn(|_| TableView::Unavailable { reason: "outside fixture".into() });
                tables[TableId::Campaign.index()] = TableView::Complete(TableRows::Campaign(vec![CampaignRow { id: CAMPAIGN.into() }]));
                tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(countries.clone()));
                tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links.clone()));
                tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(factions.clone()));
                tables[TableId::FactionMembers.index()] = TableView::Complete(TableRows::FactionMembers(members.clone()));
                tables[TableId::CountryFactories.index()] = if hour == 128 {
                    TableView::Truncated { rows: TableRows::CountryFactories(vec![fac_ger.clone()]), missing: 1 }
                } else {
                    TableView::Complete(TableRows::CountryFactories(vec![fac_ger.clone(), fac_ita.clone()]))
                };
                DecodedFrame { branch: 1, hour, tables: tables.map(Into::into), metrics: Metrics::default() }
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        let source = Source::from_replay(
            dashboard_base::SourceKey { source_id: "factions-trunc".into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let reply = FactionLines::load(
            &source,
            &FactionLinesParams {
                branch_id: "1".into(),
                kind: "country-factories".into(),
                measure: "damaged7".into(),
                from: 120,
                to: 130,
                width: 11,
                membership: None,
                members: None,
                unread: None,
            },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(reply.lines.len(), 2, "both factions present");
        let expected_gap = MissingSpan {
            first_hour: 128,
            last_hour: 128,
            reason: "country factories: read only in part at this hour (1 rows not read)".into(),
        };
        for line in &reply.lines {
            assert_eq!(line.missing, vec![expected_gap.clone()], "faction {} keeps table-wide truncation gap", line.id);
            assert_eq!(line.summary.buckets[8], None, "faction {} leaves truncated hour 128 empty rather than one-sided", line.id);
        }
    }
}

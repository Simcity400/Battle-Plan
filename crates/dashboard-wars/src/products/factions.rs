
use dashboard_base::products::{LoadContext, Product};
use dashboard_base::{Source, products::at_hour::{At, AtHour}};
use dashboard_world::products::territory::slot_tags;
use dashboard_declare::{Measure, Series};
use dashboard_query::{
    chart::{ChartBucket, ChartSummary},
    series::{Ask, Line, MissingSpan, grid},
};
use replay_data::rows::{GroupSpanRow as Span, TableId, tables};
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
    #[doc = " Whose membership a faction sums: `hour` (each member over its own membership; the default), `end` (the"]
    #[doc = " members at the view's last hour, over the whole view) or `ever` (anyone a member within the view, over the"]
    #[doc = " whole view)."]
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
    End,
    Ever,
}

impl Membership {
    fn of(text: Option<&str>) -> Result<Self, &'static str> {
        match text {
            None | Some("hour") => Ok(Self::Hour),
            Some("end") => Ok(Self::End),
            Some("ever") => Ok(Self::Ever),
            Some(_) => Err("A faction's membership is hour, end or ever"),
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
                Membership::End => spans.iter().any(|&(first, last)| first <= to && to <= last).then(|| vec![(from, to)]).unwrap_or_default(),
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
            Membership::End if unread_spans.iter().any(|&(first, last)| first <= to && to <= last) => vec![(from, to)],
            Membership::Ever if !unread_spans.is_empty() => vec![(from, to)],
            Membership::End | Membership::Ever => Vec::new(),
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
    fn a_roster_at_the_end_or_of_anyone_ever_a_member_sums_its_members_over_the_whole_view() {
        let runs = || vec![("GER".to_owned(), 0, 99), ("ITA".to_owned(), 20, 40), ("ITA".to_owned(), 60, 200), ("HUN".to_owned(), 150, 300), ("JAP".to_owned(), 0, 5)];
        let shown = |membership| roster_spans(runs(), membership, 10, 100);
        assert_eq!(shown(Membership::Hour), [("GER".to_owned(), vec![(10, 99)]), ("ITA".to_owned(), vec![(20, 40), (60, 100)])]);
        assert_eq!(shown(Membership::End), [("ITA".to_owned(), vec![(10, 100)])], "only who is a member at the view's last hour, over all of it");
        assert_eq!(shown(Membership::Ever), [("GER".to_owned(), vec![(10, 100)]), ("ITA".to_owned(), vec![(10, 100)])]);
        assert_eq!(Membership::of(Some("later")), Err("A faction's membership is hour, end or ever"));
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
}

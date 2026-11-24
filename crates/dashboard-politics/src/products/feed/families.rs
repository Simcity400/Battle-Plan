
use super::{
    EventFeedCategory, EventFeedItem, EventFeedKind, EventFeedRow, EventFeedUnavailable, LOG, Names, Notices, branch, invalid,
    row::{Columns, FeedEvent, Slot},
};
use dashboard_base::{Source, products::{LoadContext, Product}};
use dashboard_world::products::{groups, subjects};
use dashboard_research::products::special_projects;
use dashboard_query::{
    Query,
    log::{FacetFilter, Pattern, Reader, bin_for, facet_shift},
};
use replay_data::{
    event_feed_rows::{EventFeedCoverage, SCRIPT_FAMILY_NAMES},
    rows::FeedEventRow,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventFeedSpanParams {
    pub branch_id: String,
    #[doc = " The timeline's first and last hour when none."]
    #[serde(default)]
    pub from: Option<i32>,
    #[serde(default)]
    pub to: Option<i32>,
    #[doc = " The categories whose other families' rows are wanted."]
    pub categories: Vec<EventFeedCategory>,
    #[doc = " Only the kinds' codes, which a page's facets need before anything is read: no rows and"]
    #[doc = " no coverage notes, so the pages are never asked after a walk of the span's reads."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub kinds_only: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FeedKindCode {
    pub kind: EventFeedKind,
    pub code: u8,
    pub category: EventFeedCategory,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedSpanReply {
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub kinds: Vec<FeedKindCode>,
    #[doc = " The other families' rows in the span, by hour."]
    pub rows: Vec<EventFeedRow>,
    #[doc = " What the span could not show: reads whose events were not all observed, families not"]
    #[doc = " read, refusals of their rows."]
    pub unavailable: Vec<EventFeedUnavailable>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(EventFeedSpanReply);

pub(crate) const NOTICES: usize = 1 + SCRIPT_FAMILY_NAMES.len();

pub(crate) fn notice_patterns(kinds: &[Pattern], shift: u32) -> Vec<Pattern> {
    let facet = |pattern: &Pattern, mask: u32, value: u32, group: usize| Pattern { mask: pattern.mask | u128::from(mask) << shift, value: pattern.value | u128::from(value) << shift, group, ..*pattern };
    let mut out = Vec::new();
    for pattern in kinds {
        for coverage in [EventFeedCoverage::Unavailable, EventFeedCoverage::Partial] {
            out.push(facet(pattern, 0xFF, coverage as u32, 0));
        }
        for (at, (family, _)) in SCRIPT_FAMILY_NAMES.into_iter().enumerate() {
            let bit = (family as u32) << FeedEventRow::REFUSALS_SHIFT;
            out.push(facet(pattern, bit | 1 << 31, bit, 1 + at));
        }
    }
    out
}

pub(crate) fn coverage(source: &Source, query: &Query<'_>, branch: u64, from: i32, to: i32, notices: &mut Notices, cancelled: &AtomicBool) -> io::Result<()> {
    let declared = dashboard_declare::log(LOG).ok_or_else(|| invalid("The feed's log is not declared"))?;
    let reader = Reader::new(query, declared)?;
    let at = declared.facets.iter().position(|facet| facet.name == "incomplete").ok_or_else(|| invalid("The feed's log has no incomplete facet"))?;
    let filters = [FacetFilter { name: "kind".into(), values: vec![i64::from(FeedEventRow::READ_INCOMPLETE)], ..Default::default() }];
    let selection = reader.select(&filters, query.timeline.first, to)?;
    let patterns = notice_patterns(&selection.patterns, facet_shift(at));
    let bin = bin_for(i64::from(to) - i64::from(from) + 1, 1);
    let mut counts = [0u64; NOTICES];
    for (at, bins) in reader.bins(&patterns, from, to, bin)?.into_iter().enumerate() {
        counts[patterns[at].group] += bins.iter().map(|(_, count)| count).sum::<u64>();
    }
    let columns = Columns::new()?;
    let mut raised: Vec<(usize, u64, Vec<(i32, u128, FeedEvent)>)> = Vec::new();
    for (notice, &count) in counts.iter().enumerate() {
        if count == 0 {
            continue;
        }
        dashboard_text::metadata::check_cancel(cancelled)?;
        let asked: Vec<Pattern> = patterns.iter().filter(|pattern| pattern.group == notice).map(|pattern| Pattern { group: 0, ..*pattern }).collect();
        let page = reader.page(&asked, 1, from, to, 0, 3)?;
        let examples = page.rows.iter().map(|paged| Ok((paged.row.hour, paged.row.key, columns.read(&paged.row)?))).collect::<io::Result<Vec<_>>>()?;
        raised.push((notice, count, examples));
    }
    let losses: Vec<(i32, i32)> = source.replay_ref().event_losses(branch).collect();
    raised_notices(raised, &losses, notices);
    let windows = reader.select(&[FacetFilter { name: "kind".into(), values: vec![i64::from(FeedEventRow::WINDOWS_UNRECORDED)], ..Default::default() }], query.timeline.first, to)?;
    let page = reader.page(&windows.patterns, windows.groups.len(), from, to, 0, 3)?;
    let total = u32::try_from(page.total()).unwrap_or(u32::MAX);
    for (at, paged) in page.rows.iter().enumerate() {
        let read = columns.read(&paged.row)?;
        notices.sampled("eventWindows", "Event windows", format!("Read {}: {}", read.read_hour, String::from_utf8_lossy(&read.failure)), if at == 0 { total } else { 0 });
    }
    Ok(())
}

pub(crate) fn raised_notices(mut raised: Vec<(usize, u64, Vec<(i32, u128, FeedEvent)>)>, losses: &[(i32, i32)], notices: &mut Notices) {
    raised.retain(|(_, _, examples)| !examples.is_empty());
    raised.sort_by_key(|(notice, _, examples)| (examples[0].0, examples[0].1, *notice));
    for (notice, count, examples) in raised {
        let count = u32::try_from(count).unwrap_or(u32::MAX);
        for (at, (_, _, read)) in examples.iter().enumerate() {
            let counted = if at == 0 { count } else { 0 };
            if notice == 0 {
                if let Some(reason) = coverage_reason(read, losses) {
                    notices.sampled("nativeEventCoverage", "Native event coverage", reason, counted);
                }
            } else {
                let name = SCRIPT_FAMILY_NAMES[notice - 1].1;
                notices.sampled("nativeScriptFamilyCoverage", name, family_reason(name, read), counted);
            }
        }
    }
}

fn coverage_reason(read: &FeedEvent, losses: &[(i32, i32)]) -> Option<String> {
    let coverage = read.coverage.filter(|coverage| *coverage != EventFeedCoverage::Complete as u32)?;
    let lost = losses.iter().find(|&&(_, hour)| hour == read.read_hour).map(|&(after, hour)| format!("{} (between {after} and {hour})", dashboard_text::metadata::EVENT_LOSS_REASON));
    let recorded = (!read.failure.is_empty()).then(|| String::from_utf8_lossy(&read.failure).into_owned());
    let failure = match (lost, recorded) {
        (Some(lost), Some(recorded)) => format!("{lost}; {recorded}"),
        (Some(lost), None) => lost,
        (None, Some(recorded)) => recorded,
        (None, None) => "The counted interval is not fully observed".to_owned(),
    };
    let name = if coverage == EventFeedCoverage::Partial as u32 { "Partial" } else { "Unavailable" };
    let outstanding = read.outstanding.map_or_else(|| "an unrecorded number of".to_owned(), |count| count.to_string());
    Some(format!("{name} read at {}: {failure}; {outstanding} outstanding observations. Missing rows do not prove no event occurred.", read.read_hour))
}

fn family_reason(name: &str, read: &FeedEvent) -> String {
    format!("{name}: interval ending at {} has unobserved native writers; missing changes and unchanged carry are unavailable.", read.read_hour)
}

#[cfg(test)]
pub(crate) fn read_notices(read: &FeedEvent, losses: &[(i32, i32)], notices: &mut Notices) {
    if let Some(reason) = coverage_reason(read, losses) {
        notices.sampled("nativeEventCoverage", "Native event coverage", reason, 1);
    }
    let refused = read.refusals.unwrap_or(0);
    for (family, name) in SCRIPT_FAMILY_NAMES {
        if refused & family as u32 != 0 {
            notices.sampled("nativeScriptFamilyCoverage", name, family_reason(name, read), 1);
        }
    }
}

fn family_row(kind: EventFeedKind, hour: i32, read_hour: i32, name_hour: i32, tags: Vec<String>, subject: Option<String>, item: Option<EventFeedItem>) -> EventFeedRow {
    EventFeedRow {
        id: format!("{kind:?}:{hour}:{}:{}", tags.join(","), subject.unwrap_or_default()),
        hour,
        read_hour,
        name_hour,
        exact: true,
        category: kind.category(),
        kind,
        tags,
        participants: None,
        item,
        amount: None,
        cause: None,
        strike: None,
        decision_state: None,
        window: None,
    }
}

fn follows_observed(query: &Query<'_>, unobserved: &[dashboard_base::span::HourSpan], hour: i32) -> bool {
    let before = hour - 1;
    query.timeline.at(before).is_some_and(|stretch| stretch.first <= before && query.timeline.at(hour).is_some_and(|other| other.run == stretch.run))
        && !unobserved.iter().any(|span| span.first_hour <= before && before <= span.last_hour)
}

pub(crate) fn group_rows(source: &Source, query: &Query<'_>, context: &LoadContext, from: i32, to: i32, categories: &[EventFeedCategory], cancelled: &AtomicBool) -> io::Result<(Vec<EventFeedRow>, Vec<String>)> {
    let wanted = |kind: EventFeedKind| categories.contains(&kind.category());
    if !wanted(EventFeedKind::WarStarted) && !wanted(EventFeedKind::FactionJoined) {
        return Ok((Vec::new(), Vec::new()));
    }
    let reply = groups::shared(source, query, context, cancelled)?;
    let within = |hour: i32| from <= hour && hour <= to;
    let mut rows = Vec::new();
    if wanted(EventFeedKind::WarStarted) {
        rows.extend(reply.war_starts.iter().filter(|start| within(start.hour)).map(|start| family_row(EventFeedKind::WarStarted, start.hour, start.hour, start.hour, vec![start.attacking.clone(), start.defending.clone()], None, None)));
        rows.extend(reply.capitulations.iter().filter(|capitulation| within(capitulation.hour)).map(|capitulation| family_row(EventFeedKind::Capitulated, capitulation.hour, capitulation.hour, capitulation.hour, vec![capitulation.tag.clone()], None, None)));
        let mut ends: Vec<(i32, String, String)> = Vec::new();
        for war in &reply.wars {
            let ended = war.last_hour + 1;
            if within(ended) && follows_observed(query, &reply.unobserved.wars, ended) {
                ends.push((ended, war.first.clone(), war.second.clone()));
            }
        }
        let native = native_war_ends(query, &ends, cancelled)?;
        for (hour, first, second) in ends {
            let pair = |at: i32| if first <= second { (at, first.clone(), second.clone()) } else { (at, second.clone(), first.clone()) };
            if native.contains(&pair(hour)) || native.contains(&pair(hour - 1)) {
                continue;
            }
            rows.push(family_row(EventFeedKind::Peace, hour, hour, hour - 1, vec![first, second], None, None));
        }
    }
    if wanted(EventFeedKind::FactionJoined) {
        for faction in &reply.factions {
            let name = |hour: i32| {
                let named = faction.names.iter().find(|run| run.first_hour <= hour && hour <= run.last_hour).and_then(|run| run.name.clone()).or_else(|| faction.name.clone());
                Some(EventFeedItem { token: (!faction.recorded_name.is_empty()).then(|| faction.recorded_name.clone()), name: named, icon: None, description: None, ..Default::default() })
            };
            for member in &faction.members {
                if within(member.first_hour) && follows_observed(query, &reply.unobserved.factions, member.first_hour) {
                    rows.push(family_row(EventFeedKind::FactionJoined, member.first_hour, member.first_hour, member.first_hour, vec![member.tag.clone()], Some(faction.id.clone()), name(member.first_hour)));
                }
                let left = member.last_hour + 1;
                if within(left) && follows_observed(query, &reply.unobserved.factions, left) {
                    rows.push(family_row(EventFeedKind::FactionLeft, left, left, member.last_hour, vec![member.tag.clone()], Some(faction.id.clone()), name(member.last_hour)));
                }
            }
        }
    }
    Ok((rows, reply.warnings.clone()))
}

fn native_war_ends(query: &Query<'_>, ends: &[(i32, String, String)], cancelled: &AtomicBool) -> io::Result<BTreeSet<(i32, String, String)>> {
    let mut out = BTreeSet::new();
    let mut ranges: Vec<(i32, i32)> = ends.iter().map(|(hour, ..)| (hour - 1, *hour)).collect();
    ranges.sort_unstable();
    let mut merged: Vec<(i32, i32)> = Vec::new();
    for (first, last) in ranges {
        match merged.last_mut() {
            Some(held) if held.1 + 1 >= first => held.1 = held.1.max(last),
            _ => merged.push((first, last)),
        }
    }
    if merged.is_empty() {
        return Ok(out);
    }
    let declared = dashboard_declare::log(LOG).ok_or_else(|| invalid("The feed's log is not declared"))?;
    let reader = Reader::new(query, declared)?;
    let ended = EventFeedKind::WarEnded.code().map(i64::from).ok_or_else(|| invalid("A war end has no feed code"))?;
    let selection = reader.select(&[FacetFilter { name: "kind".into(), values: vec![ended], ..Default::default() }], query.timeline.first, query.timeline.last)?;
    let columns = Columns::new()?;
    let names = Names::new(&query.holders);
    let mut runs: BTreeMap<u32, Vec<(i32, i32)>> = BTreeMap::new();
    for &(first, last) in &merged {
        for stretch in query.timeline.within(first, last) {
            runs.entry(stretch.run).or_default().push((first.max(stretch.first), last.min(stretch.last)));
        }
    }
    for (run, ranges) in runs {
        dashboard_text::metadata::check_cancel(cancelled)?;
        for (_, row) in reader.rows(run, &selection.patterns, &ranges)? {
            let event = columns.read(&row)?;
            let tag = |slot: Slot| match slot {
                Slot::Of(slot) => names.slot_tag(slot, event.hour).map(str::to_owned),
                _ => None,
            };
            if let (Some(first), Some(second)) = (tag(event.actor.slot), tag(event.target.slot)) {
                out.insert(if first <= second { (event.hour, first, second) } else { (event.hour, second, first) });
            }
        }
    }
    Ok(out)
}

pub(crate) fn subject_rows(source: &Source, query: &Query<'_>, from: i32, to: i32, cancelled: &AtomicBool) -> io::Result<Vec<EventFeedRow>> {
    let (runs, _) = subjects::history(source, query, cancelled)?;
    let mut seen = BTreeSet::new();
    let mut rows = Vec::new();
    for run in runs {
        let start = run.start_hour;
        if start < query.timeline.first || start < from || start > to || !seen.insert((run.overlord.clone(), run.subject.clone(), start)) {
            continue;
        }
        rows.push(family_row(EventFeedKind::BecameSubject, start, run.first_hour.max(start), start, vec![run.subject, run.overlord], None, None));
    }
    Ok(rows)
}

pub(crate) fn project_rows(source: &Source, query: &Query<'_>, context: &LoadContext, from: i32, to: i32, notices: &mut Notices, cancelled: &AtomicBool) -> io::Result<(Vec<EventFeedRow>, Vec<String>)> {
    let found = special_projects::completions(source, context, query, from, to, cancelled)?;
    if !found.unrecorded.is_empty() {
        let hours: i64 = found.unrecorded.iter().map(|span| i64::from(span.last_hour) - i64::from(span.first_hour) + 1).sum();
        notices.unavailable("projectFinished", "Special projects", format!("Project histories were not recorded for {hours} hours of this span; a completion dated then is not listed."), None);
    }
    let rows = found.events.into_iter().filter_map(completion_row).collect();
    Ok((rows, found.warnings))
}

pub(super) fn completion_row(event: special_projects::ProjectCompletion) -> Option<EventFeedRow> {
    let tag = event.tag?;
    let item = EventFeedItem { token: None, name: event.name, icon: event.icon, ..Default::default() };
    let subject = format!("{}:{}:{}", event.owner, event.project, event.ordinal);
    Some(family_row(EventFeedKind::ProjectFinished, event.date, event.observed_hour, event.observed_hour, vec![tag], Some(subject), Some(item)))
}

pub(crate) fn unread(categories: &[EventFeedCategory], notices: &mut Notices) {
    for (kind, label, family) in [
        ("lawChanged", "Laws and advisors", "the ideas family (24)"),
        ("rulingPartyChanged", "Ruling parties", "the party history family (23)"),
        ("focusCompleted", "Focus completions", "the focus family (31)"),
        ("nukesIncreased", "Nuclear stockpile", "the nukes family (6)"),
    ] {
        let category = match kind {
            "lawChanged" | "rulingPartyChanged" => EventFeedCategory::Politics,
            _ => EventFeedCategory::Milestones,
        };
        if categories.contains(&category) {
            notices.unavailable(kind, label, format!("These rows come from {family}, which the feed does not read from the recording yet."), None);
        }
    }
    if categories.contains(&EventFeedCategory::WarsPeace) {
        notices.unavailable(
            "warDeclarationOrJoin",
            "War declaration or join",
            "war relations supplies the pair becoming at war. Declarations and joins require the native event history; refused intervals cannot distinguish those actions.",
            None,
        );
    }
}

fn kinds() -> Vec<FeedKindCode> {
    super::CODES.iter().map(|&(code, kind)| FeedKindCode { kind, code, category: kind.category() }).collect()
}

pub(crate) fn kinds_only(branch_id: &str, first_hour: i32, last_hour: i32) -> EventFeedSpanReply {
    EventFeedSpanReply { branch_id: branch_id.to_owned(), first_hour, last_hour, kinds: kinds(), rows: Vec::new(), unavailable: Vec::new(), warnings: Vec::new() }
}

pub struct EventFeedSpan;

impl Product for EventFeedSpan {
    const KEY: &'static str = "event-feed-span";
    const CONTENT_DEPENDENT: bool = true;
    type Params = EventFeedSpanParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, from: Start, to: End, categories, kindsOnly];
    type Reply = EventFeedSpanReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![EventFeedSpanParams::decl(), FeedKindCode::decl(), EventFeedSpanReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let branch: u64 = params.branch_id.parse().map_err(|_| invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let (from, to) = (params.from.unwrap_or(query.timeline.first), params.to.unwrap_or(query.timeline.last));
        if from > to {
            return Err(invalid("Invalid span"));
        }
        if params.kinds_only == Some(true) {
            return Ok(kinds_only(&params.branch_id, from, to));
        }
        let mut notices = Notices::default();
        coverage(source, &query, branch, from, to, &mut notices, cancelled)?;
        let (mut rows, mut warnings) = group_rows(source, &query, context, from, to, &params.categories, cancelled)?;
        if params.categories.contains(&EventFeedCategory::Diplomacy) {
            rows.extend(subject_rows(source, &query, from, to, cancelled)?);
        }
        if params.categories.contains(&EventFeedCategory::Milestones) {
            let (projects, notes) = project_rows(source, &query, context, from, to, &mut notices, cancelled)?;
            rows.extend(projects);
            warnings.extend(notes);
        }
        if params.categories.contains(&EventFeedKind::NuclearStrike.category()) {
            let localisation = match context.content.as_ref() {
                Some(content) => dashboard_base::content::localisation(source, content).map_err(|error| format!("Nuclear type names are unavailable: {error}")),
                None => Err("Nuclear type names are unavailable until the game folders are chosen".to_owned()),
            };
            if let Err(reason) = &localisation {
                warnings.push(reason.clone());
            }
            let localise = |spelling: &str| localisation.as_ref().ok().and_then(|held| dashboard_text::catalogue::localised(held, spelling, dashboard_text::catalogue::View::Plain).ok());
            rows.extend(super::nuclear::strike_rows(&query, from, to, &localise, &mut notices, cancelled)?);
        }
        unread(&params.categories, &mut notices);
        rows.sort_by(|a, b| (a.hour, a.read_hour, a.kind, &a.tags).cmp(&(b.hour, b.read_hour, b.kind, &b.tags)));
        warnings.sort();
        warnings.dedup();
        let reply = EventFeedSpanReply {
            branch_id: params.branch_id.clone(),
            first_hour: from,
            last_hour: to,
            kinds: kinds(),
            rows,
            unavailable: notices.into_list(),
            warnings,
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

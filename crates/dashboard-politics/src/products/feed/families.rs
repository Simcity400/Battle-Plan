
use super::{
    EventFeedCategory, EventFeedItem, EventFeedKind, EventFeedRow, LOG, Names, Notices, invalid,
    row::{Columns, FeedEvent, Slot},
};
use dashboard_base::{Source, products::LoadContext};
use dashboard_world::products::{groups, subjects};
use dashboard_research::products::special_projects;
use dashboard_base::span::HourSpan;
use dashboard_query::{
    Query,
    log::{FacetFilter, Reader},
};
use replay_data::event_feed_rows::{EventFeedCoverage, SCRIPT_FAMILY_NAMES};
use std::{collections::{BTreeMap, BTreeSet}, io, sync::atomic::AtomicBool};

pub(crate) const NOTICES: usize = 1 + SCRIPT_FAMILY_NAMES.len();

pub(crate) fn incomplete_notices(incomplete: &[(i32, u128, FeedEvent)], losses: &[(i32, i32)], from: i32, to: i32, notices: &mut Notices) {
    let mut raised: Vec<(usize, u64, Vec<(i32, u128, FeedEvent)>)> = (0..NOTICES).map(|notice| (notice, 0, Vec::new())).collect();
    let first = incomplete.partition_point(|(hour, ..)| *hour < from);
    for (hour, key, read) in incomplete[first..].iter().take_while(|(hour, ..)| *hour <= to) {
        let mut raise = |notice: usize| {
            let held = &mut raised[notice];
            held.1 += 1;
            if held.2.len() < 3 {
                held.2.push((*hour, *key, read.clone()));
            }
        };
        if read.coverage.is_some_and(|coverage| coverage != EventFeedCoverage::Complete as u32) {
            raise(0);
        }
        let refused = read.refusals.unwrap_or(0);
        for (at, (family, _)) in SCRIPT_FAMILY_NAMES.into_iter().enumerate() {
            if refused & family as u32 != 0 {
                raise(1 + at);
            }
        }
    }
    raised_notices(raised, losses, notices);
}

pub(crate) fn window_notices(unrecorded: &[(i32, i32, String)], from: i32, to: i32, notices: &mut Notices) {
    for (first, _, reason) in unrecorded.iter().filter(|(first, last, _)| *first <= to && *last >= from) {
        notices.unavailable("eventWindows", "Event windows", format!("From read {}: {reason}", first.max(&from)), None);
    }
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
        owner_missing: None,
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

pub(crate) type DatedNotes = Vec<(i32, String)>;

pub(crate) struct Unrecorded {
    pub category: EventFeedCategory,
    pub kind: &'static str,
    pub label: &'static str,
    pub listed: &'static str,
    pub why: &'static str,
    pub spans: Vec<(i32, i32)>,
}

impl Unrecorded {
    fn of<'s>(category: EventFeedCategory, (kind, label): (&'static str, &'static str), (listed, why): (&'static str, &'static str), spans: impl IntoIterator<Item = &'s HourSpan>) -> Self {
        let mut sorted: Vec<(i32, i32)> = spans.into_iter().filter(|span| span.first_hour <= span.last_hour).map(|span| (span.first_hour, span.last_hour)).collect();
        sorted.sort_unstable();
        let mut merged: Vec<(i32, i32)> = Vec::with_capacity(sorted.len());
        for (first, last) in sorted {
            match merged.last_mut() {
                Some(held) if first <= held.1.saturating_add(1) => held.1 = held.1.max(last),
                _ => merged.push((first, last)),
            }
        }
        Self { category, kind, label, listed, why, spans: merged }
    }
}

pub(crate) fn group_rows(source: &Source, query: &Query<'_>, context: &LoadContext, from: i32, to: i32, categories: &[EventFeedCategory], cancelled: &AtomicBool) -> io::Result<(Vec<EventFeedRow>, DatedNotes, Vec<i32>)> {
    let wanted = |kind: EventFeedKind| categories.contains(&kind.category());
    if !wanted(EventFeedKind::WarStarted) && !wanted(EventFeedKind::FactionJoined) {
        return Ok((Vec::new(), Vec::new(), Vec::new()));
    }
    let mut notes: DatedNotes = Vec::new();
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
            notes.extend(faction.names.iter().filter_map(|run| run.reason.as_ref().map(|reason| (run.first_hour, format!("Faction names are unavailable: {reason}")))));
            let name = |hour: i32| {
                let run = faction.names.iter().find(|run| run.first_hour <= hour && hour <= run.last_hour);
                let named = run.and_then(|run| run.name.as_deref()).filter(|name| !name.is_empty()).map(str::to_owned);
                let name_missing = named.is_none().then(|| run.and_then(|run| run.reason.clone()).unwrap_or_else(|| "Faction name not recorded at this hour".to_owned()));
                Some(EventFeedItem { token: run.and_then(|run| run.token.clone()), name: named, icon: None, description: None, name_missing, ..Default::default() })
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
    let unattributed = if wanted(EventFeedKind::WarStarted) { reply.unobserved.unattributed.iter().copied().filter(|&hour| within(hour)).collect() } else { Vec::new() };
    Ok((rows, notes, unattributed))
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

pub(crate) fn project_completions(source: &Source, query: &Query<'_>, context: &LoadContext, from: i32, to: i32, cancelled: &AtomicBool) -> io::Result<(Vec<(EventFeedRow, Option<u32>)>, Vec<String>, Unrecorded)> {
    let found = special_projects::completions(source, context, query, from, to, cancelled)?;
    let unrecorded = Unrecorded::of(EventFeedCategory::Milestones, ("projectFinished", "Special projects"), ("A completion dated in", "may not be listed: project histories were not recorded then"), &found.unrecorded);
    let rows = found.events.into_iter().map(completion_row).collect();
    Ok((rows, found.warnings, unrecorded))
}

pub(super) fn completion_row(event: special_projects::ProjectCompletion) -> (EventFeedRow, Option<u32>) {
    let slot = event.attribution.slot();
    let (tag, owner_missing) = match event.attribution {
        special_projects::CompletionOwner::Resolved { tag, .. } => (Some(tag), None),
        special_projects::CompletionOwner::NotRecorded { reason, .. } => (None, Some(reason)),
    };
    let tags: Vec<String> = tag.iter().cloned().collect();
    let item = EventFeedItem { token: None, name: event.name, icon: event.icon, ..Default::default() };
    let subject = format!("{}:{}:{}", event.owner, event.project, event.ordinal);
    let mut row = family_row(EventFeedKind::ProjectFinished, event.date, event.observed_hour, event.date, tags, Some(subject), Some(item));
    row.participants = Some([tag, None]);
    row.owner_missing = owner_missing;
    (row, slot)
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


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
    identity::Holder,
    log::{FacetFilter, Reader},
};
use replay_data::event_feed_rows::{EventFeedCoverage, SCRIPT_FAMILY_NAMES};
use std::{collections::{BTreeMap, BTreeSet, HashMap}, io, sync::atomic::{AtomicBool, Ordering}};

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
        item,
        amount: None,
        cause: None,
        strike: None,
        decision_state: None,
        window: None,
    }
}

fn changed_at(query: &Query<'_>, unobserved: &[&[HourSpan]], hour: i32) -> bool {
    let before = hour - 1;
    let run = query.timeline.at(before).map(|stretch| stretch.run);
    run.is_some()
        && query.timeline.at(hour).map(|stretch| stretch.run) == run
        && !unobserved.iter().flat_map(|spans| spans.iter()).any(|span| span.first_hour <= hour && before <= span.last_hour)
}

pub(crate) type DatedNotes = Vec<(i32, String)>;

pub(crate) struct Unrecorded {
    pub category: EventFeedCategory,
    pub kind: &'static str,
    pub label: &'static str,
    pub what: &'static str,
    pub lost: &'static str,
    pub spans: Vec<(i32, i32)>,
}

impl Unrecorded {
    fn of<'s>(category: EventFeedCategory, kind: &'static str, label: &'static str, what: &'static str, lost: &'static str, spans: impl IntoIterator<Item = &'s HourSpan>) -> Self {
        let mut sorted: Vec<(i32, i32)> = spans.into_iter().map(|span| (span.first_hour, span.last_hour)).filter(|(first, last)| first <= last).collect();
        sorted.sort_unstable();
        let mut merged: Vec<(i32, i32)> = Vec::with_capacity(sorted.len());
        for (first, last) in sorted {
            match merged.last_mut() {
                Some(held) if first <= held.1.saturating_add(1) => held.1 = held.1.max(last),
                _ => merged.push((first, last)),
            }
        }
        Self { category, kind, label, what, lost, spans: merged }
    }
}

pub(crate) struct Groups {
    pub rows: Vec<EventFeedRow>,
    pub notes: DatedNotes,
    pub unattributed: Vec<i32>,
    pub unrecorded: Vec<Unrecorded>,
    pub refused: Option<String>,
}

pub(crate) fn group_rows(source: &Source, query: &Query<'_>, context: &LoadContext, from: i32, to: i32, categories: &[EventFeedCategory], cancelled: &AtomicBool) -> io::Result<Groups> {
    let wanted = |kind: EventFeedKind| categories.contains(&kind.category());
    let mut out = Groups { rows: Vec::new(), notes: Vec::new(), unattributed: Vec::new(), unrecorded: Vec::new(), refused: None };
    if !wanted(EventFeedKind::WarStarted) && !wanted(EventFeedKind::FactionJoined) {
        return Ok(out);
    }
    let reply = groups::shared(source, query, context, cancelled)?;
    let within = |hour: i32| from <= hour && hour <= to;
    let slot = |tag: &str, hour: i32| match query.holders.at(tag, hour).map(|run| run.holder) {
        Some(Holder::Found { slot, .. }) => Some(slot),
        _ => None,
    };
    if wanted(EventFeedKind::WarStarted) {
        out.unrecorded.push(Unrecorded::of(EventFeedCategory::WarsPeace, "peace", "Peace", "War relations were not observed", "a peace dated then is not listed", &reply.unobserved.wars));
        out.rows.extend(reply.war_starts.iter().filter(|start| within(start.hour)).map(|start| family_row(EventFeedKind::WarStarted, start.hour, start.hour, start.hour, vec![start.attacking.clone(), start.defending.clone()], None, None)));
        out.rows.extend(reply.capitulations.iter().filter(|capitulation| within(capitulation.hour)).map(|capitulation| family_row(EventFeedKind::Capitulated, capitulation.hour, capitulation.hour, capitulation.hour, vec![capitulation.tag.clone()], None, None)));
        let mut fought: HashMap<(u32, u32), Vec<(i32, i32)>> = HashMap::new();
        for war in &reply.wars {
            if let (Some(one), Some(other)) = (slot(&war.first, war.first_hour), slot(&war.second, war.first_hour)) {
                fought.entry((one.min(other), one.max(other))).or_default().push((war.first_hour, war.last_hour));
            }
        }
        let mut ends: Vec<(i32, String, String)> = Vec::new();
        for war in &reply.wars {
            let ended = war.last_hour + 1;
            if !within(ended) || !changed_at(query, &[&reply.unobserved.wars], ended) {
                continue;
            }
            let (Some(one), Some(other)) = (slot(&war.first, war.last_hour), slot(&war.second, war.last_hour)) else { continue };
            let renamed = fought.get(&(one.min(other), one.max(other))).is_some_and(|runs| runs.iter().any(|&(first, last)| first <= ended && ended <= last));
            if !renamed {
                ends.push((ended, war.first.clone(), war.second.clone()));
            }
        }
        let native = match native_war_ends(query, &ends, cancelled) {
            Ok(native) => native,
            Err(error) if cancelled.load(Ordering::Relaxed) => return Err(error),
            Err(error) => {
                out.refused = Some(format!("No peace is inferred from war relations: the native war ends it would repeat could not be read: {error}"));
                ends.clear();
                BTreeSet::new()
            }
        };
        for (hour, first, second) in ends {
            let pair = |at: i32| if first <= second { (at, first.clone(), second.clone()) } else { (at, second.clone(), first.clone()) };
            if native.contains(&pair(hour)) || native.contains(&pair(hour - 1)) {
                continue;
            }
            out.rows.push(family_row(EventFeedKind::Peace, hour, hour, hour - 1, vec![first, second], None, None));
        }
    }
    if wanted(EventFeedKind::FactionJoined) {
        out.unrecorded.push(Unrecorded::of(
            EventFeedCategory::Diplomacy,
            "factionMembership",
            "Faction joins and departures",
            "Faction membership was not fully observed",
            "a join or departure dated then is not listed",
            reply.unobserved.factions.iter().chain(reply.factions.iter().flat_map(|faction| &faction.incomplete)),
        ));
        for faction in &reply.factions {
            out.notes.extend(faction.names.iter().filter_map(|run| run.reason.as_ref().map(|reason| (run.first_hour, format!("Faction names are unavailable: {reason}")))));
            let name = |hour: i32| {
                let run = faction.names.iter().find(|run| run.first_hour <= hour && hour <= run.last_hour);
                let named = run.and_then(|run| run.name.as_deref()).filter(|name| !name.is_empty()).map(str::to_owned);
                let name_missing = named.is_none().then(|| run.and_then(|run| run.reason.clone()).unwrap_or_else(|| "Faction name not recorded at this hour".to_owned()));
                Some(EventFeedItem { token: run.and_then(|run| run.token.clone()), name: named, icon: None, description: None, name_missing, ..Default::default() })
            };
            let unknown: [&[HourSpan]; 2] = [&reply.unobserved.factions, &faction.incomplete];
            let mut held: HashMap<u32, Vec<(i32, i32)>> = HashMap::new();
            for member in &faction.members {
                if let Some(slot) = slot(&member.tag, member.first_hour) {
                    held.entry(slot).or_default().push((member.first_hour, member.last_hour));
                }
            }
            let member_at = |tag: &str, named: i32, hour: i32| slot(tag, named).is_some_and(|slot| held.get(&slot).is_some_and(|runs| runs.iter().any(|&(first, last)| first <= hour && hour <= last)));
            for member in &faction.members {
                if within(member.first_hour) && changed_at(query, &unknown, member.first_hour) && !member_at(&member.tag, member.first_hour, member.first_hour - 1) {
                    out.rows.push(family_row(EventFeedKind::FactionJoined, member.first_hour, member.first_hour, member.first_hour, vec![member.tag.clone()], Some(faction.id.clone()), name(member.first_hour)));
                }
                let left = member.last_hour + 1;
                if within(left) && changed_at(query, &unknown, left) && !member_at(&member.tag, member.last_hour, left) {
                    out.rows.push(family_row(EventFeedKind::FactionLeft, left, left, member.last_hour, vec![member.tag.clone()], Some(faction.id.clone()), name(member.last_hour)));
                }
            }
        }
    }
    if wanted(EventFeedKind::WarStarted) {
        out.unattributed = reply.unobserved.unattributed.iter().copied().filter(|&hour| within(hour)).collect();
    }
    Ok(out)
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

pub(crate) fn project_completions(source: &Source, query: &Query<'_>, context: &LoadContext, from: i32, to: i32, cancelled: &AtomicBool) -> io::Result<(Vec<EventFeedRow>, Vec<String>, Unrecorded)> {
    let found = special_projects::completions(source, context, query, from, to, cancelled)?;
    let unrecorded = Unrecorded::of(EventFeedCategory::Milestones, "projectFinished", "Special projects", "Project histories were not recorded", "a completion dated then may not be listed", &found.unrecorded);
    let rows = found.events.into_iter().map(completion_row).collect();
    Ok((rows, found.warnings, unrecorded))
}

pub(super) fn completion_row(event: special_projects::ProjectCompletion) -> EventFeedRow {
    let tag = event.tag;
    let item = EventFeedItem { token: None, name: event.name, icon: event.icon, ..Default::default() };
    let subject = format!("{}:{}:{}", event.owner, event.project, event.ordinal);
    let mut row = family_row(EventFeedKind::ProjectFinished, event.date, event.observed_hour, event.date, tag.iter().cloned().collect(), Some(subject), Some(item));
    row.participants = Some([tag, None]);
    row
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


use super::{EventFeedCategory, EventFeedItem, EventFeedKind, EventFeedRow, Notices, row::FeedEvent};
use dashboard_base::{Source, products::LoadContext};
use dashboard_world::products::{groups, subjects};
use dashboard_research::products::special_projects;
use dashboard_base::span::HourSpan;
use dashboard_query::Query;
use replay_data::event_feed_rows::{EventFeedCoverage, SCRIPT_FAMILY_NAMES};
use std::{collections::{BTreeSet, HashMap}, io, sync::atomic::AtomicBool};

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

fn covers(spans: &[HourSpan], hour: i32) -> bool {
    spans.get(spans.partition_point(|span| span.last_hour < hour)).is_some_and(|span| span.first_hour <= hour)
}

fn absent_observed(query: &Query<'_>, unobserved: &[&[HourSpan]], absent: i32, present: i32) -> bool {
    let run = query.timeline.at(present).map(|stretch| stretch.run);
    run.is_some() && query.timeline.at(absent).map(|stretch| stretch.run) == run && !unobserved.iter().any(|spans| covers(spans, absent))
}

fn merged(mut runs: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    runs.sort_unstable();
    let mut out: Vec<(i32, i32)> = Vec::with_capacity(runs.len());
    for (first, last) in runs {
        match out.last_mut() {
            Some(held) if first <= held.1.saturating_add(1) => held.1 = held.1.max(last),
            _ => out.push((first, last)),
        }
    }
    out
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
    fn of<'s>(category: EventFeedCategory, (kind, label): (&'static str, &'static str), (listed, why): (&'static str, &'static str), reach: i32, spans: impl IntoIterator<Item = &'s HourSpan>) -> Self {
        let mut sorted: Vec<(i32, i32)> = spans.into_iter().filter(|span| span.first_hour <= span.last_hour).map(|span| (span.first_hour, span.last_hour.saturating_add(reach))).collect();
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

pub(crate) struct Groups {
    pub rows: Vec<EventFeedRow>,
    pub peaces: Vec<((i32, u32, u32), EventFeedRow)>,
    pub notes: DatedNotes,
    pub unattributed: Vec<i32>,
    pub unrecorded: Vec<Unrecorded>,
}

pub(crate) fn group_rows(source: &Source, query: &Query<'_>, context: &LoadContext, from: i32, to: i32, categories: &[EventFeedCategory], cancelled: &AtomicBool) -> io::Result<Groups> {
    let wanted = |kind: EventFeedKind| categories.contains(&kind.category());
    let mut out = Groups { rows: Vec::new(), peaces: Vec::new(), notes: Vec::new(), unattributed: Vec::new(), unrecorded: Vec::new() };
    if !wanted(EventFeedKind::WarStarted) && !wanted(EventFeedKind::FactionJoined) {
        return Ok(out);
    }
    let reply = groups::shared(source, query, context, cancelled)?;
    let within = |hour: i32| from <= hour && hour <= to;
    let tag_at = |slot: u32, hour: i32| query.holders.slot_tag(slot, hour).map(str::to_owned);
    if wanted(EventFeedKind::WarStarted) {
        out.unrecorded.push(Unrecorded::of(
            EventFeedCategory::WarsPeace,
            ("peace", "Peace"),
            ("A peace dated in", "is not listed: war relations were not observed then or the hour before"),
            1,
            &reply.unobserved.wars,
        ));
        out.rows.extend(reply.war_starts.iter().filter(|start| within(start.hour)).map(|start| family_row(EventFeedKind::WarStarted, start.hour, start.hour, start.hour, vec![start.attacking.clone(), start.defending.clone()], None, None)));
        out.rows.extend(reply.capitulations.iter().filter(|capitulation| within(capitulation.hour)).map(|capitulation| family_row(EventFeedKind::Capitulated, capitulation.hour, capitulation.hour, capitulation.hour, vec![capitulation.tag.clone()], None, None)));
        for ((one, other), runs) in groups::war_slots(source, query)? {
            dashboard_text::metadata::check_cancel(cancelled)?;
            for (_, last) in merged(runs) {
                let ended = last.saturating_add(1);
                if within(ended)
                    && absent_observed(query, &[&reply.unobserved.wars], ended, last)
                    && let (Some(first_tag), Some(second_tag)) = (tag_at(one, last), tag_at(other, last))
                {
                    out.peaces.push(((ended, one, other), family_row(EventFeedKind::Peace, ended, ended, last, vec![first_tag, second_tag], None, None)));
                }
            }
        }
    }
    if wanted(EventFeedKind::FactionJoined) {
        out.unrecorded.push(Unrecorded::of(
            EventFeedCategory::Diplomacy,
            ("factionMembership", "Faction joins and departures"),
            ("A join or departure dated in", "is not listed: faction membership was not fully observed then or the hour before"),
            1,
            reply.unobserved.factions.iter().chain(reply.factions.iter().flat_map(|faction| &faction.incomplete)),
        ));
        let mut slots = groups::member_slots(source, query)?;
        for faction in &reply.factions {
            out.notes.extend(faction.names.iter().filter_map(|run| run.reason.as_ref().map(|reason| (run.first_hour, format!("Faction names are unavailable: {reason}")))));
            let name = |hour: i32| {
                let run = faction.names.iter().find(|run| run.first_hour <= hour && hour <= run.last_hour);
                let named = run.and_then(|run| run.name.as_deref()).filter(|name| !name.is_empty()).map(str::to_owned);
                let name_missing = named.is_none().then(|| run.and_then(|run| run.reason.clone()).unwrap_or_else(|| "Faction name not recorded at this hour".to_owned()));
                Some(EventFeedItem { token: run.and_then(|run| run.token.clone()), name: named, icon: None, description: None, name_missing, ..Default::default() })
            };
            let Some(members) = faction.id.parse::<u64>().ok().and_then(|id| slots.remove(&id)) else { continue };
            let unknown: [&[HourSpan]; 2] = [&reply.unobserved.factions, &faction.incomplete];
            let mut by_slot: HashMap<u32, Vec<(i32, i32)>> = HashMap::new();
            for (slot, first, last) in members {
                by_slot.entry(slot).or_default().push((first, last));
            }
            for (slot, runs) in by_slot {
                for (first, last) in merged(runs) {
                    if within(first)
                        && absent_observed(query, &unknown, first - 1, first)
                        && let Some(tag) = tag_at(slot, first)
                    {
                        out.rows.push(family_row(EventFeedKind::FactionJoined, first, first, first, vec![tag], Some(faction.id.clone()), name(first)));
                    }
                    let gone = last.saturating_add(1);
                    if within(gone)
                        && absent_observed(query, &unknown, gone, last)
                        && let Some(tag) = tag_at(slot, last)
                    {
                        out.rows.push(family_row(EventFeedKind::FactionLeft, gone, gone, last, vec![tag], Some(faction.id.clone()), name(last)));
                    }
                }
            }
        }
    }
    if wanted(EventFeedKind::WarStarted) {
        out.unattributed = reply.unobserved.unattributed.iter().copied().filter(|&hour| within(hour)).collect();
    }
    Ok(out)
}

pub(crate) fn retryable(error: &io::Error) -> bool {
    matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock)
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
    let unrecorded = Unrecorded::of(EventFeedCategory::Milestones, ("projectFinished", "Special projects"), ("A completion dated in", "may not be listed: project histories were not recorded then"), 0, &found.unrecorded);
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

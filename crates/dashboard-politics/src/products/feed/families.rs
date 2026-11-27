
use super::{
    EventFeedCategory, EventFeedItem, EventFeedKind, EventFeedRow, Notices,
    row::FeedEvent,
    transitions::{BranchTransitions, NativeWarEnd, ResolvedParty, native_supersedes},
};
use dashboard_base::{Source, products::LoadContext};
use dashboard_world::products::{groups, subjects};
use dashboard_research::products::special_projects;
use dashboard_query::Query;
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
        participants_missing: None,
        owner_missing: None,
        item,
        amount: None,
        cause: None,
        strike: None,
        decision_state: None,
        window: None,
    }
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
    fn of(category: EventFeedCategory, (kind, label): (&'static str, &'static str), (listed, why): (&'static str, &'static str), spans: impl IntoIterator<Item = (i32, i32)>) -> Self {
        let mut sorted: Vec<(i32, i32)> = spans.into_iter().filter(|&(first, last)| first <= last).collect();
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

fn disambiguate(rows: &mut [(FamilyRow, String)]) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (row, _) in rows.iter() { *counts.entry(row.row.id.clone()).or_default() += 1; }
    for (row, key) in rows.iter_mut() { if counts[&row.row.id] > 1 { row.row.id.push_str(key); } }
}

fn peace_row(hour: i32, parties: &[ResolvedParty; 2], until: Option<i32>) -> (FamilyRow, String) {
    let ([a, b], mut row) = (parties, family_row(EventFeedKind::Peace, hour, hour, hour - 1, Vec::new(), None, None));
    let (k0, k1) = (a.id.unwrap_or(a.raw), b.id.unwrap_or(b.raw));
    let key = format!("#{},{}", k0.min(k1), k0.max(k1));
    let slots = match (&a.tag, &b.tag) {
        (Some(ta), Some(tb)) => {
            let (first, second, slots) = if ta <= tb { (ta.clone(), tb.clone(), vec![a.slot, b.slot]) } else { (tb.clone(), ta.clone(), vec![b.slot, a.slot]) };
            row.id = format!("Peace:{hour}:{first},{second}:");
            row.tags = vec![first, second];
            slots
        }
        _ => {
            let [p0, p1] = if (a.tag.is_none(), a.tag.as_deref(), a.raw) <= (b.tag.is_none(), b.tag.as_deref(), b.raw) { [a, b] } else { [b, a] };
            let label = |p: &ResolvedParty| p.tag.clone().unwrap_or_else(|| format!("#{}", p.raw));
            row.id = format!("Peace:{hour}:{},{}:", label(p0), label(p1));
            row.tags = [p0.tag.as_ref(), p1.tag.as_ref()].into_iter().flatten().cloned().collect();
            row.participants = Some([p0.tag.clone(), p1.tag.clone()]);
            row.participants_missing = Some([p0.missing.clone(), p1.missing.clone()]);
            vec![p0.slot, p1.slot]
        }
    };
    (FamilyRow { row, icon: None, slots: Some(slots), until }, key)
}

fn faction_row(kind: EventFeedKind, hour: i32, name_hour: i32, faction: u64, party: &ResolvedParty, item: Option<EventFeedItem>) -> (FamilyRow, String) {
    let mut row = family_row(kind, hour, hour, name_hour, party.tag.iter().cloned().collect(), Some(faction.to_string()), item);
    if party.tag.is_none() {
        row.id = format!("{kind:?}:{hour}:#{}:{faction}", party.raw);
        row.participants = Some([None, None]);
        row.participants_missing = Some([party.missing.clone(), None]);
    }
    (FamilyRow { row, icon: None, slots: Some(vec![party.slot]), until: None }, format!("#{}", party.id.unwrap_or(party.raw)))
}

pub(crate) fn group_rows(reply: Option<&groups::CountryGroupReply>, group_err: Option<&str>, transitions: &BranchTransitions, native_ends: &[NativeWarEnd], (from, to): (i32, i32), categories: &[EventFeedCategory]) -> (Vec<FamilyRow>, DatedNotes, Vec<i32>, Vec<Unrecorded>) {
    let wanted = |kind: EventFeedKind| categories.contains(&kind.category());
    if !wanted(EventFeedKind::WarStarted) && !wanted(EventFeedKind::FactionJoined) {
        return (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    }
    let mut notes: DatedNotes = Vec::new();
    let within = |hour: i32| from <= hour && hour <= to;
    let mut rows: Vec<FamilyRow> = Vec::new();
    let mut unrecorded: Vec<Unrecorded> = Vec::new();
    if wanted(EventFeedKind::WarStarted) {
        if let Some(reply) = reply {
            for start in reply.war_starts.iter().filter(|start| within(start.hour)) {
                let participants = [start.attacking.clone(), start.defending.clone()];
                let missing = [start.attacking_missing.clone(), start.defending_missing.clone()];
                for reason in missing.iter().flatten() {
                    notes.push((start.hour, format!("War start: {reason}. Country-filtered war-start totals may be incomplete.")));
                }
                let mut row = family_row(EventFeedKind::WarStarted, start.hour, start.hour, start.hour, participants.iter().flatten().cloned().collect(), None, None);
                row.id = format!("WarStarted:{}:{},{}:", start.hour, participants[0].as_deref().unwrap_or_default(), participants[1].as_deref().unwrap_or_default());
                row.participants = Some(participants);
                row.participants_missing = Some(missing);
                rows.push(row.into());
            }
            rows.extend(reply.capitulations.iter().filter(|capitulation| within(capitulation.hour)).map(|capitulation| family_row(EventFeedKind::Capitulated, capitulation.hour, capitulation.hour, capitulation.hour, vec![capitulation.tag.clone()], None, None).into()));
        }
        let mut peace: Vec<(FamilyRow, String)> = Vec::new();
        for end in transitions.war_ends.iter().filter(|end| within(end.hour)) {
            let superseded = native_supersedes(end, native_ends);
            if superseded.is_some_and(|observed| observed <= end.hour) {
                continue;
            }
            peace.push(peace_row(end.hour, &end.parties, superseded));
        }
        disambiguate(&mut peace);
        rows.extend(peace.into_iter().map(|(row, _)| row));
        if !transitions.war_unrecorded.is_empty() {
            unrecorded.push(Unrecorded::of(EventFeedCategory::WarsPeace, ("peace", "Peace transitions"), ("A peace in", "may not be listed: war relations or country identities were not completely recorded then"), transitions.war_unrecorded.iter().copied()));
        }
    }
    if wanted(EventFeedKind::FactionJoined) {
        let mut by_faction: BTreeMap<u64, &groups::FactionGroup> = BTreeMap::new();
        for faction in reply.into_iter().flat_map(|r| &r.factions) {
            notes.extend(faction.names.iter().filter_map(|run| run.reason.as_ref().map(|reason| (run.first_hour, format!("Faction names are unavailable: {reason}")))));
            if let Ok(id) = faction.id.parse::<u64>() {
                by_faction.insert(id, faction);
            }
        }
        let name_item = |faction: u64, hour: i32| {
            let run = by_faction.get(&faction).and_then(|held| held.names.iter().find(|run| run.first_hour <= hour && hour <= run.last_hour));
            let named = run.and_then(|run| run.name.as_deref()).filter(|name| !name.is_empty()).map(str::to_owned);
            let name_missing = named.is_none().then(|| run.and_then(|run| run.reason.clone()).or_else(|| group_err.map(|e| format!("Faction names are unavailable: {e}"))).unwrap_or_else(|| "Faction name not recorded at this hour".to_owned()));
            Some(EventFeedItem { token: run.and_then(|run| run.token.clone()), name: named, icon: None, description: None, name_missing, ..Default::default() })
        };
        let mut faction_rows: Vec<(FamilyRow, String)> = Vec::new();
        for step in transitions.factions.iter().filter(|step| within(step.hour)) {
            let (kind, name_hour) = if step.joined { (EventFeedKind::FactionJoined, step.hour) } else { (EventFeedKind::FactionLeft, step.hour - 1) };
            faction_rows.push(faction_row(kind, step.hour, name_hour, step.faction, &step.party, name_item(step.faction, name_hour)));
        }
        disambiguate(&mut faction_rows);
        rows.extend(faction_rows.into_iter().map(|(row, _)| row));
        if !transitions.faction_unrecorded.is_empty() {
            unrecorded.push(Unrecorded::of(EventFeedCategory::Diplomacy, ("factionMembership", "Faction membership"), ("A faction join or departure in", "may not be listed: faction membership was not completely recorded then"), transitions.faction_unrecorded.iter().copied()));
        }
    }
    let unattributed = if wanted(EventFeedKind::WarStarted) { reply.into_iter().flat_map(|r| &r.unobserved.unattributed).copied().filter(|&hour| within(hour)).collect() } else { Vec::new() };
    (rows, notes, unattributed, unrecorded)
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

#[derive(Clone)]
pub(crate) struct FamilyRow {
    pub row: EventFeedRow,
    icon: Option<special_projects::CompletionIcon>,
    pub slots: Option<Vec<Option<u32>>>,
    pub until: Option<i32>,
}

impl From<EventFeedRow> for FamilyRow {
    fn from(row: EventFeedRow) -> Self {
        Self { row, icon: None, slots: None, until: None }
    }
}

impl FamilyRow {
    pub fn shown<'row>(&'row self, art_warnings: &mut BTreeMap<String, BTreeSet<&'row str>>) -> EventFeedRow {
        let mut row = self.row.clone();
        if let Some((icon, item)) = self.icon.as_ref().zip(row.item.as_mut()) {
            match icon.draw() {
                Ok(icon) => item.icon = Some(icon),
                Err(reason) => {
                    art_warnings.entry(reason).or_default().insert(&self.row.id);
                }
            }
        }
        row
    }
}

pub(crate) fn project_completions(source: &Source, query: &Query<'_>, context: &LoadContext, from: i32, to: i32, cancelled: &AtomicBool) -> io::Result<(Vec<FamilyRow>, Vec<String>, Unrecorded)> {
    let found = special_projects::completions(source, context, query, from, to, cancelled)?;
    let unrecorded = Unrecorded::of(EventFeedCategory::Milestones, ("projectFinished", "Special projects"), ("A completion dated in", "may not be listed: project histories were not recorded then"), found.unrecorded.iter().map(|s| (s.first_hour, s.last_hour)));
    let rows = found.events.into_iter().map(completion_row).collect();
    Ok((rows, found.warnings, unrecorded))
}

pub(super) fn completion_row(event: special_projects::ProjectCompletion) -> FamilyRow {
    let slot = event.attribution.slot();
    let (tag, owner_missing) = match event.attribution {
        special_projects::CompletionOwner::Resolved { tag, .. } => (Some(tag), None),
        special_projects::CompletionOwner::NotRecorded { reason, .. } => (None, Some(reason)),
    };
    let tags: Vec<String> = tag.iter().cloned().collect();
    let item = EventFeedItem { token: None, name: event.name, ..Default::default() };
    let subject = format!("{}:{}:{}", event.owner, event.project, event.ordinal);
    let mut row = family_row(EventFeedKind::ProjectFinished, event.date, event.observed_hour, event.date, tags, Some(subject), Some(item));
    row.participants = Some([tag, None]);
    row.owner_missing = owner_missing;
    FamilyRow { row, icon: event.icon, slots: Some(vec![slot]), until: None }
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

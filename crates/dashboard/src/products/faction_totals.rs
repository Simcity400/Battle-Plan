//! Shared sums of dated country measures over recorded faction rosters.
//! Neither a missing member value nor an unread roster supplies a partial sum.
use super::{
    LoadContext, Product, country_factories,
    country_factories::{FactoryCount, FactoryGroupKey},
    country_groups::{
        self, FactionHistory as History, FactionNameRun, HourSpan, MembershipRun, RosterRun,
    },
    territory::{self, TerritoryMeasure, TerritoryRule},
};
use crate::{Source, metadata::check_cancel, protocol::MAX_EXACT_JS_INTEGER};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

mod native_members;
use native_members::NativeMembers;

pub struct FactionTotals;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum FactionMembership {
    Hour,
    End,
    Ever,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FactionMeasure {
    Territory {
        measure: TerritoryMeasure,
        rule: TerritoryRule,
    },
    Factories {
        mode: FactoryCount,
        groups: Vec<FactoryGroupKey>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactionTotalsParams {
    pub branch_id: String,
    pub source: FactionMeasure,
    pub membership: FactionMembership,
    #[ts(type = "number")]
    pub top: u64,
    /// A chosen faction is returned unranked, using the same sum rule as the race.
    #[serde(default)]
    #[ts(optional)]
    pub faction_id: Option<String>,
    /// Selected member measures and the branch's native country identity census.
    #[serde(default)]
    #[ts(optional)]
    pub parts: Option<bool>,
    /// Native faction inventory, including zero and unknown totals outside rank.
    #[serde(default)]
    #[ts(optional)]
    pub inventory: Option<bool>,
}

/// The adapters retain gaps and identity breaks in the source's dated runs.
#[derive(Clone, Debug)]
pub(crate) struct MeasureRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    pub value: Option<i64>,
}

#[derive(Clone, Debug)]
pub(crate) struct CountryTrack {
    pub tag: String,
    pub runs: Vec<MeasureRun>,
}

pub(crate) struct CountryMeasures {
    pub tracks: Vec<CountryTrack>,
    pub unit: Option<String>,
    pub unavailable_reason: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionTotalRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    // The sum sweep excludes integers a JavaScript number cannot hold exactly.
    #[ts(type = "number")]
    pub value: i64,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionTotalSeries {
    pub id: String,
    pub runs: Vec<FactionTotalRun>,
    /// Names and members are the recorded history, independent of the sum rule.
    pub names: Vec<FactionNameRun>,
    pub members: Vec<MembershipRun>,
    /// The members used in each sum, under the requested membership mode.
    pub counted_members: Vec<MembershipRun>,
    pub membership: Vec<HourSpan>,
    /// End membership uses a dissolved faction's last recorded roster.
    pub dissolution_hour: Option<i32>,
    pub unavailable_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionMemberValueRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    /// Exact signed measure integer, independent of JavaScript's numeric range.
    pub value: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionMemberTrack {
    pub tag: String,
    pub runs: Vec<FactionMemberValueRun>,
    pub identities: Vec<super::country_history::CountryIdentityRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionTotalsReply {
    pub branch_id: String,
    pub membership_mode: FactionMembership,
    /// Hours when the catalogue and every existing faction total are known.
    pub recorded: Vec<HourSpan>,
    /// Hours when all counted country rosters are known and do not overlap.
    pub disjoint: Vec<HourSpan>,
    pub series: Vec<FactionTotalSeries>,
    pub unit: Option<String>,
    pub unavailable_reason: Option<String>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub member_tracks: Option<Vec<FactionMemberTrack>>,
}

fn history(source: &Source, branch: u64, cancelled: &AtomicBool) -> io::Result<Arc<History>> {
    check_cancel(cancelled)?;
    country_groups::cached_scan(source, branch, cancelled)?.faction_history(cancelled)
}

/// Visits complete country reads minus a tag's presence, in hour order. The
/// caller can count or copy proved zeros without first allocating their history.
pub(crate) fn for_each_absent(
    rosters: &[HourSpan],
    present: &[HourSpan],
    cancelled: &AtomicBool,
    mut visit: impl FnMut(i32, i32) -> io::Result<()>,
) -> io::Result<()> {
    check_cancel(cancelled)?;
    let mut index = 0usize;
    let mut steps = 0usize;
    for read in rosters {
        check_cancel(cancelled)?;
        let mut first = i64::from(read.first_hour);
        let last = i64::from(read.last_hour);
        while let Some(span) = present.get(index) {
            steps += 1;
            if steps.is_multiple_of(4096) {
                check_cancel(cancelled)?;
            }
            if i64::from(span.first_hour) > last {
                break;
            }
            if first < i64::from(span.first_hour) {
                visit(first as i32, (i64::from(span.first_hour) - 1) as i32)?;
            }
            first = first.max(i64::from(span.last_hour) + 1);
            // A presence spanning more than one read is needed by the next read
            // too. Widened hour arithmetic also keeps i32::MAX from wrapping.
            if i64::from(span.last_hour) > last {
                break;
            }
            index += 1;
        }
        if first <= last {
            visit(first as i32, last as i32)?;
        }
    }
    Ok(())
}

pub(crate) fn push_measure(
    runs: &mut Vec<MeasureRun>,
    run: MeasureRun,
    count: &mut usize,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    *count += 1;
    if count.is_multiple_of(4096) {
        check_cancel(cancelled)?;
    }
    runs.push(run);
    Ok(())
}

type EffectiveRoster = (i32, i32, Option<Arc<[usize]>>);

#[derive(Clone)]
enum Event {
    Country(usize, Option<i64>, bool),
    // Outer None ends existence; inner None is an existing but unknown roster.
    Roster(usize, Option<Option<Arc<[usize]>>>),
    Catalogue(bool),
}

#[derive(Default)]
struct Total {
    members: Arc<[usize]>,
    known: usize,
    value: i128,
    active: bool,
    roster_known: bool,
    since: i64,
    starts_segment: bool,
    runs: Vec<FactionTotalRun>,
}

impl Total {
    fn complete(&self) -> bool {
        self.active && self.roster_known && self.known == self.members.len()
    }

    fn recorded_value(&self) -> Option<i64> {
        if !self.complete() {
            return None;
        }
        let value = i64::try_from(self.value).ok()?;
        (value.unsigned_abs() <= MAX_EXACT_JS_INTEGER).then_some(value)
    }

    fn flush(&mut self, end: i64) -> usize {
        if self.since >= end {
            return 0;
        }
        let Some(value) = self.recorded_value() else {
            // Count faction-hours, so a changed event boundary cannot hide overflow.
            return if self.complete() {
                (end - self.since) as usize
            } else {
                0
            };
        };
        let first_hour = self.since as i32;
        let last_hour = (end - 1) as i32;
        match self.runs.last_mut() {
            Some(previous)
                if !self.starts_segment
                    && previous.last_hour.checked_add(1) == Some(first_hour)
                    && previous.value == value =>
            {
                previous.last_hour = last_hour
            }
            _ => self.runs.push(FactionTotalRun {
                first_hour,
                last_hour,
                starts_segment: self.starts_segment,
                value,
            }),
        }
        0
    }
}

#[derive(Debug, Default)]
struct Sums {
    series: BTreeMap<String, Vec<FactionTotalRun>>,
    recorded: Vec<HourSpan>,
    disjoint: Vec<HourSpan>,
    overflowed: usize,
    #[cfg(test)]
    roster_events: usize,
    #[cfg(test)]
    member_visits: usize,
}

fn incomplete_ever(history: &History, rosters: &[RosterRun]) -> bool {
    // A missing catalogue can hide this faction before its first or after its
    // last readable roster too; a fixed ever set requires the whole branch.
    rosters.is_empty() || rosters.iter().any(|run| !run.complete) || !history.unobserved.is_empty()
}

fn roster_reason(
    history: &History,
    rosters: &[RosterRun],
    membership: FactionMembership,
) -> Option<String> {
    match membership {
        FactionMembership::Ever if incomplete_ever(history, rosters) => Some(
            "Anyone ever cannot be determined because this faction's roster was not completely recorded".into()),
        FactionMembership::End if !history.end_recorded => Some(
            "This faction's membership was not recorded at the end of this campaign branch".into()),
        FactionMembership::End if rosters.last().is_none_or(|run| !run.complete) => Some(
            "This faction's last recorded member list is incomplete, so its total at the end is not known".into()),
        FactionMembership::End if rosters.last().is_some_and(|run|
            run.last_hour < history.last_hour && !dissolution_recorded(history, run.last_hour)) => Some(
            "This faction's dissolution was not recorded, so its last member list cannot establish its total at the end".into()),
        _ => None,
    }
}

fn dissolution_recorded(history: &History, last_hour: i32) -> bool {
    let Some(hour) = last_hour.checked_add(1) else {
        return false;
    };
    let index = history
        .faction_reads
        .partition_point(|span| span.last_hour < hour);
    history
        .faction_reads
        .get(index)
        .is_some_and(|span| span.first_hour <= hour)
}

fn dissolution_hour(
    history: &History,
    rosters: &[RosterRun],
    membership: FactionMembership,
) -> Option<i32> {
    if membership != FactionMembership::End || !history.end_recorded {
        return None;
    }
    rosters
        .last()
        .filter(|run| {
            run.last_hour < history.last_hour && dissolution_recorded(history, run.last_hour)
        })
        .map(|run| run.last_hour)
}

/// Country changes visit only their current factions. Proof spans use the same
/// sweep, so neither ranking nor a world-share view needs a second sum rule.
fn sum_prepared(
    history: &History,
    tracks: &[Vec<MeasureRun>],
    rosters: &BTreeMap<String, Vec<EffectiveRoster>>,
    cancelled: &AtomicBool,
) -> io::Result<Sums> {
    check_cancel(cancelled)?;
    let faction_ids: Vec<_> = history.factions.keys().collect();
    let mut events: BTreeMap<i64, Vec<Event>> = BTreeMap::new();
    for span in &history.faction_reads {
        events
            .entry(i64::from(span.first_hour))
            .or_default()
            .push(Event::Catalogue(true));
        events
            .entry(i64::from(span.last_hour) + 1)
            .or_default()
            .push(Event::Catalogue(false));
    }
    for (index, track) in tracks.iter().enumerate() {
        check_cancel(cancelled)?;
        for run in track {
            events
                .entry(i64::from(run.last_hour) + 1)
                .or_default()
                .push(Event::Country(index, None, false));
            events
                .entry(i64::from(run.first_hour))
                .or_default()
                .push(Event::Country(index, run.value, run.starts_segment));
        }
    }
    #[cfg(test)]
    let mut effective_rosters = 0usize;
    for (index, rosters) in rosters.values().enumerate() {
        check_cancel(cancelled)?;
        for (first, last, members) in rosters {
            #[cfg(test)]
            {
                effective_rosters += 1;
            }
            events
                .entry(i64::from(*last) + 1)
                .or_default()
                .push(Event::Roster(index, None));
            events
                .entry(i64::from(*first))
                .or_default()
                .push(Event::Roster(index, Some(members.clone())));
        }
    }
    let mut current = vec![None; tracks.len()];
    let mut subscribers: Vec<HashSet<usize>> = vec![HashSet::new(); tracks.len()];
    let mut totals: Vec<Total> = (0..faction_ids.len()).map(|_| Total::default()).collect();
    let mut sums = Sums::default();
    #[cfg(test)]
    {
        sums.roster_events = effective_rosters;
    }
    let (mut catalogue, mut unknown, mut overlaps) = (false, 0usize, 0usize);
    let mut previous_hour = None;
    for (hour, mut changes) in events {
        check_cancel(cancelled)?;
        if let Some(first) = previous_hour
            && catalogue
            && unknown == 0
            && first < hour
        {
            country_groups::mark_unobserved(&mut sums.recorded, first as i32, (hour - 1) as i32);
            if overlaps == 0 {
                country_groups::mark_unobserved(
                    &mut sums.disjoint,
                    first as i32,
                    (hour - 1) as i32,
                );
            }
        }
        previous_hour = Some(hour);
        let mut affected = HashSet::new();
        for change in &changes {
            match change {
                Event::Country(index, _, _) => affected.extend(subscribers[*index].iter().copied()),
                Event::Roster(index, _) => {
                    affected.insert(*index);
                }
                Event::Catalogue(_) => {}
            }
        }
        for &index in &affected {
            let total = &mut totals[index];
            unknown -= usize::from(total.active && total.recorded_value().is_none());
            sums.overflowed = sums.overflowed.saturating_add(total.flush(hour));
            total.since = hour;
            total.starts_segment = total.recorded_value().is_none();
        }
        // Ends precede starts, and new rosters see this boundary's country values.
        changes.sort_by_key(|change| match change {
            Event::Country(_, None, _) => 0,
            Event::Country(_, Some(_), _) => 1,
            Event::Roster(_, None) => 2,
            Event::Roster(_, Some(_)) => 3,
            Event::Catalogue(false) => 4,
            Event::Catalogue(true) => 5,
        });
        for change in changes {
            match change {
                Event::Country(index, value, starts_segment) => {
                    for &faction in &subscribers[index] {
                        let total = &mut totals[faction];
                        if let Some(previous) = current[index] {
                            total.known -= 1;
                            total.value -= i128::from(previous);
                        }
                        if let Some(value) = value {
                            total.known += 1;
                            total.value += i128::from(value);
                        }
                        total.starts_segment |= starts_segment;
                    }
                    current[index] = value;
                }
                Event::Roster(index, roster) => {
                    let total = &mut totals[index];
                    #[cfg(test)]
                    {
                        sums.member_visits += total.members.len();
                    }
                    for &member in total.members.iter() {
                        let subscribers = &mut subscribers[member];
                        overlaps -= usize::from(subscribers.len() > 1);
                        subscribers.remove(&index);
                        overlaps += usize::from(subscribers.len() > 1);
                    }
                    total.active = roster.is_some();
                    total.roster_known = roster.as_ref().is_some_and(Option::is_some);
                    total.members = roster.flatten().unwrap_or_default();
                    total.known = 0;
                    total.value = 0;
                    #[cfg(test)]
                    {
                        sums.member_visits += total.members.len();
                    }
                    for &member in total.members.iter() {
                        let subscribers = &mut subscribers[member];
                        overlaps -= usize::from(subscribers.len() > 1);
                        subscribers.insert(index);
                        overlaps += usize::from(subscribers.len() > 1);
                        if let Some(value) = current[member] {
                            total.known += 1;
                            total.value += i128::from(value);
                        }
                    }
                }
                Event::Catalogue(known) => catalogue = known,
            }
        }
        for &index in &affected {
            let total = &totals[index];
            unknown += usize::from(total.active && total.recorded_value().is_none());
        }
    }
    sums.series = faction_ids
        .into_iter()
        .zip(totals)
        .map(|(id, total)| (id.clone(), total.runs))
        .collect();
    Ok(sums)
}

fn member_runs(rosters: &[RosterRun]) -> Vec<MembershipRun> {
    let mut runs: Vec<MembershipRun> = Vec::new();
    let mut open: HashMap<&String, usize> = HashMap::new();
    for roster in rosters {
        if !roster.complete {
            continue;
        }
        for tag in &roster.members {
            if let Some(&index) = open.get(tag)
                && let Some(previous) = runs.get_mut(index)
                && previous.last_hour.checked_add(1) == Some(roster.first_hour)
            {
                previous.last_hour = roster.last_hour;
                continue;
            }
            open.insert(tag, runs.len());
            runs.push(MembershipRun {
                tag: tag.clone(),
                first_hour: roster.first_hour,
                last_hour: roster.last_hour,
            });
        }
    }
    runs
}

impl Product for FactionTotals {
    const KEY: &'static str = "faction-totals";
    const CONTENT_DEPENDENT: bool = true;
    type Params = FactionTotalsParams;
    type Reply = FactionTotalsReply;

    fn types() -> Vec<String> {
        vec![
            FactionTotalsParams::decl(),
            FactionMembership::decl(),
            FactionMeasure::decl(),
            FactionTotalRun::decl(),
            FactionNameRun::decl(),
            FactionTotalSeries::decl(),
            FactionMemberValueRun::decl(),
            FactionMemberTrack::decl(),
            FactionTotalsReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid faction totals branch");
        }
        if params.top == 0 {
            return Err("Invalid top factions");
        }
        if params.parts == Some(true) && params.faction_id.is_none() {
            return Err("Faction member parts require a selected faction");
        }
        if params
            .faction_id
            .as_ref()
            .is_some_and(|id| id.parse::<u64>().is_err())
        {
            return Err("Invalid faction id");
        }
        if let FactionMeasure::Factories { groups, .. } = &params.source {
            let unique: BTreeSet<_> = groups.iter().collect();
            if unique.len() != groups.len() {
                return Err("Invalid factory groups");
            }
        }
        Ok(())
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().expect("validated branch");
        let history = history(source, branch, cancelled)?;
        let identities =
            super::country_history::branch_histories(source, branch, false, &[], cancelled)?;
        let mut members = NativeMembers::new(&history, &identities, params.membership, cancelled)?;
        let tags = members.tags();
        let measures = match &params.source {
            FactionMeasure::Territory { measure, rule } => territory::faction_tracks(
                source, branch, &tags, *measure, *rule, context, cancelled,
            )?,
            FactionMeasure::Factories { mode, groups } => country_factories::faction_tracks(
                source, branch, &tags, *mode, groups, &history, cancelled,
            )?,
        };
        let native_measures = members.measures(&measures.tracks, cancelled)?;
        let sums = sum_prepared(&history, &native_measures, &members.rosters, cancelled)?;
        let totals = &sums.series;
        let ranked = if params.faction_id.is_some() || params.inventory == Some(true) {
            if let Some(id) = &params.faction_id
                && !history.factions.contains_key(id)
            {
                return Err(crate::invalid("This faction is not in this recording"));
            }
            history
                .factions
                .iter()
                .filter(|(id, _)| {
                    params
                        .faction_id
                        .as_ref()
                        .is_none_or(|chosen| chosen == *id)
                })
                .map(|(id, rosters)| {
                    let mut spans = Vec::new();
                    for run in rosters {
                        country_groups::mark_unobserved(&mut spans, run.first_hour, run.last_hour);
                    }
                    (id.clone(), spans)
                })
                .collect()
        } else {
            crate::ranking::top_membership(
                totals,
                params.top.min(totals.len() as u64) as usize,
                |run| (run.first_hour, run.last_hour),
                |run| Some(run.value),
                cancelled,
            )?
        };
        let mut warnings = measures.warnings;
        if sums.overflowed > 0 {
            warnings.push(format!("{} faction-total hours exceeded the exact integer range; affected totals are gaps.", sums.overflowed));
        }
        let refused_rosters = members.reasons.len();
        if refused_rosters > 0 {
            warnings.push(format!("{refused_rosters} faction member sets could not be established for this membership rule; all their totals are gaps."));
        }
        if members.unread_hours > 0 {
            warnings.push(format!("{} faction-member hours had no recorded native identity; affected totals are gaps.", members.unread_hours));
        }
        if identities.unresolved_countries > 0 || identities.unnamed > 0 {
            warnings.push(format!("{} country identity reads could not resolve a native holder or tag; affected faction member sets and totals stay unknown.", identities.unresolved_countries + identities.unnamed));
        }
        if history.incomplete > 0 {
            warnings.push(format!(
                "{} faction roster reads were incomplete or unavailable; affected totals are gaps.",
                history.incomplete
            ));
        }
        let needs_templates = ranked.keys().any(|id| {
            history.names[&id.parse::<u64>().expect("recorded faction id")]
                .iter()
                .any(country_groups::NameOperands::needs_templates)
        });
        let mut names = country_groups::with_name_content(
            source,
            context,
            needs_templates,
            &mut warnings,
            |templates, localisation, language, refused| {
                ranked
                    .keys()
                    .map(|id| {
                        let raw = id.parse::<u64>().expect("recorded faction id");
                        let runs = country_groups::name_runs(
                            raw,
                            &history.names[&raw],
                            templates.as_ref().copied().map_err(Clone::clone),
                            localisation,
                            language,
                            refused,
                        );
                        (id.clone(), runs)
                    })
                    .collect::<BTreeMap<_, _>>()
            },
        );
        let mut series = Vec::with_capacity(ranked.len());
        for (id, membership) in ranked {
            check_cancel(cancelled)?;
            let rosters = &history.factions[&id];
            let mut runs = Vec::new();
            crate::ranking::for_each_optional_piece(
                &totals[&id],
                Some(&membership),
                |run| (run.first_hour, run.last_hour),
                cancelled,
                |run, first_hour, last_hour| {
                    runs.push(FactionTotalRun {
                        first_hour,
                        last_hour,
                        value: run.value,
                        starts_segment: run.starts_segment && first_hour == run.first_hour,
                    });
                    Ok(())
                },
            )?;
            let names = names.remove(&id).expect("ranked faction name reads");
            series.push(FactionTotalSeries {
                unavailable_reason: members.reasons.remove(&id),
                dissolution_hour: dissolution_hour(&history, rosters, params.membership),
                counted_members: members
                    .counted
                    .remove(&id)
                    .expect("prepared faction members"),
                id,
                runs,
                names,
                members: member_runs(rosters),
                membership,
            });
        }
        let member_tracks = if params.parts == Some(true) {
            let members: BTreeSet<_> = series
                .iter()
                .flat_map(|series| &series.counted_members)
                .map(|member| member.tag.as_str())
                .collect();
            let mut tracks = Vec::with_capacity(identities.countries.len());
            let values: BTreeMap<_, _> = measures
                .tracks
                .iter()
                .map(|track| (track.tag.as_str(), track))
                .collect();
            for (tag, country) in &identities.countries {
                check_cancel(cancelled)?;
                tracks.push(FactionMemberTrack {
                    tag: tag.to_owned(),
                    runs: members
                        .contains(tag.as_str())
                        .then(|| values.get(tag.as_str()))
                        .flatten()
                        .map(|track| {
                            track
                                .runs
                                .iter()
                                .map(|run| FactionMemberValueRun {
                                    first_hour: run.first_hour,
                                    last_hour: run.last_hour,
                                    starts_segment: run.starts_segment,
                                    value: run.value.map(|value| value.to_string()),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    identities: country.identities.as_ref().clone(),
                });
            }
            Some(tracks)
        } else {
            None
        };
        let reply = FactionTotalsReply {
            branch_id: params.branch_id.clone(),
            membership_mode: params.membership,
            series,
            recorded: sums.recorded,
            disjoint: sums.disjoint,
            unit: measures.unit,
            unavailable_reason: measures.unavailable_reason,
            warnings,
            member_tracks,
        };
        super::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;

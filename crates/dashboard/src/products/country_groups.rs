//! Who belonged to each smart country group, hour by hour, over one branch:
//! human players, every recorded faction (including ones founded or dissolved
//! partway through) and the countries at war with each country.
//!
//! Membership is kept as runs of consecutive recorded hours; a gap or an hour
//! whose tables are not complete ends every run, so nothing is carried across
//! missing data. Those hours are listed as unobserved for each group history
//! (up to the branch's own first and last hour), so a break there is not
//! mistaken for a country joining or leaving. A war counts only while its
//! recorded relation is ongoing. Faction names are the recorded
//! engine names, localised when they are a localisation key. Which member leads
//! a faction is not recorded: member order is kept, but order zero only means
//! "first member".

use super::{LoadContext, Product};
use crate::{
    Source,
    catalogue::{self, Localised},
    content,
    metadata::{branch_bounds, validate_campaign},
};
use installed_content::localisation::Localisation;
use replay::{
    container::Replay,
    frame::{FrameDecoder, SharedView},
    rows::tables,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct CountryGroups;

/// Runs kept per branch before the scan refuses.
const MAX_RUNS: usize = 500_000;
/// Branches kept per source, so tracks on different branches do not evict each other.
const CACHED_BRANCHES: usize = 4;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryGroupParams {
    pub branch_id: String,
}

/// A country's unbroken membership of a group.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct MembershipRun {
    pub tag: String,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionGroup {
    /// The recorded faction id, as a decimal string.
    pub id: String,
    /// The faction's name as the game showed it; null when it cannot be resolved.
    pub name: Option<String>,
    pub first_hour: i32,
    pub last_hour: i32,
    pub members: Vec<MembershipRun>,
}

/// Two countries at war with each other over consecutive recorded hours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarPairRun {
    pub first: String,
    pub second: String,
    pub first_hour: i32,
    pub last_hour: i32,
}

/// Consecutive hours of the branch whose group tables were not observed.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct HourSpan {
    pub first_hour: i32,
    pub last_hour: i32,
}

/// Hours of the branch whose memberships were not observed, per group history.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnobservedHours {
    pub humans: Vec<HourSpan>,
    pub factions: Vec<HourSpan>,
    pub wars: Vec<HourSpan>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryGroupReply {
    pub branch_id: String,
    pub humans: Vec<MembershipRun>,
    pub factions: Vec<FactionGroup>,
    pub wars: Vec<WarPairRun>,
    /// Hours inside the branch where each history was not observed: a run that
    /// ends or starts next to one of these is a gap, not a join or a leave.
    pub unobserved: UnobservedHours,
    pub warnings: Vec<String>,
}

impl Product for CountryGroups {
    const KEY: &'static str = "country-groups";
    type Params = CountryGroupParams;
    type Reply = CountryGroupReply;

    fn types() -> Vec<String> {
        vec![
            CountryGroupParams::decl(),
            MembershipRun::decl(),
            FactionGroup::decl(),
            WarPairRun::decl(),
            HourSpan::decl(),
            UnobservedHours::decl(),
            CountryGroupReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid country group branch")
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let state = source.state::<Vec<(u64, Arc<Scan>)>>();
        let cached = {
            let state = state.lock().unwrap_or_else(|error| error.into_inner());
            state
                .iter()
                .find(|(cached, _)| *cached == branch)
                .map(|(_, scan)| Arc::clone(scan))
        };
        // Scan outside the lock, so requests for other branches are not held up.
        let scan = match cached {
            Some(scan) => scan,
            None => {
                let scan = Arc::new(scan_branch(&source.replay(), branch, cancelled)?);
                let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
                state.retain(|(cached, _)| *cached != branch);
                state.insert(0, (branch, Arc::clone(&scan)));
                state.truncate(CACHED_BRANCHES);
                scan
            }
        };
        let mut warnings = Vec::new();
        let localisation = match context.content.as_ref() {
            Some(content) => match content::localisation(source, content) {
                Ok(localisation) => Some(localisation),
                Err(error) => {
                    warnings.push(format!("Faction names show as recorded: {error}"));
                    None
                }
            },
            None => {
                warnings.push(
                    "Faction names show as recorded until the game folders are chosen.".into(),
                );
                None
            }
        };
        let factions = scan
            .factions
            .iter()
            .map(|(id, faction)| FactionGroup {
                id: id.to_string(),
                name: faction_name(&faction.name, localisation.as_deref()),
                first_hour: faction.first_hour,
                last_hour: faction.last_hour,
                members: faction.members.clone(),
            })
            .collect();
        warnings.extend(scan.warnings.iter().cloned());
        Ok(CountryGroupReply {
            branch_id: params.branch_id.clone(),
            humans: scan.humans.clone(),
            factions,
            wars: scan.wars.clone(),
            unobserved: scan.unobserved.clone(),
            warnings,
        })
    }
}

/// The engine stores either a localisation key or the literal name a player
/// typed. A key is shown localised; a key whose text needs runtime scope has no
/// name rather than showing the key; anything the localisation does not know is
/// the recorded literal name.
pub fn faction_name(recorded: &str, localisation: Option<&Localisation>) -> Option<String> {
    if recorded.is_empty() {
        return None;
    }
    match localisation.map(|localisation| catalogue::localise(localisation, recorded)) {
        Some(Localised::Text(text)) => Some(text),
        Some(Localised::NeedsScope) => None,
        Some(Localised::Missing) | None => Some(recorded.to_owned()),
    }
}

struct ScannedFaction {
    name: String,
    first_hour: i32,
    last_hour: i32,
    members: Vec<MembershipRun>,
}

/// One branch's group history, before names are localised.
pub struct Scan {
    humans: Vec<MembershipRun>,
    factions: BTreeMap<u64, ScannedFaction>,
    wars: Vec<WarPairRun>,
    unobserved: UnobservedHours,
    warnings: Vec<String>,
}

/// A run of consecutive hours.
trait HourRun {
    fn last_hour(&self) -> i32;
    fn set_last_hour(&mut self, hour: i32);
}
impl HourRun for MembershipRun {
    fn last_hour(&self) -> i32 {
        self.last_hour
    }
    fn set_last_hour(&mut self, hour: i32) {
        self.last_hour = hour;
    }
}
impl HourRun for WarPairRun {
    fn last_hour(&self) -> i32 {
        self.last_hour
    }
    fn set_last_hour(&mut self, hour: i32) {
        self.last_hour = hour;
    }
}

/// Extends the open run for `key` when `hour` follows its last hour, else starts
/// one with `start`. A second row for the same key in the same hour changes
/// nothing. Returns whether a run was started.
fn extend<K: Ord, R: HourRun>(
    open: &mut BTreeMap<K, usize>,
    runs: &mut Vec<R>,
    key: K,
    hour: i32,
    start: impl FnOnce() -> R,
) -> bool {
    if let Some(&index) = open.get(&key) {
        let last = runs[index].last_hour();
        if last == hour {
            return false;
        }
        if last.checked_add(1) == Some(hour) {
            runs[index].set_last_hour(hour);
            return false;
        }
    }
    open.insert(key, runs.len());
    runs.push(start());
    true
}

/// Whether a recorded war relation is ongoing at `hour`. The engine writes an end
/// only when the raw start is below the raw end, so a row whose end is not after
/// its start has no end; the viewer reads it the same way (`allegiance::ongoing`
/// in the viewer crate, verified by the recorder's war-relation oracle).
fn war_ongoing(start_hour: i32, end_hour: i32, hour: i32) -> bool {
    end_hour <= start_hour || hour < end_hour
}

/// Marks hours unobserved for every group history.
fn mark_all(unobserved: &mut UnobservedHours, first_hour: i32, last_hour: i32) {
    for spans in [
        &mut unobserved.humans,
        &mut unobserved.factions,
        &mut unobserved.wars,
    ] {
        mark_unobserved(spans, first_hour, last_hour);
    }
}

/// Adds `hour` to the unobserved spans, merging it with the span it follows.
fn mark_unobserved(spans: &mut Vec<HourSpan>, first_hour: i32, last_hour: i32) {
    match spans.last_mut() {
        Some(span) if span.last_hour.checked_add(1) >= Some(first_hour) => {
            span.last_hour = span.last_hour.max(last_hour);
        }
        _ => spans.push(HourSpan {
            first_hour,
            last_hour,
        }),
    }
}

pub fn scan_branch(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Scan> {
    use replay::rows::TableId;
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::HumanControllers,
        TableId::Factions,
        TableId::FactionMembers,
        TableId::WarRelations,
    ];
    let cancel = || {
        io::Error::new(
            io::ErrorKind::Interrupted,
            "Country group request cancelled",
        )
    };
    let mut humans = Vec::new();
    let mut human_open = BTreeMap::new();
    let mut factions: BTreeMap<u64, ScannedFaction> = BTreeMap::new();
    let mut member_open: BTreeMap<(u64, String), usize> = BTreeMap::new();
    let mut wars: Vec<WarPairRun> = Vec::new();
    let mut war_open: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut unobserved = UnobservedHours::default();
    let mut unmapped = BTreeSet::new();
    let mut incomplete = 0_u32;
    let mut unreadable_blocks = 0_u32;
    let mut stopped = 0_u32;
    let mut runs = 0_usize;
    // Hours before the first readable frame and after the last are unobserved too,
    // so a branch that starts or ends in a gap or an unreadable block shows no
    // false joins or leaves there.
    let bounds = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .map(|start| branch_bounds(replay, start));
    let mut previous_hour: Option<i32> = bounds.and_then(|(first, _)| first.checked_sub(1));
    // Link id to tag, rebuilt only when the country tables change (unchanged tables
    // share one allocation from hour to hour). The views are held, not just their
    // addresses, so a freed table's address can never be mistaken for it.
    let mut tags: HashMap<u32, String> = HashMap::new();
    let mut tags_from: Option<(SharedView, SharedView)> = None;
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        if cancelled.load(Ordering::Acquire) {
            return Err(cancel());
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            unreadable_blocks += 1;
            continue;
        };
        for frame in frames {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancel());
            }
            let Ok(decoded) = decoder.advance(&frame) else {
                stopped += 1;
                break;
            };
            if validate_campaign(decoded, replay).is_err() {
                stopped += 1;
                break;
            }
            let hour = frame.hour;
            if let Some(previous) = previous_hour
                && hour > previous + 1
            {
                mark_all(&mut unobserved, previous + 1, hour - 1);
            }
            previous_hour = Some(hour);
            let (Ok(countries), Ok(links)) = (
                decoded.complete::<tables::Countries>(),
                decoded.complete::<tables::CountryLinks>(),
            ) else {
                incomplete += 1;
                mark_all(&mut unobserved, hour, hour);
                continue;
            };
            let views = (
                decoded.tables[replay::rows::TableId::Countries.index()].clone(),
                decoded.tables[replay::rows::TableId::CountryLinks.index()].clone(),
            );
            let same = tags_from.as_ref().is_some_and(|(countries, links)| {
                SharedView::ptr_eq(countries, &views.0) && SharedView::ptr_eq(links, &views.1)
            });
            if !same {
                let slots: HashMap<u32, &[u8]> = countries
                    .iter()
                    .map(|country| (country.canonical_slot, country.tag.as_slice()))
                    .collect();
                tags = links
                    .iter()
                    .filter_map(|link| {
                        let tag = std::str::from_utf8(slots.get(&link.canonical_slot)?).ok()?;
                        (!tag.is_empty()).then(|| (link.id, tag.to_owned()))
                    })
                    .collect();
                tags_from = Some(views);
            }
            // Every group table names countries by their raw link id.
            let tag_of = |raw: u32, unmapped: &mut BTreeSet<u32>| -> Option<&String> {
                let tag = tags.get(&raw);
                if tag.is_none() {
                    unmapped.insert(raw);
                }
                tag
            };
            // Each history is unobserved only where its own tables are incomplete.
            let mut missed = false;
            match decoded.complete::<tables::HumanControllers>() {
                Ok(rows) => {
                    for row in rows.iter().filter(|row| row.count > 0) {
                        if let Some(tag) = tag_of(row.country, &mut unmapped) {
                            let started =
                                extend(&mut human_open, &mut humans, tag.clone(), hour, || {
                                    MembershipRun {
                                        tag: tag.clone(),
                                        first_hour: hour,
                                        last_hour: hour,
                                    }
                                });
                            runs += usize::from(started);
                        }
                    }
                }
                Err(_) => {
                    missed = true;
                    mark_unobserved(&mut unobserved.humans, hour, hour);
                }
            }
            match (
                decoded.complete::<tables::Factions>(),
                decoded.complete::<tables::FactionMembers>(),
            ) {
                (Ok(rows), Ok(members)) => {
                    for row in rows {
                        let name = String::from_utf8_lossy(&row.name);
                        let faction = factions.entry(row.id).or_insert_with(|| ScannedFaction {
                            name: name.clone().into_owned(),
                            first_hour: hour,
                            last_hour: hour,
                            members: Vec::new(),
                        });
                        faction.last_hour = hour;
                        // A renamed faction keeps its id; the latest name is shown.
                        if faction.name != name {
                            faction.name = name.into_owned();
                        }
                    }
                    let mut ordered: Vec<_> = members.iter().collect();
                    ordered.sort_by_key(|member| (member.faction, member.ordinal));
                    for member in ordered {
                        let Some(faction) = factions.get_mut(&member.faction) else {
                            continue;
                        };
                        if let Some(tag) = tag_of(member.country, &mut unmapped) {
                            let started = extend(
                                &mut member_open,
                                &mut faction.members,
                                (member.faction, tag.clone()),
                                hour,
                                || MembershipRun {
                                    tag: tag.clone(),
                                    first_hour: hour,
                                    last_hour: hour,
                                },
                            );
                            runs += usize::from(started);
                        }
                    }
                }
                _ => {
                    missed = true;
                    mark_unobserved(&mut unobserved.factions, hour, hour);
                }
            }
            match decoded.complete::<tables::WarRelations>() {
                Ok(rows) => {
                    for row in rows
                        .iter()
                        .filter(|row| war_ongoing(row.start_hour, row.end_hour, hour))
                    {
                        let (Some(first), Some(second)) = (
                            tag_of(row.first_country, &mut unmapped),
                            tag_of(row.second_country, &mut unmapped),
                        ) else {
                            continue;
                        };
                        // A war is one pair whichever side the row lists first.
                        let (first, second) = if first <= second {
                            (first, second)
                        } else {
                            (second, first)
                        };
                        let started = extend(
                            &mut war_open,
                            &mut wars,
                            (first.clone(), second.clone()),
                            hour,
                            || WarPairRun {
                                first: first.clone(),
                                second: second.clone(),
                                first_hour: hour,
                                last_hour: hour,
                            },
                        );
                        runs += usize::from(started);
                    }
                }
                Err(_) => {
                    missed = true;
                    mark_unobserved(&mut unobserved.wars, hour, hour);
                }
            }
            if missed {
                incomplete += 1;
            }
            if runs > MAX_RUNS {
                return Err(io::Error::other(
                    "Country group history exceeds its memory budget",
                ));
            }
        }
    }
    if let (Some((first, last)), previous) = (bounds, previous_hour) {
        let from = previous.map_or(first, |previous| previous.saturating_add(1));
        if from <= last {
            mark_all(&mut unobserved, from, last);
        }
    }
    let mut warnings = Vec::new();
    if !unmapped.is_empty() {
        warnings.push(format!(
            "{} recorded country references had no country identity and are left out of groups.",
            unmapped.len()
        ));
    }
    if incomplete > 0 {
        warnings.push(format!(
            "Group tables were incomplete at {incomplete} hours; memberships break there rather than carry over."
        ));
    }
    if unreadable_blocks > 0 || stopped > 0 {
        warnings.push(format!(
            "Part of this branch could not be read ({unreadable_blocks} unreadable blocks, {stopped} stopped reads); group history is missing there."
        ));
    }
    Ok(Scan {
        humans,
        factions,
        wars,
        unobserved,
        warnings,
    })
}

#[cfg(test)]
impl Scan {
    pub fn humans(&self) -> &[MembershipRun] {
        &self.humans
    }
    pub fn wars(&self) -> &[WarPairRun] {
        &self.wars
    }
    pub fn unobserved(&self) -> &UnobservedHours {
        &self.unobserved
    }
    pub fn faction(&self, id: u64) -> Option<(&str, i32, i32, &[MembershipRun])> {
        self.factions.get(&id).map(|faction| {
            (
                faction.name.as_str(),
                faction.first_hour,
                faction.last_hour,
                faction.members.as_slice(),
            )
        })
    }
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

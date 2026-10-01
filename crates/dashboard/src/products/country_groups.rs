//! Who belonged to each smart country group, hour by hour, over one branch:
//! human players, every recorded faction (including ones founded or dissolved
//! partway through) and the countries at war with each country.
//!
//! Membership is kept as runs of consecutive recorded hours; a gap or an hour
//! whose tables are not complete ends every run, so nothing is carried across
//! missing data. Those hours are listed as unobserved for each group history
//! (up to the branch's own first and last hour), so a break there is not
//! mistaken for a country joining or leaving. An hour whose list names a
//! country the links cannot resolve is unobserved for that history too, since
//! the list is incomplete, though the runs of the countries it names go on.
//! A war counts only while its
//! recorded relation is ongoing. Faction names are the recorded
//! engine names, localised when they are a localisation key; a faction formed
//! from a template has no literal name (its template is not recorded), so its
//! name is null.
//!
//! A faction's leader is its member at order zero. The engine keeps the leader
//! first in the faction's member vector (`engine::cfaction::MEMBERS`,
//! `engine::cfaction_system::MemberRow::ordinal`), the save writes the members in
//! that order (`installed_content::save::Faction::members`), and the fresh-save oracle
//! compares every recorded ordinal with the save's order (`compare_factions` in
//! the recorder). A 1936 save lists each faction's leader first (GER, ENG, SOV,
//! JAP). An hour whose order-zero member the links cannot name has no known
//! leader, and that faction's roster is incomplete then.

use super::{LoadContext, Product};
use crate::{
    Source, catalogue, content,
    metadata::{LinkTags, branch_bounds, validate_campaign},
};
use installed_content::localisation::Localisation;
use replay::{container::Replay, frame::FrameDecoder, rows::tables};
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
/// War starts and capitulations kept per branch.
const MAX_EVENTS: usize = 100_000;
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
    /// The name as recorded (the engine's literal: a localisation key or a player's
    /// text), before localisation; empty when the faction has none. Stable across
    /// languages and content, so settings can key a faction by it.
    pub recorded_name: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub members: Vec<MembershipRun>,
    /// Who led the faction (its member at order zero), as runs of consecutive
    /// recorded hours; a change of leader ends one run and starts another.
    pub leaders: Vec<MembershipRun>,
    /// Hours this faction's roster named a country the links cannot resolve: its
    /// roster is not known then (other factions' rosters are).
    pub incomplete: Vec<HourSpan>,
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

/// The start of a war between two countries as the engine dates it (the war
/// relation's start hour). `attacking` is the instigating side's country
/// (`first_was_instigator`); it may be an ally that joined rather than the
/// country that declared, which the relation does not record.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarStart {
    pub hour: i32,
    pub attacking: String,
    pub defending: String,
}

/// A capitulation as the engine dates it (Capitulation's capitulated hour).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub struct Capitulation {
    pub hour: i32,
    pub tag: String,
}

/// A recorded country id naming another tag from one recorded hour to the next
/// (a tag switch, or an id the game gives another country): runs kept by tag end
/// and start there without the country joining or leaving anything.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub struct IdentityChange {
    pub hour: i32,
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryGroupReply {
    pub branch_id: String,
    pub humans: Vec<MembershipRun>,
    pub factions: Vec<FactionGroup>,
    pub wars: Vec<WarPairRun>,
    /// Every war start the recorded war relations name, in hour order.
    pub war_starts: Vec<WarStart>,
    /// Every capitulation the recording saw, in hour order.
    pub capitulations: Vec<Capitulation>,
    /// Every change of an id's tag between two consecutive recorded hours, in hour order.
    pub identity_changes: Vec<IdentityChange>,
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
            WarStart::decl(),
            Capitulation::decl(),
            IdentityChange::decl(),
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
        let mut refused = catalogue::Refusals::default();
        let factions = scan
            .factions
            .iter()
            .map(|(id, faction)| FactionGroup {
                id: id.to_string(),
                name: faction_name(&faction.name, localisation.as_deref())
                    .inspect_err(|reason| refused.note(reason.clone(), format!("faction {id}")))
                    .ok()
                    .flatten(),
                recorded_name: faction.name.clone(),
                first_hour: faction.first_hour,
                last_hour: faction.last_hour,
                members: faction.members.clone(),
                leaders: faction.leaders.clone(),
                incomplete: faction.incomplete.clone(),
            })
            .collect();
        warnings.extend(refused.lines("faction names are"));
        warnings.extend(scan.warnings.iter().cloned());
        Ok(CountryGroupReply {
            branch_id: params.branch_id.clone(),
            humans: scan.humans.clone(),
            factions,
            wars: scan.wars.clone(),
            war_starts: scan.war_starts.iter().cloned().collect(),
            capitulations: scan.capitulations.iter().cloned().collect(),
            identity_changes: scan.identity_changes.clone(),
            unobserved: scan.unobserved.clone(),
            warnings,
        })
    }
}

/// The engine stores either a localisation key or the literal name a player
/// typed. Both go through the game's localiser, which localises a key and
/// copies anything the content does not define as its own bytes (142230940);
/// without content the recorded name is shown as recorded. `Err` names why the
/// name cannot be shown.
pub fn faction_name(
    recorded: &str,
    localisation: Option<&Localisation>,
) -> Result<Option<String>, String> {
    if recorded.is_empty() {
        return Ok(None);
    }
    match localisation {
        // CFaction::display_name (140D77360) reaches its text boxes plain.
        Some(localisation) => {
            catalogue::localised(localisation, recorded, catalogue::View::Plain).map(Some)
        }
        None => Ok(Some(recorded.to_owned())),
    }
}

struct ScannedFaction {
    name: String,
    first_hour: i32,
    last_hour: i32,
    members: Vec<MembershipRun>,
    leaders: Vec<MembershipRun>,
    incomplete: Vec<HourSpan>,
}

/// One branch's group history, before names are localised.
pub struct Scan {
    humans: Vec<MembershipRun>,
    factions: BTreeMap<u64, ScannedFaction>,
    wars: Vec<WarPairRun>,
    war_starts: BTreeSet<WarStart>,
    capitulations: BTreeSet<Capitulation>,
    identity_changes: Vec<IdentityChange>,
    unobserved: UnobservedHours,
    warnings: Vec<String>,
}

/// A run of consecutive hours.
pub(super) trait HourRun {
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
pub(super) fn extend<K: Ord, R: HourRun>(
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
pub(super) fn war_ongoing(start_hour: i32, end_hour: i32, hour: i32) -> bool {
    end_hour <= start_hour || hour < end_hour
}

/// The branch's first hour, if it is known.
fn bounds_first(replay: &Replay, branch: u64) -> Option<i32> {
    replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .map(|start| branch_bounds(replay, start).0)
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

/// Adds unobserved hours, merging overlapping or adjacent spans.
pub(super) fn mark_unobserved(spans: &mut Vec<HourSpan>, first_hour: i32, last_hour: i32) {
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
        TableId::Capitulation,
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
    let mut leader_open: BTreeMap<(u64, String), usize> = BTreeMap::new();
    let mut wars: Vec<WarPairRun> = Vec::new();
    let mut war_starts: BTreeSet<WarStart> = BTreeSet::new();
    let mut capitulations: BTreeSet<Capitulation> = BTreeSet::new();
    // Each event is considered once, when first seen, keyed by raw ids and its hour:
    // a later hour may map the same id to another identity. At most MAX_EVENTS are
    // kept; past that, later ones are left out and the reply says so.
    let mut named_starts: BTreeSet<(u32, u32, i32)> = BTreeSet::new();
    let mut named_capitulations: BTreeSet<(u32, i32)> = BTreeSet::new();
    let mut events_capped = false;
    // An event is named by the tags its ids had at the event's own hour. That is
    // known only when every hour since was recorded with the same tag for the id:
    // `named_since` holds each id's tag and the hour it has had it without a break,
    // `continuous_since` the hour since which every hour was recorded.
    let mut named_since: HashMap<u32, (String, i32)> = HashMap::new();
    let mut identity_changes: Vec<IdentityChange> = Vec::new();
    let mut continuous_since: Option<i32> = None;
    // Events in the branch whose countries at the event hour cannot be established.
    let mut unattributed = 0_usize;
    let branch_first = bounds_first(replay, branch);
    // Hours whose capitulations could not be read, so an empty list is not passed off as none.
    let mut capitulations_unread = 0_usize;
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
    // Link id to tag, rebuilt only when the country tables change.
    let mut tags = LinkTags::default();
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
                continuous_since = None;
            }
            previous_hour = Some(hour);
            let Ok(rebuilt) = tags.update(decoded) else {
                incomplete += 1;
                mark_all(&mut unobserved, hour, hour);
                continuous_since = None;
                continue;
            };
            if rebuilt {
                // An id naming another tag than the hour before, with no gap between.
                if continuous_since.is_some() {
                    let mut changed: Vec<IdentityChange> = named_since
                        .iter()
                        .filter_map(|(id, (from, _))| {
                            let to = tags.get(*id)?;
                            (to != from).then(|| IdentityChange {
                                hour,
                                from: from.clone(),
                                to: to.clone(),
                            })
                        })
                        .collect();
                    // Several ids may name one country: each change is listed once.
                    changed.sort();
                    changed.dedup();
                    for change in changed {
                        if identity_changes.len() >= MAX_EVENTS {
                            events_capped = true;
                            break;
                        }
                        identity_changes.push(change);
                    }
                }
                named_since.retain(|id, (tag, _)| tags.get(*id) == Some(tag));
                for (id, tag) in tags.iter() {
                    named_since
                        .entry(*id)
                        .or_insert_with(|| (tag.clone(), hour));
                }
            }
            let since = *continuous_since.get_or_insert(hour);
            // The tag an id had at `at`, when that is known (see `named_since`).
            let tag_at = |raw: u32, at: i32| -> Option<&String> {
                let (tag, from) = named_since.get(&raw)?;
                (since.max(*from) <= at).then_some(tag)
            };
            // Every group table names countries by their raw link id.
            let tag_of = |raw: u32, unmapped: &mut BTreeSet<u32>| -> Option<&String> {
                let tag = tags.get(raw);
                if tag.is_none() {
                    unmapped.insert(raw);
                }
                tag
            };
            // Each history is unobserved only where its own tables are incomplete.
            let mut missed = false;
            match decoded.complete::<tables::HumanControllers>() {
                Ok(rows) => {
                    // A country the links cannot name leaves the hour's list incomplete.
                    let mut incomplete = false;
                    for row in rows.iter().filter(|row| row.count > 0) {
                        let Some(tag) = tag_of(row.country, &mut unmapped) else {
                            incomplete = true;
                            continue;
                        };
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
                    if incomplete {
                        mark_unobserved(&mut unobserved.humans, hour, hour);
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
                            leaders: Vec::new(),
                            incomplete: Vec::new(),
                        });
                        faction.last_hour = hour;
                        // A renamed faction keeps its id; the latest name is shown.
                        if faction.name != name {
                            faction.name = name.into_owned();
                        }
                    }
                    let mut ordered: Vec<_> = members.iter().collect();
                    ordered.sort_by_key(|member| (member.faction, member.ordinal));
                    // A member the links cannot name leaves that faction's roster
                    // incomplete this hour: no join, leave or founding is read there. A
                    // member of a faction the Factions table does not list belongs to
                    // no faction this reply names.
                    for member in ordered {
                        let Some(faction) = factions.get_mut(&member.faction) else {
                            continue;
                        };
                        let tag = tag_of(member.country, &mut unmapped);
                        if tag.is_none() {
                            mark_unobserved(&mut faction.incomplete, hour, hour);
                        }
                        if let Some(tag) = tag {
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
                            // The member at order zero leads the faction (module notes).
                            if member.ordinal == 0 {
                                let started = extend(
                                    &mut leader_open,
                                    &mut faction.leaders,
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
                }
                _ => {
                    missed = true;
                    mark_unobserved(&mut unobserved.factions, hour, hour);
                }
            }
            match decoded.complete::<tables::WarRelations>() {
                Ok(rows) => {
                    // When each war started, and which side instigated it.
                    for row in rows.iter() {
                        let key = (row.first_country, row.second_country, row.start_hour);
                        if named_starts.contains(&key) {
                            continue;
                        }
                        if named_starts.len() >= MAX_EVENTS {
                            events_capped = true;
                            continue;
                        }
                        let (Some(_), Some(_)) = (
                            tag_of(row.first_country, &mut unmapped),
                            tag_of(row.second_country, &mut unmapped),
                        ) else {
                            continue;
                        };
                        named_starts.insert(key);
                        let (Some(first), Some(second)) = (
                            tag_at(row.first_country, row.start_hour),
                            tag_at(row.second_country, row.start_hour),
                        ) else {
                            // Before the branch it is not this branch's event at all.
                            unattributed += usize::from(
                                branch_first.is_some_and(|first| row.start_hour >= first),
                            );
                            continue;
                        };
                        let (attacking, defending) = if row.first_was_instigator != 0 {
                            (first, second)
                        } else {
                            (second, first)
                        };
                        let start = WarStart {
                            hour: row.start_hour,
                            attacking: attacking.clone(),
                            defending: defending.clone(),
                        };
                        war_starts.insert(start);
                    }
                    // A war the links cannot name leaves the hour's wars incomplete.
                    let mut incomplete = false;
                    for row in rows
                        .iter()
                        .filter(|row| war_ongoing(row.start_hour, row.end_hour, hour))
                    {
                        let (Some(first), Some(second)) = (
                            tag_of(row.first_country, &mut unmapped),
                            tag_of(row.second_country, &mut unmapped),
                        ) else {
                            incomplete = true;
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
                    if incomplete {
                        mark_unobserved(&mut unobserved.wars, hour, hour);
                    }
                }
                Err(_) => {
                    missed = true;
                    mark_unobserved(&mut unobserved.wars, hour, hour);
                }
            }
            // Capitulations as the engine dates them, read while the flag is set; the
            // date stays with the event after recovery. The hour word is certified
            // only while the flag is set (the fresh-save oracle lists the retained word
            // of an inactive capitulation as a limitation), so a capitulation that began
            // and ended between two readable samples is not recorded, never guessed
            // from that word.
            let capitulation_rows = decoded.complete::<tables::Capitulation>();
            if capitulation_rows.is_err() {
                capitulations_unread += 1;
            }
            if let Ok(rows) = capitulation_rows {
                for row in rows.iter().filter(|row| row.capitulated != 0) {
                    let key = (row.country, row.capitulated_hour);
                    if named_capitulations.contains(&key) {
                        continue;
                    }
                    if named_capitulations.len() >= MAX_EVENTS {
                        events_capped = true;
                        continue;
                    }
                    if tag_of(row.country, &mut unmapped).is_some() {
                        named_capitulations.insert(key);
                        let Some(tag) = tag_at(row.country, row.capitulated_hour) else {
                            unattributed += usize::from(
                                branch_first.is_some_and(|first| row.capitulated_hour >= first),
                            );
                            continue;
                        };
                        let event = Capitulation {
                            hour: row.capitulated_hour,
                            tag: tag.clone(),
                        };
                        capitulations.insert(event);
                    }
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
    if capitulations_unread > 0 {
        warnings.push(format!(
            "Capitulations could not be read at {capitulations_unread} recorded hours; some may be missing."
        ));
    }
    if events_capped {
        warnings.push(format!(
            "War starts and capitulations exceed {MAX_EVENTS} each; later ones are left out."
        ));
    }
    if unattributed > 0 {
        warnings.push(format!(
            "{unattributed} war starts or capitulations were first seen after an unrecorded hour or a change of country; which countries they were is not known, so they are left out."
        ));
    }
    Ok(Scan {
        humans,
        factions,
        wars,
        war_starts,
        capitulations,
        identity_changes,
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
    pub fn faction_leaders(&self, id: u64) -> Option<&[MembershipRun]> {
        self.factions
            .get(&id)
            .map(|faction| faction.leaders.as_slice())
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
    pub fn faction_incomplete(&self, id: u64) -> Option<&[HourSpan]> {
        self.factions
            .get(&id)
            .map(|faction| faction.incomplete.as_slice())
    }
    pub fn war_starts(&self) -> Vec<WarStart> {
        self.war_starts.iter().cloned().collect()
    }
    pub fn capitulations(&self) -> Vec<Capitulation> {
        self.capitulations.iter().cloned().collect()
    }
    pub fn identity_changes(&self) -> &[IdentityChange] {
        &self.identity_changes
    }
}

#[cfg(test)]
mod tests;

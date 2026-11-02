//! Faction state's replies, read from the family's series publication ([`super::emit`]) through
//! the gate's prefix: a faction's influence, rules and upgrades, manpower, and goal history.
//! Before the family's first publication every view answers with no lane, its hours unobserved.

use super::{
    emit::{BOOKS, LANES, Meta},
    prepared::{
        CHANNELS, Kind, NULL,
        goals::{self, ABSENT, Budget, GoalBook},
    },
    *,
};
use dashboard_base::{
    Source,
    metadata::check_cancel,
    prepared::{engine, prefix::Prefix, series::Run},
    products::check_reply_budget,
};
use dashboard_series::{KeyHandle, PageRead, Snapshot, family::Directory};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};

/// A lane a reply reads: its kind, key and catalogue.
struct Found {
    kind: Kind,
    key: KeyHandle,
    meta: Meta,
}

/// What a reply reads of the publication: its directory, and each key's runs over a span,
/// clipped to it.
struct View<'v> {
    directory: Option<&'v Directory>,
    runs: &'v mut dyn FnMut(KeyHandle, i32, i32) -> io::Result<Vec<Run>>,
}

impl View<'_> {
    /// `faction`'s lanes of `kinds`, in the order the publication first held them.
    fn lanes(&self, faction: u64, kinds: &[Kind]) -> io::Result<Vec<Found>> {
        let Some(directory) = self.directory else {
            return Ok(Vec::new());
        };
        let Some(actor) = directory.actor(faction.to_string().as_bytes()) else {
            return Ok(Vec::new());
        };
        let mut found = Vec::new();
        for &kind in kinds {
            for &(ordinal, key) in directory.dimensions(actor, kind.measure()) {
                let meta = Meta::read(directory.scalar(LANES + ordinal))?;
                found.push((ordinal, Found { kind, key, meta }));
            }
        }
        found.sort_unstable_by_key(|&(ordinal, _)| ordinal);
        Ok(found.into_iter().map(|(_, found)| found).collect())
    }

    /// `faction`'s lane of `kind` named `identity`.
    fn lane(&self, faction: u64, kind: Kind, identity: &str) -> io::Result<Option<Found>> {
        Ok(self
            .lanes(faction, &[kind])?
            .into_iter()
            .find(|found| found.meta.identity == identity))
    }

    fn runs(&mut self, key: KeyHandle, from: i32, to: i32) -> io::Result<Vec<Run>> {
        if from > to {
            return Ok(Vec::new());
        }
        (self.runs)(key, from, to)
    }

    /// `faction`'s goal book.
    fn book(&self, faction: u64) -> io::Result<GoalBook> {
        let Some(directory) = self.directory else {
            return Ok(GoalBook::default());
        };
        let Some(actor) = directory.actor(faction.to_string().as_bytes()) else {
            return Ok(GoalBook::default());
        };
        match directory.scalar(BOOKS + actor) {
            Some(book) => {
                let text = book
                    .text()
                    .ok_or_else(|| dashboard_base::invalid("A faction goal book is not valid"))?;
                Ok(serde_json::from_slice(text)?)
            }
            None => Ok(GoalBook::default()),
        }
    }
}

/// `params` answered from the publication `snapshot` pins, its keys in `directory`.
pub fn reply<P: PageRead>(
    source: &Source,
    context: &LoadContext,
    params: &FactionStateParams,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    prefix: Prefix,
    cancelled: &AtomicBool,
) -> io::Result<FactionStateReply> {
    let mut runs = |key: KeyHandle, from: i32, to: i32| -> io::Result<Vec<Run>> {
        let mut runs: Vec<Run> = engine::runs(snapshot, key, from, to)?
            .into_iter()
            .map(|run| Run::new(run.span.first, run.span.last, run.value, run.break_before))
            .collect();
        // A run the window clips starts no segment at the window's first hour.
        if let Some(first) = runs.first_mut()
            && first.break_before
            && first.first_hour == from
            && engine::run_at(snapshot, key, from)?.is_some_and(|(run, _)| run.span.first < from)
        {
            first.break_before = false;
        }
        Ok(runs)
    };
    let view = View {
        directory: Some(directory),
        runs: &mut runs,
    };
    answer(source, context, params, view, prefix, cancelled)
}

/// `params` answered before the family's first publication of the branch.
pub fn unpublished(
    source: &Source,
    context: &LoadContext,
    params: &FactionStateParams,
    prefix: Prefix,
    cancelled: &AtomicBool,
) -> io::Result<FactionStateReply> {
    let mut runs = |_: KeyHandle, _: i32, _: i32| -> io::Result<Vec<Run>> { Ok(Vec::new()) };
    let view = View {
        directory: None,
        runs: &mut runs,
    };
    answer(source, context, params, view, prefix, cancelled)
}

/// The hours of `runs` holding `value`, joined where they go on without a break.
fn spans(runs: &[Run], value: i128) -> Vec<HourSpan> {
    let mut result: Vec<HourSpan> = Vec::new();
    for run in runs.iter().filter(|run| run.value == value) {
        if let Some(last) = result.last_mut()
            && last.last_hour.checked_add(1) == Some(run.first_hour)
            && !run.break_before
        {
            last.last_hour = run.last_hour;
        } else {
            result.push(HourSpan {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
            });
        }
    }
    result
}

/// The hours of `first..=last` outside `known`.
fn missing(first: i32, last: i32, known: &[HourSpan]) -> Vec<HourSpan> {
    let mut start = i64::from(first);
    let mut result = Vec::new();
    for span in known {
        if start < i64::from(span.first_hour) {
            result.push(HourSpan {
                first_hour: start as i32,
                last_hour: span.first_hour - 1,
            });
        }
        start = start.max(i64::from(span.last_hour) + 1);
    }
    if start <= i64::from(last) {
        result.push(HourSpan {
            first_hour: start as i32,
            last_hour: last,
        });
    }
    result
}

fn choices(meta: &Meta, runs: Vec<Run>) -> io::Result<Vec<FactionSelectionRun>> {
    runs.into_iter()
        .map(|run| {
            let choice = usize::try_from(run.value)
                .ok()
                .and_then(|value| meta.choices.get(value))
                .ok_or_else(|| {
                    dashboard_base::invalid("Faction selection variant is unavailable")
                })?;
            Ok(FactionSelectionRun {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
                token: choice.token.clone(),
                name: choice.name.clone(),
                reason: choice.reason.clone(),
                starts_segment: run.break_before,
            })
        })
        .collect()
}

fn manpower_runs(runs: Vec<Run>) -> Vec<FactionManpowerRun> {
    runs.into_iter()
        .map(|run| FactionManpowerRun {
            first_hour: run.first_hour,
            last_hour: run.last_hour,
            value: i32::try_from(run.value).ok(),
            starts_segment: run.break_before,
        })
        .collect()
}

fn answer(
    source: &Source,
    context: &LoadContext,
    params: &FactionStateParams,
    mut view: View<'_>,
    prefix: Prefix,
    cancelled: &AtomicBool,
) -> io::Result<FactionStateReply> {
    check_cancel(cancelled)?;
    FactionState::validate(params).map_err(dashboard_base::invalid)?;
    let faction: u64 = params
        .identity()
        .1
        .parse()
        .map_err(|_| dashboard_base::invalid("Invalid faction state faction"))?;
    let mut reply = match params {
        FactionStateParams::Goals {
            first_hour,
            last_hour,
            ..
        } => FactionStateReply::Goals(goal_history(
            source,
            context,
            &mut view,
            faction,
            (*first_hour, *last_hour),
            prefix,
            cancelled,
        )?),
        _ => histories(params, &mut view, faction, prefix, cancelled)?,
    };
    if let Some(options) = &context.content {
        dashboard_base::content::check_replay(
            source.replay_ref(),
            options.allow_unverified,
            dashboard_base::products::ReplyWarnings::warnings_mut(&mut reply),
        )?;
    }
    check_reply_budget(&reply)?;
    Ok(reply)
}

/// The influence, rules or manpower view of `faction` over the prefix's hours.
fn histories(
    params: &FactionStateParams,
    view: &mut View<'_>,
    faction: u64,
    prefix: Prefix,
    cancelled: &AtomicBool,
) -> io::Result<FactionStateReply> {
    // A requested prefix can end inside a published segment. These observations never revise
    // an earlier hour, so each requested run is clipped.
    let from = prefix.coverage.first_hour;
    let to = prefix.coverage.last_hour;
    let (wanted, kinds): (&[&str], &[Kind]) = match params {
        FactionStateParams::Influence { .. } => (&["influence"], &[Kind::Association]),
        FactionStateParams::Manpower { .. } => (
            &["manpower"],
            &[Kind::Manpower, Kind::Total, Kind::ManpowerDiagnostic],
        ),
        FactionStateParams::Rules { .. } => (
            &["rules", "programmes", "minimums", "members", "doctrines"],
            &[
                Kind::Rule,
                Kind::Programme,
                Kind::Minimum,
                Kind::Doctrine,
                Kind::Upgrade,
            ],
        ),
        FactionStateParams::Goals { .. } => unreachable!("goal views are answered apart"),
    };
    let mut channels = Vec::new();
    let mut warnings = BTreeMap::<String, u64>::new();
    let known_lanes = view.lanes(faction, &[Kind::Known, Kind::Read])?;
    for &channel in wanted {
        let mut read_spans = |kind: Kind| -> io::Result<Vec<HourSpan>> {
            match known_lanes
                .iter()
                .find(|found| found.kind == kind && found.meta.identity == channel)
            {
                Some(found) => Ok(spans(&view.runs(found.key, from, to)?, 1)),
                None => Ok(Vec::new()),
            }
        };
        let known = read_spans(Kind::Known)?;
        let read = read_spans(Kind::Read)?;
        let unknown = missing(from, to, &known);
        if !unknown.is_empty() {
            warnings.insert(
                format!("Faction {channel} inputs are unobserved"),
                unknown.len() as u64,
            );
        }
        channels.push(FactionChannelCoverage {
            channel: channel.into(),
            known,
            read,
        });
    }
    let mut unobserved = Vec::<HourSpan>::new();
    let mut unknown: Vec<_> = channels
        .iter()
        .flat_map(|channel| missing(from, to, &channel.known))
        .collect();
    unknown.sort_by_key(|span| span.first_hour);
    for span in unknown {
        if let Some(last) = unobserved.last_mut()
            && i64::from(last.last_hour) + 1 >= i64::from(span.first_hour)
        {
            last.last_hour = last.last_hour.max(span.last_hour);
        } else {
            unobserved.push(span);
        }
    }
    let mut reply = match params {
        FactionStateParams::Influence { .. } => FactionStateReply::Influence(FactionInfluenceReply {
            accounts: Vec::new(),
            unobserved,
            pending: prefix.pending,
            coverage: prefix.coverage,
            channels,
            warnings: Vec::new(),
        }),
        FactionStateParams::Rules { .. } => FactionStateReply::Rules(FactionRulesReply {
            rules: Vec::new(),
            programmes: Vec::new(),
            minimums: Vec::new(),
            members: Vec::new(),
            doctrines: Vec::new(),
            groups: Vec::new(),
            unobserved,
            pending: prefix.pending,
            coverage: prefix.coverage,
            channels,
            warnings: Vec::new(),
        }),
        _ => FactionStateReply::Manpower(FactionManpowerReply {
            members: Vec::new(),
            total: Vec::new(),
            unobserved,
            pending: prefix.pending,
            coverage: prefix.coverage,
            channels,
            warnings: Vec::new(),
        }),
    };
    let lanes = view.lanes(faction, kinds)?;
    // The accounts' shares, by identity, which every faction's associations read.
    let shares: HashMap<String, KeyHandle> = match &reply {
        FactionStateReply::Influence(_) if !lanes.is_empty() => view
            .lanes(0, &[Kind::Share])?
            .into_iter()
            .map(|found| (found.meta.identity, found.key))
            .collect(),
        _ => HashMap::new(),
    };
    let mut groups = BTreeSet::new();
    for lane in lanes {
        check_cancel(cancelled)?;
        match (&mut reply, lane.kind) {
            (FactionStateReply::Influence(influence), Kind::Association) => {
                let association = view.runs(lane.key, from, to)?;
                let mut runs = Vec::new();
                if let Some(&key) = shares.get(&lane.meta.identity) {
                    let shares = view.runs(key, from, to)?;
                    let mut at = 0;
                    for association in &association {
                        if !matches!(association.value, 1 | 2 | NULL) {
                            continue;
                        }
                        while at < shares.len() && shares[at].last_hour < association.first_hour {
                            at += 1;
                        }
                        for share in shares[at..]
                            .iter()
                            .take_while(|share| share.first_hour <= association.last_hour)
                        {
                            runs.push(FactionShareRun {
                                first_hour: share.first_hour.max(association.first_hour),
                                last_hour: share.last_hour.min(association.last_hour),
                                share_raw: (share.value != NULL && association.value != NULL)
                                    .then(|| share.value.to_string()),
                                starts_segment: (share.break_before
                                    && share.first_hour >= association.first_hour)
                                    || (association.break_before
                                        && association.first_hour >= share.first_hour),
                            });
                        }
                    }
                }
                let mut coalesced: Vec<FactionShareRun> = Vec::new();
                for run in runs {
                    if let Some(last) = coalesced.last_mut()
                        && last.last_hour.checked_add(1) == Some(run.first_hour)
                        && last.share_raw == run.share_raw
                        && !run.starts_segment
                    {
                        last.last_hour = run.last_hour;
                    } else {
                        coalesced.push(run);
                    }
                }
                if !association.is_empty() {
                    influence.accounts.push(FactionInfluenceAccount {
                        key: lane.meta.identity.clone(),
                        tag: lane.meta.tag.clone(),
                        slot: lane.meta.holder.map(|(slot, _)| slot),
                        country_id: lane.meta.holder.map(|(_, id)| id),
                        runs: coalesced,
                        member: spans(&association, 1),
                        retained: spans(&association, 2),
                    });
                }
            }
            (FactionStateReply::Rules(rules), kind) => {
                let runs = choices(&lane.meta, view.runs(lane.key, from, to)?)?;
                if runs.is_empty() {
                    continue;
                }
                for run in &runs {
                    if let Some(reason) = &run.reason {
                        *warnings.entry(reason.clone()).or_default() += 1;
                    }
                }
                let meta = lane.meta;
                let slot = meta.slot;
                let selection = move |runs| FactionSelectionLane { slot, runs };
                match kind {
                    Kind::Rule => rules.rules.push(selection(runs)),
                    Kind::Programme => rules.programmes.push(selection(runs)),
                    Kind::Doctrine => rules.doctrines.push(selection(runs)),
                    Kind::Minimum => {
                        groups.insert(meta.group);
                        rules.minimums.push(FactionMinimumLane {
                            group: meta.group,
                            runs,
                        });
                    }
                    Kind::Upgrade => {
                        groups.insert(meta.group);
                        rules.members.push(FactionMemberUpgradeLane {
                            slot: meta.holder.map(|(slot, _)| slot),
                            country_id: meta.holder.map(|(_, id)| id),
                            key: meta.identity,
                            tag: meta.tag,
                            group: meta.group,
                            runs,
                        });
                    }
                    _ => unreachable!("the rules view reads selections alone"),
                }
            }
            (FactionStateReply::Manpower(manpower), Kind::Manpower) => {
                let runs = manpower_runs(view.runs(lane.key, from, to)?);
                if !runs.is_empty() {
                    manpower.members.push(FactionManpowerMember {
                        key: lane.meta.identity,
                        tag: lane.meta.tag,
                        slot: lane.meta.holder.map(|(slot, _)| slot),
                        country_id: lane.meta.holder.map(|(_, id)| id),
                        runs,
                    });
                }
            }
            (FactionStateReply::Manpower(manpower), Kind::Total) => {
                manpower.total = manpower_runs(view.runs(lane.key, from, to)?)
            }
            (FactionStateReply::Manpower(_), Kind::ManpowerDiagnostic) => {
                for run in choices(&lane.meta, view.runs(lane.key, from, to)?)? {
                    if let Some(reason) = run.reason {
                        *warnings.entry(reason).or_default() += 1;
                    }
                }
            }
            _ => {}
        }
    }
    if let FactionStateReply::Rules(rules) = &mut reply {
        for group in groups {
            let mut name = None;
            if let Some(lane) = view.lane(faction, Kind::Group, &group.to_string())? {
                for run in choices(&lane.meta, view.runs(lane.key, from, to)?)? {
                    name = run.name;
                    if let Some(reason) = run.reason {
                        *warnings.entry(reason).or_default() += 1;
                    }
                }
            }
            rules.groups.push(FactionUpgradeGroup { group, name });
        }
        rules.rules.sort_by_key(|lane| lane.slot);
        rules.programmes.sort_by_key(|lane| lane.slot);
        rules.doctrines.sort_by_key(|lane| lane.slot);
    }
    dashboard_base::products::ReplyWarnings::warnings_mut(&mut reply).extend(
        warnings
            .into_iter()
            .map(|(reason, count)| format!("{count} faction facts: {reason}")),
    );
    Ok(reply)
}

/// `faction`'s goal history over the asked hours (None is open-ended) within the prefix.
fn goal_history(
    source: &Source,
    context: &LoadContext,
    view: &mut View<'_>,
    faction: u64,
    (first_hour, last_hour): (Option<i32>, Option<i32>),
    prefix: Prefix,
    cancelled: &AtomicBool,
) -> io::Result<FactionGoalsReply> {
    let (first, last) = (first_hour.unwrap_or(i32::MIN), last_hour.unwrap_or(i32::MAX));
    // Prefix observations never revise an earlier hour, so runs are clipped.
    let from = prefix.coverage.first_hour.max(first);
    let to = prefix.coverage.last_hour.min(last);
    let channel = CHANNELS[7].0;
    let mut coverage = |kind: Kind| -> io::Result<Vec<HourSpan>> {
        Ok(match view.lane(faction, kind, channel)? {
            Some(lane) => spans(&view.runs(lane.key, from, to)?, 1),
            None => Vec::new(),
        })
    };
    let known = coverage(Kind::Known)?;
    let reads = coverage(Kind::Read)?;
    let pending = prefix
        .pending
        .iter()
        .filter(|span| span.first_hour <= last && first <= span.last_hour)
        .map(|span| HourSpan {
            first_hour: span.first_hour.max(first),
            last_hour: span.last_hour.min(last),
        })
        .collect();
    let mut reply = FactionGoalsReply {
        runs: Vec::new(),
        lists: Vec::new(),
        definitions: Vec::new(),
        progress: Vec::new(),
        unobserved: missing(from, to, &known),
        pending,
        coverage: prefix.coverage,
        channels: vec![FactionChannelCoverage {
            channel: channel.into(),
            known,
            read: reads,
        }],
        warnings: Vec::new(),
    };
    let mut budget = Budget::default();
    budget.add(&reply)?;
    let mut lists = BTreeMap::<u32, u32>::new();
    let mut held = Vec::new();
    for lane in view.lanes(faction, &[Kind::GoalList, Kind::GoalProgress])? {
        check_cancel(cancelled)?;
        match lane.kind {
            Kind::GoalList => {
                for run in view.runs(lane.key, from, to)? {
                    let list = u32::try_from(run.value)
                        .ok()
                        .filter(|list| *list != ABSENT)
                        .ok_or_else(|| {
                            dashboard_base::invalid("Faction goal list is unavailable")
                        })?;
                    held.push(HourSpan {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                    });
                    let next = lists.len() as u32;
                    let run = FactionGoalListRun {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                        list: *lists.entry(list).or_insert(next),
                        starts_segment: run.break_before,
                    };
                    budget.add(&run)?;
                    reply.runs.push(run);
                }
            }
            _ => {
                let mut runs = Vec::new();
                for run in view.runs(lane.key, from, to)? {
                    let raw = i64::try_from(run.value).map_err(|_| {
                        dashboard_base::invalid("Faction goal progress is not a fixed-point value")
                    })?;
                    let run = FactionGoalProgressRun {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                        progress_raw: raw.to_string(),
                        progress_percent: dashboard_base::products::faction_goal_progress_percent(
                            raw,
                        ),
                        starts_segment: run.break_before,
                    };
                    budget.add(&run)?;
                    runs.push(run);
                }
                if !runs.is_empty() {
                    reply.progress.push(FactionGoalProgressLane {
                        ordinal: lane.meta.slot,
                        runs,
                    });
                }
            }
        }
    }
    // Before the faction's first lists, only a complete roster proves it absent.
    if let Some(roster) = view.lane(0, Kind::Roster, "")? {
        let complete = spans(&view.runs(roster.key, from, to)?, 1);
        held.sort_by_key(|span| span.first_hour);
        let absent = goals::outside(&complete, &held);
        if !absent.is_empty() {
            let list = lists.len() as u32;
            lists.insert(ABSENT, list);
            for span in absent {
                let run = FactionGoalListRun {
                    first_hour: span.first_hour,
                    last_hour: span.last_hour,
                    list,
                    starts_segment: false,
                };
                budget.add(&run)?;
                reply.runs.push(run);
            }
            reply.runs.sort_by_key(|run| run.first_hour);
        }
    }
    check_cancel(cancelled)?;
    let book = if lists.keys().any(|list| *list != ABSENT) {
        view.book(faction)?
    } else {
        GoalBook::default()
    };
    let mut ordered: Vec<_> = lists.into_iter().collect();
    ordered.sort_by_key(|(_, shown)| *shown);
    goals::definitions(
        source,
        context,
        &book,
        ordered,
        &mut budget,
        &mut reply,
        cancelled,
    )?;
    reply.progress.sort_by_key(|lane| lane.ordinal);
    Ok(reply)
}

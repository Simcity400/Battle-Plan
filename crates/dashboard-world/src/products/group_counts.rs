use dashboard_base::products::{LoadContext, Product};
use dashboard_base::{Source, span::HourSpan};
use dashboard_query::Query;
use replay_data::rows::GroupSpanRow as Span;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::BTreeMap,
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum GroupKind {
    #[doc = " Factions, keyed by recorded faction id, counted by member."]
    Factions,
    #[doc = " Overlords, keyed by their tag at the hour, counted by subject."]
    Overlords,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroupCountsParams {
    pub branch_id: String,
    pub of: GroupKind,
    #[doc = " The hour counted: nothing recorded after it is read into the reply."]
    pub hour: i32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct GroupCount {
    pub key: String,
    #[doc = " Members (factions) or subjects (overlords) at the hour; null when not known then."]
    pub count: Option<u32>,
    #[doc = " Why the count is not known at the hour."]
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct GroupCountsReply {
    pub branch_id: String,
    pub of: GroupKind,
    pub hour: i32,
    #[doc = " The first hour after `hour` at which a count, a group or a reason changes; null when none does."]
    pub next_change: Option<i32>,
    #[doc = " Why no count is known at the hour for any group."]
    pub missing: Option<String>,
    #[doc = " Every faction begun by the hour, or every overlord holding a subject at it: the most first, then the"]
    #[doc = " earliest, then by key. A group whose count is not known follows them with its reason, unranked."]
    pub groups: Vec<GroupCount>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(GroupCountsReply);

const FACTIONS_UNREAD: &str = "Faction memberships were not read at this hour";
const ROSTER_UNREAD: &str = "Its roster was not read whole at this hour";
const SUBJECTS_UNREAD: &str = "Subject relations were not read at this hour";

#[derive(Debug, Default)]
struct Group {
    key: String,
    first: i32,
    steps: Vec<(i32, u32)>,
    unread: Vec<HourSpan>,
}

#[derive(Debug, Default)]
struct Index {
    groups: Vec<Group>,
    unread: Vec<HourSpan>,
    boundaries: Vec<i32>,
}

fn settled(mut spans: Vec<HourSpan>) -> Vec<HourSpan> {
    spans.sort_unstable_by_key(|span| span.first_hour);
    let mut out: Vec<HourSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        match out.last_mut() {
            Some(held) if i64::from(held.last_hour) + 1 >= i64::from(span.first_hour) => held.last_hour = held.last_hour.max(span.last_hour),
            _ => out.push(span),
        }
    }
    out
}

fn within(spans: &[HourSpan], hour: i32) -> bool {
    let at = spans.partition_point(|span| span.last_hour < hour);
    spans.get(at).is_some_and(|span| span.first_hour <= hour)
}

#[derive(Default)]
struct Building {
    first: Option<i32>,
    changes: Vec<(i32, i64)>,
    unread: Vec<HourSpan>,
}

impl Building {
    fn seen(&mut self, first: i32) {
        self.first = Some(self.first.map_or(first, |held| held.min(first)));
    }

    fn held(&mut self, first: i32, last: i32) {
        self.seen(first);
        self.changes.push((first, 1));
        self.changes.push((last.saturating_add(1), -1));
    }
}

impl Index {
    fn of(built: BTreeMap<String, Building>, unread: Vec<HourSpan>) -> Self {
        let unread = settled(unread);
        let mut boundaries: Vec<i32> = unread.iter().flat_map(|span| [span.first_hour, span.last_hour.saturating_add(1)]).collect();
        let mut groups = Vec::with_capacity(built.len());
        for (key, mut building) in built {
            let Some(first) = building.first else { continue };
            building.changes.sort_unstable();
            let mut steps: Vec<(i32, u32)> = Vec::new();
            let mut count = 0_i64;
            for (hour, delta) in building.changes {
                count += delta;
                let value = u32::try_from(count.max(0)).unwrap_or(u32::MAX);
                match steps.last_mut() {
                    Some(step) if step.0 == hour => step.1 = value,
                    _ => steps.push((hour, value)),
                }
            }
            let unread = settled(building.unread);
            boundaries.push(first);
            boundaries.extend(steps.iter().map(|step| step.0));
            boundaries.extend(unread.iter().flat_map(|span| [span.first_hour, span.last_hour.saturating_add(1)]));
            groups.push(Group { key, first, steps, unread });
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        Self { groups, unread, boundaries }
    }

    fn answer(&self, of: GroupKind, hour: i32) -> (Option<String>, Vec<GroupCount>) {
        let (whole, own) = match of {
            GroupKind::Factions => (FACTIONS_UNREAD, ROSTER_UNREAD),
            GroupKind::Overlords => (SUBJECTS_UNREAD, SUBJECTS_UNREAD),
        };
        let unread = within(&self.unread, hour);
        if unread && of == GroupKind::Overlords {
            return (Some(whole.to_owned()), Vec::new());
        }
        let mut known: Vec<(Reverse<u32>, i32, &str)> = Vec::new();
        let mut unknown: Vec<(i32, &str, &str)> = Vec::new();
        for group in self.groups.iter().filter(|group| group.first <= hour) {
            if unread || within(&group.unread, hour) {
                unknown.push((group.first, &group.key, if unread { whole } else { own }));
                continue;
            }
            let count = group.steps.partition_point(|step| step.0 <= hour).checked_sub(1).map_or(0, |at| group.steps[at].1);
            if count > 0 || of == GroupKind::Factions {
                known.push((Reverse(count), group.first, &group.key));
            }
        }
        known.sort_unstable();
        unknown.sort_unstable();
        let groups = known
            .into_iter()
            .map(|(Reverse(count), _, key)| GroupCount { key: key.to_owned(), count: Some(count), missing: None })
            .chain(unknown.into_iter().map(|(_, key, why)| GroupCount { key: key.to_owned(), count: None, missing: Some(why.to_owned()) }))
            .collect();
        (unread.then(|| whole.to_owned()), groups)
    }

    fn next_change(&self, hour: i32) -> Option<i32> {
        self.boundaries.get(self.boundaries.partition_point(|boundary| *boundary <= hour)).copied()
    }
}

fn factions(source: &Source, query: &Query<'_>) -> io::Result<Index> {
    let mut unread: Vec<HourSpan> = query.timeline.unheld.iter().map(|gap| HourSpan { first_hour: gap.first, last_hour: gap.last }).collect();
    let mut built: BTreeMap<String, Building> = BTreeMap::new();
    for stretch in &query.timeline.stretches {
        let run = super::groups::cached_groups(source, stretch.run)?;
        for &(kind, group, _, first, last) in &run.spans {
            let (first, last) = (first.max(stretch.first), last.min(stretch.last));
            if first > last {
                continue;
            }
            if kind == Span::UNOBSERVED && group == Span::FACTIONS {
                unread.push(HourSpan { first_hour: first, last_hour: last });
                continue;
            }
            if !matches!(kind, Span::MEMBER | Span::PRESENT | Span::INCOMPLETE) || (kind == Span::MEMBER && group == Span::HUMANS) {
                continue;
            }
            let Some(&id) = run.factions.get(&group) else { continue };
            let building = built.entry(id.to_string()).or_default();
            match kind {
                Span::MEMBER => building.held(first, last),
                Span::PRESENT => building.seen(first),
                _ => building.unread.push(HourSpan { first_hour: first, last_hour: last }),
            }
        }
    }
    Ok(Index::of(built, unread))
}

fn overlords(source: &Source, query: &Query<'_>) -> io::Result<Index> {
    let (runs, mut unread) = super::subjects::history(source, query, &AtomicBool::new(false))?;
    unread.extend(super::subjects::unknown_hours(query));
    let mut built: BTreeMap<String, Building> = BTreeMap::new();
    for run in &runs {
        built.entry(run.overlord.clone()).or_default().held(run.first_hour, run.last_hour);
    }
    Ok(Index::of(built, unread))
}

const KEPT: usize = 4;

#[derive(Default)]
struct Kept(Vec<((u64, GroupKind), Arc<Index>)>);

fn index(source: &Source, branch: u64, of: GroupKind) -> io::Result<Arc<Index>> {
    let state = source.state::<Kept>();
    if let Some((_, held)) = state.lock().unwrap_or_else(PoisonError::into_inner).0.iter().find(|(key, _)| *key == (branch, of)) {
        return Ok(Arc::clone(held));
    }
    let query = dashboard_base::queries::query(source, branch)?;
    let built = Arc::new(match of {
        GroupKind::Factions => factions(source, &query)?,
        GroupKind::Overlords => overlords(source, &query)?,
    });
    let mut kept = state.lock().unwrap_or_else(PoisonError::into_inner);
    kept.0.push(((branch, of), Arc::clone(&built)));
    if kept.0.len() > KEPT {
        kept.0.remove(0);
    }
    Ok(built)
}

pub struct GroupCounts;

impl Product for GroupCounts {
    const KEY: &'static str = "group-counts";
    const CONTENT_DEPENDENT: bool = false;
    type Params = GroupCountsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, of, hour: Read];
    type Reply = GroupCountsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![GroupKind::decl(), GroupCountsParams::decl(), GroupCount::decl(), GroupCountsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid group counts branch")
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, _cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid group counts branch"))?;
        let index = index(source, branch, params.of)?;
        let query = dashboard_base::queries::query(source, branch)?;
        let hour = params.hour.min(query.timeline.last);
        let (missing, groups) = index.answer(params.of, hour);
        Ok(GroupCountsReply { branch_id: params.branch_id.clone(), of: params.of, hour, next_change: index.next_change(hour), missing, groups, warnings: Vec::new() })
    }
}

#[cfg(test)]
mod tests;

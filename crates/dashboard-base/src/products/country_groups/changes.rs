//! Witnessed changes from the country-group run and native-date builders.
use super as country_groups;
use super::{HourSpan, Scan as GroupScan};
use replay::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub kind: String,
    pub name: Option<Result<String, String>>,
    pub hour: i32,
    pub read_hour: i32,
    pub name_hour: i32,
    pub exact: bool,
    /// For aggregated wars/peace the focal country comes first, then counterparties.
    pub tags: Vec<String>,
    pub faction: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub key: String,
    pub label: String,
    pub reason: String,
    pub count: u64,
}

pub struct Scan {
    pub moments: Vec<Change>,
    pub observed: BTreeSet<TableId>,
    pub unavailable: Vec<Refusal>,
}

impl Scan {
    pub fn refuse(&mut self, key: &str, label: &str, reason: &str) -> io::Result<()> {
        if let Some(entry) = self.unavailable.iter_mut().find(|entry| entry.key == key) {
            entry.count = entry.count.checked_add(1).ok_or_else(|| {
                io::Error::other(format!("Event feed {key} unavailable count exceeds u64"))
            })?;
        } else {
            self.unavailable.push(Refusal {
                key: key.to_owned(),
                label: label.to_owned(),
                reason: reason.to_owned(),
                count: 1,
            });
        }
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
pub const KEYS: [u64; 10] = [
    0,
    1,
    2,
    3,
    4,
    1 << 63,
    (1 << 63) | 1,
    (1 << 63) | 2,
    (1 << 63) | 3,
    (1 << 63) | 4,
];

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    events: crate::prepared::series::Accumulator,
    late: crate::prepared::series::LateIndex,
    pub observed: BTreeSet<String>,
    pub refusals: Vec<Refusal>,
    pub warnings: BTreeSet<String>,
    name_refusals: BTreeMap<String, BTreeSet<u64>>,
    rates: Vec<Refusal>,
}

#[derive(Serialize, Deserialize)]
pub struct Metadata {
    pub late: crate::prepared::series::LateIndex,
    pub observed: BTreeSet<String>,
    pub refusals: Vec<Refusal>,
    pub warnings: BTreeSet<String>,
    pub name_refusals: BTreeMap<String, BTreeSet<u64>>,
    pub unattributed_starts: usize,
    pub unattributed_capitulations: usize,
    pub unread_capitulations: usize,
}
impl Prepared {
    pub fn observe_changed(
        &mut self,
        groups: &mut GroupScan,
        frame: &replay::frame::DecodedFrame,
        changes: &crate::prepared::frame_changes::FrameChanges,
        source: &crate::Source,
        context: &crate::products::LoadContext,
    ) -> io::Result<()> {
        let Some((members, wars)) = groups.advance_changed(frame, changes)? else {
            self.events.through(frame.hour);
            for rate in &self.rates {
                let old = self
                    .refusals
                    .iter_mut()
                    .find(|old| old.key == rate.key && old.reason == rate.reason)
                    .expect("configured group refusal rate");
                old.count = old
                    .count
                    .checked_add(rate.count)
                    .ok_or_else(|| crate::invalid("Country group refusal count exceeds u64"))?;
            }
            return Ok(());
        };
        self.publish_changes(groups, frame, source, context, members, wars)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(
        &mut self,
        groups: &mut GroupScan,
        frame: &replay::frame::DecodedFrame,
        source: &crate::Source,
        context: &crate::products::LoadContext,
    ) -> io::Result<()> {
        let prior_members: Vec<_> = groups
            .continuation
            .member_open
            .iter()
            .map(|((id, _), &index)| (*id, index))
            .collect();
        let prior_wars: Vec<_> = groups.continuation.war_open.values().copied().collect();
        groups.observe(frame)?;
        self.publish_changes(groups, frame, source, context, prior_members, prior_wars)
    }
    fn publish_changes(
        &mut self,
        groups: &mut GroupScan,
        frame: &replay::frame::DecodedFrame,
        source: &crate::Source,
        context: &crate::products::LoadContext,
        prior_members: Vec<(u64, usize)>,
        prior_wars: Vec<usize>,
    ) -> io::Result<()> {
        groups.unobserved = groups.continuation.unobserved.clone();
        let hour = frame.hour;
        let members: BTreeSet<_> = prior_members
            .into_iter()
            .filter(|(id, index)| groups.factions[id].members[*index].last_hour < hour)
            .map(|(id, index)| (id, index, true))
            .chain(
                groups
                    .continuation
                    .member_open
                    .iter()
                    .take(if groups.members_changed {
                        usize::MAX
                    } else {
                        0
                    })
                    .filter(|((id, _), index)| {
                        groups.factions[id].members[**index].first_hour == hour
                    })
                    .map(|((id, _), &index)| (*id, index, false)),
            )
            .collect();
        let mut changes = Scan {
            moments: Vec::new(),
            observed: BTreeSet::new(),
            unavailable: Vec::new(),
        };
        let first = groups.continuation.bounds.map_or(hour, |bounds| bounds.0);
        project(
            groups,
            first,
            hour,
            &mut changes,
            members
                .iter()
                .map(|(id, index, end)| (*id, &groups.factions[id].members[*index], *end)),
            prior_wars
                .iter()
                .map(|index| &groups.wars[*index])
                .filter(|run| run.last_hour < hour),
            &groups.recent_starts,
            &groups.recent_capitulations,
        )?;
        if groups.members_changed {
            groups
                .continuation
                .member_open
                .retain(|(id, _), index| groups.factions[id].members[*index].last_hour == hour);
        }
        if groups.wars_changed {
            groups
                .continuation
                .war_open
                .retain(|_, index| groups.wars[*index].last_hour == hour);
        }
        self.events.through(hour);
        self.observed
            .extend(changes.observed.iter().map(|table| format!("{table:?}")));
        self.rates = changes.unavailable.clone();
        for refusal in changes.unavailable {
            if let Some(old) = self
                .refusals
                .iter_mut()
                .find(|old| old.key == refusal.key && old.reason == refusal.reason)
            {
                old.count = old.count.checked_add(refusal.count).ok_or_else(|| {
                    crate::invalid("Country group change refusal count exceeds u64")
                })?;
            } else {
                self.refusals.push(refusal);
            }
        }
        changes.moments.sort_by(|a, b| {
            (a.hour, &a.kind, &a.tags, a.faction).cmp(&(b.hour, &b.kind, &b.tags, b.faction))
        });
        for mut change in changes
            .moments
            .into_iter()
            .filter(|change| change.read_hour == hour)
        {
            if let Some(id) = change.faction {
                groups.seal_name(id, hour);
                let history = &groups.continuation.names[&id];
                let mut left = 0;
                let mut right = history.len();
                while left < right {
                    let mid = left + (right - left) / 2;
                    if history[mid].last_hour < change.name_hour {
                        left = mid + 1;
                    } else {
                        right = mid;
                    }
                }
                let name = (left < history.len())
                    .then(|| &history[left])
                    .filter(|name| name.first_hour <= change.name_hour);
                let mut warnings = Vec::new();
                change.name = Some(match name {
                    Some(name) => country_groups::with_name_content(
                        source,
                        context,
                        name.needs_templates(),
                        &mut warnings,
                        |templates, localisation, language, _| {
                            name.resolve(templates, localisation, language)
                        },
                    ),
                    None => Err(format!(
                        "Faction {id} has no recorded name at {}",
                        change.name_hour
                    )),
                });
                if let Some(Err(reason)) = &change.name {
                    self.name_refusals
                        .entry(reason.clone())
                        .or_default()
                        .insert(id);
                }
                self.warnings.extend(warnings);
            }
            let kind = match change.kind.as_str() {
                "factionJoined" => 0,
                "factionLeft" => 1,
                "peace" => 2,
                "warStarted" => 3,
                "capitulated" => 4,
                _ => return Err(crate::invalid("Unsupported country group change kind")),
            };
            let key = kind | (u64::from(change.read_hour > change.hour) << 63);
            let ordinal = self.events.state(key).map_or(0, |state| state.events);
            self.events.event(key, change.read_hour, &change)?;
            self.late
                .insert(change.hour, change.read_hour, key, ordinal);
        }
        Ok(())
    }
    pub fn bytes(&self) -> usize {
        self.events.bytes()
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        groups: &GroupScan,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> io::Result<()> {
        self.events.flush(
            writer,
            &format!("country-group-changes-{branch}"),
            0,
            cancelled,
        )?;
        crate::prepared::series::write_catalogue(
            writer,
            "country-group-changes",
            branch,
            &BTreeMap::new(),
            &Metadata {
                late: self.late.clone(),
                observed: self.observed.clone(),
                refusals: self.refusals.clone(),
                warnings: self
                    .warnings
                    .iter()
                    .cloned()
                    .chain(groups.warnings.iter().cloned())
                    .collect(),
                name_refusals: self.name_refusals.clone(),
                unattributed_starts: groups.unattributed_war_starts,
                unattributed_capitulations: groups.unattributed_capitulations,
                unread_capitulations: groups.capitulation_unread_hours,
            },
        )
    }
}

fn missing(spans: &[HourSpan], hour: i32) -> bool {
    spans
        .get(spans.partition_point(|span| span.last_hour < hour))
        .is_some_and(|span| span.first_hour <= hour)
}

fn any_observed(spans: &[HourSpan], first: i32, last: i32) -> bool {
    // Missing spans are coalesced, so at most one can cover the whole window.
    first <= last
        && spans
            .get(spans.partition_point(|span| span.last_hour < first))
            .is_none_or(|span| span.first_hour > first || span.last_hour < last)
}

fn switched(groups: &GroupScan, tag: &str, hour: i32) -> bool {
    let changes = &groups.identity_changes;
    let (mut first, mut last) = (0, changes.len());
    while first < last {
        let mid = first + (last - first) / 2;
        if changes[mid].hour < hour {
            first = mid + 1;
        } else {
            last = mid;
        }
    }
    (first..changes.len())
        .map(|index| &changes[index])
        .take_while(|change| change.hour == hour)
        .any(|change| change.from == tag || change.to == tag)
}

/// Boundaries come from the exact runs the topic widgets use. A boundary next
/// to an unobserved hour is counted, rather than silently converted into a join
/// or peace. Initial memberships and final campaign truncation are not events.
#[allow(clippy::too_many_arguments)]
pub fn project<'a>(
    groups: &GroupScan,
    first: i32,
    last: i32,
    out: &mut Scan,
    members: impl IntoIterator<Item = (u64, &'a country_groups::MembershipRun, bool)>,
    wars: impl IntoIterator<Item = &'a country_groups::WarPairRun>,
    starts: impl IntoIterator<Item = &'a country_groups::WarStart>,
    capitulations: impl IntoIterator<Item = &'a country_groups::Capitulation>,
) -> io::Result<()> {
    if any_observed(&groups.unobserved.factions, first, last) {
        out.observed.insert(TableId::FactionMembers);
    }
    if any_observed(&groups.unobserved.wars, first, last) {
        out.observed.insert(TableId::WarRelations);
    }
    for (id, run, end) in members {
        let faction = &groups.factions[&id];
        let (kind, hour, name_hour, neighbor) = if end {
            (
                "factionLeft",
                run.last_hour.checked_add(1),
                run.last_hour,
                run.last_hour.checked_add(1),
            )
        } else {
            (
                "factionJoined",
                Some(run.first_hour),
                run.first_hour,
                run.first_hour.checked_sub(1),
            )
        };
        let (Some(hour), Some(neighbor)) = (hour, neighbor) else {
            continue;
        };
        if hour <= first || hour > last || switched(groups, &run.tag, hour) {
            continue;
        }
        if missing(&groups.unobserved.factions, neighbor) || missing(&faction.incomplete, neighbor)
        {
            out.refuse("factionBoundary", "Faction changes",
                        "Faction membership boundaries adjoin an unobserved or unresolved roster; the changes are not established.")?;
            continue;
        }
        out.moments.push(Change {
            kind: kind.to_owned(),
            name: None,
            hour,
            read_hour: hour,
            name_hour,
            exact: false,
            tags: vec![run.tag.clone()],
            faction: Some(id),
        });
    }
    // Removing a war relation clears both status links. A complete following
    // observation only proves the countries are no longer at war; it does not
    // identify a peace treaty, annexation or release cause.
    let mut peace = BTreeMap::<(i32, String), BTreeSet<String>>::new();
    for run in wars {
        let Some(hour) = run.last_hour.checked_add(1).filter(|hour| *hour <= last) else {
            continue;
        };
        if switched(groups, &run.first, hour) || switched(groups, &run.second, hour) {
            continue;
        }
        if missing(&groups.unobserved.wars, hour) {
            out.refuse("warBoundary", "Wars and peace",
                "War-pair boundaries adjoin an unobserved or unresolved war roster; their removal dates are not established.")?;
            continue;
        }
        for (tag, other) in [(&run.first, &run.second), (&run.second, &run.first)] {
            peace
                .entry((hour, tag.clone()))
                .or_default()
                .insert(other.clone());
        }
    }
    for ((hour, tag), others) in peace {
        out.moments.push(Change {
            kind: "peace".into(),
            name: None,
            hour,
            read_hour: hour,
            name_hour: hour,
            exact: false,
            tags: std::iter::once(tag).chain(others).collect(),
            faction: None,
        });
    }
    // The relation start is shared by declarations and joins. Group's exact
    // relation start is recorded; counterparties aggregate by country/hour.
    let mut war_starts = BTreeMap::<(i32, i32, String), BTreeSet<String>>::new();
    for start in starts {
        let read_hour = groups.war_start_reads.get(start).copied();
        let Some(read_hour) = read_hour else {
            out.refuse(
                "warReadHour",
                "Wars and peace",
                "War starts have no recorded supplying observation.",
            )?;
            continue;
        };
        war_starts
            .entry((start.hour, read_hour, start.attacking.clone()))
            .or_default()
            .insert(start.defending.clone());
    }
    for ((hour, read_hour, tag), others) in war_starts {
        out.moments.push(Change {
            kind: "warStarted".into(),
            name: None,
            hour,
            read_hour,
            name_hour: hour,
            exact: true,
            tags: std::iter::once(tag).chain(others).collect(),
            faction: None,
        });
    }
    if groups.capitulations_observed {
        out.observed.insert(TableId::Capitulation);
    }
    for event in capitulations {
        if let Some(&read_hour) = groups.capitulation_reads.get(event) {
            out.moments.push(Change {
                kind: "capitulated".into(),
                name: None,
                hour: event.hour,
                read_hour,
                name_hour: event.hour,
                exact: true,
                tags: vec![event.tag.clone()],
                faction: None,
            });
        } else {
            out.refuse(
                "capitulationReadHour",
                "Capitulations",
                "Capitulations have no recorded supplying observation.",
            )?;
        }
    }
    Ok(())
}

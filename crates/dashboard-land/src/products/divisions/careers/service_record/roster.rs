use super::{Content, ServiceDivision, TITLE};
use dashboard_base::{
    products::at_hour::{self, Frames, Read},
    queries::country_ids::{Names, RunLinks},
};
use dashboard_query::{
    Query,
    coverage::witnessed,
    identity::{Country, Holder},
    log::occurrence::Selection,
    timeline::Stretch,
};
use dashboard_text::metadata::check_cancel;
use replay_data::rows::{DivisionRow, DivisionStatusRow, Recorded, TableId, TableRows, tables};
use replay_lanes::lanes::{self, Cell, Keys};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};

#[derive(Default)]
pub(super) struct Steps<T>(Vec<(i32, T)>);

impl<T: PartialEq> Steps<T> {
    pub(super) fn put(&mut self, hour: i32, value: T) {
        if self.0.last().is_some_and(|s| s.0 == hour) {
            self.0.pop();
        }
        if self.0.last().is_none_or(|s| s.1 != value) {
            self.0.push((hour, value));
        }
    }

    fn at(&self, hour: i32) -> Option<&T> {
        self.0[..self.0.partition_point(|s| s.0 <= hour)].last().map(|s| &s.1)
    }

    fn next(&self, hour: i32) -> Option<i32> {
        self.0.get(self.0.partition_point(|s| s.0 <= hour)).map(|s| s.0)
    }
}

#[derive(Default, PartialEq)]
struct Posting {
    template: u64,
    slot: Option<u32>,
    tag: Option<String>,
}

#[derive(Default)]
pub(super) struct Division {
    pub(super) seen: Vec<(i32, i32)>,
    pub(super) removals: Vec<(i32, i32, String)>,
    pub(super) facts: Vec<i32>,
    pub(super) tokens: BTreeSet<u128>,
    pub(super) medals: Steps<Option<i32>>,
    posting: Steps<Posting>,
    name: Steps<Option<String>>,
    experience: Steps<(Option<i64>, Option<i32>)>,
    manpower: Steps<Option<i32>>,
}

impl Division {
    pub(super) fn seen_at(&self, hour: i32) -> Option<i32> {
        self.seen[..self.seen.partition_point(|s| s.0 <= hour)].last().map(|s| hour.min(s.1))
    }

    pub(super) fn removed(&self, hour: i32) -> Option<&(i32, i32, String)> {
        let seen = self.seen_at(hour)?;
        self.removals[..self.removals.partition_point(|r| r.0 <= hour)].last().filter(|r| r.0 >= seen)
    }

    pub(super) fn next_change(&self, hour: i32, roster: &Roster) -> Option<i32> {
        let next_seen = self.seen.get(self.seen.partition_point(|s| s.0 <= hour)).map(|s| s.0);
        let removal = self.removals.get(self.removals.partition_point(|r| r.0 <= hour)).map(|r| r.0);
        let next_facts = self.facts.get(self.facts.partition_point(|&at| at <= hour)).copied();
        let template = self.posting.at(hour).and_then(|p| roster.templates.get(&p.template)).and_then(|t| t.next(hour));
        let seen = self.seen.get(self.seen.partition_point(|s| s.1 < hour)).filter(|s| s.0 <= hour);
        let live = seen.and_then(|&(_, end)| [next_facts, template, Some(end.saturating_add(1))].into_iter().flatten().min());
        [live, next_seen, removal].into_iter().flatten().min()
    }
}

#[derive(Clone, Default, PartialEq)]
pub(super) struct Template {
    name: Option<String>,
    sprite: Option<String>,
}

type Rank = (i32, Option<i64>, Option<i32>, Reverse<u64>);

#[derive(Default)]
pub(super) struct Roster {
    pub(super) divisions: BTreeMap<u64, Division>,
    pub(super) templates: HashMap<u64, Steps<Template>>,
    world: Steps<(Option<Rank>, bool)>,
    countries: HashMap<Option<u32>, Steps<(Option<Rank>, bool)>>,
}

#[derive(Default)]
struct Ranking {
    decorated: BTreeSet<Rank>,
    unknown: usize,
}

impl Ranking {
    fn change(&mut self, rank: Option<Rank>, insert: bool) {
        if let Some(rank) = rank {
            if insert {
                self.decorated.insert(rank);
            } else {
                self.decorated.remove(&rank);
            }
        } else if insert {
            self.unknown += 1;
        } else {
            self.unknown -= 1;
        }
    }

    fn top(&self) -> (Option<Rank>, bool) {
        (self.decorated.last().copied(), self.unknown != 0)
    }
}

impl Roster {
    pub(super) fn finish(&mut self, last: i32) {
        let mut edits = BTreeMap::<i32, Vec<_>>::new();
        for (&id, d) in &mut self.divisions {
            d.removals.sort_unstable();
            let experiences = std::mem::take(&mut d.experience);
            let manpower = std::mem::take(&mut d.manpower);
            let mut hours: Vec<_> = d
                .seen
                .iter()
                .flat_map(|&(first, last)| [first, last.saturating_add(1)])
                .chain(d.removals.iter().flat_map(|r| [r.0, r.0.saturating_add(1)]))
                .chain(d.posting.0.iter().map(|s| s.0))
                .chain(d.medals.0.iter().map(|s| s.0))
                .chain(experiences.0.iter().map(|s| s.0))
                .chain(manpower.0.iter().map(|s| s.0))
                .collect();
            hours.sort_unstable();
            hours.dedup();
            let mut held = None;
            let mut last_facts = None;
            for hour in hours.into_iter().take_while(|&h| h <= last) {
                let serving = d.seen_at(hour) == Some(hour);
                let medals = d.medals.at(hour).copied().flatten();
                let (raw, casualties) = experiences.at(hour).copied().unwrap_or_default();
                let experience = manpower.at(hour).copied().flatten().and_then(|manpower| {
                    DivisionStatusRow { experience: Recorded::Value(raw), ..Default::default() }.normalized_experience(manpower)
                });
                let facts = (experience, casualties);
                if serving && last_facts != Some(facts) {
                    d.facts.push(hour);
                }
                last_facts = Some(facts);
                let slot = d.posting.at(hour).and_then(|p| p.slot);
                let next = if serving && d.removed(hour).is_none() && medals.is_none_or(|n| n > 0) {
                    Some((slot, medals.map(|n| (n, experience, casualties, Reverse(id)))))
                } else {
                    None
                };
                if held != next {
                    if let Some((slot, rank)) = held {
                        edits.entry(hour).or_default().push((slot, rank, false));
                    }
                    if let Some((slot, rank)) = next {
                        edits.entry(hour).or_default().push((slot, rank, true));
                    }
                    held = next;
                }
            }
            d.facts.extend(d.name.0.iter().map(|s| s.0));
            d.facts.extend(d.posting.0.iter().map(|s| s.0));
            d.facts.sort_unstable();
            d.facts.dedup();
        }
        let mut world = Ranking::default();
        let mut by_slot = HashMap::<_, Ranking>::new();
        for (hour, edits) in edits {
            let mut changed = BTreeSet::new();
            for (slot, rank, insert) in edits {
                world.change(rank, insert);
                by_slot.entry(slot).or_default().change(rank, insert);
                changed.insert(slot);
            }
            self.world.put(hour, world.top());
            for slot in changed {
                self.countries.entry(slot).or_default().put(hour, by_slot[&slot].top());
            }
        }
    }

    pub(super) fn leader(&self, scope: &Scope, hour: i32) -> (Option<u64>, bool, Option<i32>) {
        let lists: Vec<_> = match &scope.slots {
            None => vec![&self.world],
            Some(slots) => slots.iter().filter_map(|slot| self.countries.get(&Some(*slot))).collect(),
        };
        let mut at = hour;
        let mut answer = None;
        loop {
            let mut best = None;
            let mut unknown = false;
            let mut next = None;
            for steps in &lists {
                let (rank, partial) = steps.at(at).copied().unwrap_or_default();
                best = best.max(rank);
                unknown |= partial;
                next = [next, steps.next(at)].into_iter().flatten().min();
            }
            if scope.slots.is_some()
                && let Some(unlinked) = self.countries.get(&None)
            {
                unknown |= unlinked.at(at).is_some_and(|(rank, missing)| rank.is_some() || *missing);
                next = [next, unlinked.next(at)].into_iter().flatten().min();
            }
            let current = (best.map(|rank| rank.3.0), unknown);
            let (id, partial) = *answer.get_or_insert(current);
            if current != (id, partial) {
                return (id, partial, Some(at));
            }
            let Some(change) = next else { return (id, partial, None) };
            at = change;
        }
    }

    pub(super) fn choice(&self, id: u64, hour: i32, scope: &Scope) -> Option<ServiceDivision> {
        let d = self.divisions.get(&id)?;
        let at = d.seen_at(hour)?;
        let posting = d.posting.at(at)?;
        if scope.slots.as_ref().is_some_and(|slots| !posting.slot.is_some_and(|s| slots.contains(&s))) {
            return None;
        }
        let template = self.templates.get(&posting.template).and_then(|steps| steps.at(at));
        let removed = d.removed(hour);
        Some(ServiceDivision {
            division: id.to_string(),
            last_seen: at,
            name: d.name.at(at).cloned().flatten(),
            tag: posting.tag.clone(),
            medals: d.medals.at(at).copied().flatten(),
            template: template.and_then(|t| t.name.clone()),
            sprite: template.and_then(|t| t.sprite.clone()),
            removed: removed.map(|r| r.2.clone()),
            removed_at: removed.map(|r| r.1),
        })
    }
}

#[derive(Default)]
pub(super) struct Scope {
    slots: Option<BTreeSet<u32>>,
    pub(super) unknown: bool,
    pub(super) next: Option<i32>,
}

impl Scope {
    pub(super) fn at(query: &Query<'_>, selection: Option<&Selection>, hour: i32) -> Self {
        let Some(set) = selection else { return Self::default() };
        let mut slots = BTreeSet::new();
        let mut unknown = false;
        let mut next = None;
        let mut span = |first, last: i32| {
            for at in [first, last.saturating_add(1)] {
                if at > hour && at <= query.timeline.last {
                    next = Some(next.map_or(at, |n: i32| n.min(at)));
                }
            }
            first <= hour && hour <= last
        };
        for member in &set.membership {
            if !span(member.first_hour, member.last_hour) {
                continue;
            }
            match &member.country {
                Country::Slot(slot) => {
                    slots.insert(*slot);
                }
                country => {
                    for run in query.holders.within(country, member.first_hour, member.last_hour) {
                        if span(run.first, run.last)
                            && let Holder::Found { slot, .. } = run.holder
                        {
                            slots.insert(slot);
                        }
                    }
                    unknown |= query.holders.slot_at(country, hour).is_none();
                }
            }
        }
        for gap in &set.unknown {
            unknown |= span(gap.first_hour, gap.last_hour);
        }
        for gap in &set.unobserved {
            unknown |= span(gap.first_hour, gap.last_hour);
        }
        Self { slots: Some(slots), unknown, next }
    }
}

pub(super) fn scan(
    query: &Query<'_>,
    stretch: &Stretch,
    links: &RunLinks,
    divisions: &mut BTreeMap<u64, Division>,
    templates: &mut HashMap<u64, Steps<Template>>,
    content: &Content,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let (first, last, run) = (stretch.first, stretch.last, stretch.run);
    let recording = query.replay.recording();
    let history = |table, columns: &[usize]| lanes::history(recording, run, table, columns, Keys::All, first, last);
    let names = Names::new(&query.holders);
    let columns = at_hour::field_columns(TableId::Divisions, &["roster_owner", "expeditionary_owner", "template", "reveal_skip"]);
    let census = history(TableId::Divisions, &columns)?;
    for (key, steps) in &census.keys {
        let d = divisions.entry(*key as u64).or_default();
        for (i, step) in steps.iter().enumerate() {
            let Some(cells) = &step.cells else { continue };
            let [Cell::Number(owner), Cell::Number(expeditionary), Cell::Number(template), Cell::Number(0)] = **cells else {
                continue;
            };
            let raw = replay_data::military_aid::owner(&DivisionRow {
                roster_owner: owner as u32,
                expeditionary_owner: expeditionary as u32,
                ..Default::default()
            });
            let end = steps.get(i + 1).map_or(last, |s| s.hour - 1);
            if let Some(prev) = d.seen.last_mut().filter(|prev| prev.1.checked_add(1) == Some(step.hour)) {
                prev.1 = end;
            } else {
                d.seen.push((step.hour, end));
            }
            for (slot, a, b) in links.within(raw, step.hour, end) {
                if let Some(slot) = slot {
                    for (tag, start, _) in names.slot_tags(slot, a, b) {
                        d.posting.put(start, Posting { template: template as u64, slot: Some(slot), tag: tag.map(str::to_owned) });
                    }
                } else {
                    d.posting.put(a, Posting { template: template as u64, slot: None, tag: None });
                }
            }
        }
    }
    for table in [TableId::DivisionStatus, TableId::DivisionLandInputs] {
        let fields: &[&str] = if table == TableId::DivisionStatus { &["naming", "country_ordinal", "name"] } else { &["manpower"] };
        let columns = at_hour::field_columns(table, fields);
        let hist = history(table, &columns)?;
        let mask = at_hour::field_mask(table, fields);
        let asks: Vec<_> = hist.keys.iter().flat_map(|(key, steps)| steps.iter().map(move |s| (s.hour, *key))).collect();
        lanes::at_each(recording, run, table, Some(&columns), &asks, &mut |hour, key, row| {
            check_cancel(cancelled)?;
            let d = divisions.entry(key as u64).or_default();
            let rows = row
                .map(|(view, index)| replay_frame::frame::cells_rows_of(table, &view, &[(key, index)], recording.format(), mask))
                .transpose()?;
            match rows {
                Some(TableRows::DivisionStatus(rows)) => d.name.put(hour, content.division_name(rows.first()).ok()),
                Some(TableRows::DivisionLandInputs(rows)) => {
                    d.manpower.put(hour, rows.first().map(|r| r.manpower.iter().fold(0i32, |sum, m| sum.wrapping_add(m.amount))))
                }
                _ if table == TableId::DivisionStatus => d.name.put(hour, None),
                _ => d.manpower.put(hour, None),
            }
            Ok(())
        })?;
    }
    let status =
        history(TableId::DivisionStatus, &at_hour::field_columns(TableId::DivisionStatus, &["experience", "career_counter"]))?;
    for (key, steps) in status.keys {
        let d = divisions.entry(key as u64).or_default();
        for s in steps {
            let num = |i: usize| match s.cells.as_ref()?.get(i)? {
                Cell::Number(n) => Some(*n),
                _ => None,
            };
            d.experience.put(s.hour, (num(0), num(1).map(|n| n as i32)));
        }
    }
    for d in divisions.values_mut() {
        d.name.put(last.saturating_add(1), None);
        d.experience.put(last.saturating_add(1), (None, None));
        d.manpower.put(last.saturating_add(1), None);
    }
    let mut hours = BTreeMap::<i32, BTreeSet<u128>>::new();
    for table in [TableId::DivisionTemplates, TableId::DivisionTemplateIcons] {
        let columns = template_columns(table).unwrap_or_default();
        let hist = history(table, &columns)?;
        for (key, steps) in &hist.keys {
            for step in steps {
                hours.entry(step.hour).or_default().insert(*key);
            }
            for status in hist.statuses.windows(2).filter(|s| s[0].status != s[1].status) {
                hours.entry(status[1].first).or_default().insert(*key);
            }
        }
    }
    let mut frames = Frames::new(query)?;
    for (at, ids) in hours {
        check_cancel(cancelled)?;
        let reads =
            [TableId::DivisionTemplates, TableId::DivisionTemplateIcons].map(|table| Read::these(table, ids.iter().copied()));
        let frame = frames.frame_with(at, &reads, template_columns, TITLE)?;
        for key in ids {
            let icon = witnessed::<tables::DivisionTemplateIcons>(&frame, key).row();
            let sprite = icon.zip(content.cat.as_ref().ok()).and_then(|(r, cat)| {
                super::super::super::icon_key(r.priority_unit, r.custom_icon, &r.priority_unit_name, |key| {
                    cat.sprites.find(key).map(|s| s.is_some())
                })
                .ok()
            });
            let name = content.template_name(&frame, key as u64).ok();
            let steps = templates.entry(key as u64).or_default();
            steps.put(at, Template { name: name.clone(), sprite: sprite.clone() });
            if name.is_some()
                && icon
                    .and_then(|r| {
                        r.display_name.require("display name").ok().and_then(|n| n.as_ref()).and_then(|n| n.sample_hour.stamp())
                    })
                    .is_some()
            {
                steps.put(at.saturating_add(1), Template { name: None, sprite });
            }
        }
    }
    for steps in templates.values_mut() {
        steps.put(last.saturating_add(1), Template::default());
    }
    Ok(())
}

fn template_columns(table: TableId) -> Option<Vec<usize>> {
    let fields: &[&str] = match table {
        TableId::DivisionTemplates => &["name", "name_key"],
        TableId::DivisionTemplateIcons => &["priority_unit", "priority_unit_name", "custom_icon", "country", "display_name"],
        _ => return None,
    };
    Some(at_hour::field_columns(table, fields))
}

use super::{EventFeedLine, EventFeedRow, Item, Locator, Notices, PlannedLine, Prepared, ScriptStatus, catalogue, families};
use crate::products::feed::index::HeldRow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

type Key = (u32, bool);
type Evidence = (i32, HeldRow);
type ArtWarnings<'a> = BTreeMap<String, BTreeSet<&'a str>>;

fn key(line: &PlannedLine) -> Option<Key> {
    match line {
        PlannedLine::Event { at, .. } => Some((*at, true)),
        PlannedLine::Fold { first_at, .. } => Some((*first_at, false)),
        PlannedLine::More { .. } => None,
    }
}

pub(super) struct ProjectedLine {
    pub row: Option<EventFeedLine>,
    evidence: [Option<Evidence>; 2],
    art: Option<(catalogue::OwnRefusals, Option<String>)>,
}

impl ProjectedLine {
    pub fn new(head: Option<Evidence>, choice: Option<Evidence>) -> Self {
        Self { row: None, evidence: [head, choice], art: None }
    }

    pub fn family(&mut self, family: &families::FamilyRow) -> EventFeedRow {
        let mut warnings = BTreeMap::new();
        let (row, own) = catalogue::capture_isolated(|| family.shown(&mut warnings));
        let warning = warnings.into_keys().next();
        if !own.is_empty() || warning.is_some() {
            self.art = Some((own, warning));
        }
        row
    }

    fn update(&mut self, plan: &PlannedLine, prepared: &Prepared, (last, hour): (i32, i32)) -> bool {
        match (&mut self.row, plan) {
            (Some(EventFeedLine::Event(row)), PlannedLine::Event { at, parent }) => {
                let event = prepared.index.get(*at);
                let state = event.state_at(hour);
                if event.state_at(last) != state {
                    return false;
                }
                if let Item::Log { edges, .. } = &event.payload {
                    let choice = |at: i32| -> Option<Locator> { prepared.scanned.visible_choice(edges, at) };
                    if choice(last) != choice(hour) {
                        return false;
                    }
                }
                if state.and_then(ScriptStatus::of_code) == Some(ScriptStatus::NotRecorded)
                    && row.not_recorded_reason.as_deref() != prepared.scanned.choice_break(event.observed, hour).map(|(_, reason)| reason)
                {
                    return false;
                }
                if row.parent != *parent { Arc::make_mut(row).parent.clone_from(parent); }
                true
            }
            (Some(EventFeedLine::Fold(row)), PlannedLine::Fold { key, observed, count, kind, country, .. }) => {
                let count = count.to_string();
                if row.key != *key || row.observed != *observed || row.count != count || row.kind != *kind || row.country != *country {
                    let row = Arc::make_mut(row);
                    row.key.clone_from(key);
                    row.observed = *observed;
                    row.count = count;
                    row.kind = *kind;
                    row.country.clone_from(country);
                }
                true
            }
            _ => false,
        }
    }
}

#[derive(Default)]
pub(super) struct FeedPage {
    hour: i32,
    lines: HashMap<Key, ProjectedLine>,
}

pub(super) struct StagedPage<'a> {
    cache: Option<&'a mut FeedPage>,
    hour: i32,
    keys: Vec<Option<Key>>,
    lines: Vec<Option<ProjectedLine>>,
}

impl<'a> StagedPage<'a> {
    pub fn new(cache: Option<&'a mut FeedPage>, prepared: &Prepared, hour: i32, planned: Vec<PlannedLine>) -> (Self, Vec<PlannedLine>) {
        let mut seen = HashSet::new();
        let duplicates: HashSet<_> = if cache.is_some() {
            planned.iter().filter_map(key).map(|(at, _)| at).filter(|&at| !seen.insert(at)).collect()
        } else {
            HashSet::new()
        };
        let mut page = Self { cache, hour, keys: Vec::with_capacity(planned.len()), lines: Vec::with_capacity(planned.len()) };
        let mut missing = Vec::new();
        for plan in planned {
            let key = key(&plan).filter(|(at, _)| !duplicates.contains(at));
            let line = key.and_then(|key| {
                let previous = page.cache.as_mut()?;
                let mut held = previous.lines.remove(&key)?;
                (previous.hour <= hour && held.update(&plan, prepared, (previous.hour, hour))).then_some(held)
            });
            if line.is_none() { missing.push(plan); }
            page.keys.push(key);
            page.lines.push(line);
        }
        (page, missing)
    }

    pub fn finish(&mut self, mut projected: impl Iterator<Item = ProjectedLine>) -> &mut Self {
        for line in &mut self.lines {
            if line.is_none() { *line = Some(projected.next().expect("a projection for each missing line")); }
        }
        self
    }

    pub fn credit(&self, notices: &mut Notices) -> ArtWarnings<'_> {
        let mut records: Vec<_> = self.lines.iter().flatten().flat_map(|line| line.evidence.iter().flatten()).collect();
        records.sort_by_key(|(hour, _)| *hour);
        for (_, source) in records {
            catalogue::credit(&source.2);
            notices.absorb(&source.1);
        }
        let mut art = BTreeMap::<_, BTreeSet<_>>::new();
        for line in self.lines.iter().flatten() {
            let Some((own, warning)) = &line.art else { continue };
            catalogue::credit(own);
            if let Some(warning) = warning {
                let id = match line.row.as_ref().expect("projected family row") {
                    EventFeedLine::Event(row) => row.key.as_str(),
                    EventFeedLine::Fold(row) => row.first_row.as_ref().expect("family fold shows its first row").id.as_str(),
                    EventFeedLine::More(_) => unreachable!(),
                };
                art.entry(warning.clone()).or_default().insert(id);
            }
        }
        art
    }

    pub fn rows(&self) -> Vec<EventFeedLine> {
        self.lines.iter().flatten().map(|line| line.row.as_ref().expect("projected row").clone()).collect()
    }

    pub fn keep(&mut self, rows: &[EventFeedLine]) {
        if let Some(cache) = self.cache.as_mut() {
            cache.hour = self.hour;
            cache.lines.clear();
            for ((key, line), row) in self.keys.drain(..).zip(self.lines.drain(..)).zip(rows) {
                if let Some((key, mut line)) = key.zip(line) {
                    line.row = Some(row.clone());
                    cache.lines.insert(key, line);
                }
            }
        }
    }
}

//! Each faction's goal lists and labels, published as that faction's own page.
use super::*;
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct GoalList {
    pub absent: bool,
    pub reason: Option<String>,
    /// Ordinal, label and native status of each runtime goal.
    pub goals: Vec<(u32, u32, i32)>,
    pub completed: Vec<u32>,
    pub cancelled: Vec<u32>,
}
impl GoalList {
    pub fn absent() -> Self {
        Self {
            absent: true,
            ..Default::default()
        }
    }
}
/// Values in first-seen order, found again by their hash.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct Interned<T> {
    values: Vec<T>,
    #[serde(skip)]
    index: HashMap<u64, Vec<u32>>,
    #[serde(skip)]
    indexed: usize,
}
fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}
impl<T: Hash + Eq + Serialize> Interned<T> {
    pub fn get(&self, index: u32) -> Option<&T> {
        self.values.get(index as usize)
    }
    /// Its index, and the bytes it added when it is new.
    pub fn intern(&mut self, value: T) -> (u32, usize) {
        // A restored book holds no index: build it once.
        if self.indexed != self.values.len() {
            self.index.clear();
            for (at, held) in self.values.iter().enumerate() {
                self.index.entry(hash_of(held)).or_default().push(at as u32);
            }
            self.indexed = self.values.len();
        }
        let hash = hash_of(&value);
        if let Some(&at) = self.index.get(&hash).and_then(|candidates| {
            candidates
                .iter()
                .find(|at| self.values[**at as usize] == value)
        }) {
            return (at, 0);
        }
        let at = self.values.len() as u32;
        let bytes = serde_json::to_vec(&value).map_or(0, |text| text.len());
        self.values.push(value);
        self.index.entry(hash).or_default().push(at);
        self.indexed += 1;
        (at, bytes)
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct GoalBook {
    pub labels: Interned<GoalLabel>,
    pub lists: Interned<GoalList>,
}
/// The list a reply synthesizes for hours a complete roster lacks the faction.
pub(super) const ABSENT: u32 = u32::MAX;
pub(super) fn family(faction: u64) -> String {
    format!("faction-goals-{faction}")
}
/// The books, and which changed since their pages were last published.
#[derive(Default, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct GoalPages {
    books: BTreeMap<u64, GoalBook>,
    #[serde(skip)]
    changed: BTreeSet<u64>,
    #[serde(skip)]
    pending: usize,
    #[serde(skip)]
    writers: BTreeMap<u64, CatalogueWriter>,
}
impl GoalPages {
    pub fn label(&mut self, faction: u64, label: GoalLabel) -> u32 {
        let (at, bytes) = self.books.entry(faction).or_default().labels.intern(label);
        self.added(faction, bytes);
        at
    }
    pub fn list(&mut self, faction: u64, list: GoalList) -> u32 {
        let (at, bytes) = self.books.entry(faction).or_default().lists.intern(list);
        self.added(faction, bytes);
        at
    }
    fn added(&mut self, faction: u64, bytes: usize) {
        if bytes > 0 {
            self.changed.insert(faction);
            self.pending += bytes;
        }
    }
    /// Bytes held for the next publication.
    pub fn pending(&self) -> usize {
        self.pending
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let keys = BTreeMap::new();
        for faction in std::mem::take(&mut self.changed) {
            check_cancel(cancelled)?;
            self.writers.entry(faction).or_default().write(
                writer,
                &family(faction),
                branch,
                &keys,
                &self.books[&faction],
            )?;
        }
        self.pending = 0;
        Ok(())
    }
}
pub(super) fn read(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    faction: u64,
) -> io::Result<GoalBook> {
    Ok(
        series::read_catalogue::<GoalBook>(store, key, &family(faction), branch)?
            .map(|(_, book)| book)
            .unwrap_or_default(),
    )
}
/// The JSON a reply grows by, refused once it would pass the transport budget.
#[derive(Default)]
pub(super) struct Budget(usize);
impl Budget {
    pub fn add<T: Serialize>(&mut self, value: &T) -> io::Result<()> {
        self.0 += serde_json::to_vec(value)?.len() + 1;
        if self.0 > dashboard_base::products::MAX_PRODUCT_REPLY_BYTES {
            return Err(io::Error::other(format!(
                "This product reply exceeds the transport reply budget of {} bytes",
                dashboard_base::products::MAX_PRODUCT_REPLY_BYTES
            )));
        }
        Ok(())
    }
}
/// Ordered disjoint spans of `spans` outside every span of `held`.
pub(super) fn outside(spans: &[HourSpan], held: &[HourSpan]) -> Vec<HourSpan> {
    let mut result = Vec::new();
    for span in spans {
        let mut start = i64::from(span.first_hour);
        for gap in held {
            if i64::from(gap.last_hour) < start || gap.first_hour > span.last_hour {
                continue;
            }
            if start < i64::from(gap.first_hour) {
                result.push(HourSpan {
                    first_hour: start as i32,
                    last_hour: gap.first_hour - 1,
                });
            }
            start = start.max(i64::from(gap.last_hour) + 1);
        }
        if start <= i64::from(span.last_hour) {
            result.push(HourSpan {
                first_hour: start as i32,
                last_hour: span.last_hour,
            });
        }
    }
    result
}
/// The reply's lists in shown order, each label they name drawn once as faction details draw it.
fn definitions(
    source: &Source,
    context: &LoadContext,
    book: &GoalBook,
    lists: Vec<(u32, u32)>,
    budget: &mut Budget,
    reply: &mut FactionGoalsReply,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let cat = match context
        .content
        .as_ref()
        .map(|options| catalogue::catalogue(source, options))
        .transpose()
    {
        Ok(cat) => cat,
        Err(reason) => {
            reply
                .warnings
                .push(format!("Faction goal content: {reason}"));
            None
        }
    };
    if let Some(cat) = cat.as_deref() {
        reply
            .warnings
            .extend(faction_details::goal_content_warnings(cat));
    }
    budget.add(&reply.warnings)?;
    let mut labels = BTreeMap::<u32, u32>::new();
    let absent = GoalList::absent();
    for (list, _) in lists {
        check_cancel(cancelled)?;
        let held = if list == ABSENT {
            &absent
        } else {
            book.lists
                .get(list)
                .ok_or_else(|| dashboard_base::invalid("Faction goal list is unavailable"))?
        };
        let mut label = |index: u32| -> io::Result<u32> {
            if let Some(&shown) = labels.get(&index) {
                return Ok(shown);
            }
            check_cancel(cancelled)?;
            let found = book
                .labels
                .get(index)
                .ok_or_else(|| dashboard_base::invalid("Faction goal label is unavailable"))?;
            let definition = faction_details::finish_goal(found, cat.as_deref());
            budget.add(&definition)?;
            let shown = reply.definitions.len() as u32;
            reply.definitions.push(definition);
            labels.insert(index, shown);
            Ok(shown)
        };
        let mut shown = FactionGoalList {
            absent: held.absent,
            reason: held.reason.clone(),
            goals: Vec::new(),
            completed: Vec::new(),
            cancelled: Vec::new(),
        };
        for &(ordinal, index, status) in &held.goals {
            shown.goals.push(FactionHeldGoal {
                ordinal,
                definition: label(index)?,
                status,
            });
        }
        for &index in &held.completed {
            shown.completed.push(label(index)?);
        }
        for &index in &held.cancelled {
            shown.cancelled.push(label(index)?);
        }
        budget.add(&shown)?;
        reply.lists.push(shown);
    }
    Ok(())
}
/// The goal view's faction and the hours it asks for; None is open-ended.
pub(super) struct Asked {
    pub branch: u64,
    pub faction: u64,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
}
/// One faction's goal history over the asked hours, reading only its own page.
pub(super) fn query(
    source: &Source,
    context: &LoadContext,
    (store, key): (&store::Store, &store::SourceKey),
    (keys, catalogue, reader): (&BTreeMap<String, u64>, &Catalogue, &mut Reader<'_>),
    asked: Asked,
    prefix: Prefix,
    cancelled: &AtomicBool,
) -> io::Result<FactionGoalsReply> {
    let (first, last) = (
        asked.first_hour.unwrap_or(i32::MIN),
        asked.last_hour.unwrap_or(i32::MAX),
    );
    // Prefix observations never revise an earlier hour, so runs are clipped, not batches.
    let from = prefix.coverage.first_hour.max(first);
    let to = prefix.coverage.last_hour.min(last);
    let faction = asked.faction;
    let lane = |kind: Kind, identity: &str| -> io::Result<Option<u64>> {
        Ok(keys
            .get(&serde_json::to_string(&(
                faction.to_string(),
                kind,
                identity,
            ))?)
            .copied())
    };
    let known = match lane(Kind::Known, "goals")? {
        Some(id) => spans(&reader.runs(id, from, to)?, 1),
        None => Vec::new(),
    };
    let reads = match lane(Kind::Read, "goals")? {
        Some(id) => spans(&reader.runs(id, from, to)?, 1),
        None => Vec::new(),
    };
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
            channel: "goals".into(),
            known,
            read: reads,
        }],
        warnings: Vec::new(),
    };
    let mut budget = Budget::default();
    budget.add(&reply)?;
    let mut lists = BTreeMap::<u32, u32>::new();
    let mut held = Vec::new();
    for (&id, lane) in catalogue
        .lanes
        .iter()
        .filter(|(_, lane)| lane.faction == faction)
    {
        check_cancel(cancelled)?;
        match lane.kind {
            Kind::GoalList => {
                for run in reader.runs(id, from, to)? {
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
            Kind::GoalProgress => {
                let mut runs = Vec::new();
                for run in reader.runs(id, from, to)? {
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
                        ordinal: lane.slot,
                        runs,
                    });
                }
            }
            _ => {}
        }
    }
    // Before the faction's first lists, only a complete roster proves it absent.
    if let Some(roster) = keys.get(&serde_json::to_string(&("0", Kind::Roster, ""))?) {
        let complete = spans(&reader.runs(*roster, from, to)?, 1);
        held.sort_by_key(|span| span.first_hour);
        let absent = outside(&complete, &held);
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
        read(store, key, asked.branch, faction)?
    } else {
        GoalBook::default()
    };
    let mut ordered: Vec<_> = lists.into_iter().collect();
    ordered.sort_by_key(|(_, shown)| *shown);
    definitions(
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_restored_book_finds_its_values_again_in_first_seen_order() {
        let mut pages = GoalPages::default();
        let label = |name: &str| GoalLabel {
            token: Some(name.into()),
            ..Default::default()
        };
        assert_eq!(pages.label(9, label("a")), 0);
        assert_eq!(pages.label(9, label("b")), 1);
        assert_eq!(pages.label(9, label("a")), 0);
        assert_eq!(pages.list(9, GoalList::absent()), 0);
        let restored: GoalPages =
            serde_json::from_value(serde_json::to_value(&pages).unwrap()).unwrap();
        let mut restored = restored;
        assert_eq!(restored.pending(), 0);
        assert_eq!(restored.label(9, label("b")), 1);
        assert_eq!(
            restored.pending(),
            0,
            "a held value adds nothing to publish"
        );
        assert_eq!(restored.label(9, label("c")), 2);
        assert!(restored.pending() > 0);
        assert_eq!(restored.list(9, GoalList::absent()), 0);
        assert_eq!(
            restored.label(10, label("a")),
            0,
            "each faction has its own book"
        );
    }
    #[test]
    fn absence_is_only_the_complete_hours_outside_held_lists() {
        let span = |first_hour, last_hour| HourSpan {
            first_hour,
            last_hour,
        };
        assert_eq!(
            outside(
                &[span(100, 101), span(103, 110)],
                &[span(105, 106), span(108, 120)]
            ),
            [span(100, 101), span(103, 104), span(107, 107)]
        );
        assert!(outside(&[], &[span(1, 2)]).is_empty());
    }
}

use super::{
    rows::{Groups, Stamps, unrecorded},
    *,
};
use replay::{
    columnar_frames::{Views, metrics},
    columns,
    segments::{self, Operator, Sink, Start, State},
};
use replay_data::{
    cadence,
    rows::{IncomingDiplomaticActionRow, LexicalTokenRow, TABLE_COUNT},
    scope::CaptureMode,
};

struct Fields {
    country_tag: usize,
    country_slot: usize,
    opinion: usize,
    sample_hour: usize,
}

pub struct Projector {
    format: FormatVersion,
    naming: Views,
    fields: Fields,
    shapes: [Option<columns::Status>; TABLE_COUNT],
    ids: Option<Arc<Ids>>,
    opinions: Groups,
    tuples: Groups,
    stamps: Stamps,
    /// The last values by country, and whether its rows are unchanged since.
    opinion_values: HashMap<u32, (Arc<Values>, bool)>,
    tuple_values: HashMap<u32, (Arc<TupleRows>, bool)>,
    spellings: HashMap<u32, Option<Arc<str>>>,
    orphan_counts: HashMap<u32, usize>,
    orphan_opinions: usize,
    orphan_tuples: BTreeSet<u32>,
    orphan_slices: BTreeMap<u32, BTreeSet<u32>>,
    /// Whether the emit may hold another orphan set than `orphan_tuples`.
    orphans_pending: bool,
    queue: Option<Arc<proposals::Queue>>,
    /// Whether the emit may admit a baseline at the next hour.
    baseline: bool,
}

fn slice(hour: i32) -> u32 {
    hour.rem_euclid(cadence::STAGGER_PERIOD) as u32
}

impl Ids {
    fn new(joins: Joins, canonical: Vec<u32>) -> Self {
        let countries: Vec<DiplomacyCountry> = joins.countries.values().cloned().collect();
        let index: Vec<(u32, u32)> = joins
            .raw
            .iter()
            .filter_map(|(&raw, country)| {
                Some((raw, countries.binary_search(country).ok()? as u32))
            })
            .collect();
        let aliases: Vec<Box<[u32]>> = countries
            .iter()
            .map(|country| {
                joins
                    .aliases
                    .get(country)
                    .map_or_else(Default::default, |raws| raws.as_slice().into())
            })
            .collect();
        let mut slices = vec![Vec::new(); cadence::STAGGER_PERIOD as usize];
        for (at, country) in countries.iter().enumerate() {
            slices[cadence::slice_of(country.country_id) as usize].push(at as u32);
        }
        Self {
            joins,
            countries,
            index,
            aliases,
            slices,
            canonical,
        }
    }
}
impl Values {
    /// Values of `pairs`, sharing what the owner's last values hold alike.
    fn new(pairs: Vec<(u32, i32)>, unresolved: usize, old: Option<&Arc<Self>>) -> Arc<Self> {
        let targets = old.map(|old| &old.targets).filter(|targets| {
            targets
                .iter()
                .copied()
                .eq(pairs.iter().map(|&(target, _)| target))
        });
        if let Some(old) = old
            && targets.is_some()
            && old.unresolved == unresolved
            && old
                .values
                .iter()
                .copied()
                .eq(pairs.iter().map(|&(_, value)| value))
        {
            return Arc::clone(old);
        }
        Arc::new(Self {
            targets: match targets {
                Some(targets) => Arc::clone(targets),
                None => pairs.iter().map(|&(target, _)| target).collect(),
            },
            values: pairs.iter().map(|&(_, value)| value).collect(),
            unresolved,
        })
    }
}
impl TupleRows {
    /// Rows of one owner, sharing what its last rows hold alike.
    fn new(
        rows: Vec<RelationTupleRow>,
        spellings: Vec<Option<Arc<str>>>,
        old: Option<&Arc<Self>>,
    ) -> Arc<Self> {
        let identity = old
            .filter(|old| {
                *old.identity == *spellings
                    && old.rows.len() == rows.len()
                    && old.rows.iter().zip(&rows).all(|(old, row)| {
                        old.key() == row.key()
                            && old.first_country == row.first_country
                            && old.second_country == row.second_country
                            && old.start_hour == row.start_hour
                    })
            })
            .map(|old| &old.identity);
        if let Some(old) = old
            && identity.is_some()
            && *old.rows == *rows
        {
            return Arc::clone(old);
        }
        Arc::new(Self {
            identity: match identity {
                Some(identity) => Arc::clone(identity),
                None => spellings.into(),
            },
            rows: rows.into_boxed_slice(),
        })
    }
}

impl Projector {
    pub fn new(format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        Ok(Self {
            format,
            naming: Views::new(&[TableId::Countries, TableId::CountryLinks], format, mode)?,
            fields: Fields {
                country_tag: columns::field(TableId::Countries, "tag")?,
                country_slot: columns::field(TableId::Countries, "canonical_slot")?,
                opinion: columns::field(TableId::RelationOpinions, "opinion")?,
                sample_hour: columns::field(TableId::SliceReads, "sample_hour")?,
            },
            shapes: [None; TABLE_COUNT],
            ids: None,
            opinions: Groups::new(|key| (key >> 32) as u32),
            tuples: Groups::new(|key| (key >> 80) as u32),
            stamps: Stamps::default(),
            opinion_values: HashMap::new(),
            tuple_values: HashMap::new(),
            spellings: HashMap::new(),
            orphan_counts: HashMap::new(),
            orphan_opinions: 0,
            orphan_tuples: BTreeSet::new(),
            orphan_slices: BTreeMap::new(),
            orphans_pending: true,
            queue: None,
            baseline: true,
        })
    }
    /// Follows the hour's tables; whether the identities may have changed.
    fn tables(&mut self, hour: &segments::Hour<'_>) -> io::Result<bool> {
        let state = hour.state;
        let mut changed = [false; TABLE_COUNT];
        let mut reshaped = [false; TABLE_COUNT];
        for &table in super::super::super::block::TABLES {
            let index = table.index();
            let shape = columns::status(state, table);
            reshaped[index] = self.shapes[index] != Some(shape);
            let edits = columns::changes(state, table)?;
            changed[index] = hour.starts
                || hour.opens
                || reshaped[index]
                || !edits.changed.is_empty()
                || !edits.fill_slots.is_empty()
                || !edits.inserted.is_empty()
                || !edits.deleted.is_empty()
                || edits.truncated;
            self.shapes[index] = Some(shape);
        }
        let countries = columns::changes(state, TableId::Countries)?;
        let fields = [self.fields.country_tag, self.fields.country_slot];
        let named = |&mask: &u64| {
            fields
                .iter()
                .any(|&field| field >= 64 || mask >> field & 1 != 0)
        };
        let naming = hour.starts
            || hour.opens
            || reshaped[TableId::Countries.index()]
            || changed[TableId::CountryLinks.index()]
            || !countries.inserted.is_empty()
            || !countries.deleted.is_empty()
            || countries
                .masks
                .iter()
                .chain(&countries.fill_masks)
                .any(named);
        self.opinions.update(hour, TableId::RelationOpinions)?;
        self.tuples.update(hour, TableId::RelationTuples)?;
        self.stamps.update(hour, self.fields.sample_hour)?;
        if changed[TableId::LexicalTokens.index()] {
            self.spellings.clear();
            self.tuple_values.clear();
        }
        for raw in &self.tuples.touched {
            if let Some(rows) = self.tuple_values.get_mut(raw) {
                rows.1 = false;
            }
        }
        Ok(naming)
    }
    fn orphans(&mut self, ids: &Ids, naming: bool) {
        if naming {
            self.orphan_counts = self
                .opinions
                .owners()
                .filter(|&(raw, _)| ids.index_of(raw).is_none())
                .collect();
            self.orphan_opinions = self.orphan_counts.values().sum();
            let orphans: BTreeSet<u32> = self
                .tuples
                .owners()
                .map(|(raw, _)| raw)
                .filter(|&raw| ids.index_of(raw).is_none())
                .collect();
            if orphans != self.orphan_tuples {
                self.orphan_slices.clear();
                for &raw in &orphans {
                    self.orphan_slices
                        .entry(cadence::slice_of(raw))
                        .or_default()
                        .insert(raw);
                }
                self.orphan_tuples = orphans;
                self.orphans_pending = true;
            }
            return;
        }
        for &raw in &self.opinions.touched {
            self.orphan_opinions -= self.orphan_counts.remove(&raw).unwrap_or(0);
            let count = self.opinions.owner(raw).len();
            if count > 0 && ids.index_of(raw).is_none() {
                self.orphan_counts.insert(raw, count);
                self.orphan_opinions += count;
            }
        }
        for &raw in &self.tuples.touched {
            let slice = cadence::slice_of(raw);
            if ids.index_of(raw).is_none() && !self.tuples.owner(raw).is_empty() {
                if self.orphan_tuples.insert(raw) {
                    self.orphan_slices.entry(slice).or_default().insert(raw);
                    self.orphans_pending = true;
                }
            } else if self.orphan_tuples.remove(&raw) {
                if let Some(owners) = self.orphan_slices.get_mut(&slice) {
                    owners.remove(&raw);
                }
                self.orphans_pending = true;
            }
        }
    }
    fn read_at(&self, ids: &Ids, table: TableId, raw: u32, hour: i32, full: bool) -> Option<i32> {
        if full {
            return Some(hour);
        }
        if let Some(read) = self.stamps.stamp(table, raw) {
            return (read <= hour
                && i64::from(hour) - i64::from(read) < i64::from(cadence::STAGGER_PERIOD))
            .then_some(read);
        }
        (ids.canonical.binary_search(&raw).is_ok() && cadence::slice_of(raw) == slice(hour))
            .then_some(hour)
    }
    fn read(&self, ids: &Ids, table: TableId, country: u32, hour: i32, full: bool) -> Option<i32> {
        std::iter::once(ids.countries[country as usize].country_id)
            .chain(ids.aliases[country as usize].iter().copied())
            .filter_map(|raw| self.read_at(ids, table, raw, hour, full))
            .max()
    }
    fn selection(
        &self,
        ids: &Ids,
        table: TableId,
        hour: i32,
        full: bool,
    ) -> Vec<(u32, Option<i32>)> {
        let mut selected: Vec<u32> = if full {
            (0..ids.countries.len() as u32).collect()
        } else {
            ids.slices[slice(hour) as usize]
                .iter()
                .copied()
                .filter(|&country| {
                    let id = ids.countries[country as usize].country_id;
                    self.read_at(ids, table, id, hour, full) == Some(hour)
                })
                .collect()
        };
        selected.extend(
            self.stamps
                .current(table, hour)
                .filter_map(|raw| ids.index_of(raw)),
        );
        selected.sort_unstable();
        selected.dedup();
        selected
            .into_iter()
            .map(|country| (country, self.read(ids, table, country, hour, full)))
            .collect()
    }
    fn baseline_selection(
        &self,
        ids: &Ids,
        table: TableId,
        hour: i32,
        full: bool,
        current: &[(u32, Option<i32>)],
    ) -> Vec<(u32, Option<i32>)> {
        (0..ids.countries.len() as u32)
            .filter(|&country| {
                let id = ids.countries[country as usize].country_id;
                current
                    .binary_search_by_key(&country, |read| read.0)
                    .is_err()
                    && self.read_at(ids, table, id, hour, full).is_some()
            })
            .map(|country| (country, self.read(ids, table, country, hour, full)))
            .collect()
    }
    fn opinion(
        &mut self,
        state: &State,
        ids: &Ids,
        selected: Vec<(u32, Option<i32>)>,
    ) -> io::Result<Vec<Opinion>> {
        if selected.is_empty() {
            return Ok(Vec::new());
        }
        let field = self.fields.opinion;
        let held = columns::table(state, TableId::RelationOpinions)?;
        let cells = columns::plain::<i32>(held, field)?;
        let mut projected = Vec::with_capacity(selected.len());
        for (country, read) in selected {
            let old = match self.opinion_values.get(&country) {
                Some((values, true)) => {
                    projected.push((country, read, Arc::clone(values)));
                    continue;
                }
                Some((values, false)) => Some(Arc::clone(values)),
                None => None,
            };
            let rows = ids.aliases[country as usize]
                .iter()
                .flat_map(|&raw| self.opinions.owner(raw))
                .map(|&slot| -> io::Result<(u32, i32)> {
                    match (held.key(slot), cells.get(slot as usize)) {
                        (Some(key), Some(&opinion)) if held.recorded(field, slot) => {
                            Ok((key as u32, opinion))
                        }
                        _ => Err(unrecorded(TableId::RelationOpinions, slot)),
                    }
                });
            let (pairs, unresolved) =
                opinions::project(rows, |raw| ids.index_of(raw), &mut || Ok(()))?;
            let values = Values::new(pairs, unresolved, old.as_ref());
            self.opinion_values
                .insert(country, (Arc::clone(&values), true));
            projected.push((country, read, values));
        }
        Ok(projected)
    }
    fn spelling(&mut self, state: &State, token: u32) -> io::Result<Option<Arc<str>>> {
        if let Some(spelling) = self.spellings.get(&token) {
            return Ok(spelling.clone());
        }
        let spelling: Option<Arc<str>> = if columns::complete(state, TableId::LexicalTokens)
            && let Some(slot) =
                columns::table(state, TableId::LexicalTokens)?.slot(u128::from(token))
        {
            let row: LexicalTokenRow =
                columns::row(state, TableId::LexicalTokens, slot, self.format.version())?;
            std::str::from_utf8(&row.literal).ok().map(Arc::from)
        } else {
            None
        };
        self.spellings.insert(token, spelling.clone());
        Ok(spelling)
    }
    fn tuple_rows(&mut self, state: &State, raw: u32) -> io::Result<Arc<TupleRows>> {
        let old = match self.tuple_values.get(&raw) {
            Some((rows, true)) => return Ok(Arc::clone(rows)),
            Some((rows, false)) => Some(Arc::clone(rows)),
            None => None,
        };
        let format = self.format.version();
        let rows = self
            .tuples
            .owner(raw)
            .iter()
            .map(|&slot| columns::row(state, TableId::RelationTuples, slot, format))
            .collect::<io::Result<Vec<RelationTupleRow>>>()?;
        let mut spellings = Vec::with_capacity(rows.len());
        for row in &rows {
            spellings.push(self.spelling(state, row.type_id)?);
        }
        let rows = TupleRows::new(rows, spellings, old.as_ref());
        self.tuple_values.insert(raw, (Arc::clone(&rows), true));
        Ok(rows)
    }
    /// For a baseline, every orphan with a proven read and none of `baseline`'s owners.
    fn tuple_owners(
        &mut self,
        state: &State,
        ids: &Ids,
        countries: &[(u32, i32)],
        hour: i32,
        full: bool,
        baseline: Option<&[TupleOwner]>,
    ) -> io::Result<Vec<TupleOwner>> {
        let mut selected = BTreeMap::new();
        for &(country, read) in countries {
            for &raw in ids.aliases[country as usize].iter() {
                selected.insert(raw, read);
            }
        }
        if full || baseline.is_some() {
            for &raw in &self.orphan_tuples {
                if let Some(read) = self.read_at(ids, TableId::RelationTuples, raw, hour, full) {
                    selected.insert(raw, read);
                }
            }
        } else {
            for raw in self.stamps.current(TableId::RelationTuples, hour) {
                if self.orphan_tuples.contains(&raw) {
                    selected.insert(raw, hour);
                }
            }
            // Unresolved canonical rows can still prove their own rowless scheduled read.
            for &raw in self.orphan_slices.get(&slice(hour)).into_iter().flatten() {
                if self.read_at(ids, TableId::RelationTuples, raw, hour, full) == Some(hour) {
                    selected.insert(raw, hour);
                }
            }
        }
        let current = baseline.unwrap_or_default();
        selected
            .into_iter()
            .filter(|(raw, _)| current.binary_search_by_key(raw, |owner| owner.0).is_err())
            .map(|(raw, read)| Ok((raw, read, self.tuple_rows(state, raw)?)))
            .collect()
    }
    /// The queue's census with `ids`' roles; None while the table is not complete.
    fn census(&mut self, hour: &segments::Hour<'_>, ids: &Ids, naming: bool) -> io::Result<()> {
        let table = TableId::IncomingDiplomaticActions;
        let state = hour.state;
        if !columns::complete(state, table) {
            self.queue = None;
            return Ok(());
        }
        let held = columns::table(state, table)?;
        let format = self.format.version();
        let project = |slot: u32| -> io::Result<(u128, Arc<proposals::Proposal>)> {
            let row: IncomingDiplomaticActionRow = columns::row(state, table, slot, format)?;
            Ok((row.key(), proposals::project_one(&row, &ids.joins)?))
        };
        let follow = !(naming || hour.starts || hour.opens);
        if let Some(queue) = self.queue.as_mut().filter(|_| follow) {
            let edits = columns::changes(state, table)?;
            if edits.deleted.is_empty()
                && edits.changed.is_empty()
                && edits.fill_slots.is_empty()
                && edits.inserted.is_empty()
            {
                return Ok(());
            }
            let queue = Arc::make_mut(queue);
            for &key in &edits.deleted_keys {
                queue.set(key, None);
            }
            for &slot in edits
                .changed
                .iter()
                .chain(&edits.fill_slots)
                .chain(&edits.inserted)
            {
                if held.live().get(slot) {
                    let (key, proposal) = project(slot)?;
                    queue.set(key, Some(proposal));
                }
            }
        } else {
            let mut queue = proposals::Queue::default();
            for slot in held.live().ones() {
                let (key, proposal) = project(slot)?;
                queue.set(key, Some(proposal));
            }
            self.queue = Some(Arc::new(queue));
        }
        Ok(())
    }
}
impl Operator for Projector {
    type Output = Hour;
    fn start(&mut self, start: Start<'_>, _: u64, _: i32) -> io::Result<()> {
        // The emit admits a baseline only at its branch's first hour and after a refused one.
        self.baseline = match start {
            Start::Restart(state) => {
                !(columns::complete(state, TableId::Countries)
                    && columns::complete(state, TableId::CountryLinks))
            }
            Start::Run { .. } => true,
        };
        Ok(())
    }
    fn hour(&mut self, hour: &segments::Hour<'_>, out: &mut dyn Sink<Hour>) -> io::Result<()> {
        let state = hour.state;
        let naming = {
            let _cpu = replay_frame::cpu::scope("project", Some("diplomacy_state.tables"));
            self.tables(hour)?
        };
        if naming {
            let _cpu = replay_frame::cpu::scope("project", Some("diplomacy_state.identities"));
            let frame = self.naming.advance(state, hour.manifest, true)?;
            self.ids = Joins::read(&frame).map(|joins| {
                let canonical: Vec<u32> = frame
                    .complete::<tables::Countries>()
                    .map(|rows| rows.iter().map(|row| row.id).collect())
                    .unwrap_or_default();
                Arc::new(Ids::new(joins, canonical))
            });
            self.opinion_values.clear();
        }
        let Some(ids) = self.ids.clone() else {
            self.baseline = true;
            self.queue = None;
            return out.push(Hour {
                branch: hour.branch,
                hour: hour.hour,
                ids: None,
                refused: true,
                baseline: false,
                opinions: None,
                tuples: None,
                queue: QueueRead::Unavailable,
            });
        };
        for raw in &self.opinions.touched {
            if let Some(country) = ids.index_of(*raw)
                && let Some(values) = self.opinion_values.get_mut(&country)
            {
                values.1 = false;
            }
        }
        self.orphans(&ids, naming);
        let metrics = metrics(hour.manifest);
        let full = evidence::full_refresh_at(hour.hour, &metrics, self.format);
        let ready = |table| {
            columns::complete(state, table)
                && (full || columns::complete(state, TableId::SliceReads))
        };
        let baseline = std::mem::take(&mut self.baseline);
        let opinions = if ready(TableId::RelationOpinions) {
            let _cpu = replay_frame::cpu::scope("project", Some("diplomacy_state.opinions"));
            let table = TableId::RelationOpinions;
            let current = self.selection(&ids, table, hour.hour, full);
            let added = if baseline {
                self.baseline_selection(&ids, table, hour.hour, full, &current)
            } else {
                Vec::new()
            };
            Some(Opinions {
                current: self.opinion(state, &ids, current)?,
                baseline: self.opinion(state, &ids, added)?,
                orphans: self.orphan_opinions,
            })
        } else {
            None
        };
        let tuples = if ready(TableId::RelationTuples) {
            let _cpu = replay_frame::cpu::scope("project", Some("diplomacy_state.tuples"));
            let table = TableId::RelationTuples;
            let current = self.selection(&ids, table, hour.hour, full);
            let added = if baseline {
                self.baseline_selection(&ids, table, hour.hour, full, &current)
            } else {
                Vec::new()
            };
            let read = |(country, read): (u32, Option<i32>)| read.map(|read| (country, read));
            let countries: Vec<_> = current.into_iter().filter_map(read).collect();
            let baseline_countries: Vec<_> = added.into_iter().filter_map(read).collect();
            let owners = self.tuple_owners(state, &ids, &countries, hour.hour, full, None)?;
            let baseline_owners = if baseline {
                self.tuple_owners(
                    state,
                    &ids,
                    &baseline_countries,
                    hour.hour,
                    full,
                    Some(owners.as_slice()),
                )?
            } else {
                Vec::new()
            };
            Some(Tuples {
                countries,
                baseline_countries,
                owners,
                baseline_owners,
                orphans: std::mem::take(&mut self.orphans_pending)
                    .then(|| self.orphan_tuples.clone()),
            })
        } else {
            None
        };
        let queue = {
            let _cpu = replay_frame::cpu::scope("project", Some("diplomacy_state.queue"));
            self.census(hour, &ids, naming)?;
            match &self.queue {
                None => QueueRead::Unavailable,
                Some(queue)
                    if slot_read(
                        self.format,
                        &metrics,
                        TableId::IncomingDiplomaticActions,
                        hour.hour,
                    ) =>
                {
                    QueueRead::Fresh(Arc::clone(queue))
                }
                Some(_) => QueueRead::Carried,
            }
        };
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            ids: naming.then_some(ids),
            refused: false,
            baseline,
            opinions,
            tuples,
            queue,
        })
    }
}

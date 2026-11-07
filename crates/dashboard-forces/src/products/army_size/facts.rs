//! Army facts, projected hour by hour from a columnar walk ([`replay::segments`]): each country
//! whose totals or composition the hour may have changed, with its holder, from the walk's
//! countries ([`dashboard_world`]), the division census, the division inventory and the aid
//! lists. Only countries the hour's rows touch leave the projection, unless the countries,
//! the reads or the recording's coverage changed; a segment's first hour projects every one.

use super::{
    Mix, Totals, aid,
    inventory::{Inventory, rows},
};
use dashboard_text::metadata::{link_slot, slot_read};
use dashboard_world::{Divisions, Needs, Tagged, World};
use replay::{
    columnar_frames::metrics,
    columns,
    campaign::Replay,
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{CountryLinkRow, DivisionCensusRow, TableId},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::DivisionCensus,
    TableId::Divisions,
    TableId::DivisionTemplateIcons,
    TableId::DivisionLandInputs,
    TableId::LexicalTokens,
    TableId::SentVolunteers,
    TableId::VolunteerHosts,
    TableId::CountryCounterInputs,
];

/// The tables whose rows can change a country's totals or composition.
const ARMY: [TableId; 5] = [
    TableId::DivisionCensus,
    TableId::Divisions,
    TableId::DivisionTemplateIcons,
    TableId::DivisionLandInputs,
    TableId::LexicalTokens,
];

/// What a projection reads of its walk's world.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        divisions: true,
        ..Needs::default()
    }
}

/// One country's army at a projected hour.
#[derive(Clone, Debug, PartialEq)]
#[derive(replay_spool::Spooled)]
pub struct Country {
    /// Its tag's index in the projection ([`Hour::named`]).
    pub index: u32,
    /// Its canonical slot and id.
    pub holder: (u32, u32),
    /// Owner/host divisions, HP, battalions, then manpower ([`super::channel`]); None when
    /// neither the census nor the inventory was read.
    pub totals: Option<Totals>,
    /// Its composition, when the templates were read; the same allocation while it is unchanged.
    pub mix: Option<Arc<Mix>>,
}

/// One projected hour.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Tags the projection numbers from this hour on, by index.
    pub named: Vec<(u32, Arc<str>)>,
    /// The tags whose countries are live, when they may have changed: none in a gap or while
    /// the countries are unread.
    pub live: Option<Vec<u32>>,
    /// Countries whose army may have changed, at least every one that did.
    pub countries: Vec<Country>,
    /// Whether the hour read divisions, strength, manpower and battalions, and whether it read
    /// divisions without the census.
    pub divisions: bool,
    pub strength: bool,
    pub manpower: bool,
    pub battalions: bool,
    pub fallback: bool,
    /// Division and country attribution reads the hour could not resolve.
    pub unreadable: usize,
    /// Country attribution-hours without a known composition.
    pub composition_unreadable: usize,
    /// Table reads the hour refused, by reason.
    pub invalid: Vec<(String, usize)>,
    pub aid: aid::Projection,
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    gaps: Vec<(u64, i32, i32)>,
    inventory: Inventory,
    aid: aid::Runtime,
    /// The hour's tagged countries by tag: tag, (slot, id).
    identities: Vec<(Arc<str>, (u32, u32))>,
    /// Each identity's tag index.
    numbers: HashMap<Arc<str>, u32>,
    /// Each slot's identities, by position.
    slots: HashMap<u32, Vec<usize>>,
    /// Found tags that are not UTF-8.
    unreadable_tags: usize,
    valid: Option<bool>,
    census_counts: HashMap<u32, Option<i64>>,
    census_invalid: bool,
    census_unreadable: usize,
    census_dirty: BTreeSet<u32>,
    census_ready: Option<bool>,
    /// Each census row's slot and division count, by country.
    census_rows: HashMap<u32, (Option<u32>, i64)>,
    /// Each slot's census rows and their summed division count.
    census_sums: HashMap<u32, (usize, i128)>,
    /// The hour whose census `census_rows` hold, while they hold one.
    census_hour: Option<i32>,
    reads: Option<[bool; 4]>,
    gap: Option<bool>,
    /// The last changed hour's counts, which an unchanged hour repeats.
    rates: (usize, usize, BTreeMap<String, usize>),
    composition_rates: HashMap<Arc<str>, usize>,
    composition_rate: usize,
    /// Each slot's composition last projected, by version.
    mixes: HashMap<u32, ((u64, u64, [bool; 2]), Arc<Mix>)>,
    live: bool,
    last: Option<i32>,
}

/// Visits the keys of the rows the hour changed, inserted or filled in `table`, and of those
/// it deleted: whether it touched any.
fn touched(state: &State, table: TableId) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(!changes.changed.is_empty()
        || !changes.fill_slots.is_empty()
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty()
        || changes.truncated)
}

impl Projector {
    pub fn new(replay: &Replay) -> Self {
        Self::with(
            replay.format,
            replay
                .gaps
                .iter()
                .map(|gap| (gap.branch, gap.first_hour, gap.last_hour))
                .collect(),
        )
    }

    /// A projection of a recording in `format` with these gaps: branch, first and last hour.
    pub fn with(format: FormatVersion, gaps: Vec<(u64, i32, i32)>) -> Self {
        Self {
            format,
            gaps,
            inventory: Inventory::default(),
            aid: aid::Runtime::default(),
            identities: Vec::new(),
            numbers: HashMap::new(),
            slots: HashMap::new(),
            unreadable_tags: 0,
            valid: None,
            census_counts: HashMap::new(),
            census_invalid: false,
            census_unreadable: 0,
            census_dirty: BTreeSet::new(),
            census_ready: None,
            census_rows: HashMap::new(),
            census_sums: HashMap::new(),
            census_hour: None,
            reads: None,
            gap: None,
            rates: Default::default(),
            composition_rates: HashMap::new(),
            composition_rate: 0,
            mixes: HashMap::new(),
            live: false,
            last: None,
        }
    }

    /// Reads the hour's tagged countries: each tag's first holder by id the links resolve,
    /// ordered by tag. Whether they changed.
    fn identities(
        &mut self,
        world: &World,
        named: &mut Vec<(u32, Arc<str>)>,
        valid: bool,
        whole: bool,
    ) -> io::Result<bool> {
        let roster = world.countries()?;
        if !valid {
            let changed = !self.identities.is_empty() || self.valid != Some(false);
            self.identities.clear();
            self.slots.clear();
            self.unreadable_tags = 0;
            return Ok(changed);
        }
        if !whole && self.valid == Some(true) && !roster.changed() {
            return Ok(false);
        }
        let mut found = Vec::new();
        let mut unreadable = 0;
        for tag in roster.held() {
            if let Tagged::Found(country) = roster.tagged(tag) {
                match std::str::from_utf8(roster.tag(tag)) {
                    Ok(text) => found.push((text, (country.slot, country.id))),
                    Err(_) => unreadable += 1,
                }
            }
        }
        found.sort_unstable_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
        self.unreadable_tags = unreadable;
        let same = found.len() == self.identities.len()
            && found
                .iter()
                .zip(&self.identities)
                .all(|((tag, identity), (held, at))| *tag == &**held && identity == at);
        if same {
            return Ok(false);
        }
        let mut identities = Vec::with_capacity(found.len());
        for (text, identity) in found {
            let tag = match self.numbers.get_key_value(text) {
                Some((tag, _)) => Arc::clone(tag),
                None => {
                    let tag: Arc<str> = Arc::from(text);
                    let index = self.numbers.len() as u32;
                    self.numbers.insert(Arc::clone(&tag), index);
                    named.push((index, Arc::clone(&tag)));
                    tag
                }
            };
            identities.push((tag, identity));
        }
        self.identities = identities;
        self.slots.clear();
        for (position, (_, identity)) in self.identities.iter().enumerate() {
            self.slots.entry(identity.0).or_default().push(position);
        }
        Ok(true)
    }

    /// The census this hour: read whole, or by the rows it changed through the same links.
    fn census(
        &mut self,
        state: &State,
        hour: i32,
        whole: bool,
        links: &[CountryLinkRow],
        changed: bool,
        read: bool,
    ) -> io::Result<()> {
        if self.census_ready == Some(read) && !changed {
            if self
                .census_hour
                .is_some_and(|at| at.checked_add(1) == Some(hour))
            {
                self.census_hour = Some(hour);
            }
            return Ok(());
        }
        let format = self.format.version();
        let changes = columns::changes(state, TableId::DivisionCensus)?;
        let whole = whole
            || !read
            || self.census_ready != Some(true)
            || self
                .census_hour
                .is_none_or(|at| at.checked_add(1) != Some(hour))
            || changes.deleted_keys.iter().any(|&key| {
                u32::try_from(key)
                    .ok()
                    .is_none_or(|key| !self.census_rows.contains_key(&key))
            });
        if whole {
            let rows = if read {
                Some(rows::<DivisionCensusRow>(
                    state,
                    TableId::DivisionCensus,
                    format,
                )?)
            } else {
                None
            };
            self.read_census(rows.as_deref(), links, hour);
        } else {
            let slots: BTreeSet<_> = changes
                .changed
                .iter()
                .chain(&changes.fill_slots)
                .chain(&changes.inserted)
                .copied()
                .collect();
            let rows = slots
                .into_iter()
                .map(|slot| columns::row(state, TableId::DivisionCensus, slot, format))
                .collect::<io::Result<Vec<DivisionCensusRow>>>()?;
            let deleted = changes.deleted_keys.iter().map(|&key| key as u32).collect();
            self.apply_census_edits(&rows, links, deleted, hour);
        }
        self.census_ready = Some(read);
        Ok(())
    }

    /// Reads the census whole: each slot's division count, the slots whose count changed,
    /// and the rows no link resolves.
    fn read_census(
        &mut self,
        census: Option<&[DivisionCensusRow]>,
        links: &[CountryLinkRow],
        hour: i32,
    ) {
        let mut counts = HashMap::<u32, Option<i64>>::new();
        let mut invalid = false;
        let mut unreadable = 0;
        self.census_rows.clear();
        self.census_sums.clear();
        for row in census.into_iter().flatten() {
            let slot = link_slot(links, row.country);
            let value = i64::from(row.grouped) + i64::from(row.ungrouped);
            self.census_rows.insert(row.country, (slot, value));
            if let Some(slot) = slot {
                let count = counts.entry(slot).or_insert(Some(0));
                *count = count.and_then(|count| count.checked_add(value));
                let sum = self.census_sums.entry(slot).or_default();
                sum.0 += 1;
                sum.1 += i128::from(value);
            } else if row.grouped != 0 || row.ungrouped != 0 {
                invalid = true;
                unreadable += 1;
            }
        }
        self.census_dirty.extend(
            self.census_counts
                .keys()
                .chain(counts.keys())
                .filter(|slot| self.census_counts.get(slot) != counts.get(slot))
                .copied(),
        );
        self.census_counts = counts;
        self.census_invalid = invalid;
        self.census_unreadable = unreadable;
        // The held rows answer a later hour's changed rows only when each country had one row.
        self.census_hour = census
            .filter(|census| census.len() == self.census_rows.len())
            .map(|_| hour);
    }

    fn apply_census_edits(
        &mut self,
        read: &[DivisionCensusRow],
        links: &[CountryLinkRow],
        deleted: Vec<u32>,
        hour: i32,
    ) {
        let mut slots = Vec::new();
        let mut unlinked = self.census_unreadable;
        for country in deleted {
            let held = self.census_rows.remove(&country).expect("held census row");
            census_edit(
                &mut self.census_sums,
                &mut unlinked,
                &mut slots,
                held,
                false,
            );
        }
        for row in read {
            let value = i64::from(row.grouped) + i64::from(row.ungrouped);
            let slot = link_slot(links, row.country);
            if let Some(held) = self.census_rows.insert(row.country, (slot, value)) {
                census_edit(
                    &mut self.census_sums,
                    &mut unlinked,
                    &mut slots,
                    held,
                    false,
                );
            }
            census_edit(
                &mut self.census_sums,
                &mut unlinked,
                &mut slots,
                (slot, value),
                true,
            );
        }
        for slot in slots {
            // Counts are not negative, so a running sum overflows exactly when the slot's
            // whole sum does.
            let count = self
                .census_sums
                .get(&slot)
                .map(|&(_, sum)| i64::try_from(sum).ok());
            if self.census_counts.get(&slot) != count.as_ref() {
                self.census_dirty.insert(slot);
                match count {
                    Some(count) => self.census_counts.insert(slot, count),
                    None => self.census_counts.remove(&slot),
                };
            }
        }
        self.census_invalid = unlinked > 0;
        self.census_unreadable = unlinked;
        self.census_hour = Some(hour);
    }

    /// The composition of `slot` now, the held allocation while its version or its parts
    /// stay the same.
    fn mix(&mut self, slot: u32) -> Arc<Mix> {
        let version = self.inventory.parts_version(slot);
        if let Some((held, mix)) = self.mixes.get(&slot)
            && *held == version
        {
            return Arc::clone(mix);
        }
        let parts = self.inventory.parts_at(slot);
        let mix = match self.mixes.get(&slot) {
            Some((_, mix)) if **mix == parts => Arc::clone(mix),
            _ => Arc::new(parts),
        };
        self.mixes.insert(slot, (version, Arc::clone(&mix)));
        mix
    }
}

/// Count one held census row in (`adding`) or out: in its slot's sum, or among the unresolved
/// rows when no link names it and it counts divisions.
fn census_edit(
    sums: &mut HashMap<u32, (usize, i128)>,
    unlinked: &mut usize,
    slots: &mut Vec<u32>,
    (slot, value): (Option<u32>, i64),
    adding: bool,
) {
    match slot {
        Some(slot) => {
            let sum = sums.entry(slot).or_default();
            if adding {
                sum.0 += 1;
                sum.1 += i128::from(value);
            } else {
                sum.0 -= 1;
                sum.1 -= i128::from(value);
            }
            if sum.0 == 0 {
                sums.remove(&slot);
            }
            slots.push(slot);
        }
        None if value != 0 => {
            if adding {
                *unlinked += 1;
            } else {
                *unlinked -= 1;
            }
        }
        None => {}
    }
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let state = hour.state;
        let at = hour.hour;
        let format = self.format;
        let world = hour.world;
        let roster = world.countries()?;
        let manifest = metrics(hour.manifest);
        let read = |table: TableId| slot_read(format, &manifest, table, at);
        let continuous = self
            .last
            .is_some_and(|last| last.checked_add(1) == Some(at));
        let whole = hour.starts || !continuous;
        let gap = self
            .gaps
            .iter()
            .any(|&(branch, first, last)| branch == hour.branch && first <= at && at <= last);
        let valid = roster.complete();
        let links = roster.links_complete().then(|| roster.links());
        let changed_links = roster.rebuilt() || !roster.changed_links().is_empty();
        let mut named = Vec::new();
        let identity_changed = self.identities(world, &mut named, valid, whole)?;
        self.valid = Some(valid);
        let mut changed = [false; 5];
        for (index, &table) in ARMY.iter().enumerate() {
            changed[index] = whole || world.restated(table) || touched(state, table)?;
        }
        let reads = [
            TableId::DivisionCensus,
            TableId::Divisions,
            TableId::DivisionTemplateIcons,
            TableId::DivisionLandInputs,
        ]
        .map(read);
        let unchanged = continuous
            && valid
            && !identity_changed
            && !changed_links
            && self.reads == Some(reads)
            && self.gap == Some(gap)
            && !changed.iter().any(|&changed| changed);
        let mut facts = Hour {
            branch: hour.branch,
            hour: at,
            named,
            live: None,
            countries: Vec::new(),
            divisions: false,
            strength: false,
            manpower: false,
            battalions: false,
            fallback: false,
            unreadable: 0,
            composition_unreadable: 0,
            invalid: Vec::new(),
            aid: aid::Projection::default(),
        };
        if unchanged {
            facts.unreadable = self.rates.0;
            facts.composition_unreadable = self.rates.1;
            facts.invalid = self
                .rates
                .2
                .iter()
                .map(|(reason, &count)| (reason.clone(), count))
                .collect();
            // The census held is still this hour's.
            if self
                .census_hour
                .is_some_and(|held| held.checked_add(1) == Some(at))
            {
                self.census_hour = Some(at);
            }
        } else {
            let inventory_state = |inventory: &Inventory| {
                (
                    inventory.known(),
                    inventory.composition_known(),
                    inventory.unreadable() != 0,
                )
            };
            let before = inventory_state(&self.inventory);
            let walked = world.divisions()?;
            self.inventory.apply(
                state,
                (format.version(), at),
                hour.starts,
                (links, changed_links),
                walked,
            );
            self.inventory.compose(
                state,
                format.version(),
                hour.starts || world.restated(TableId::LexicalTokens),
            )?;
            if identity_changed {
                let present: BTreeSet<&str> =
                    self.identities.iter().map(|(tag, _)| &**tag).collect();
                self.composition_rates
                    .retain(|tag, _| present.contains(&**tag));
                self.composition_rate = self.composition_rates.values().sum();
            }
            let restricted = continuous
                && !identity_changed
                && self.reads == Some(reads)
                && self.gap == Some(gap)
                && before == inventory_state(&self.inventory);
            self.observe(
                &mut facts,
                (state, walked),
                (at, hour.starts, gap, valid),
                (links.unwrap_or_default(), changed_links, changed[0]),
                reads,
                restricted,
            )?;
            let errors: BTreeMap<String, usize> = self
                .inventory
                .errors(walked)
                .map(|(table, reason)| (format!("{table:?}: {reason}"), 1))
                .collect();
            self.rates = (facts.unreadable, facts.composition_unreadable, errors);
            if gap || !valid || identity_changed || !continuous || !self.live {
                facts.live = Some(if gap || !valid {
                    Vec::new()
                } else {
                    self.identities
                        .iter()
                        .map(|(tag, _)| self.numbers[tag])
                        .collect()
                });
                self.live = !gap && valid && !self.identities.is_empty();
            }
        }
        facts.aid = self.aid.step(
            aid::Inputs {
                state,
                format: format.version(),
                hour: at,
                whole: hour.starts,
                links,
                links_changed: changed_links,
                valid: !gap && valid,
                identities: &self.identities,
                identity_changed,
                divisions_changed: changed[1],
                divisions: world.divisions()?,
            },
            &self.inventory,
        );
        if identity_changed {
            self.mixes.retain(|slot, _| self.slots.contains_key(slot));
        }
        self.reads = Some(reads);
        self.gap = Some(gap);
        self.last = Some(at);
        out.push(facts)
    }
}

impl Projector {
    /// The countries the hour's rows can have changed, with their totals and composition.
    fn observe(
        &mut self,
        facts: &mut Hour,
        (state, walked): (&State, &Divisions),
        (hour, starts, gap, valid): (i32, bool, bool, bool),
        (links, changed_links, changed_census): (&[CountryLinkRow], bool, bool),
        [census_read, division_slot, template_slot, land_slot]: [bool; 4],
        restricted: bool,
    ) -> io::Result<()> {
        let mut invalid = BTreeMap::<String, usize>::new();
        for (table, reason) in self.inventory.errors(walked) {
            *invalid.entry(format!("{table:?}: {reason}")).or_default() += 1;
        }
        facts.invalid = invalid.into_iter().collect();
        let division_read = columns::complete(state, TableId::Divisions) && division_slot;
        let template_read =
            columns::complete(state, TableId::DivisionTemplateIcons) && template_slot;
        if division_read {
            facts.strength = true;
            facts.divisions = true;
            facts.manpower |= land_slot && columns::complete(state, TableId::DivisionLandInputs);
            facts.battalions |= template_read;
        }
        if gap || !valid {
            // The census is not kept through this hour, so the next one reads it whole.
            self.census_ready = None;
            return Ok(());
        }
        facts.unreadable += self.unreadable_tags;
        let census = columns::complete(state, TableId::DivisionCensus) && census_read;
        facts.fallback |= division_read && !census;
        self.census_dirty.clear();
        self.census(
            state,
            hour,
            starts || changed_links,
            links,
            changed_links || changed_census,
            census,
        )?;
        facts.divisions |= census;
        facts.unreadable += self.census_unreadable;
        let inventory = division_read && self.inventory.known();
        let composition = inventory && template_read && self.inventory.composition_known();
        if inventory {
            facts.unreadable += self.inventory.unreadable();
        }
        let indices: Vec<usize> = if restricted {
            self.inventory
                .dirty
                .iter()
                .chain(&self.census_dirty)
                .filter_map(|slot| self.slots.get(slot))
                .flatten()
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        } else {
            (0..self.identities.len()).collect()
        };
        facts.composition_unreadable += self.composition_rate;
        facts.countries.reserve(indices.len());
        for position in indices {
            let (tag, identity) = self.identities[position].clone();
            let slot = identity.0;
            let mut total = if inventory {
                self.inventory.totals(slot, composition)
            } else {
                [None; 8]
            };
            if !land_slot {
                total[6] = None;
                total[7] = None;
            }
            if census {
                total[0] = (!self.census_invalid)
                    .then(|| {
                        self.census_counts
                            .get(&slot)
                            .copied()
                            .flatten()
                            .map(i128::from)
                    })
                    .flatten();
            }
            let notes = if composition {
                self.inventory
                    .parts_version(slot)
                    .2
                    .iter()
                    .filter(|known| !**known)
                    .count()
            } else {
                0
            };
            let before = self
                .composition_rates
                .insert(Arc::clone(&tag), notes)
                .unwrap_or(0);
            self.composition_rate = self.composition_rate + notes - before;
            facts.composition_unreadable = facts.composition_unreadable + notes - before;
            let totals = (census || inventory).then_some(total);
            let mix = (composition && totals.is_some()).then(|| self.mix(slot));
            facts.countries.push(Country {
                index: self.numbers[&tag],
                holder: identity,
                totals,
                mix,
            });
        }
        Ok(())
    }
}

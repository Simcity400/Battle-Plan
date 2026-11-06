use super::{Inputs, Observed, Roster, Selection, selection};
use crate::{
    prepared::prepass::{Held, HeldBytes},
    products::{
        equipment::{
            RawVariant, VariantHour,
            columnar::{Naming, slots},
        },
        production_lines::factory_output,
    },
};
use dashboard_world::{Hold, Needs, Roster as Countries, Tagged, World};
use replay::{
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        CountryProductionModifiersRow, NavalBaseRow, NavalRepairBaseRow, NavalRepairCountryRow,
        NavalRepairQueueRow, ProvinceRow, RowCodec, ShipModelRow, ShipRow, TABLE_COUNT, TableId,
        TaskForceRow,
    },
};
use replay_derive::{
    naval_repairs::{self, Operands, QueuedForce, QueuedModel, QueuedShip, Unavailable},
    visibility::{AccessLinks, Relations},
};
use std::{
    collections::{BTreeSet, HashMap},
    io,
};

pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantUpgrades,
    TableId::VariantChanges,
    TableId::NavalRepairCountries,
    TableId::CountryProductionModifiers,
    TableId::NavalBases,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
    TableId::Ships,
    TableId::ShipModels,
    TableId::TaskForces,
    TableId::Provinces,
    TableId::FactionMembers,
    TableId::WarRelations,
    TableId::Subjects,
    TableId::RelationLinks,
];
const RELATIONS: &[(TableId, Hold)] = &[
    (TableId::FactionMembers, Hold::Whole),
    (TableId::WarRelations, Hold::Whole),
    (TableId::Subjects, Hold::Whole),
    (TableId::RelationLinks, Hold::Whole),
];
const SELECTION: &[TableId] = &[
    TableId::NavalBases,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
    TableId::Ships,
    TableId::ShipModels,
    TableId::TaskForces,
    TableId::Provinces,
];
pub fn needs() -> Needs {
    Needs {
        countries: true,
        naming: true,
        frame: RELATIONS.to_vec(),
        ..Needs::default()
    }
}

#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub hour: i32,
    pub whole: bool,
    pub roster: Option<Box<Roster>>,
    pub modifiers: (bool, bool),
    pub inputs: Vec<(u32, Inputs)>,
    pub variants: VariantHour,
    pub built: Vec<(u64, RawVariant)>,
    pub variants_whole: bool,
}
impl<'a> From<&'a mut Hour> for Observed<'a> {
    fn from(hour: &'a mut Hour) -> Self {
        Self {
            hour: hour.hour,
            whole: hour.whole,
            roster: hour.roster.take().map(|roster| *roster),
            modifiers: hour.modifiers,
            inputs: &hour.inputs,
            variants: hour.variants,
            built: std::mem::take(&mut hour.built),
            variants_whole: hour.variants_whole,
        }
    }
}
impl Held for Selection {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.designs);
    }
}
impl Held for Inputs {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.selection);
    }
}
impl Held for Roster {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.tags).heap(&self.owners);
    }
}

struct Base {
    row: NavalBaseRow,
    holder: Option<u32>,
    result: Option<Selection>,
}
impl Held for Base {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.add(self.row.heap_bytes()).heap(&self.result);
    }
}
struct Fields {
    ship_force: usize,
    strength: usize,
    owner: usize,
    force_province: usize,
    controller: usize,
    masks: [u64; TABLE_COUNT],
}
impl Fields {
    fn new() -> io::Result<Self> {
        let mut masks = [u64::MAX; TABLE_COUNT];
        for (table, names) in [
            (TableId::Ships, &["task_force", "strength"][..]),
            (
                TableId::ShipModels,
                &["variant", "counter", "has_criticals"][..],
            ),
            (TableId::TaskForces, &["roster_owner", "province"][..]),
            (TableId::Provinces, &["controller"][..]),
            (TableId::NavalRepairBases, &["allocated_dockyards"][..]),
            (TableId::NavalRepairCountries, &["num_used_dockyards"][..]),
            (
                TableId::CountryProductionModifiers,
                &["power_raw", "factory_speed", "trees"][..],
            ),
        ] {
            let mut mask = 0;
            for name in names {
                mask |= 1 << columns::field(table, name)?;
            }
            masks[table.index()] = mask;
        }
        Ok(Self {
            ship_force: columns::field(TableId::Ships, "task_force")?,
            strength: columns::field(TableId::Ships, "strength")?,
            owner: columns::field(TableId::TaskForces, "roster_owner")?,
            force_province: columns::field(TableId::TaskForces, "province")?,
            controller: columns::field(TableId::Provinces, "controller")?,
            masks,
        })
    }
}
struct Columnar<'a> {
    state: &'a State,
    format: FormatVersion,
    fields: &'a Fields,
    roster: &'a Countries,
    queues: &'a HashMap<u32, Vec<(u32, u64)>>,
    relations: Result<&'a Relations, &'static str>,
    access: Result<&'a AccessLinks, &'static str>,
}
impl Columnar<'_> {
    fn row<R: RowCodec>(&self, table: TableId, key: u128) -> Result<Option<R>, Unavailable> {
        if !columns::complete(self.state, table) {
            return Err(Unavailable);
        }
        let held = columns::table(self.state, table).map_err(|_| Unavailable)?;
        held.slot(key)
            .map(|slot| {
                columns::row(self.state, table, slot, self.format.version())
                    .map_err(|_| Unavailable)
            })
            .transpose()
    }
}
fn cell<T: Copy>(values: &replay::columns::Paged<T>, slot: u32) -> Result<T, Unavailable> {
    values.get(slot as usize).copied().ok_or(Unavailable)
}
impl Operands for Columnar<'_> {
    type Queue<'q>
        = std::iter::Copied<std::slice::Iter<'q, (u32, u64)>>
    where
        Self: 'q;
    fn allocation(&self, province: u32) -> Result<Option<i32>, Unavailable> {
        Ok(self
            .row::<NavalRepairBaseRow>(TableId::NavalRepairBases, province.into())?
            .map(|row| row.allocated_dockyards))
    }
    fn queue(&self, province: u32) -> Result<(Self::Queue<'_>, usize), Unavailable> {
        if !columns::complete(self.state, TableId::NavalRepairQueue) {
            return Err(Unavailable);
        }
        let queue = self.queues.get(&province).map_or(&[][..], Vec::as_slice);
        Ok((queue.iter().copied(), queue.len()))
    }
    fn ship(&self, id: u64) -> Result<Option<QueuedShip>, Unavailable> {
        let table = TableId::Ships;
        if !columns::complete(self.state, table) {
            return Err(Unavailable);
        }
        let held = columns::table(self.state, table).map_err(|_| Unavailable)?;
        let Some(slot) = held.slot(id.into()) else {
            return Ok(None);
        };
        let (force, strength) = (self.fields.ship_force, self.fields.strength);
        if held.recorded(force, slot) && held.recorded(strength, slot) {
            Ok(Some(QueuedShip {
                task_force: cell(
                    columns::plain::<u64>(held, force).map_err(|_| Unavailable)?,
                    slot,
                )?,
                strength: cell(
                    columns::plain::<i64>(held, strength).map_err(|_| Unavailable)?,
                    slot,
                )?,
            }))
        } else {
            Ok(self
                .row::<ShipRow>(table, id.into())?
                .map(|row| QueuedShip {
                    task_force: row.task_force,
                    strength: row.strength,
                }))
        }
    }
    fn force(&self, id: u64) -> Result<Option<QueuedForce>, Unavailable> {
        let table = TableId::TaskForces;
        if !columns::complete(self.state, table) {
            return Err(Unavailable);
        }
        let held = columns::table(self.state, table).map_err(|_| Unavailable)?;
        let Some(slot) = held.slot(id.into()) else {
            return Ok(None);
        };
        let (owner, province) = (self.fields.owner, self.fields.force_province);
        if held.recorded(owner, slot) && held.recorded(province, slot) {
            Ok(Some(QueuedForce {
                roster_owner: cell(
                    columns::plain::<u32>(held, owner).map_err(|_| Unavailable)?,
                    slot,
                )?,
                province: cell(
                    columns::plain::<u32>(held, province).map_err(|_| Unavailable)?,
                    slot,
                )?,
            }))
        } else {
            Ok(self
                .row::<TaskForceRow>(table, id.into())?
                .map(|row| QueuedForce {
                    roster_owner: row.roster_owner,
                    province: row.province,
                }))
        }
    }
    fn model(&self, ship: u64) -> Result<Option<QueuedModel>, Unavailable> {
        Ok(self
            .row::<ShipModelRow>(TableId::ShipModels, ship.into())?
            .as_ref()
            .map(naval_repairs::model_operands))
    }
    fn controller(&self, province: u32) -> Result<Option<u32>, Unavailable> {
        let table = TableId::Provinces;
        if !columns::complete(self.state, table) {
            return Err(Unavailable);
        }
        let held = columns::table(self.state, table).map_err(|_| Unavailable)?;
        let Some(slot) = held.slot(province.into()) else {
            return Ok(None);
        };
        let field = self.fields.controller;
        if held.recorded(field, slot) {
            Ok(Some(cell(
                columns::plain::<u32>(held, field).map_err(|_| Unavailable)?,
                slot,
            )?))
        } else {
            Ok(self
                .row::<ProvinceRow>(table, province.into())?
                .map(|row| row.controller))
        }
    }
    fn slot_of(&self, raw: u32) -> Option<u32> {
        self.roster.slot_of(raw)
    }
    fn port(&self, owner: u32, controller: u32) -> Result<bool, &'static str> {
        Ok(self.relations?.known_port(self.access?, owner, controller))
    }
}

pub struct Projector {
    format: FormatVersion,
    fields: Fields,
    last: Option<i32>,
    naming: Naming,
    shapes: [Option<columns::Status>; TABLE_COUNT],
    bases: HashMap<u32, Base>,
    holders: HashMap<u32, BTreeSet<u32>>,
    queues: HashMap<u32, Vec<(u32, u64)>>,
    queued: HashMap<u64, Vec<u32>>,
    sent: HashMap<u32, Inputs>,
    relations: Result<Relations, &'static str>,
    access: Result<AccessLinks, &'static str>,
    relations_heap: usize,
    #[cfg(test)]
    pub base_visits: usize,
}
impl Projector {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        Ok(Self {
            format,
            fields: Fields::new()?,
            last: None,
            naming: Naming::new()?,
            shapes: [None; TABLE_COUNT],
            bases: HashMap::new(),
            holders: HashMap::new(),
            queues: HashMap::new(),
            queued: HashMap::new(),
            sent: HashMap::new(),
            relations: Err("Relations not read"),
            access: Err("AccessLinks not read"),
            relations_heap: 0,
            #[cfg(test)]
            base_visits: 0,
        })
    }
    fn keys(&self, state: &State, table: TableId) -> io::Result<BTreeSet<u128>> {
        let mut keys = BTreeSet::new();
        if columns::complete(state, table) {
            let held = columns::table(state, table)?;
            keys.extend(columns::changes(state, table)?.deleted_keys.iter().copied());
            for slot in slots::edits(state, table, self.fields.masks[table.index()])? {
                if let Some(key) = held.key(slot) {
                    keys.insert(key);
                }
            }
        }
        Ok(keys)
    }
    fn queue_remove(&mut self, province: u32) {
        if let Some(queue) = self.queues.remove(&province) {
            for (_, ship) in queue {
                if let Some(bases) = self.queued.get_mut(&ship) {
                    bases.retain(|&base| base != province);
                    if bases.is_empty() {
                        self.queued.remove(&ship);
                    }
                }
            }
        }
    }
    fn queues(&mut self, state: &State, whole: bool) -> io::Result<BTreeSet<u32>> {
        let table = TableId::NavalRepairQueue;
        let mut dirty = BTreeSet::new();
        if whole {
            dirty.extend(self.queues.keys().copied());
            self.queues.clear();
            self.queued.clear();
            if columns::complete(state, table) {
                for slot in columns::table(state, table)?.live().ones() {
                    let row: NavalRepairQueueRow =
                        columns::row(state, table, slot, self.format.version())?;
                    self.queues
                        .entry(row.province)
                        .or_default()
                        .push((row.ordinal, row.ship));
                }
            }
            dirty.extend(self.queues.keys().copied());
        } else if columns::complete(state, table) {
            let keys = self.keys(state, table)?;
            dirty.extend(keys.iter().map(|key| (key >> 32) as u32));
            let mut queues: HashMap<u32, Vec<(u32, u64)>> = dirty
                .iter()
                .map(|&province| {
                    (
                        province,
                        self.queues.get(&province).cloned().unwrap_or_default(),
                    )
                })
                .collect();
            let held = columns::table(state, table)?;
            for key in keys {
                let province = (key >> 32) as u32;
                let ordinal = key as u32;
                let queue = queues.get_mut(&province).expect("changed queue");
                queue.retain(|&(at, _)| at != ordinal);
                if let Some(slot) = held.slot(key) {
                    let row: NavalRepairQueueRow =
                        columns::row(state, table, slot, self.format.version())?;
                    queue.push((row.ordinal, row.ship));
                }
            }
            for (province, queue) in queues {
                self.queue_remove(province);
                if !queue.is_empty() {
                    self.queues.insert(province, queue);
                }
            }
        }
        for &province in &dirty {
            if let Some(queue) = self.queues.get_mut(&province) {
                queue.sort_unstable_by_key(|&(ordinal, _)| ordinal);
                for &(_, ship) in queue.iter() {
                    let bases = self.queued.entry(ship).or_default();
                    if !bases.contains(&province) {
                        bases.push(province);
                    }
                }
            }
        }
        Ok(dirty)
    }
    fn set_base(&mut self, province: u32, base: Option<Base>, dirty: &mut BTreeSet<u32>) {
        if let Some(old) = self.bases.remove(&province)
            && let Some(holder) = old.holder
        {
            dirty.insert(holder);
            if let Some(bases) = self.holders.get_mut(&holder) {
                bases.remove(&province);
                if bases.is_empty() {
                    self.holders.remove(&holder);
                }
            }
        }
        if let Some(base) = base {
            if let Some(holder) = base.holder {
                dirty.insert(holder);
                self.holders.entry(holder).or_default().insert(province);
            }
            self.bases.insert(province, base);
        }
    }
    fn unique<R: RowCodec>(
        &self,
        state: &State,
        roster: &Countries,
        slot: u32,
        table: TableId,
    ) -> io::Result<Option<R>> {
        if !columns::complete(state, table) {
            return Ok(None);
        }
        let held = columns::table(state, table)?;
        let mut found = None;
        for raw in roster.aliases(slot) {
            if let Some(at) = held.slot((*raw).into()) {
                if found.is_some() {
                    return Ok(None);
                }
                found = Some(at);
            }
        }
        found
            .map(|at| columns::row(state, table, at, self.format.version()))
            .transpose()
    }
    fn input(&self, state: &State, roster: &Countries, slot: u32) -> io::Result<Inputs> {
        let count = self
            .unique::<NavalRepairCountryRow>(state, roster, slot, TableId::NavalRepairCountries)?
            .map(|row| row.num_used_dockyards);
        let output = self
            .unique::<CountryProductionModifiersRow>(
                state,
                roster,
                slot,
                TableId::CountryProductionModifiers,
            )?
            .and_then(|row| factory_output(&row, true));
        let selection = if roster.countries_complete()
            && roster.links_complete()
            && count.is_some_and(|count| count > 0)
            && output.is_some()
        {
            let readable = columns::complete(state, TableId::NavalBases)
                && columns::complete(state, TableId::NavalRepairBases);
            let mut total = Selection {
                readable,
                ..Selection::default()
            };
            if readable {
                for province in self.holders.get(&slot).into_iter().flatten() {
                    if let Some(result) = self.bases[province].result.as_ref() {
                        total.allocated = total
                            .allocated
                            .checked_add(result.allocated)
                            .ok_or_else(|| crate::invalid("Repair allocation sum overflow"))?;
                        total.occurrences = total
                            .occurrences
                            .checked_add(result.occurrences)
                            .ok_or_else(|| crate::invalid("Repair occurrence count overflow"))?;
                        for (&design, &count) in &result.designs {
                            let value = total.designs.entry(design).or_default();
                            *value = value
                                .checked_add(count)
                                .ok_or_else(|| crate::invalid("Repair design count overflow"))?;
                        }
                    }
                }
            }
            Some(total)
        } else {
            None
        };
        Ok(Inputs {
            count,
            output,
            selection,
        })
    }
    fn heap(&self) -> usize {
        let mut bytes = HeldBytes::default();
        bytes
            .heap(&self.bases)
            .heap(&self.holders)
            .heap(&self.queues)
            .heap(&self.queued)
            .heap(&self.sent);
        bytes.total() as usize + self.naming.heap() + self.relations_heap
    }
}
impl Operator<World> for Projector {
    type Output = Hour;
    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.last = None;
        Ok(())
    }
    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let (state, world, at) = (hour.state, hour.world, hour.hour);
        let whole =
            hour.starts || hour.opens || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        self.last = Some(at);
        if whole {
            self.bases.clear();
            self.holders.clear();
            self.queues.clear();
            self.queued.clear();
            self.sent.clear();
        }
        let roster = world.countries()?;
        let regrouped = whole || roster.changed();
        let links = whole
            || roster.rebuilt()
            || !roster.changed_links().is_empty()
            || world.restated(TableId::CountryLinks);
        let mut shaped = [false; TABLE_COUNT];
        let mut changed = [false; TABLE_COUNT];
        for &table in TABLES {
            let index = table.index();
            let status = columns::status(state, table);
            shaped[index] = whole || self.shapes[index] != Some(status);
            changed[index] = shaped[index] || slots::relevant(state, table, u64::MAX)?;
            self.shapes[index] = Some(status);
        }
        let reread = links
            || RELATIONS
                .iter()
                .any(|&(table, _)| world.touched(table, u64::MAX) || world.restated(table));
        if reread {
            let frame = world.columns(state);
            let links = roster
                .links()
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect();
            self.relations = Relations::read_tables(&frame, &links);
            self.access = AccessLinks::read_tables(&frame, &links);
            self.relations_heap = 0;
            for &(table, _) in RELATIONS {
                if columns::complete(state, table) {
                    // Each relation row can supply two directed tree entries.
                    self.relations_heap += columns::table(state, table)?.live().count() * 256;
                }
            }
        }
        let mut dirty_holders = BTreeSet::new();
        let mut dirty_bases =
            self.queues(state, whole || shaped[TableId::NavalRepairQueue.index()])?;
        let all_bases =
            whole || links || reread || SELECTION.iter().any(|table| shaped[table.index()]);
        let base_table = TableId::NavalBases;
        let base_keys: BTreeSet<u32> = if whole || shaped[base_table.index()] {
            let mut keys: BTreeSet<_> = self.bases.keys().copied().collect();
            if columns::complete(state, base_table) {
                let held = columns::table(state, base_table)?;
                keys.extend(
                    held.live()
                        .ones()
                        .filter_map(|slot| held.key(slot).map(|key| key as u32)),
                );
            }
            keys
        } else {
            self.keys(state, base_table)?
                .into_iter()
                .map(|key| key as u32)
                .collect()
        };
        for province in base_keys {
            let row = if columns::complete(state, base_table) {
                columns::table(state, base_table)?
                    .slot(province.into())
                    .map(|slot| {
                        columns::row::<NavalBaseRow>(state, base_table, slot, self.format.version())
                    })
                    .transpose()?
            } else {
                None
            };
            let base = row.map(|row| Base {
                holder: roster.slot_of(row.country),
                row,
                result: None,
            });
            self.set_base(province, base, &mut dirty_holders);
            dirty_bases.insert(province);
        }
        if all_bases {
            dirty_bases.extend(self.bases.keys().copied());
        } else {
            for table in [TableId::NavalRepairBases, TableId::Provinces] {
                dirty_bases.extend(self.keys(state, table)?.into_iter().map(|key| key as u32));
            }
            for table in [TableId::Ships, TableId::ShipModels] {
                for key in self.keys(state, table)? {
                    dirty_bases.extend(
                        self.queued
                            .get(&(key as u64))
                            .into_iter()
                            .flatten()
                            .copied(),
                    );
                }
            }
            let forces = self.keys(state, TableId::TaskForces)?;
            if !forces.is_empty() {
                let ops = Columnar {
                    state,
                    format: self.format,
                    fields: &self.fields,
                    roster,
                    queues: &self.queues,
                    relations: self.relations.as_ref().map_err(|why| *why),
                    access: self.access.as_ref().map_err(|why| *why),
                };
                for (&ship, bases) in &self.queued {
                    if ops
                        .ship(ship)
                        .ok()
                        .flatten()
                        .is_some_and(|ship| forces.contains(&u128::from(ship.task_force)))
                    {
                        dirty_bases.extend(bases.iter().copied());
                    }
                }
            }
        }
        let ops = Columnar {
            state,
            format: self.format,
            fields: &self.fields,
            roster,
            queues: &self.queues,
            relations: self.relations.as_ref().map_err(|why| *why),
            access: self.access.as_ref().map_err(|why| *why),
        };
        for province in dirty_bases {
            if let Some(base) = self.bases.get_mut(&province) {
                let holder = roster.slot_of(base.row.country);
                let result = selection(&ops, &base.row)?;
                #[cfg(test)]
                if result.allocated > 0 {
                    self.base_visits += 1;
                }
                if base.holder != holder || base.result.as_ref() != Some(&result) {
                    dirty_holders.extend(base.holder);
                    dirty_holders.extend(holder);
                    if base.holder != holder {
                        if let Some(old) = base.holder
                            && let Some(bases) = self.holders.get_mut(&old)
                        {
                            bases.remove(&province);
                            if bases.is_empty() {
                                self.holders.remove(&old);
                            }
                        }
                        if let Some(new) = holder {
                            self.holders.entry(new).or_default().insert(province);
                        }
                    }
                    base.holder = holder;
                    base.result = Some(result);
                }
            }
        }
        for table in [
            TableId::NavalRepairCountries,
            TableId::CountryProductionModifiers,
        ] {
            if !regrouped && !shaped[table.index()] {
                dirty_holders.extend(
                    self.keys(state, table)?
                        .into_iter()
                        .filter_map(|key| roster.slot_of(key as u32)),
                );
            }
        }
        let all_holders = regrouped
            || [
                TableId::NavalRepairCountries,
                TableId::CountryProductionModifiers,
            ]
            .iter()
            .any(|table| shaped[table.index()]);
        if all_holders {
            if !whole {
                dirty_holders.extend(self.sent.keys().copied());
            }
            dirty_holders.extend(roster.links().iter().map(|link| link.canonical_slot));
        }
        if shaped[TableId::NavalBases.index()] || shaped[TableId::NavalRepairBases.index()] {
            dirty_holders.extend(self.sent.keys().copied());
        }
        let mut inputs = Vec::new();
        if whole {
            self.sent.clear();
        }
        for slot in dirty_holders {
            let input = self.input(state, roster, slot)?;
            if whole || self.sent.get(&slot) != Some(&input) {
                self.sent.insert(slot, input.clone());
                inputs.push((slot, input));
            }
        }
        let tags = if regrouped {
            let mut tags: Vec<_> = roster
                .held()
                .map(|tag| {
                    (
                        roster.tag(tag).to_vec(),
                        match roster.tagged(tag) {
                            Tagged::Found(country) => Some((country.slot, country.id)),
                            _ => None,
                        },
                    )
                })
                .collect();
            tags.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            let found: BTreeSet<u32> = tags
                .iter()
                .filter_map(|(_, identity)| identity.map(|(slot, _)| slot))
                .collect();
            let mut owners: Vec<(u32, u32)> = roster
                .links()
                .iter()
                .filter(|link| found.contains(&link.canonical_slot))
                .map(|link| (link.id, link.canonical_slot))
                .collect();
            owners.sort_unstable();
            Some(Box::new(Roster {
                ok: roster.countries_complete() && roster.links_complete(),
                tags,
                owners,
            }))
        } else {
            None
        };
        let read =
            |table| dashboard_text::metadata::slot_read(self.format, world.metrics(), table, at);
        let variants = self.naming.read(
            state,
            (roster, world.naming()?),
            self.format,
            at,
            whole,
            &shaped,
            &changed,
            read(TableId::VariantDefinitions),
        )?;
        out.push(Hour {
            hour: at,
            whole,
            roster: tags,
            modifiers: (
                columns::complete(state, TableId::CountryProductionModifiers),
                read(TableId::CountryProductionModifiers),
            ),
            inputs,
            variants: variants.evidence,
            built: variants.built,
            variants_whole: variants.whole,
        })
    }
    fn carried(&self) -> Option<usize> {
        Some(self.heap())
    }
    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }
    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(crate::invalid("Not a saved repairs projector"));
        }
        *self = Self::new(self.format)?;
        Ok(())
    }
}
const SAVED: &[u8] = b"repairs projector: cold v2";

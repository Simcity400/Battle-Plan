//! Demanded country operands, refreshed by existing accumulator notifications.
//! Everything here runs on acquisition/observer workers. Quiet publication only
//! increments an immutable snapshot's reference count.
use super::*;
use collector_observers::country_input_changes::{self as changes, Changes};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};

// The native designer requests the common factor and its equipment-family factor.
const COMMON_COST_FACTOR: u32 = 0x1d7;
const AIR_COST_FACTOR: u32 = 0x1da;
const TANK_COST_FACTOR: u32 = 0x1d9;
const SHIP_COST_FACTOR: u32 = 0x1d8;
const ORGANISATION_CHANGE_FACTOR: u32 = 0x269;
const CONVERSION_SPEED_FACTOR: u32 = 0xdc;
const REFIT_IC_FACTOR: u32 = 0x1d6;
const AIR_EXPERIENCE_TRAINING_FACTOR: u32 = 0x73;
pub(super) type Snapshot = Option<Arc<rows::TableRows>>;
type Snapshots = (Snapshot, Snapshot);
struct Demand {
    country: u32,
    keys: Vec<u32>,
    generation: u64,
}

/// Only nonidentity country references need an index. Resolve them during the
/// already demanded numeric read, and route native owner notifications back to
/// those raw creator scopes. No country lookup is needed while decoding an
/// owned design event.
#[derive(Default)]
struct Owners {
    aliases: HashMap<u32, u32>,
    reverse: HashMap<u32, HashSet<u32>>,
    nested_bytes: usize,
}
impl Owners {
    fn remove(&mut self, raw: u32) {
        let Some(owner) = self.aliases.remove(&raw) else {
            return;
        };
        if let Some(group) = self.reverse.get_mut(&owner) {
            group.remove(&raw);
            if group.is_empty() {
                let group = self.reverse.remove(&owner).unwrap();
                self.nested_bytes -= group.capacity() * size_of::<u32>();
            }
        }
    }
    fn bind(&mut self, raw: u32, owner: u32) -> Result<(), &'static str> {
        if raw == owner {
            self.remove(raw);
            return Ok(());
        }
        if self.aliases.get(&raw) == Some(&owner) {
            return Ok(());
        }
        reserve(&mut self.aliases)?;
        reserve(&mut self.reverse)?;
        let group = self.reverse.entry(owner).or_default();
        let before = group.capacity();
        engine_allocation::reserve_set(group, 1)
            .map_err(|_| "design context alias allocation failed")?;
        self.nested_bytes += (group.capacity() - before) * size_of::<u32>();
        group.insert(raw);
        self.remove(raw);
        self.aliases.insert(raw, owner);
        Ok(())
    }
    fn mark<V>(
        &self,
        demanded: &HashMap<u32, V>,
        dirty: &mut HashMap<u32, bool>,
        owner: u32,
    ) -> Result<(), &'static str> {
        if demanded.contains_key(&owner) && !self.aliases.contains_key(&owner) {
            reserve(dirty)?;
            dirty.insert(owner, true);
        }
        if let Some(group) = self.reverse.get(&owner) {
            engine_allocation::reserve_map(dirty, group.len())
                .map_err(|_| "design context alias mark allocation failed")?;
            for raw in group {
                if demanded.contains_key(raw) {
                    dirty.insert(*raw, true);
                }
            }
        }
        Ok(())
    }
    fn allocated_bytes(&self) -> usize {
        self.aliases.capacity() * size_of::<(u32, u32)>()
            + self.reverse.capacity() * size_of::<(u32, HashSet<u32>)>()
            + self.nested_bytes
    }
}
#[derive(Default)]
struct Cache {
    designs: HashMap<u64, Demand>,
    organisation_demands: HashMap<u64, u32>,
    template_demands: HashMap<u32, usize>,
    countries: HashMap<u32, HashMap<u32, usize>>,
    types: HashMap<u32, u32>,
    modules: HashMap<u32, u32>,
    upgrade_currencies: HashMap<u32, u32>,
    owners: Owners,
    /// True for a native writer/epoch; false for newly demanded keys only.
    dirty: HashMap<u32, bool>,
    values: HashMap<(u32, u32), rows::DesignCountryModifierRow>,
    snapshot: Snapshot,
    template_values: HashMap<u32, rows::DesignCountryTemplatesRow>,
    template_dirty: HashMap<u32, bool>,
    template_snapshot: Snapshot,
    template_changed: bool,
    template_epoch: Option<u64>,
    template_admitted: bool,
    template_heap_bytes: usize,
    template_published_bytes: usize,
    epoch: Option<u64>,
    generation: u64,
    changed: bool,
    seeded: bool,
    failures: u64,
    failure: Option<&'static str>,
    last_refusal: Option<&'static str>,
    demand_bytes: usize,
    spare_keys: Vec<u32>,
    owner_key_bytes: usize,
    published_bytes: usize,
}
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
static INVALIDATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn worker_reset(cache: &mut Cache) {
    if INVALIDATED.swap(false, std::sync::atomic::Ordering::AcqRel) {
        *cache = Cache::default();
    }
}
fn cache() -> MutexGuard<'static, Cache> {
    CACHE
        .get_or_init(|| Mutex::new(Cache::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn reserve<K: Eq + std::hash::Hash, V>(map: &mut HashMap<K, V>) -> Result<(), &'static str> {
    engine_allocation::reserve_map(map, 1).map_err(|_| "design context allocation failed")
}
impl Cache {
    fn dirty(&mut self, country: u32) -> Result<(), &'static str> {
        reserve(&mut self.dirty)?;
        self.dirty.insert(country, true);
        Ok(())
    }
    fn prune_country(&mut self, country: u32) {
        if self.countries.get(&country).is_some_and(HashMap::is_empty) {
            if let Some(keys) = self.countries.remove(&country) {
                self.owner_key_bytes -= keys.capacity() * size_of::<(u32, usize)>();
            }
            self.dirty.remove(&country);
            self.owners.remove(country);
        }
    }
    fn remove_key(&mut self, country: u32, key: u32) -> Result<(), &'static str> {
        let counts = self
            .countries
            .get_mut(&country)
            .ok_or("design context owner missing")?;
        let count = counts
            .get_mut(&key)
            .ok_or("design context demand missing")?;
        *count -= 1;
        if *count == 0 {
            counts.remove(&key);
            self.values.remove(&(country, key));
            self.changed = true;
        }
        self.prune_country(country);
        Ok(())
    }
    fn replace_organisation(&mut self, id: u64, owner: Option<u32>) -> Result<(), &'static str> {
        if self.organisation_demands.get(&id).copied() == owner {
            return Ok(());
        }
        if let Some(old) = self.organisation_demands.remove(&id) {
            self.remove_key(old, ORGANISATION_CHANGE_FACTOR)?;
        }
        if let Some(country) = owner {
            reserve(&mut self.organisation_demands)?;
            reserve(&mut self.countries)?;
            let keys = self.countries.entry(country).or_default();
            let before = keys.capacity();
            reserve(keys)?;
            self.owner_key_bytes += (keys.capacity() - before) * size_of::<(u32, usize)>();
            let count = keys.entry(ORGANISATION_CHANGE_FACTOR).or_default();
            let new_key = *count == 0;
            *count = count
                .checked_add(1)
                .ok_or("design context demand overflow")?;
            self.organisation_demands.insert(id, country);
            if new_key {
                reserve(&mut self.dirty)?;
                self.dirty.entry(country).or_insert(false);
            }
        }
        Ok(())
    }
    fn remove(&mut self, id: u64) -> Result<(), &'static str> {
        self.replace_organisation(id, None)?;
        let Some(old) = self.designs.remove(&id) else {
            return Ok(());
        };
        self.demand_bytes -= old.keys.capacity() * size_of::<u32>();
        for key in old.keys {
            self.remove_key(old.country, key)?;
        }
        let count = self
            .template_demands
            .get_mut(&old.country)
            .ok_or("design template demand missing")?;
        *count -= 1;
        if *count == 0 {
            self.template_demands.remove(&old.country);
            if let Some(row) = self.template_values.remove(&old.country) {
                self.template_heap_bytes -= rows::RowCodec::heap_bytes(&row);
                self.template_changed = true;
            }
            self.template_dirty.remove(&old.country);
        }
        Ok(())
    }
    fn replace(&mut self, id: u64, country: u32, keys: Vec<u32>) -> Result<(), &'static str> {
        if let Some(old) = self.designs.get_mut(&id) {
            if old.country == country && old.keys == keys {
                old.generation = self.generation;
                self.spare_keys = keys;
                return Ok(());
            }
            if old.country == country {
                let counts = self
                    .countries
                    .get_mut(&country)
                    .ok_or("design context owner missing")?;
                for key in old
                    .keys
                    .iter()
                    .filter(|key| keys.binary_search(key).is_err())
                {
                    let count = counts.get_mut(key).ok_or("design context demand missing")?;
                    *count -= 1;
                    if *count == 0 {
                        counts.remove(key);
                        self.values.remove(&(country, *key));
                        self.changed = true;
                    }
                }
                let mut added = false;
                for key in keys
                    .iter()
                    .filter(|key| old.keys.binary_search(key).is_err())
                {
                    let before = counts.capacity();
                    reserve(counts)?;
                    self.owner_key_bytes +=
                        (counts.capacity() - before) * size_of::<(u32, usize)>();
                    let count = counts.entry(*key).or_default();
                    added |= *count == 0;
                    *count = count
                        .checked_add(1)
                        .ok_or("design context demand overflow")?;
                }
                self.demand_bytes -= old.keys.capacity() * size_of::<u32>();
                self.demand_bytes += keys.capacity() * size_of::<u32>();
                self.spare_keys = std::mem::replace(&mut old.keys, keys);
                old.generation = self.generation;
                if added {
                    reserve(&mut self.dirty)?;
                    self.dirty.entry(country).or_insert(false);
                }
                return Ok(());
            }
        }
        self.remove(id)?;
        let new_owner = !self.template_demands.contains_key(&country);
        reserve(&mut self.countries)?;
        let counts = self.countries.entry(country).or_default();
        let mut newly_demanded = false;
        for key in &keys {
            let before = counts.capacity();
            reserve(counts)?;
            self.owner_key_bytes += (counts.capacity() - before) * size_of::<(u32, usize)>();
            let count = counts.entry(*key).or_default();
            newly_demanded |= *count == 0;
            *count = count
                .checked_add(1)
                .ok_or("design context demand overflow")?;
        }
        if newly_demanded {
            reserve(&mut self.dirty)?;
            self.dirty.entry(country).or_insert(false);
        }
        reserve(&mut self.template_demands)?;
        let template_count = self.template_demands.entry(country).or_default();
        *template_count = template_count
            .checked_add(1)
            .ok_or("design template demand overflow")?;
        if new_owner {
            reserve(&mut self.template_dirty)?;
            self.template_dirty.insert(country, true);
        }
        reserve(&mut self.designs)?;
        self.demand_bytes += keys.capacity() * size_of::<u32>();
        self.designs.insert(
            id,
            Demand {
                country,
                keys,
                generation: self.generation,
            },
        );
        Ok(())
    }
    fn track(
        &mut self,
        memory: &impl Memory,
        definition: &cequipment_variant::DefinitionRow<Arc<rows::VariantDesignState>>,
        modules: &[equipment_module_container::Row],
        first_upgrade: Option<&cequipment_upgrades_instance::Row>,
        organisation_owner: Option<u32>,
    ) -> Result<(), &'static str> {
        let country = definition.identity.creator_link;
        let mut keys = std::mem::take(&mut self.spare_keys);
        keys.clear();
        engine_allocation::reserve(&mut keys, 1).map_err(|_| "design context allocation failed")?;
        keys.push(COMMON_COST_FACTOR);
        let currency = first_upgrade
            .map(|upgrade| {
                immutable_key(
                    memory,
                    &mut self.upgrade_currencies,
                    upgrade.definition_token as u32,
                    upgrade.definition_pointer,
                    engine_native::cequipment_upgrade::COST_CURRENCY,
                )
            })
            .transpose()?
            .unwrap_or(0);
        engine_allocation::reserve(&mut keys, 1).map_err(|_| "design context allocation failed")?;
        keys.push(match currency {
            1 => SHIP_COST_FACTOR,
            2 => AIR_COST_FACTOR,
            _ => TANK_COST_FACTOR,
        });
        let flags = definition.identity.classifier_flags;
        engine_allocation::reserve(&mut keys, 1).map_err(|_| "design context allocation failed")?;
        keys.push(if flags & engine_native::cequipment_type::SHIP_MASK != 0 {
            REFIT_IC_FACTOR
        } else {
            CONVERSION_SPEED_FACTOR
        });
        let family = if flags & engine_native::cequipment_type::SHIP_MASK != 0 {
            Some(SHIP_COST_FACTOR)
        } else if flags & engine_native::cequipment_type::TANK_MASK != 0 {
            Some(TANK_COST_FACTOR)
        } else if flags & engine_native::cequipment_type::PLANE_MASK != 0 {
            Some(AIR_COST_FACTOR)
        } else {
            None
        };
        if let Some(family) = family {
            engine_allocation::reserve(&mut keys, 2)
                .map_err(|_| "design context allocation failed")?;
            keys.push(family);
            if family == AIR_COST_FACTOR {
                keys.push(AIR_EXPERIENCE_TRAINING_FACTOR);
            }
        }
        let token = definition.identity.archetype_token;
        let key = immutable_key(
            memory,
            &mut self.types,
            token,
            definition.archetype_pointer,
            engine_native::cequipment_type::DESIGN_COST_MODIFIER,
        )?;
        engine_allocation::reserve(&mut keys, modules.len() + 1)
            .map_err(|_| "design context allocation failed")?;
        keys.push(key);
        for module in modules {
            keys.push(immutable_key(
                memory,
                &mut self.modules,
                module.definition_token,
                module.definition_pointer,
                engine_native::cequipment_module::DESIGN_COST_MODIFIER,
            )?);
        }
        keys.sort_unstable();
        keys.dedup();
        self.replace(definition.identity.id, country, keys)?;
        self.replace_organisation(definition.identity.id, organisation_owner)
    }
    fn publish(&mut self) -> Result<Snapshot, &'static str> {
        if self.changed || self.snapshot.is_none() {
            let mut output = engine_allocation::vec(self.values.len())
                .map_err(|_| "design context publication allocation failed")?;
            for value in self.values.values() {
                output.push(value.clone());
            }
            output.sort_unstable_by_key(|row| (row.country, row.modifier));
            self.published_bytes = output.capacity() * size_of::<rows::DesignCountryModifierRow>();
            self.snapshot = Some(Arc::new(rows::TableRows::DesignCountryModifiers(output)));
            self.changed = false;
        }
        Ok(self.snapshot.clone())
    }
    fn refresh(&mut self, memory: &impl Memory, base: usize) -> Result<Snapshot, &'static str> {
        if self.dirty.is_empty() && (!self.template_admitted || self.template_dirty.is_empty()) {
            return self.publish();
        }
        let reader = country_reader(memory, base);
        for (country, all) in self.dirty.drain() {
            let Some(keys) = self.countries.get(&country) else {
                continue;
            };
            let owner = reader
                .as_ref()
                .map_err(|e| *e)
                .and_then(|r| r.resolve(country));
            if let Ok(owner) = &owner {
                self.owners.bind(country, owner.id)?;
                if self.template_demands.contains_key(&country) {
                    super::design_template_changes::observe_owner(owner.id);
                }
            }
            if self.template_admitted && self.template_dirty.remove(&country).is_some() {
                let row = template_row(memory, base, country, owner.as_ref().map_err(|e| *e));
                put_template(
                    &mut self.template_values,
                    &mut self.template_heap_bytes,
                    &mut self.template_changed,
                    &mut self.failures,
                    row,
                )?;
            }
            for key in keys.keys() {
                if !all && self.values.contains_key(&(country, *key)) {
                    continue;
                }
                let value = owner.as_ref().map_err(|e| *e).and_then(|owner| {
                    let address = owner
                        .pointer
                        .checked_add(ccountry::MODIFIER_ACCUMULATOR)
                        .ok_or("design country accumulator address overflow")?;
                    engine_native::modifier_accumulator::requested_effective_at(
                        memory, base, address, *key,
                    )
                });
                let row = rows::DesignCountryModifierRow {
                    country,
                    modifier: *key,
                    value: value.as_ref().ok().copied(),
                    failure: if value.is_ok() {
                        0
                    } else if owner.is_err() {
                        1
                    } else {
                        2
                    },
                };
                self.failures = self.failures.saturating_add(u64::from(value.is_err()));
                if self.values.get(&(country, *key)) != Some(&row) {
                    reserve(&mut self.values)?;
                    self.values.insert((country, *key), row);
                    self.changed = true;
                }
            }
        }
        if self.template_admitted {
            for (country, _) in self.template_dirty.drain() {
                if !self.template_demands.contains_key(&country) {
                    continue;
                }
                let _reads = read_accounting::table(TableId::DesignCountryTemplates);
                let owner = reader
                    .as_ref()
                    .map_err(|e| *e)
                    .and_then(|reader| reader.resolve(country));
                if let Ok(owner) = &owner {
                    self.owners.bind(country, owner.id)?;
                    super::design_template_changes::observe_owner(owner.id);
                }
                let row = template_row(memory, base, country, owner.as_ref().map_err(|e| *e));
                put_template(
                    &mut self.template_values,
                    &mut self.template_heap_bytes,
                    &mut self.template_changed,
                    &mut self.failures,
                    row,
                )?;
            }
        }
        super::design_template_changes::prepare();
        self.publish()
    }
    fn admit_templates(&mut self) -> Result<(), &'static str> {
        let mut allocation_failed = false;
        let epoch = super::design_template_changes::take(|id| {
            if let Ok(id) = u32::try_from(id)
                && self
                    .owners
                    .mark(&self.template_demands, &mut self.template_dirty, id)
                    .is_err()
            {
                allocation_failed = true;
            }
        });
        if allocation_failed {
            return Err("design template owner mark allocation failed");
        }
        if epoch.is_some() && self.template_epoch != epoch {
            engine_allocation::reserve_map(&mut self.template_dirty, self.template_demands.len())
                .map_err(|_| "design template owner mark allocation failed")?;
            for country in self.template_demands.keys() {
                self.template_dirty.insert(*country, true);
            }
        }
        self.template_epoch = epoch;
        self.template_admitted = epoch.is_some();
        if epoch.is_none() {
            self.template_snapshot = None;
        }
        Ok(())
    }
    fn publish_templates(&mut self) -> Result<Snapshot, &'static str> {
        if !self.template_admitted {
            return Ok(None);
        }
        if self.template_changed || self.template_snapshot.is_none() {
            let mut rows = engine_allocation::vec(self.template_values.len())
                .map_err(|_| "design template publication allocation failed")?;
            for row in self.template_values.values() {
                rows.push(
                    rows::RowCodec::try_clone(row)
                        .map_err(|_| "design template publication allocation failed")?,
                );
            }
            rows.sort_unstable_by_key(|row| row.country);
            self.template_published_bytes = rows.capacity()
                * size_of::<rows::DesignCountryTemplatesRow>()
                + rows.iter().map(rows::RowCodec::heap_bytes).sum::<usize>();
            self.template_snapshot = Some(Arc::new(rows::TableRows::DesignCountryTemplates(rows)));
            self.template_changed = false;
        }
        Ok(self.template_snapshot.clone())
    }
}

fn template_row(
    memory: &impl Memory,
    base: usize,
    country: u32,
    owner: Result<&ccountry::Country, &'static str>,
) -> rows::DesignCountryTemplatesRow {
    let _reads = read_accounting::table(TableId::DesignCountryTemplates);
    let entries = owner.and_then(|owner| {
        let status = engine_native::read::<u64>(
            memory,
            owner
                .pointer
                .checked_add(ccountry::PRODUCTION_STATUS)
                .ok_or("design template production owner address overflow")?,
        )
        .ok_or("design template production owner unreadable")? as usize;
        if engine_native::read::<u64>(
            memory,
            status
                .checked_add(cproduction_status::COUNTRY)
                .ok_or("design template production owner address overflow")?,
        ) != Some(owner.pointer as u64)
        {
            return Err("design template country backlink mismatch");
        }
        cproduction_status::design_templates::read_templates(memory, base, status)
    });
    let mut row = rows::DesignCountryTemplatesRow {
        country,
        failure: if entries.is_ok() {
            0
        } else if owner.is_err() {
            1
        } else {
            2
        },
        ..Default::default()
    };
    let entries = entries.unwrap_or_default();
    if engine_allocation::reserve(&mut row.uses, entries.len()).is_err()
        || engine_allocation::reserve(&mut row.factors, entries.len()).is_err()
        || engine_allocation::reserve(&mut row.equipment, entries.len()).is_err()
    {
        row.failure = 2;
        return row;
    }
    for entry in entries {
        row.uses.push(entry.uses);
        row.factors.push(entry.factor);
        row.equipment.push(entry.equipment);
    }
    row
}
fn put_template(
    values: &mut HashMap<u32, rows::DesignCountryTemplatesRow>,
    heap: &mut usize,
    changed: &mut bool,
    failures: &mut u64,
    row: rows::DesignCountryTemplatesRow,
) -> Result<(), &'static str> {
    *failures = failures.saturating_add(u64::from(row.failure != 0));
    if values.get(&row.country) != Some(&row) {
        reserve(values)?;
        *heap += rows::RowCodec::heap_bytes(&row);
        if let Some(old) = values.insert(row.country, row) {
            *heap -= rows::RowCodec::heap_bytes(&old);
        }
        *changed = true;
    }
    Ok(())
}
fn organisation_owner(
    memory: &impl Memory,
    base: usize,
    definition: &cequipment_variant::DefinitionRow<Arc<rows::VariantDesignState>>,
) -> Result<Option<u32>, &'static str> {
    use engine_production::nindustrial_organisation_corganisation as organisation;
    let state = &definition.design;
    if state.design_team == 0 || !state.design_team_serialized {
        return Ok(None);
    }
    if state.design_team as u32 != organisation::REFERENCE_TYPE_ID {
        return Err("design organisation owner has wrong reference kind");
    }
    let pointer = definition
        .organisation_pointer
        .ok_or("design organisation owner resolution not held by this visit")?;
    organisation_owner_at(memory, base, state.design_team, pointer)
}
fn reseed_organisation_owner(
    memory: &impl Memory,
    base: usize,
    definition: &cequipment_variant::DefinitionRow<Arc<rows::VariantDesignState>>,
) -> Result<Option<u32>, &'static str> {
    let state = &definition.design;
    if definition.organisation_pointer.is_some() || state.design_team == 0 {
        return organisation_owner(memory, base, definition);
    }
    // A carried design deliberately has no visit-local organisation pointer.
    // Context recovery resolves only that reached raw author, not the design.
    // A frozen negative lookup is not proof of current author liveness.
    if state.design_team_serialized
        && state.design_team as u32
            != engine_production::nindustrial_organisation_corganisation::REFERENCE_TYPE_ID
    {
        return Err("design organisation owner has wrong reference kind");
    }
    let pointer = engine_native::creference_registry::resolve(memory, base, state.design_team)?;
    pointer.map_or(Ok(None), |pointer| {
        organisation_owner_at(memory, base, state.design_team, pointer)
    })
}
fn organisation_owner_at(
    memory: &impl Memory,
    base: usize,
    handle: u64,
    pointer: usize,
) -> Result<Option<u32>, &'static str> {
    use engine_production::nindustrial_organisation_corganisation as organisation;
    if handle as u32 != organisation::REFERENCE_TYPE_ID {
        return Err("design organisation owner has wrong reference kind");
    }
    if engine_native::read::<u64>(memory, pointer)
        != base.checked_add(organisation::VTABLE_RVA).map(|p| p as u64)
        || engine_native::read::<u64>(
            memory,
            pointer
                .checked_add(organisation::REFERENCE)
                .ok_or("design organisation owner address overflow")?,
        ) != Some(handle)
    {
        return Err("design organisation owner identity mismatch");
    }
    engine_native::read::<u32>(
        memory,
        pointer
            .checked_add(organisation::OWNER)
            .ok_or("design organisation owner address overflow")?,
    )
    .map(Some)
    .ok_or("design organisation owner unreadable")
}

fn country_reader<'a, M: Memory>(
    memory: &'a M,
    base: usize,
) -> Result<ccountry::Reader<'a, M>, &'static str> {
    let state = base
        .checked_add(cgame_state::SINGLETON_RVA)
        .and_then(|p| engine_native::read::<u64>(memory, p))
        .ok_or("design context game state unreadable")? as usize;
    ccountry::Reader::new(memory, base, state)
}
fn immutable_key(
    memory: &impl Memory,
    cache: &mut HashMap<u32, u32>,
    token: u32,
    pointer: usize,
    offset: usize,
) -> Result<u32, &'static str> {
    if let Some(key) = cache.get(&token) {
        return Ok(*key);
    }
    let key = pointer
        .checked_add(offset)
        .and_then(|p| engine_native::read(memory, p))
        .ok_or("design content modifier key unreadable")?;
    reserve(cache)?;
    cache.insert(token, key);
    Ok(key)
}
pub(super) fn seed(
    memory: &impl Memory,
    base: usize,
    variants: &Variants,
    full: bool,
) -> Result<Snapshots, &'static str> {
    let _reads = read_accounting::table(TableId::DesignCountryModifiers);
    let mut cache = cache();
    worker_reset(&mut cache);
    seed_cached(
        &mut cache,
        memory,
        base,
        variants,
        full,
        super::design_changes::enabled(),
        Cache::admit_templates,
    )
}
fn seed_cached(
    cache: &mut Cache,
    memory: &impl Memory,
    base: usize,
    variants: &Variants,
    full: bool,
    designs_observed: bool,
    admit_templates: impl FnOnce(&mut Cache) -> Result<(), &'static str>,
) -> Result<Snapshots, &'static str> {
    // Country-context admission is independent of the three design proofs.
    // Retry only on this existing daily visit, never by forcing design reads.
    let full = full || !cache.seeded || cache.failure.is_some();
    if cache.failure.is_some() {
        let failures = cache.failures;
        *cache = Cache::default();
        cache.failures = failures;
    }
    let result = seed_demands(
        cache,
        memory,
        base,
        variants,
        full,
        designs_observed,
        admit_templates,
    );
    if let Err(reason) = result {
        cache.seeded = false;
        cache.failure = Some(reason);
        cache.last_refusal = Some(reason);
        cache.failures = cache.failures.saturating_add(1);
        cache.snapshot = None;
        cache.template_snapshot = None;
        cache.template_epoch = None;
    }
    result
}
fn seed_demands(
    cache: &mut Cache,
    memory: &impl Memory,
    base: usize,
    variants: &Variants,
    full: bool,
    designs_observed: bool,
    admit_templates: impl FnOnce(&mut Cache) -> Result<(), &'static str>,
) -> Result<Snapshots, &'static str> {
    admit_templates(cache)?;
    cache.generation = cache
        .generation
        .checked_add(1)
        .ok_or("design context census generation overflow")?;
    let generation = cache.generation;
    let modules = &variants.modules.rows[..variants.modules.outcome.len()];
    let upgrades = &variants.upgrades.rows[..variants.upgrades.outcome.len()];
    let mut module_index = 0;
    let mut upgrade_index = 0;
    for definition in &variants.definitions.rows[..variants.definitions.outcome.len()] {
        let first_module = module_index;
        while modules
            .get(module_index)
            .is_some_and(|module| module.variant == definition.identity.id)
        {
            module_index += 1;
        }
        let first_upgrade = upgrade_index;
        while upgrades
            .get(upgrade_index)
            .is_some_and(|upgrade| upgrade.variant == definition.identity.id)
        {
            upgrade_index += 1;
        }
        if designs_observed
            && definition.source_links.is_none()
            && let Some(demand) = cache.designs.get_mut(&definition.identity.id)
        {
            demand.generation = generation;
            continue;
        }
        // The legacy module census is already owned; no new native module walk.
        cache.track(
            memory,
            definition,
            &modules[first_module..module_index],
            upgrades[first_upgrade..upgrade_index].first(),
            reseed_organisation_owner(memory, base, definition)?,
        )?;
    }
    if module_index != modules.len() || upgrade_index != upgrades.len() {
        return Err("design context module census order inconsistent");
    }
    let mut removed = engine_allocation::vec(0).map_err(|_| "design context allocation failed")?;
    for (id, demand) in &cache.designs {
        if demand.generation != generation {
            engine_allocation::reserve(&mut removed, 1)
                .map_err(|_| "design context allocation failed")?;
            removed.push(*id);
        }
    }
    for id in removed {
        cache.remove(id)?;
    }
    if full {
        let owners = engine_allocation::collect_vec(cache.countries.keys().copied())
            .map_err(|_| "design context allocation failed")?;
        for country in owners {
            cache.dirty(country)?;
            if cache.template_demands.contains_key(&country) {
                reserve(&mut cache.template_dirty)?;
                cache.template_dirty.insert(country, true);
            }
        }
    }
    cache.seeded = true;
    let modifiers = cache.refresh(memory, base)?;
    let templates = cache.publish_templates()?;
    Ok((modifiers, templates))
}
pub(super) fn remove(variant: u64) -> Result<(), &'static str> {
    cache().remove(variant)
}

pub(super) fn event(
    memory: &impl Memory,
    base: usize,
    variants: &Variants,
    removed: bool,
) -> Result<(), &'static str> {
    let Some(definition) = variants.definitions.rows.first() else {
        return Err("design context event has no definition");
    };
    let mut cache = cache();
    let result = if removed {
        cache.remove(definition.identity.id)
    } else {
        organisation_owner(memory, base, definition).and_then(|owner| {
            cache.track(
                memory,
                definition,
                &variants.modules.rows[..variants.modules.outcome.len()],
                variants.upgrades.rows[..variants.upgrades.outcome.len()].first(),
                owner,
            )
        })
    };
    if let Err(reason) = result {
        cache.snapshot = None;
        cache.failure = Some(reason);
        cache.failures = cache.failures.saturating_add(1);
    }
    Ok(())
}
/// Current author liveness can change its country demand independently of the
/// frozen design lookup witness. Reuse the boundary's resolved raw reference.
pub(super) fn author(
    memory: &impl Memory,
    base: usize,
    handle: u64,
    pointer: Option<usize>,
    designs: &[u64],
) -> Result<(), &'static str> {
    let mut cache = cache();
    if !cache.seeded || cache.failure.is_some() {
        return Ok(());
    }
    let result = (|| {
        let owner = pointer
            .map(|pointer| organisation_owner_at(memory, base, handle, pointer))
            .transpose()?
            .flatten();
        for id in designs {
            if cache.designs.contains_key(id) {
                cache.replace_organisation(*id, owner)?;
            }
        }
        Ok(())
    })();
    if let Err(reason) = result {
        cache.snapshot = None;
        cache.failure = Some(reason);
        cache.failures = cache.failures.saturating_add(1);
    }
    Ok(())
}

pub(super) fn capture(memory: &impl Memory, base: usize) -> Result<Snapshots, &'static str> {
    let _reads = read_accounting::table(TableId::DesignCountryModifiers);
    let result = capture_inner(memory, base);
    let mut cache = cache();
    if let Err(reason) = result {
        cache.last_refusal = Some(reason);
        cache.failures = cache.failures.saturating_add(1);
        cache.snapshot = None;
        cache.epoch = None;
        cache.template_snapshot = None;
        cache.template_epoch = None;
        cache.template_admitted = false;
    } else {
        cache.last_refusal = None;
    }
    result
}
fn capture_inner(memory: &impl Memory, base: usize) -> Result<Snapshots, &'static str> {
    let mut cache = cache();
    worker_reset(&mut cache);
    if let Some(reason) = cache.failure {
        return Err(reason);
    }
    let mut allocation_failed = false;
    let epoch = changes::Observer.take(changes::Table::Designs, &mut |id| {
        if let Ok(id) = u32::try_from(id) {
            let Cache {
                owners,
                countries,
                dirty,
                ..
            } = &mut *cache;
            if owners.mark(countries, dirty, id).is_err() {
                allocation_failed = true;
            }
        }
    });
    if allocation_failed {
        return Err("design context mark allocation failed");
    }
    let Some(epoch) = epoch else {
        cache.snapshot = None;
        return Err("design country modifier writer certificate unavailable");
    };
    if cache.epoch != Some(epoch) {
        cache.epoch = Some(epoch);
        let owners = engine_allocation::collect_vec(cache.countries.keys().copied())
            .map_err(|_| "design context allocation failed")?;
        for country in owners {
            cache.dirty(country)?;
        }
    }
    if !cache.seeded {
        return Err("design country context census not acquired");
    }
    cache.admit_templates()?;
    let modifiers = cache.refresh(memory, base)?;
    let templates = cache.publish_templates()?;
    Ok((modifiers, templates))
}
pub(super) fn invalidate() {
    // Also called at a callback boundary. Do not drop owner maps or shared
    // payloads there; the acquisition worker retires them before its next read.
    INVALIDATED.store(true, std::sync::atomic::Ordering::Release);
    super::design_template_changes::invalidate();
}
#[derive(serde::Serialize)]
pub(super) struct Status {
    admitted: bool,
    designs: usize,
    countries: usize,
    operands: usize,
    failures: u64,
    failure: Option<&'static str>,
    last_refusal: Option<&'static str>,
    allocated_bytes: usize,
}
pub(super) fn status() -> Status {
    let cache = cache();
    Status {
        admitted: cache.seeded && cache.failure.is_none(),
        designs: cache.designs.len(),
        countries: cache.countries.len(),
        operands: cache.values.len(),
        failures: cache.failures,
        failure: cache.failure,
        last_refusal: cache.last_refusal,
        allocated_bytes: cache.demand_bytes
            + cache.spare_keys.capacity() * size_of::<u32>()
            + cache.owner_key_bytes
            + cache.published_bytes
            + cache.template_heap_bytes
            + cache.template_published_bytes
            + cache.template_values.capacity()
                * size_of::<(u32, rows::DesignCountryTemplatesRow)>()
            + cache.template_dirty.capacity() * size_of::<(u32, bool)>()
            + cache.designs.capacity() * size_of::<(u64, Demand)>()
            + cache.organisation_demands.capacity() * size_of::<(u64, u32)>()
            + cache.template_demands.capacity() * size_of::<(u32, usize)>()
            + cache.countries.capacity() * size_of::<(u32, HashMap<u32, usize>)>()
            + cache.types.capacity() * size_of::<(u32, u32)>()
            + cache.modules.capacity() * size_of::<(u32, u32)>()
            + cache.upgrade_currencies.capacity() * size_of::<(u32, u32)>()
            + cache.owners.allocated_bytes()
            + cache.dirty.capacity() * size_of::<(u32, bool)>()
            + cache.values.capacity() * size_of::<((u32, u32), rows::DesignCountryModifierRow)>(),
    }
}
pub(super) fn input(snapshot: &Snapshot) -> TableInput {
    snapshot.as_ref().map_or_else(
        || unavailable("design country context unavailable").into(),
        |rows| TableInput::SharedComplete(Arc::clone(rows)),
    )
}

pub(super) fn template_input(snapshot: &Snapshot) -> TableInput {
    snapshot.as_ref().map_or_else(
        || unavailable("design production-template writer certificate unavailable").into(),
        |rows| TableInput::SharedComplete(Arc::clone(rows)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    struct World {
        bytes: Vec<u8>,
        reads: Cell<usize>,
    }
    impl Memory for World {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            if address == cgame_state::SINGLETON_RVA && output.len() == 8 {
                output.copy_from_slice(&0x1000_u64.to_le_bytes());
                true
            } else if address == engine_native::modifier_definition::TABLE_RVA && output.len() == 8
            {
                output.copy_from_slice(&0x14000_u64.to_le_bytes());
                true
            } else {
                engine_native::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &self.bytes,
                }
                .copy(address, output)
            }
        }
    }
    #[test]
    fn design_organisation_demand_reuses_sample_pointer_and_authenticates_owner_without_registry_walk()
     {
        use engine_production::nindustrial_organisation_corganisation as organisation;
        let mut bytes = vec![0; 0x1000];
        let handle = (123_u64 << 32) | u64::from(organisation::REFERENCE_TYPE_ID);
        bytes[..8].copy_from_slice(&(organisation::VTABLE_RVA as u64).to_le_bytes());
        bytes[organisation::REFERENCE..organisation::REFERENCE + 8]
            .copy_from_slice(&handle.to_le_bytes());
        bytes[organisation::OWNER..organisation::OWNER + 4]
            .copy_from_slice(&4_999_u32.to_le_bytes());
        let memory = World {
            bytes,
            reads: Cell::new(0),
        };
        let mut definition = cequipment_variant::DefinitionRow {
            organisation_pointer: Some(0x1000),
            design: Arc::new(rows::VariantDesignState {
                design_team: handle,
                design_team_serialized: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(organisation_owner(&memory, 0, &definition), Ok(Some(4_999)));
        assert_eq!(
            memory.reads.get(),
            3,
            "vtable, self-reference and owner only"
        );
        definition.organisation_pointer = None;
        assert!(
            organisation_owner(&memory, 0, &definition)
                .unwrap_err()
                .contains("this visit")
        );
        Arc::make_mut(&mut definition.design).design_team_serialized = false;
        assert_eq!(organisation_owner(&memory, 0, &definition), Ok(None));
        assert_eq!(memory.reads.get(), 3, "unresolved author has no owner read");
    }
    #[test]
    fn design_organisation_owner_demand_is_shared_and_never_adds_template_owners() {
        let mut cache = Cache::default();
        for id in [1, 2] {
            cache.replace(id, 5_000, vec![1]).unwrap();
            cache.replace_organisation(id, Some(4_999)).unwrap();
        }
        assert_eq!(cache.countries[&4_999][&ORGANISATION_CHANGE_FACTOR], 2);
        assert_eq!(cache.template_demands.len(), 1);
        assert_eq!(cache.template_demands[&5_000], 2);
        assert!(!cache.template_dirty.contains_key(&4_999));
        cache.remove(1).unwrap();
        assert_eq!(cache.countries[&4_999][&ORGANISATION_CHANGE_FACTOR], 1);
        cache.remove(2).unwrap();
        assert!(cache.countries.is_empty());
        assert!(cache.organisation_demands.is_empty());
        assert!(cache.template_demands.is_empty());
        assert!(cache.template_dirty.is_empty());
    }
    #[test]
    fn repaired_author_context_rederives_every_dependant_without_changing_creator_demand() {
        use engine_production::nindustrial_organisation_corganisation as organisation;
        let handle = (123u64 << 32) | u64::from(organisation::REFERENCE_TYPE_ID);
        let mut bytes = vec![0; (organisation::OWNER + 4).max(organisation::REFERENCE + 8)];
        bytes[..8].copy_from_slice(&(organisation::VTABLE_RVA as u64).to_le_bytes());
        bytes[organisation::REFERENCE..organisation::REFERENCE + 8]
            .copy_from_slice(&handle.to_le_bytes());
        bytes[organisation::OWNER..organisation::OWNER + 4]
            .copy_from_slice(&4_998u32.to_le_bytes());
        {
            let mut cache = cache();
            for id in [1, 2] {
                cache.replace(id, 5_000, vec![1]).unwrap();
                cache.replace_organisation(id, Some(4_999)).unwrap();
            }
            cache.seeded = true;
        }
        let memory = engine_native::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        author(&memory, 0, handle, Some(0x1000), &[1, 2]).unwrap();
        {
            let cache = cache();
            assert_eq!(cache.organisation_demands[&1], 4_998);
            assert_eq!(cache.organisation_demands[&2], 4_998);
            assert_eq!(cache.countries[&4_998][&ORGANISATION_CHANGE_FACTOR], 2);
            assert!(!cache.countries.contains_key(&4_999));
            assert_eq!(cache.template_demands[&5_000], 2);
        }
        author(&memory, 0, handle, None, &[1, 2]).unwrap();
        let cache = cache();
        assert!(cache.organisation_demands.is_empty());
        assert!(!cache.countries.contains_key(&4_998));
        assert_eq!(cache.template_demands[&5_000], 2);
    }
    #[test]
    fn carried_context_authors_recheck_current_liveness_after_a_frozen_negative_lookup() {
        use engine_production::nindustrial_organisation_corganisation as organisation;
        struct Roots(std::cell::Cell<usize>);
        impl Memory for Roots {
            fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
                assert_eq!(
                    address,
                    engine_native::creference_registry::LOW_ROOTS_RVA
                        + organisation::REFERENCE_TYPE_ID as usize * 8
                );
                assert_eq!(bytes.len(), 8);
                self.0.set(self.0.get() + 1);
                bytes.fill(0);
                true
            }
        }
        let memory = Roots(std::cell::Cell::new(0));
        let definition = cequipment_variant::DefinitionRow {
            design: Arc::new(rows::VariantDesignState {
                design_team: (123u64 << 32) | u64::from(organisation::REFERENCE_TYPE_ID),
                design_team_serialized: false,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(reseed_organisation_owner(&memory, 0, &definition), Ok(None));
        assert_eq!(memory.0.get(), 1);
    }
    #[test]
    fn carried_context_authors_refuse_the_wrong_kind_before_any_registry_read() {
        struct Unreadable;
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("wrong-kind author reached native storage");
            }
        }
        let definition = cequipment_variant::DefinitionRow {
            design: Arc::new(rows::VariantDesignState {
                design_team: u64::from(
                    engine_production::nindustrial_organisation_corganisation::REFERENCE_TYPE_ID
                        + 1,
                ),
                design_team_serialized: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            reseed_organisation_owner(&Unreadable, 0, &definition),
            Err("design organisation owner has wrong reference kind")
        );
    }

    #[test]
    fn failed_cost_keys_recover_on_the_next_daily_context_seed_with_designs_carried() {
        use super::super::production_rest_test_kit as kit;
        const MODULE: usize = 0x15000;
        struct Recovering<'a> {
            world: &'a World,
            failed_key: usize,
            failing: Cell<bool>,
        }
        impl Memory for Recovering<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                assert!(
                    !(kit::VARIANT..kit::VARIANT + cequipment_variant::DESIGN_TEAM_TRAITS + 4)
                        .contains(&address),
                    "country-context recovery followed a carried design"
                );
                if self.failing.get() && address == self.failed_key {
                    return false;
                }
                self.world.copy(address, output)
            }
        }
        fn admitted(cache: &mut Cache) -> Result<(), &'static str> {
            cache.template_admitted = true;
            cache.template_epoch = Some(1);
            Ok(())
        }
        for key in [
            kit::EQUIPMENT_TYPE + engine_native::cequipment_type::DESIGN_COST_MODIFIER,
            MODULE + engine_native::cequipment_module::DESIGN_COST_MODIFIER,
        ] {
            let mut bytes = kit::fixture();
            bytes.resize(0x16000, 0);
            for (address, value) in [
                (
                    kit::EQUIPMENT_TYPE + engine_native::cequipment_type::DESIGN_COST_MODIFIER,
                    777_u32,
                ),
                (
                    MODULE + engine_native::cequipment_module::DESIGN_COST_MODIFIER,
                    778_u32,
                ),
            ] {
                bytes[address - kit::BASE..address - kit::BASE + 4]
                    .copy_from_slice(&value.to_le_bytes());
            }
            let templates = 0x10000 + cproduction_status::design_templates::RECORDS - kit::BASE;
            bytes[templates..templates + 16].fill(0);
            let world = World {
                bytes,
                reads: Cell::new(0),
            };
            let memory = Recovering {
                world: &world,
                failed_key: key,
                failing: Cell::new(true),
            };
            let design = Arc::new(rows::VariantDesignState {
                niches: vec![1, 2, 3],
                ..Default::default()
            });
            let mut variants = Variants::vacant();
            variants.definitions.rows = vec![cequipment_variant::DefinitionRow {
                identity: cequipment_variant::Row {
                    id: 70,
                    creator_link: 2,
                    archetype_token: 3,
                    ..Default::default()
                },
                archetype_pointer: kit::EQUIPMENT_TYPE,
                design: Arc::clone(&design),
                source_links: None,
                ..Default::default()
            }];
            variants.definitions.outcome = Outcome::Rows { len: 1 };
            variants.modules.rows = vec![equipment_module_container::Row {
                variant: 70,
                definition_token: 4,
                definition_pointer: MODULE,
                ..Default::default()
            }];
            variants.modules.outcome = Outcome::Rows { len: 1 };
            variants.upgrades.outcome = Outcome::Rows { len: 0 };
            let mut cache = Cache::default();
            assert!(seed_cached(&mut cache, &memory, 0, &variants, false, true, admitted).is_err());
            assert!(!cache.seeded);
            assert!(cache.failure.is_some());
            assert!(cache.snapshot.is_none() && cache.template_snapshot.is_none());
            memory.failing.set(false);
            let (modifiers, templates) =
                seed_cached(&mut cache, &memory, 0, &variants, false, true, admitted).unwrap();
            assert!(cache.seeded && cache.failure.is_none());
            let modifiers = modifiers.unwrap();
            let templates = templates.unwrap();
            assert!(
                modifiers
                    .rows::<tables::DesignCountryModifiers>()
                    .unwrap()
                    .iter()
                    .all(|row| row.failure == 0)
            );
            assert!(
                templates
                    .rows::<tables::DesignCountryTemplates>()
                    .unwrap()
                    .iter()
                    .all(|row| row.failure == 0)
            );
            assert!(
                cache.designs[&70].keys.contains(&777) && cache.designs[&70].keys.contains(&778)
            );
            assert!(Arc::ptr_eq(&variants.definitions.rows[0].design, &design));
            let reads = world.reads.get();
            let (quiet, quiet_templates) =
                seed_cached(&mut cache, &memory, 0, &variants, false, true, admitted).unwrap();
            assert!(Arc::ptr_eq(&modifiers, &quiet.unwrap()));
            assert!(Arc::ptr_eq(&templates, &quiet_templates.unwrap()));
            assert_eq!(
                world.reads.get(),
                reads,
                "an admitted daily context carries without native reads"
            );
        }
    }

    #[test]
    fn variant_country_context_only_reads_new_or_dirty_demands_and_carries_without_allocation() {
        let mut bytes = super::super::production_rest_test_kit::fixture();
        bytes.resize(0x15000, 0);
        let put = |bytes: &mut [u8], address: usize, value: &[u8]| {
            bytes[address - 0x1000..address - 0x1000 + value.len()].copy_from_slice(value);
        };
        let accumulator = 0x3000 + ccountry::MODIFIER_ACCUMULATOR;
        put(&mut bytes, accumulator, &0x13500_u64.to_le_bytes());
        put(&mut bytes, accumulator + 8, &1_u32.to_le_bytes());
        put(&mut bytes, accumulator + 12, &1_u32.to_le_bytes());
        put(&mut bytes, 0x13500, &1_u32.to_le_bytes());
        put(&mut bytes, 0x13508, &2375_i64.to_le_bytes());
        let world = World {
            bytes,
            reads: Cell::new(0),
        };
        let mut cache = Cache::default();
        cache.replace(1, 2, vec![1, 2, 500]).unwrap();
        let first = cache.refresh(&world, 0).unwrap().unwrap();
        let values = first.rows::<tables::DesignCountryModifiers>().unwrap();
        assert_eq!(
            values.iter().map(|row| row.value).collect::<Vec<_>>(),
            [Some(2375), Some(0), Some(0)]
        );
        assert!(values.iter().all(|row| row.failure == 0));
        let reads = world.reads.get();
        for _ in 0..48 {
            let quiet =
                collector_test_support::with_allocations_denied(|| cache.refresh(&world, 0))
                    .unwrap()
                    .unwrap();
            assert!(Arc::ptr_eq(&first, &quiet));
        }
        assert_eq!(world.reads.get(), reads);
        cache.replace(2, 2, vec![1, 2, 500]).unwrap();
        cache.remove(1).unwrap();
        assert!(Arc::ptr_eq(
            &first,
            &cache.refresh(&world, 0).unwrap().unwrap()
        ));
        assert_eq!(world.reads.get(), reads);
        cache.remove(2).unwrap();
        assert_eq!(cache.refresh(&world, 0).unwrap().unwrap().len(), 0);
        assert_eq!(world.reads.get(), reads);
    }
    #[test]
    fn variant_country_context_demand_sizes_from_owners_and_drops_retired_ones() {
        let mut cache = Cache::default();
        for id in 1..=10 {
            cache.replace(id, id as u32, vec![1, 2]).unwrap();
        }
        assert_eq!(cache.countries.len(), 10);
        assert_eq!(cache.dirty.len(), 10);
        cache.remove(5).unwrap();
        assert_eq!(cache.countries.len(), 9);
        assert!(!cache.dirty.contains_key(&5));
    }

    #[test]
    fn variant_country_context_tracks_only_reached_upgrade_currency_and_revises_daily_demand() {
        let mut bytes = vec![0; 0x2000];
        let modifier = engine_native::cequipment_type::DESIGN_COST_MODIFIER;
        bytes[modifier..modifier + 4].copy_from_slice(&777_u32.to_le_bytes());
        let currency = 0x1000 + engine_native::cequipment_upgrade::COST_CURRENCY;
        bytes[currency..currency + 4].copy_from_slice(&2_u32.to_le_bytes());
        let world = World {
            bytes,
            reads: Cell::new(0),
        };
        let definition = cequipment_variant::DefinitionRow {
            identity: cequipment_variant::Row {
                id: 70,
                creator_link: 5_000,
                ..Default::default()
            },
            archetype_pointer: 0x1000,
            ..Default::default()
        };
        let upgrade = cequipment_upgrades_instance::Row {
            definition_token: 91,
            definition_pointer: 0x2000,
            ..Default::default()
        };
        let mut cache = Cache::default();
        cache
            .track(&world, &definition, &[], Some(&upgrade), None)
            .unwrap();
        assert_eq!(
            cache.designs[&70].keys,
            [
                CONVERSION_SPEED_FACTOR,
                COMMON_COST_FACTOR,
                AIR_COST_FACTOR,
                777
            ]
        );
        let reads = world.reads.get();
        cache
            .track(&world, &definition, &[], Some(&upgrade), None)
            .unwrap();
        assert_eq!(
            world.reads.get(),
            reads,
            "immutable currency reuses its authenticated first visit"
        );
        cache.track(&world, &definition, &[], None, None).unwrap();
        assert_eq!(
            cache.designs[&70].keys,
            [
                CONVERSION_SPEED_FACTOR,
                COMMON_COST_FACTOR,
                TANK_COST_FACTOR,
                777
            ]
        );
        assert!(!cache.countries[&5_000].contains_key(&AIR_COST_FACTOR));
        let mut naval = definition.clone();
        naval.identity.classifier_flags = engine_native::cequipment_type::SHIP_MASK;
        cache.track(&world, &naval, &[], None, None).unwrap();
        assert!(cache.countries[&5_000].contains_key(&REFIT_IC_FACTOR));
        assert!(!cache.countries[&5_000].contains_key(&CONVERSION_SPEED_FACTOR));
        let mut air = definition.clone();
        air.identity.classifier_flags = engine_native::cequipment_type::PLANE_MASK
            & !engine_native::cequipment_type::SHIP_MASK
            & !engine_native::cequipment_type::TANK_MASK;
        cache.track(&world, &air, &[], None, None).unwrap();
        assert!(cache.countries[&5_000].contains_key(&AIR_EXPERIENCE_TRAINING_FACTOR));
        cache.track(&world, &definition, &[], None, None).unwrap();
        assert!(!cache.countries[&5_000].contains_key(&AIR_EXPERIENCE_TRAINING_FACTOR));
        let mut with_organisation = definition.clone();
        Arc::make_mut(&mut with_organisation.design).design_team = 90;
        cache
            .track(&world, &with_organisation, &[], None, Some(4_999))
            .unwrap();
        assert!(cache.countries[&4_999].contains_key(&ORGANISATION_CHANGE_FACTOR));
        assert!(!cache.countries[&5_000].contains_key(&ORGANISATION_CHANGE_FACTOR));
        assert!(!cache.template_demands.contains_key(&4_999));
        assert!(!cache.template_dirty.contains_key(&4_999));
        cache.track(&world, &definition, &[], None, None).unwrap();
        assert!(!cache.countries.contains_key(&4_999));
    }

    #[test]
    fn variant_country_context_routes_owner_marks_to_demanded_raw_aliases_only() {
        let mut cache = Cache::default();
        cache.replace(1, 99, vec![1]).unwrap();
        cache.replace(2, 2, vec![1]).unwrap();
        cache.dirty.clear();
        cache.owners.bind(99, 2).unwrap();
        cache
            .owners
            .mark(&cache.countries, &mut cache.dirty, 2)
            .unwrap();
        assert_eq!(cache.dirty.len(), 2);
        assert_eq!(cache.dirty.get(&99), Some(&true));
        assert_eq!(cache.dirty.get(&2), Some(&true));
        cache.dirty.clear();
        cache
            .owners
            .mark(&cache.countries, &mut cache.dirty, 99)
            .unwrap();
        assert!(cache.dirty.is_empty());
        cache.owners.bind(99, 3).unwrap();
        cache
            .owners
            .mark(&cache.countries, &mut cache.dirty, 2)
            .unwrap();
        assert_eq!(cache.dirty.len(), 1);
        assert_eq!(cache.dirty.get(&2), Some(&true));
        cache.dirty.clear();
        cache
            .owners
            .mark(&cache.countries, &mut cache.dirty, 3)
            .unwrap();
        assert_eq!(cache.dirty.get(&99), Some(&true));
        cache.remove(1).unwrap();
        assert!(cache.owners.aliases.is_empty());
        assert!(cache.owners.reverse.is_empty());
    }
    #[test]
    fn unchanged_fresh_variant_context_reuses_key_scratch_and_existing_demand() {
        use super::super::production_rest_test_kit as kit;
        let bytes = kit::fixture();
        let memory = engine_native::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let definition = cequipment_variant::DefinitionRow {
            identity: cequipment_variant::Row {
                id: 70,
                creator_link: 2,
                archetype_token: 3,
                ..Default::default()
            },
            archetype_pointer: kit::EQUIPMENT_TYPE,
            design: Arc::new(rows::VariantDesignState::default()),
            ..Default::default()
        };
        let mut cache = Cache::default();
        cache.track(&memory, &definition, &[], None, None).unwrap();
        cache.track(&memory, &definition, &[], None, None).unwrap();
        let demand = cache.designs[&70].keys.as_ptr();
        let scratch = cache.spare_keys.as_ptr();
        collector_test_support::with_allocations_denied(|| {
            cache.track(&memory, &definition, &[], None, None)
        })
        .unwrap();
        assert_eq!(cache.designs[&70].keys.as_ptr(), demand);
        assert_eq!(cache.spare_keys.as_ptr(), scratch);
    }
}

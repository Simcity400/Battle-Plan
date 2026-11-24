use crate::table_capture::{grow_capacity, grow_vec};
use engine_allocation::{copy_vec, default_vec, filled, owned_vec};
use engine_fuel::fuel_demand as native;
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::corders_group as group;
use replay_data::{rows, rows::Recorded};
use std::{collections::HashSet, io, mem::size_of};

const N: usize = 12;
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
}

collector_primitives::capacity_max!(Capacities { rows });
impl Capacities {
    pub fn raise(&mut self, rows: [usize; N]) {
        for (available, required) in self.rows.iter_mut().zip(rows) {
            crate::country_scale::raise(available, required);
        }
    }
    pub fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.rows[0], slots);
    }
}

pub struct Captured {
    countries: Vec<native::Country>,
    divisions: Vec<native::CachedUnit>,
    fleets: Vec<native::Fleet>,
    tasks: Vec<native::CachedUnit>,
    sites: Vec<native::Site>,
    wings: Vec<native::Wing>,
    weather: Vec<native::Weather>,
    contexts: Vec<native::Context>,
    context_slots: Vec<usize>,
    names: Vec<u8>,
    roots: Vec<group::Root>,
    groups: Vec<group::GroupRow>,
    members: Vec<group::MemberRow>,
    children: Vec<group::ChildRow>,
    scratch: Vec<group::RosterEntry>,
    group_names: Vec<native::Name>,
    group_countries: Vec<Option<u32>>,
    group_owner_reason: Option<&'static str>,
    lengths: [usize; N],
    required: [usize; N],
    root_outcome: Option<Outcome>,
    group_outcomes: Option<[Outcome; 3]>,
    hour: Option<i32>,
    reason: Option<&'static str>,
    navy_reason: Option<&'static str>,
    daily_released: bool,
    last_capacity: Capacities,
}
impl Captured {
    pub fn with_capacities(capacity: &Capacities) -> Self {
        let [
            countries,
            divisions,
            fleets,
            tasks,
            sites,
            wings,
            weather,
            contexts,
            names,
            groups,
            members,
            children,
        ] = capacity.rows;
        Self {
            countries: owned_vec(default_vec(countries)),
            divisions: owned_vec(default_vec(divisions)),
            fleets: owned_vec(default_vec(fleets)),
            tasks: owned_vec(default_vec(tasks)),
            sites: owned_vec(default_vec(sites)),
            wings: owned_vec(default_vec(wings)),
            weather: owned_vec(default_vec(weather)),
            contexts: owned_vec(default_vec(contexts)),
            context_slots: owned_vec(context_slots(contexts).and_then(|slots| filled(slots, 0))),
            names: owned_vec(filled(names, 0)),
            roots: owned_vec(default_vec(groups)),
            groups: owned_vec(default_vec(groups)),
            members: owned_vec(default_vec(members)),
            children: owned_vec(default_vec(children)),
            scratch: owned_vec(default_vec(groups)),
            group_names: owned_vec(default_vec(groups)),
            group_countries: owned_vec(filled(groups, None)),
            group_owner_reason: None,
            lengths: [0; N],
            required: [0; N],
            root_outcome: None,
            group_outcomes: None,
            hour: None,
            reason: None,
            navy_reason: None,
            daily_released: false,
            last_capacity: *capacity,
        }
    }
    pub fn reset(&mut self) {
        self.lengths = [0; N];
        self.required = [0; N];
        self.root_outcome = None;
        self.group_outcomes = None;
        self.hour = None;
        self.reason = None;
        self.navy_reason = None;
        self.group_owner_reason = None;
    }
    pub fn fail_capture(&mut self, hour: i32, reason: &'static str) {
        self.reset();
        self.hour = Some(hour);
        self.reason = Some(reason);
    }
    pub fn allocated_bytes(&self) -> usize {
        self.countries.capacity() * size_of::<native::Country>()
            + self.divisions.capacity() * size_of::<native::CachedUnit>()
            + self.fleets.capacity() * size_of::<native::Fleet>()
            + self.tasks.capacity() * size_of::<native::CachedUnit>()
            + self.sites.capacity() * size_of::<native::Site>()
            + self.wings.capacity() * size_of::<native::Wing>()
            + self.weather.capacity() * size_of::<native::Weather>()
            + self.contexts.capacity() * size_of::<native::Context>()
            + self.context_slots.capacity() * size_of::<usize>()
            + self.names.capacity()
            + self.roots.capacity() * size_of::<group::Root>()
            + self.groups.capacity() * size_of::<group::GroupRow>()
            + self.members.capacity() * size_of::<group::MemberRow>()
            + self.children.capacity() * size_of::<group::ChildRow>()
            + self.scratch.capacity() * size_of::<group::RosterEntry>()
            + self.group_names.capacity() * size_of::<native::Name>()
            + self.group_countries.capacity() * size_of::<Option<u32>>()
    }
    pub fn release_daily(&mut self) {
        let current = [
            self.countries.len(),
            self.divisions.len(),
            self.fleets.len(),
            self.tasks.len(),
            self.sites.len(),
            self.wings.len(),
            self.weather.len(),
            self.contexts.len(),
            self.names.len(),
            self.groups.len(),
            self.members.len(),
            self.children.len(),
        ];
        for (saved, len) in self.last_capacity.rows.iter_mut().zip(current) {
            *saved = (*saved).max(len);
        }
        self.countries = Vec::new();
        self.divisions = Vec::new();
        self.fleets = Vec::new();
        self.tasks = Vec::new();
        self.sites = Vec::new();
        self.wings = Vec::new();
        self.weather = Vec::new();
        self.contexts = Vec::new();
        self.context_slots = Vec::new();
        self.names = Vec::new();
        self.roots = Vec::new();
        self.groups = Vec::new();
        self.members = Vec::new();
        self.children = Vec::new();
        self.scratch = Vec::new();
        self.group_names = Vec::new();
        self.group_countries = Vec::new();
        self.daily_released = true;
    }
    fn allocate_rows(&mut self, rows: [usize; N]) {
        let [
            countries,
            divisions,
            fleets,
            tasks,
            sites,
            wings,
            weather,
            contexts,
            names,
            groups,
            members,
            children,
        ] = rows;
        grow_vec(&mut self.countries, countries);
        grow_vec(&mut self.divisions, divisions);
        grow_vec(&mut self.fleets, fleets);
        grow_vec(&mut self.tasks, tasks);
        grow_vec(&mut self.sites, sites);
        grow_vec(&mut self.wings, wings);
        grow_vec(&mut self.weather, weather);
        grow_vec(&mut self.contexts, contexts);
        match context_slots(contexts) {
            Ok(slots) => {
                grow_vec(&mut self.context_slots, slots);
            }
            Err(error) => {
                let _ = owned_vec::<usize>(Err(error));
            }
        }
        grow_vec(&mut self.names, names);
        grow_vec(&mut self.roots, groups);
        grow_vec(&mut self.groups, groups);
        grow_vec(&mut self.members, members);
        grow_vec(&mut self.children, children);
        grow_vec(&mut self.scratch, groups);
        grow_vec(&mut self.group_names, groups);
        grow_vec(&mut self.group_countries, groups);
    }
    pub fn rehydrate_daily(&mut self) {
        if self.daily_released {
            self.daily_released = false;
            self.allocate_rows(self.last_capacity.rows);
        }
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        for (available, required) in capacity.rows.iter_mut().zip(self.required) {
            grow_capacity(available, required);
        }
        self.last_capacity = *capacity;
        if self.daily_released {
            return;
        }
        self.allocate_rows(capacity.rows);
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize, hour: i32) {
        self.capture_buffers(memory, base, state, hour);
    }
    fn capture_buffers(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
    ) {
        self.reset();
        self.rehydrate_daily();
        self.hour = Some(hour);
        self.context_slots.fill(0);
        let mut out = native::Buffers {
            countries: rows(&mut self.countries),
            divisions: rows(&mut self.divisions),
            fleets: rows(&mut self.fleets),
            tasks: rows(&mut self.tasks),
            sites: rows(&mut self.sites),
            wings: rows(&mut self.wings),
            weather: rows(&mut self.weather),
            contexts: rows(&mut self.contexts),
            context_slots: engine_native_memory::walk::Buffer::Growing(&mut self.context_slots),
            names: engine_native_memory::walk::Buffer::Growing(&mut self.names),
            name_len: 0,
            name_required: 0,
            roots: engine_native_memory::walk::Buffer::Growing(&mut self.roots),
            groups: engine_native_memory::walk::Buffer::Growing(&mut self.groups),
            members: engine_native_memory::walk::Buffer::Growing(&mut self.members),
            children: engine_native_memory::walk::Buffer::Growing(&mut self.children),
            roster_scratch: engine_native_memory::walk::Buffer::Growing(&mut self.scratch),
            group_names: engine_native_memory::walk::Buffer::Growing(&mut self.group_names),
            group_countries: engine_native_memory::walk::Buffer::Growing(&mut self.group_countries),
            group_owner_reason: None,
            group_outcomes: None,
            roots_outcome: None,
            navy_reason: None,
        };
        self.reason = native::capture(memory, base, state, &mut out).err();
        self.navy_reason = out.navy_reason;
        self.root_outcome = out.roots_outcome;
        self.group_outcomes = out.group_outcomes;
        self.group_owner_reason = out.group_owner_reason;
        self.lengths = [
            out.countries.len,
            out.divisions.len,
            out.fleets.len,
            out.tasks.len,
            out.sites.len,
            out.wings.len,
            out.weather.len,
            out.contexts.len,
            out.name_len,
            out.group_outcomes.map_or(0, |values| values[0].len()),
            out.group_outcomes.map_or(0, |values| values[1].len()),
            out.group_outcomes.map_or(0, |values| values[2].len()),
        ];
        self.required = [
            out.countries.required,
            out.divisions.required,
            out.fleets.required,
            out.tasks.required,
            out.sites.required,
            out.wings.required,
            out.weather.required,
            out.contexts.required,
            out.name_required,
            out.roots_outcome
                .map_or(0, demand)
                .max(out.group_outcomes.map_or(0, |values| demand(values[0]))),
            out.group_outcomes.map_or(0, |values| demand(values[1])),
            out.group_outcomes.map_or(0, |values| demand(values[2])),
        ];
    }

    pub fn owned(&self) -> io::Result<Vec<(u32, rows::FuelDemandRead)>> {
        let mut result = Vec::new();
        engine_allocation::reserve_exact(&mut result, self.lengths[0])?;
        let fleets = index(&self.fleets[..self.lengths[2]], |row| row.country)?;
        let tasks = index(&self.tasks[..self.lengths[3]], |row| row.country)?;
        let groups = index(&self.group_countries[..self.lengths[9]], |country| *country)?;
        let group_refs = index(&self.groups[..self.lengths[9]], |group| group.handle)?;
        let members = index(&self.members[..self.lengths[10]], |row| row.group)?;
        let children = index(&self.children[..self.lengths[11]], |row| row.group)?;
        let mut context_sums = engine_allocation::vec(self.lengths[7])?;
        for context in &self.contexts[..self.lengths[7]] {
            let sums = if context.reason.is_some() {
                None
            } else {
                Some(
                    native::context_sums(
                        &context.counts,
                        slice(&self.weather[..self.lengths[6]], context.weather)?,
                    )
                    .map_err(io::Error::other)?,
                )
            };
            context_sums.push(sums);
        }
        let group_reason = self
            .root_outcome
            .and_then(outcome_reason)
            .or_else(|| {
                self.group_outcomes
                    .and_then(|values| values.into_iter().find_map(outcome_reason))
            })
            .or(self.group_owner_reason);
        for country in &self.countries[..self.lengths[0]] {
            let mut pack = rows::FuelDemandRead {
                hour: pack_hour(self.hour),
                country_instance: country.instance,
                reason: reason(self.reason)?,
                army_reason: reason(country.army_reason)?,
                navy_reason: reason(self.navy_reason)?,
                air_reason: reason(country.air_reason)?,
                group_reason: reason(group_reason)?,
                ..Default::default()
            };
            for row in slice(&self.divisions[..self.lengths[1]], country.divisions)? {
                push(&mut pack.divisions, self.unit(row)?)?;
            }
            for (_, index) in keyed(&fleets, country.country) {
                let fleet = &self.fleets[*index];
                let (name, name_reason) = self.name(fleet.name)?;
                push(
                    &mut pack.fleets,
                    rows::FuelDemandFleet {
                        reference: fleet.reference,
                        name,
                        name_reason,
                    },
                )?;
            }
            for (_, index) in keyed(&tasks, country.country) {
                push(&mut pack.task_forces, self.unit(&self.tasks[*index])?)?;
            }
            let mut selected = Vec::new();
            let mut seen = HashSet::new();
            let mut missing = false;
            for (_, index) in keyed(&groups, Some(country.country)) {
                select_group(
                    &group_refs,
                    self.groups[*index].handle,
                    &mut selected,
                    &mut seen,
                    &mut missing,
                )?;
            }
            for division in &pack.divisions {
                if let Some(reference) = division.group {
                    select_group(
                        &group_refs,
                        reference,
                        &mut selected,
                        &mut seen,
                        &mut missing,
                    )?;
                }
            }
            let mut next = 0;
            while next < selected.len() {
                let parent = self.groups[selected[next]].parent;
                select_group(&group_refs, parent, &mut selected, &mut seen, &mut missing)?;
                next += 1;
            }
            if missing && pack.group_reason.is_empty() {
                pack.group_reason = reason(Some(
                    "fuel demand referenced command or ancestor was not captured",
                ))?;
            }
            selected.sort_unstable();
            for index in selected {
                let group = &self.groups[index];
                let (name, name_reason) = self.name(self.group_names[index])?;
                let mut owned = rows::FuelDemandGroup {
                    reference: group.handle,
                    country: group.country,
                    parent: group.parent,
                    army_group: group.is_army_group,
                    name,
                    name_reason,
                    ..Default::default()
                };
                for (_, index) in keyed(&members, group.handle) {
                    push(&mut owned.members, self.members[*index].unit)?;
                }
                for (_, index) in keyed(&children, group.handle) {
                    push(&mut owned.children, self.children[*index].child)?;
                }
                push(&mut pack.groups, owned)?;
            }
            let mut contexts = Vec::<(usize, u32)>::new();
            for site in slice(&self.sites[..self.lengths[4]], country.sites)? {
                let mut owned = rows::FuelDemandSite {
                    list: site.list,
                    ordinal: site.ordinal,
                    base: site.base,
                    container_owner: site.container_owner,
                    capacity_raw: site.capacity,
                    demilitarized: site.demilitarized,
                    reason: reason(site.reason)?,
                    ..Default::default()
                };
                for wing in slice(&self.wings[..self.lengths[5]], site.wings)? {
                    let context = match wing.context {
                        None => None,
                        Some(index) => {
                            let existing =
                                contexts.binary_search_by_key(&index, |&(index, _)| index);
                            let mapped = match existing {
                                Ok(position) => contexts[position].1,
                                Err(position) => {
                                    let context = self.contexts[..self.lengths[7]]
                                        .get(index)
                                        .ok_or_else(|| {
                                            io::Error::other(
                                                "fuel demand context outside owned arena",
                                            )
                                        })?;
                                    let mapped = u32::try_from(pack.contexts.len())
                                        .map_err(|_| engine_allocation::oom())?;
                                    push(
                                        &mut pack.contexts,
                                        rows::FuelDemandContext {
                                            region: context.region,
                                            sums: context_sums[index].ok_or_else(|| {
                                                io::Error::other(
                                                    "fuel demand refused context was referenced",
                                                )
                                            })?,
                                        },
                                    )?;
                                    engine_allocation::reserve(&mut contexts, 1)?;
                                    contexts.insert(position, (index, mapped));
                                    mapped
                                }
                            };
                            Some(mapped)
                        }
                    };
                    let (name, name_reason) = self.name(wing.name)?;
                    push(
                        &mut owned.wings,
                        rows::FuelDemandWing {
                            reference: wing.reference,
                            owner: wing.owner,
                            country_override: wing.country_override,
                            owner_reason: reason(wing.owner_reason)?,
                            pool_ordinal: wing.pool_ordinal,
                            wing_ordinal: wing.wing_ordinal,
                            name,
                            name_reason,
                            inputs: wing.inputs,
                            context,
                            reason: reason(wing.reason)?,
                        },
                    )?;
                }
                push(&mut pack.sites, owned)?;
            }
            result.push((country.country, pack));
        }
        result.sort_unstable_by_key(|(country, _)| *country);
        Ok(result)
    }
    fn name(&self, name: native::Name) -> io::Result<(Option<Vec<u8>>, Vec<u8>)> {
        Ok((
            name.bytes
                .map(|span| copy_vec(slice(&self.names[..self.lengths[8]], span)?))
                .transpose()?,
            reason(name.reason)?,
        ))
    }
    fn unit(&self, row: &native::CachedUnit) -> io::Result<rows::FuelDemandUnit> {
        let (name, name_reason) = self.name(row.name)?;
        Ok(rows::FuelDemandUnit {
            reference: row.reference,
            parent: row.parent,
            ordinal: row.ordinal,
            owners: row.owners,
            rate_raw: row.rate,
            reason: reason(row.reason)?,
            name,
            name_reason,
            group: row.group,
            group_reason: reason(row.group_reason)?,
        })
    }
}

fn context_slots(count: usize) -> io::Result<usize> {
    if count == 0 {
        Ok(0)
    } else {
        count
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .ok_or_else(engine_allocation::oom)
    }
}
fn rows<T>(rows: &mut Vec<T>) -> native::BufferRows<'_, T> {
    native::BufferRows {
        rows: engine_native_memory::walk::Buffer::Growing(rows),
        len: 0,
        required: 0,
    }
}
fn demand(outcome: Outcome) -> usize {
    match outcome {
        Outcome::Truncated { len, unread_rows } => len.saturating_add(unread_rows as usize),
        _ => outcome.len(),
    }
}
fn outcome_reason(outcome: Outcome) -> Option<&'static str> {
    match outcome {
        Outcome::Unavailable { reason } => Some(reason),
        Outcome::Truncated { .. } => Some("fuel demand command topology truncated"),
        _ => None,
    }
}
fn reason(reason: Option<&'static str>) -> io::Result<Vec<u8>> {
    copy_vec(reason.unwrap_or("").as_bytes())
}
fn slice<T>(values: &[T], span: native::Span) -> io::Result<&[T]> {
    span.start
        .checked_add(span.len)
        .and_then(|end| values.get(span.start..end))
        .ok_or_else(|| io::Error::other("fuel demand span outside owned arena"))
}
fn push<T>(values: &mut Vec<T>, value: T) -> io::Result<()> {
    engine_allocation::reserve(values, 1)?;
    values.push(value);
    Ok(())
}
fn index<T, K: Ord>(values: &[T], key: impl Fn(&T) -> K) -> io::Result<Vec<(K, usize)>> {
    let mut result = engine_allocation::vec(values.len())?;
    result.extend(
        values
            .iter()
            .enumerate()
            .map(|(index, row)| (key(row), index)),
    );
    result.sort_unstable();
    Ok(result)
}
fn keyed<K: Ord + Copy>(values: &[(K, usize)], key: K) -> &[(K, usize)] {
    let start = values.partition_point(|(value, _)| *value < key);
    let end = values.partition_point(|(value, _)| *value <= key);
    &values[start..end]
}
fn select_group(
    groups: &[(u64, usize)],
    reference: u64,
    selected: &mut Vec<usize>,
    seen: &mut HashSet<u64>,
    missing: &mut bool,
) -> io::Result<()> {
    if reference == 0 || seen.contains(&reference) {
        return Ok(());
    }
    engine_allocation::reserve_set(seen, 1)?;
    seen.insert(reference);
    match groups.binary_search_by_key(&reference, |&(reference, _)| reference) {
        Ok(index) => push(selected, groups[index].1)?,
        Err(_) => *missing = true,
    }
    Ok(())
}

fn pack_hour(hour: Option<i32>) -> rows::FuelDemandHour {
    match hour {
        Some(_) => rows::FuelDemandHour::Host,
        None => rows::FuelDemandHour::Stamped(0),
    }
}

const TRUNCATED_PARTS: &str =
    "fuel demand parts are not split from a truncated CountryFuel read, whose packs stay whole";

pub fn attach(
    table: &mut replay_frame::frame::TableInput,
    captured: &Captured,
) -> io::Result<replay_frame::frame::TableView> {
    use replay_data::rows::TableRows;
    use replay_frame::{frame::TableInput, frame::TableView};
    let (rows, complete) = match table {
        TableInput::Sampled(TableView::Complete(TableRows::CountryFuel(rows))) => (rows, true),
        TableInput::Sampled(TableView::Truncated {
            rows: TableRows::CountryFuel(rows),
            ..
        }) => (rows, false),
        TableInput::Sampled(TableView::Unavailable { reason }) => {
            return Ok(TableView::Unavailable {
                reason: reason.clone(),
            });
        }
        _ => {
            return Ok(TableView::Unavailable {
                reason: "fuel demand parts follow a CountryFuel read".into(),
            });
        }
    };
    let mut parts = Vec::new();
    let (mut owned, decode_reason) = match captured.owned() {
        Ok(rows) => (rows, None),
        Err(error) if error.kind() == io::ErrorKind::OutOfMemory => return Err(error),
        Err(_) => (
            Vec::new(),
            Some("fuel current demand owned closure invalid"),
        ),
    };
    for row in rows {
        let pack = owned
            .binary_search_by_key(&row.country, |(country, _)| *country)
            .ok()
            .map(|index| std::mem::take(&mut owned[index].1));
        let pack = pack.unwrap_or(rows::FuelDemandRead {
            hour: pack_hour(captured.hour),
            country_instance: match &row.country_instance {
                Recorded::Value(value) => *value,
                _ => None,
            },
            reason: reason(
                decode_reason
                    .or(captured.reason)
                    .or(Some("fuel current demand country was not captured")),
            )?,
            ..Default::default()
        });
        if !(complete && pack.splittable()) {
            row.demand = Recorded::Value(pack);
            continue;
        }
        let (header, mut owned_parts) = pack.split(row.country)?;
        engine_allocation::reserve(&mut parts, owned_parts.len())?;
        parts.append(&mut owned_parts);
        row.demand = Recorded::Value(header);
    }
    if !complete {
        return Ok(TableView::Unavailable {
            reason: TRUNCATED_PARTS.into(),
        });
    }
    parts.sort_unstable_by_key(replay_data::rows::RowCodec::key);
    Ok(TableView::Complete(TableRows::FuelDemandParts(parts)))
}

#[cfg(test)]
mod tests;

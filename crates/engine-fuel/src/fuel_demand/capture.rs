use super::*;
use engine_native::{carmy, ccountry, cfleet, corders_group, cunit};
use engine_native_memory::{
    cpdx_array::{self, Array},
    msvc_string,
    walk::{Buffer, Outcome},
};
use engine_native_parts::creference_registry;

pub struct Rows<'a, T> {
    pub rows: Buffer<'a, T>,
    pub len: usize,
    pub required: usize,
}
impl<T: Default> Rows<'_, T> {
    pub(super) fn push(&mut self, value: T) -> Result<(), &'static str> {
        self.required = self.required.max(
            self.len
                .checked_add(1)
                .ok_or("fuel demand row count overflow")?,
        );
        self.rows.ensure(self.required)?;
        let destination = self
            .rows
            .get_mut(self.len)
            .ok_or("fuel demand row arena capacity exceeded")?;
        *destination = value;
        self.len += 1;
        Ok(())
    }
    pub(super) fn span(&self, start: usize) -> Span {
        Span {
            start,
            len: self.len - start,
        }
    }
    pub(super) fn demand(&mut self, count: u32) -> Result<(), &'static str> {
        self.required = self
            .required
            .checked_add(count as usize)
            .ok_or("fuel demand row count overflow")?;
        self.rows.ensure(self.required)?;
        Ok(())
    }
}
pub struct Buffers<'a> {
    pub countries: Rows<'a, Country>,
    pub divisions: Rows<'a, CachedUnit>,
    pub fleets: Rows<'a, Fleet>,
    pub tasks: Rows<'a, CachedUnit>,
    pub sites: Rows<'a, Site>,
    pub wings: Rows<'a, Wing>,
    pub weather: Rows<'a, Weather>,
    pub contexts: Rows<'a, Context>,
    pub context_slots: Buffer<'a, usize>,
    pub names: Buffer<'a, u8>,
    pub name_len: usize,
    pub name_required: usize,
    pub roots: Buffer<'a, corders_group::Root>,
    pub groups: Buffer<'a, corders_group::GroupRow>,
    pub members: Buffer<'a, corders_group::MemberRow>,
    pub children: Buffer<'a, corders_group::ChildRow>,
    pub roster_scratch: Buffer<'a, corders_group::RosterEntry>,
    pub group_names: Buffer<'a, Name>,
    pub group_countries: Buffer<'a, Option<u32>>,
    pub group_owner_reason: Option<&'static str>,
    pub group_outcomes: Option<[Outcome; 3]>,
    pub roots_outcome: Option<Outcome>,
    pub navy_reason: Option<&'static str>,
}
pub use Rows as BufferRows;

pub(super) fn name(
    memory: &impl Memory,
    address: Result<usize, &'static str>,
    out: &mut Buffers<'_>,
) -> Name {
    let value = (|| {
        let text = msvc_string::Descriptor::read(memory, address?)?;
        out.name_required = out
            .name_required
            .checked_add(text.len)
            .ok_or("fuel demand name bytes overflow")?;
        let end = out
            .name_len
            .checked_add(text.len)
            .ok_or("fuel demand name span overflow")?;
        out.names.ensure(end)?;
        text.copy(
            memory,
            out.names
                .get_mut(out.name_len..end)
                .ok_or("fuel demand name arena capacity exceeded")?,
        )?;
        let bytes = Span {
            start: out.name_len,
            len: text.len,
        };
        out.name_len = end;
        Ok(bytes)
    })();
    match value {
        Ok(bytes) => Name {
            bytes: Some(bytes),
            reason: None,
        },
        Err(reason) => Name {
            bytes: None,
            reason: Some(reason),
        },
    }
}

pub(super) fn identity(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    vtable: Rva,
    offset: usize,
    displacement: usize,
) -> Result<u64, &'static str> {
    if pointer == 0 || field::<u64>(memory, pointer, 0)? != module_at(base, vtable)? as u64 {
        return Err("fuel demand object identity mismatch");
    }
    let reference = field::<u64>(memory, pointer, offset)?;
    if reference == 0
        || creference_registry::resolve(memory, base, reference)?
            != Some(at(pointer, displacement)?)
    {
        return Err("fuel demand registry identity mismatch");
    }
    Ok(reference)
}

fn owners(memory: &impl Memory, address: usize) -> Option<[u32; 3]> {
    read::<[u8; 12]>(memory, address).map(|bytes| {
        std::array::from_fn(|index| {
            u32::from_le_bytes(std::array::from_fn(|byte| bytes[index * 4 + byte]))
        })
    })
}

fn divisions(
    memory: &impl Memory,
    base: usize,
    country: engine_native_memory::country_identity::Country,
    out: &mut Buffers<'_>,
) -> Result<(), &'static str> {
    let array = Array::read(memory, at(country.pointer, ccountry::DIVISIONS)?, 8)
        .ok_or("fuel demand division roster unreadable or invalid")?;
    out.divisions.demand(array.count)?;
    let mut registry = creference_registry::HeaderResolver::default();
    cpdx_array::pointers(
        memory,
        array,
        "fuel demand division pointer unreadable",
        |ordinal, pointer| {
            if pointer == 0 {
                return Err("fuel demand native division contributor is null");
            }
            let (reference, class) = cunit::counter_identity(memory, base, pointer, &mut registry)?;
            if class != 0 {
                return Err("fuel demand division roster contains another unit class");
            }
            let rate = field(memory, pointer, 0x628);
            let group = (|| {
                let group = field::<u64>(memory, pointer, carmy::ORDERS_GROUP)? as usize;
                if group == 0 {
                    Ok(0)
                } else {
                    let table = field::<u64>(memory, group, 0)?;
                    let expected = if field::<u8>(memory, group, corders_group::IS_ARMY_GROUP)? != 0
                    {
                        engine_native_parts::carmy_group::VTABLE_RVA
                    } else {
                        corders_group::VTABLE_RVA
                    };
                    if table != module_at(base, expected)? as u64 {
                        return Err("fuel demand orders-group vtable mismatch");
                    }
                    identity(memory, base, group, expected, corders_group::HANDLE, 0)
                }
            })();
            let stored = field::<u64>(memory, pointer, 0x340).and_then(|member| {
                if member == 0 {
                    Err("fuel demand division name member is null")
                } else {
                    at(member as usize, 0x60)
                }
            });
            let stored = name(memory, stored, out);
            out.divisions.push(CachedUnit {
                country: country.id,
                reference,
                ordinal,
                owners: owners(memory, at(pointer, cunit::ROSTER_OWNER)?),
                rate: rate.as_ref().ok().copied(),
                reason: rate.err(),
                name: stored,
                group: group.as_ref().ok().copied(),
                group_reason: group.err(),
                ..Default::default()
            })
        },
    )
}

fn topology<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    out: &mut Buffers<'_>,
) {
    let roots = corders_group::walk_roots_buffer(memory, base, out.roots.suffix(0));
    out.roots_outcome = Some(roots);
    if !matches!(roots, Outcome::Rows { .. }) {
        return;
    }
    let count = roots.len();
    let mut graph = corders_group::OrderOfBattleStorage {
        groups: out.groups.suffix(0),
        members: out.members.suffix(0),
        children: out.children.suffix(0),
        roster_scratch: out.roster_scratch.suffix(0),
    };
    let acquired =
        corders_group::walk_order_of_battle_buffers(memory, base, &out.roots[..count], &mut graph);
    out.group_outcomes = Some([acquired.groups, acquired.members, acquired.children]);
    if let Err(reason) = out
        .group_names
        .ensure(acquired.groups.len())
        .and_then(|_| out.group_countries.ensure(acquired.groups.len()))
    {
        out.group_owner_reason = Some(reason);
        return;
    }
    for index in 0..acquired.groups.len().min(out.group_names.len()) {
        let root = out.roots[index];
        let owner = if root.country == 0 {
            Ok(0)
        } else {
            countries.resolve(root.country).map(|country| country.id)
        };
        out.group_countries[index] = owner.as_ref().ok().copied();
        if let Err(reason) = owner {
            out.group_owner_reason.get_or_insert(reason);
        }
        let copied = corders_group::copy_name_buffer(
            memory,
            root.pointer(),
            out.names.suffix(0),
            &mut out.name_len,
            &mut out.name_required,
        );
        out.group_names[index] = match copied {
            Ok((start, len)) => Name {
                bytes: Some(Span { start, len }),
                reason: None,
            },
            Err(reason) => Name {
                bytes: None,
                reason: Some(reason),
            },
        };
    }
}

pub fn capture(
    memory: &impl Memory,
    base: usize,
    state: usize,
    out: &mut Buffers<'_>,
) -> Result<(), &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    out.countries.demand(countries.count().saturating_sub(1))?;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let begin = out.divisions.len;
        let army_reason = divisions(memory, base, country, out).err();
        let divisions = out.divisions.span(begin);
        let begin = out.sites.len;
        let air_reason = super::air::country(memory, base, state, &countries, country, out).err();
        let sites = out.sites.span(begin);
        out.countries.push(Country {
            country: country.id,
            instance: read(memory, at(country.pointer, ccountry::SURRENDER_TARGET)?),
            divisions,
            army_reason,
            sites,
            air_reason,
        })?;
    }
    out.navy_reason = cfleet::visit_fuel_task_forces(memory, base, &countries, |visit| {
        match visit {
            cfleet::Visit::Fleet {
                country,
                fleet,
                handle,
                tasks,
            } => {
                out.fleets.demand(1)?;
                out.tasks.demand(tasks.count)?;
                let stored = name(memory, at(fleet, cfleet::NAME), out);
                out.fleets.push(Fleet {
                    country: country.id,
                    reference: handle,
                    name: stored,
                })
            }
            cfleet::Visit::TaskForce(task, ordinal) => {
                let rate = field(memory, task.task, 0x500);
                let stored = name(memory, at(task.task, 0x258), out);
                out.tasks.push(CachedUnit {
                    country: task.country.id,
                    reference: task.id,
                    parent: task.fleet_handle,
                    ordinal,
                    owners: owners(memory, at(task.task, cunit::ROSTER_OWNER)?),
                    rate: rate.as_ref().ok().copied(),
                    reason: rate.err(),
                    name: stored,
                    ..Default::default()
                })
            }
        }
    })
    .err();
    topology(memory, base, &countries, out);
    Ok(())
}

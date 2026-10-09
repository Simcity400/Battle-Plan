use super::*;
use crate::{
    carmy, ccountry, cfleet, corders_group,
    cpdx_array::{self, Array},
    creference_registry, cunit, msvc_string,
    walk::Outcome,
};

/// Caller-owned admitted arrays. Only the collector's off-thread grow path
/// resizes them; a short array retains demand and a named refusal.
pub struct Rows<'a, T> {
    pub rows: &'a mut [T],
    pub len: usize,
    pub required: usize,
}
impl<T> Rows<'_, T> {
    pub(super) fn push(&mut self, value: T) -> Result<(), &'static str> {
        self.required = self.required.max(
            self.len
                .checked_add(1)
                .ok_or("fuel demand row count overflow")?,
        );
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
    /// Pack-local open-addressed index; caller clears it before acquisition.
    pub context_slots: &'a mut [usize],
    pub names: &'a mut [u8],
    pub name_len: usize,
    pub name_required: usize,
    pub roots: &'a mut [corders_group::Root],
    pub groups: &'a mut [corders_group::GroupRow],
    pub members: &'a mut [corders_group::MemberRow],
    pub children: &'a mut [corders_group::ChildRow],
    pub roster_scratch: &'a mut [corders_group::RosterEntry],
    pub group_names: &'a mut [Name],
    pub group_countries: &'a mut [Option<u32>],
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
    vtable: usize,
    offset: usize,
    displacement: usize,
) -> Result<u64, &'static str> {
    if pointer == 0 || field::<u64>(memory, pointer, 0)? != at(base, vtable)? as u64 {
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
    country: ccountry::Country,
    out: &mut Buffers<'_>,
) -> Result<(), &'static str> {
    let array = Array::read(memory, at(country.pointer, ccountry::DIVISIONS)?, 8)
        .ok_or("fuel demand division roster unreadable or invalid")?;
    out.divisions.demand(array.count)?;
    // This is the shared native CUnit identity primitive, not the health or
    // movement census: unreadable unrelated stats cannot hide a known cache.
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
            let rate = field(memory, pointer, 0x628); // Unit_GetFuelValue140C77090, no refresh.
            let group = (|| {
                let group = field::<u64>(memory, pointer, carmy::ORDERS_GROUP)? as usize;
                if group == 0 {
                    Ok(0)
                } else {
                    let table = field::<u64>(memory, group, 0)?;
                    let expected = if field::<u8>(memory, group, corders_group::IS_ARMY_GROUP)? != 0
                    {
                        crate::carmy_group::VTABLE_RVA
                    } else {
                        corders_group::VTABLE_RVA
                    };
                    if table != at(base, expected)? as u64 {
                        return Err("fuel demand orders-group vtable mismatch");
                    }
                    identity(memory, base, group, expected, corders_group::HANDLE, 0)
                }
            })();
            // GetName140C72F00 returns army+340 member+60 without generation.
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
    let roots = corders_group::walk_roots(memory, base, out.roots);
    out.roots_outcome = Some(roots);
    if !matches!(roots, Outcome::Rows { .. }) {
        return;
    }
    let count = roots.len();
    let mut graph = corders_group::OrderOfBattle {
        groups: out.groups,
        members: out.members,
        children: out.children,
        roster_scratch: out.roster_scratch,
    };
    let acquired =
        corders_group::walk_order_of_battle(memory, base, &out.roots[..count], &mut graph);
    out.group_outcomes = Some([acquired.groups, acquired.members, acquired.children]);
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
        // Shared LAND stored-name accessor; no separate command-name capture.
        let copied = corders_group::copy_name(
            memory,
            root.pointer(),
            out.names,
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
            instance: read(memory, at(country.pointer, ccountry::ID_INSTANCE)?),
            divisions,
            army_reason,
            sites,
            air_reason,
        })?;
    }
    // All missions, including reserve/hold/training. Shared traversal's native
    // identity checks are retained; no NavalMissionInputs filter is applied.
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
                let stored = name(memory, at(fleet, cfleet::NAME), out); // Shared LAND getter140D41680.
                out.fleets.push(Fleet {
                    country: country.id,
                    reference: handle,
                    name: stored,
                })
            }
            cfleet::Visit::TaskForce(task, ordinal) => {
                let rate = field(memory, task.task, 0x500); // GetCurrentFuelUsage140D5A290.
                let stored = name(memory, at(task.task, 0x258), out); // GetName140BEDEB0.
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

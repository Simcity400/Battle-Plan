use super::*;
use engine_sites::Rva;

const GROUP_VTABLE: Rva = crate::ctechnology_sharing_group::VTABLE_RVA;
pub(super) const FACTION_VTABLE: Rva = Rva(0x02ac_66b8);
#[derive(Clone)]
struct Group {
    id: u32,
    base: i64,
    bonus: i64,
    members: Array,
    member_pointers: bool,
    filters: Array,
    name: Text,
}
impl Default for Group {
    fn default() -> Self {
        Self {
            id: 0,
            base: 0,
            bonus: 0,
            members: Array { data: 0, count: 0 },
            member_pointers: false,
            filters: Array { data: 0, count: 0 },
            name: Text::default(),
        }
    }
}
pub(super) struct Country {
    groups: Vec<Group>,
    len: usize,
}
impl Country {
    pub(super) fn record_roster(&self, out: &mut Output<'_>) -> Result<(), &'static str> {
        out.push(
            Kind::SharingRoster,
            -1,
            0,
            self.len as u32,
            0,
            Text::default(),
        )?;
        for group in &self.groups[..self.len] {
            out.value(Kind::SharingRoster, i64::from(group.id))?;
        }
        Ok(())
    }
    pub(super) fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub(super) fn read<M: Memory>(
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        country: engine_native_memory::country_identity::Country,
        status: usize,
    ) -> Result<Self, &'static str> {
        let roster = array_at(memory, at(status, 0x118)?, 8)?;
        let mut result = Self {
            groups: crate::allocation::default_vec(roster.count as usize)
                .map_err(|_| "research sharing group allocation failed")?,
            len: 0,
        };
        pointer_pages(memory, roster, |p| {
            let bytes = read::<[u8; 72]>(memory, p).ok_or("research sharing group unreadable")?;
            let vtable = u64::from_le_bytes(bytes[..8].try_into().unwrap());
            let is_group = match usize::try_from(vtable).ok().and_then(|vtable| {
                engine_sites::which(base, vtable, &[GROUP_VTABLE, FACTION_VTABLE])
            }) {
                Some(GROUP_VTABLE) => true,
                Some(_) => false,
                None => return Err("research sharing vtable unsupported"),
            };
            let members = array(&bytes[24..40], 4)?;
            let bonuses = array(&bytes[48..64], 8)?;
            if members.count != bonuses.count {
                return Err("research sharing members invalid");
            }
            let mut group = Group {
                id: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
                base: i64_at(&bytes, 16),
                members,
                ..Group::default()
            };
            references(
                memory,
                members,
                |raw| {
                    Ok(raw == country.id
                        || countries.resolve(raw)?.canonical_slot == country.canonical_slot)
                },
                |index| {
                    group.bonus = read::<i64>(
                        memory,
                        bonuses
                            .element(index, 8)
                            .ok_or("research sharing bonus overflow")?,
                    )
                    .ok_or("research sharing bonus unreadable")?;
                    Ok(())
                },
            )?;
            if is_group {
                let template = pointer(memory, at(p, 0x48)?)?;
                if read::<u64>(memory, template) != Some(site(base, Rva(0x029c_11e0))? as u64) {
                    return Err("research sharing template identity mismatch");
                }
                let faction = read::<u8>(memory, at(template, 0x78)?)
                    .ok_or("research sharing faction mode unreadable")?;
                group.filters = array_at(memory, at(template, 0x80)?, 40)?;
                group.name = text(memory, at(template, 0x10)?)?;
                if faction != 0 {
                    let diplomacy =
                        pointer(memory, at(country.pointer, ccountry::DIPLOMACY_STATUS)?)?;
                    let faction =
                        read::<u64>(memory, at(diplomacy, engine_native_parts::cdiplomacy_status_layout::FACTION)?)
                            .ok_or("research sharing faction unreadable")?;
                    group.members = if faction == 0 {
                        Array { data: 0, count: 0 }
                    } else {
                        let faction = usize::try_from(faction)
                            .map_err(|_| "research faction address overflow")?;
                        if read::<u64>(memory, faction)
                            != Some(site(base, engine_native::cfaction::VTABLE_RVA)? as u64)
                        {
                            return Err("research sharing faction identity mismatch");
                        }
                        array_at(memory, at(faction, engine_native::cfaction::MEMBERS)?, 8)?
                    };
                    group.member_pointers = true;
                }
            }
            result.groups[result.len] = group;
            result.len += 1;
            Ok(())
        })?;
        Ok(result)
    }

    pub(super) fn capture<M: Memory>(
        &self,
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        input: &research_inputs::Row,
        memo: &mut Witnesses,
        out: &mut Output<'_>,
    ) -> Result<(), &'static str> {
        if self.len == 0 {
            return Ok(());
        }
        let doctrine = read::<u8>(memory, at(input.template_pointer, 0x409)?)
            .ok_or("research sharing doctrine flag unreadable")?;
        out.value(Kind::SharingDoctrine, i64::from(doctrine))?;
        if doctrine != 0 {
            return Ok(());
        }
        capture_categories(
            memory,
            array_at(memory, at(input.template_pointer, 0x2c0)?, 40)?,
            Kind::SharingCategory,
            u32::MAX,
            out,
        )?;
        for group in &self.groups[..self.len] {
            let ordinal = out.current.ordinal;
            out.push(
                Kind::Sharing,
                group.base,
                group.bonus,
                group.members.count,
                group.id,
                group.name.clone(),
            )?;
            capture_categories(memory, group.filters, Kind::SharingFilter, ordinal, out)?;
            if group.member_pointers {
                pointer_pages(memory, group.members, |p| {
                    let raw = read::<u32>(memory, at(p, 8)?)
                        .ok_or("research sharing faction member unreadable")?;
                    memo.capture(memory, base, countries, raw, input, ordinal, out)
                })?;
            } else {
                references(
                    memory,
                    group.members,
                    |raw| {
                        memo.capture(memory, base, countries, raw, input, ordinal, out)?;
                        Ok(false)
                    },
                    |_| Ok(()),
                )?;
            }
        }
        Ok(())
    }
}

pub(super) fn record_roster<M: Memory>(
    memory: &M,
    base: usize,
    status: usize,
    out: &mut Output<'_>,
) -> Result<(), &'static str> {
    let roster = array_at(memory, at(status, 0x118)?, 8)?;
    out.push(Kind::SharingRoster, -1, 0, roster.count, 0, Text::default())?;
    pointer_pages(memory, roster, |pointer| {
        let bytes = read::<[u8; 12]>(memory, pointer)
            .ok_or("scripted sharing group identity unreadable")?;
        let vtable = u64::from_le_bytes(bytes[..8].try_into().unwrap());
        if usize::try_from(vtable)
            .ok()
            .and_then(|vtable| engine_sites::which(base, vtable, &[GROUP_VTABLE, FACTION_VTABLE]))
            .is_none()
        {
            return Err("scripted sharing group identity unsupported");
        }
        out.value(
            Kind::SharingRoster,
            i64::from(u32::from_le_bytes(bytes[8..].try_into().unwrap())),
        )
    })
}

fn references(
    memory: &impl Memory,
    source: Array,
    mut visit: impl FnMut(u32) -> Result<bool, &'static str>,
    mut found: impl FnMut(u32) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let mut bytes = [0; 256];
    let mut first = 0;
    while first < source.count {
        let count = (source.count - first).min(64);
        if !read_slice(
            memory,
            source
                .element(first, 4)
                .ok_or("research sharing member address overflow")?,
            &mut bytes[..count as usize * 4],
        ) {
            return Err("research sharing members unreadable");
        }
        for (offset, b) in bytes[..count as usize * 4].chunks_exact(4).enumerate() {
            if visit(u32::from_le_bytes(b.try_into().unwrap()))? {
                return found(first + offset as u32);
            }
        }
        first += count;
    }
    Ok(())
}

fn capture_categories(
    memory: &impl Memory,
    categories: Array,
    kind: Kind,
    owner: u32,
    out: &mut Output<'_>,
) -> Result<(), &'static str> {
    for index in 0..categories.count {
        let p = categories
            .element(index, 40)
            .ok_or("research sharing category overflow")?;
        let hash =
            read::<u32>(memory, at(p, 32)?).ok_or("research sharing category hash unreadable")?;
        out.push(
            kind,
            0,
            0,
            owner,
            hash,
            text(memory, p)?,
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Witness {
    level: i32,
    present: bool,
    predecessors: Array,
}
#[derive(Default)]
pub(super) struct Witnesses {
    entries: std::collections::HashMap<(u32, usize), Witness>,
}
impl Default for Witness {
    fn default() -> Self {
        Self {
            level: 0,
            present: false,
            predecessors: Array { data: 0, count: 0 },
        }
    }
}
impl Witnesses {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn capture<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        raw: u32,
        input: &research_inputs::Row,
        group: u32,
        out: &mut Output<'_>,
    ) -> Result<(), &'static str> {
        let hit = self.entries.get(&(raw, input.template_pointer)).copied();
        let witness = if let Some(w) = hit {
            w
        } else {
            let country = countries.resolve(raw)?;
            let owner = crate::ctechnology_status::copy_status(memory, country.pointer)?;
            let status = crate::ctechnology_status::decode_status(base, country.pointer, &owner)?;
            let technologies = array_at(
                memory,
                at(status, crate::ctechnology_status::TECHNOLOGIES_DATA)?,
                8,
            )?;
            let index = input
                .template_index
                .checked_sub(1)
                .and_then(|i| u32::try_from(i).ok())
                .ok_or("research sharing template index invalid")?;
            let mut witness = Witness::default();
            if index < technologies.count {
                let p = read::<u64>(
                    memory,
                    technologies
                        .element(index, 8)
                        .ok_or("research sharing technology index overflow")?,
                )
                .ok_or("research sharing technology link unreadable")?;
                if p != 0 {
                    let p = usize::try_from(p)
                        .map_err(|_| "research sharing technology address overflow")?;
                    let mut bytes = [0; crate::ctechnology::INSTANCE_BYTES];
                    if crate::ctechnology::copy_one(memory, p, &mut bytes)
                        != crate::ctechnology::FIELDS.len()
                    {
                        return Err("research sharing technology unreadable");
                    }
                    let instance =
                        crate::ctechnology::decode_one(base, status, country.id, &bytes)?;
                    if instance.template != input.template_pointer
                        || instance.row.technology_handle != input.slot.technology_handle
                    {
                        return Err("research sharing technology identity mismatch");
                    }
                    witness.level = instance.row.level;
                    witness.present = true;
                    witness.predecessors = array_at(memory, at(p, 0xc8)?, 8)?;
                }
            }
            crate::allocation::reserve_map(&mut self.entries, 1)
                .map_err(|_| "research sharing witness allocation failed")?;
            self.entries.insert((raw, input.template_pointer), witness);
            witness
        };
        let ordinal = out.current.ordinal;
        out.push(
            if witness.present {
                Kind::SharingMember
            } else {
                Kind::SharingMissingMember
            },
            i64::from(witness.level),
            i64::from(input.template_max_level),
            group,
            raw,
            Text::default(),
        )?;
        pointer_pages(memory, witness.predecessors, |predecessor| {
            let classifier = read::<u32>(memory, at(predecessor, crate::ctechnology::CLASSIFIER)?)
                .ok_or("research sharing predecessor unreadable")?;
            out.push(
                Kind::SharingPredecessor,
                i64::from(classifier),
                0,
                ordinal,
                0,
                Text::default(),
            )
        })
    }
}

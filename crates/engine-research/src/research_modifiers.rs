use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    msvc_string::Descriptor,
    read,
    read_slice,
    walk::Outcome,
};
use engine_native::ccountry;
use crate::research_inputs::{self, Text};
use engine_sites::Rva;

pub mod grants;
mod sharing;
static SCRIPTED_SHARING_ROSTER: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn scripted_sharing_roster(enabled: bool) {
    SCRIPTED_SHARING_ROSTER.store(enabled, std::sync::atomic::Ordering::Relaxed);
}
#[cfg(test)]
mod tests;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Technology,
    CountrySpeed,
    CategorySpeed,
    DesignTeam,
    Experience,
    Sharing,
    License,
    Scientist,
    Year,
    AheadFactor,
    MinimumSpeed,
    CountrySource,
    IdeaSource,
    ScriptSource,
    ScientistSource,
    SharingMultiplier,
    SharingMaximum,
    ModifierMetadata,
    ScientistCategory,
    SharingMember,
    SharingPredecessor,
    SharingFilter,
    SharingCategory,
    SharingDoctrine,
    SharingMissingMember,
    GrantCounter,
    SpeedGrant,
    CostGrant,
    GrantUses,
    GrantTechnology,
    GrantCategory,
    SharingRoster,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub index: u32,
    pub ordinal: u32,
    pub technology_handle: u32,
    pub kind: Kind,
    pub raw: i64,
    pub auxiliary: i64,
    pub count: u32,
    pub source_id: u32,
    pub name: Text,
}

const DEFINES: [Rva; 4] = [
    Rva(0x033e_4658),
    Rva(0x033e_4328),
    Rva(0x033e_4280),
    Rva(0x033e_4138),
];
#[derive(Clone, Copy)]
pub struct Context {
    minimum: i64,
    license: i64,
    sharing_maximum: i64,
    ahead_factor: i64,
}
impl Context {
    pub fn read(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut bytes = [[0; 8]; 4];
        let mut addresses = [0; 4];
        for (address, rva) in addresses.iter_mut().zip(DEFINES) {
            *address = site(base, rva)?;
        }
        if memory.copy_scattered_prefetched(addresses, &mut bytes.each_mut().map(|v| &mut v[..]))
            != 4
        {
            return Err("research modifier defines unreadable");
        }
        let [minimum, license, sharing_maximum, ahead_factor] = bytes.map(i64::from_le_bytes);
        Ok(Self {
            minimum,
            license,
            sharing_maximum,
            ahead_factor,
        })
    }

    pub fn digest(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for value in [
            self.minimum,
            self.license,
            self.sharing_maximum,
            self.ahead_factor,
        ] {
            for byte in value.to_le_bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
            }
        }
        hash
    }
}

const TECHNOLOGY_FIELDS: [engine_native_memory::memory::RecordField; 4] =
    engine_native_memory::memory::packed_fields([(0x190, 24), (0x1e8, 12), (0x68, 16), (0x110, 64)]);
const TECHNOLOGY_LAYOUT: engine_native_memory::memory::RecordLayout<'static> =
    engine_native_memory::memory::RecordLayout::new(&TECHNOLOGY_FIELDS, 116).unwrap();
const IDEA_FIELDS: [engine_native_memory::memory::RecordField; 1] = engine_native_memory::memory::packed_fields([(0x38, 16)]);
const IDEA_LAYOUT: engine_native_memory::memory::RecordLayout<'static> =
    engine_native_memory::memory::RecordLayout::new(&IDEA_FIELDS, 16).unwrap();
const SCRIPT_FIELDS: [engine_native_memory::memory::RecordField; 1] = engine_native_memory::memory::packed_fields([(0x30, 8)]);
const SCRIPT_LAYOUT: engine_native_memory::memory::RecordLayout<'static> =
    engine_native_memory::memory::RecordLayout::new(&SCRIPT_FIELDS, 8).unwrap();

struct Output<'a> {
    rows: engine_native_memory::walk::Buffer<'a, Row>,
    len: usize,
    omitted: u32,
    current: Row,
}
impl Output<'_> {
    fn push(
        &mut self,
        kind: Kind,
        raw: i64,
        auxiliary: i64,
        count: u32,
        source_id: u32,
        name: Text,
    ) -> Result<(), &'static str> {
        let row = Row {
            kind,
            raw,
            auxiliary,
            count,
            source_id,
            name,
            ..self.current.clone()
        };
        self.current.ordinal = self
            .current
            .ordinal
            .checked_add(1)
            .ok_or("research modifier ordinal overflow")?;
        self.rows.ensure(
            self.len
                .checked_add(1)
                .ok_or("research modifier row count overflow")?,
        )?;
        if self.len < self.rows.len() {
            self.rows[self.len] = row;
            self.len += 1;
        } else {
            self.omitted = self
                .omitted
                .checked_add(1)
                .ok_or("research modifier count overflow")?;
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn modified(
        &mut self,
        kind: Kind,
        raw: i64,
        metadata: Option<engine_native_parts::modifier_definition::Operands>,
        count: u32,
        source_id: u32,
        name: Text,
    ) -> Result<(), &'static str> {
        let ordinal = self.current.ordinal;
        self.push(kind, raw, 0, count, source_id, name)?;
        if let Some(metadata) = metadata {
            self.push(
                Kind::ModifierMetadata,
                i64::from(metadata.flags),
                i64::from(metadata.eligibility_flags),
                metadata.selector,
                ordinal,
                Text::default(),
            )?;
        }
        Ok(())
    }
    fn value(&mut self, kind: Kind, raw: i64) -> Result<(), &'static str> {
        self.push(kind, raw, 0, 0, 0, Text::default())
    }
}

pub fn walk_country<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: engine_native_memory::country_identity::Country,
    context: Result<Context, &'static str>,
    inputs: &[research_inputs::Row],
    output: &mut Vec<Row>,
) -> Outcome {
    walk_country_buffer(
        memory,
        base,
        countries,
        country,
        context,
        inputs,
        engine_native_memory::walk::Buffer::Growing(output),
    )
}

pub fn walk_country_buffer<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: engine_native_memory::country_identity::Country,
    context: Result<Context, &'static str>,
    inputs: &[research_inputs::Row],
    output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    walk_country_buffer_with_roster(
        memory, base, countries, country, context, inputs, output, false,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keep the certified capture operands and caller-owned output explicit without new capture state."
)]
pub fn walk_country_buffer_certified<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: engine_native_memory::country_identity::Country,
    context: Result<Context, &'static str>,
    inputs: &[research_inputs::Row],
    output: engine_native_memory::walk::Buffer<'_, Row>,
    certificate_available: bool,
) -> Outcome {
    walk_country_buffer_with_roster(
        memory,
        base,
        countries,
        country,
        context,
        inputs,
        output,
        certificate_available && SCRIPTED_SHARING_ROSTER.load(std::sync::atomic::Ordering::Relaxed),
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keep roster demand separate from the acquired research inputs and caller-owned output."
)]
fn walk_country_buffer_with_roster<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: engine_native_memory::country_identity::Country,
    context: Result<Context, &'static str>,
    inputs: &[research_inputs::Row],
    output: engine_native_memory::walk::Buffer<'_, Row>,
    record_roster: bool,
) -> Outcome {
    let active = inputs.iter().any(|r| r.slot.technology_present != 0);
    if !active && !record_roster {
        return Outcome::Rows { len: 0 };
    }
    let mut out = Output {
        rows: output,
        len: 0,
        omitted: 0,
        current: Row {
            country: country.id,
            index: u32::MAX,
            ..Row::default()
        },
    };
    let result = (|| {
        if !active {
            let status = match inputs.first() {
                Some(input) if input.status_pointer != 0 => input.status_pointer,
                _ => {
                    let raw = crate::ctechnology_status::copy_status(memory, country.pointer)?;
                    crate::ctechnology_status::decode_status(base, country.pointer, &raw)?
                }
            };
            return sharing::record_roster(memory, base, status, &mut out);
        }
        let context = context?;
        let accumulator = at(country.pointer, engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)?;
        let (raw, metadata) = modifier(
            memory,
            base,
            accumulator,
            crate::modifier_keys::RESEARCH_SPEED_FACTOR,
        )?;
        out.modified(Kind::CountrySpeed, raw, metadata, 0, 0, Text::default())?;
        out.value(Kind::AheadFactor, context.ahead_factor)?;
        out.value(Kind::MinimumSpeed, context.minimum)?;
        out.value(Kind::SharingMaximum, context.sharing_maximum)?;
        country_sources(memory, base, accumulator, &mut out)?;

        let status = inputs
            .iter()
            .find(|r| r.slot.technology_present != 0)
            .unwrap()
            .status_pointer;
        let sharing = sharing::Country::read(memory, base, countries, country, status)?;
        if record_roster {
            sharing.record_roster(&mut out)?;
        }
        let (raw, metadata) = if sharing.is_empty() {
            (0, None)
        } else {
            modifier(
                memory,
                base,
                accumulator,
                crate::modifier_keys::RESEARCH_SHARING_PER_COUNTRY_BONUS_FACTOR,
            )?
        };
        out.modified(
            Kind::SharingMultiplier,
            raw,
            metadata,
            0,
            0,
            Text::default(),
        )?;
        let mut witnesses = sharing::Witnesses::default();
        let mut licenses = None;
        let mut scientist = None;
        let technology_pointers = crate::allocation::advisory(|| {
            let mut pointers = Vec::new();
            crate::allocation::reserve_exact(&mut pointers, inputs.len()).ok()?;
            pointers.extend(
                inputs
                    .iter()
                    .filter(|r| r.slot.technology_present != 0)
                    .map(|r| r.technology_pointer),
            );
            Some(pointers)
        })
        .unwrap_or_default();
        let technology_batch = crate::governance_batch::Records::gather(
            memory,
            engine_native_memory::batch_fallback::Batch::GovernanceModifierTechnologies,
            engine_native_memory::memory::RecordSources::Pointers(&technology_pointers),
            &TECHNOLOGY_LAYOUT,
        );
        let technology_memory = technology_batch.view(memory);
        for input in inputs.iter().filter(|r| r.slot.technology_present != 0) {
            if input.status_pointer != status {
                return Err("research modifier owner changed");
            }
            out.current = Row {
                country: country.id,
                index: input.slot.index,
                technology_handle: input.slot.technology_handle,
                ..Row::default()
            };
            out.value(Kind::Technology, i64::from(input.slot.technology_handle))?;
            out.value(Kind::Year, i64::from(input.template_year))?;
            let tech = input.technology_pointer;
            let mut cached = [0; 24];
            let mut xp = [0; 12];
            let mut equipment = [0; 16];
            let mut lists = [0; 64];
            if technology_memory.copy_scattered_prefetched(
                [
                    at(tech, 0x190)?,
                    at(tech, 0x1e8)?,
                    at(tech, 0x68)?,
                    at(tech, 0x110)?,
                ],
                &mut [
                    &mut cached[..],
                    &mut xp[..],
                    &mut equipment[..],
                    &mut lists[..],
                ],
            ) != 4
            {
                return Err("research modifier operands unreadable");
            }
            out.value(Kind::CategorySpeed, i64_at(&cached, 0))?;
            out.push(
                Kind::DesignTeam,
                i64_at(&cached, 16),
                i64_at(&xp, 4),
                u32::from_le_bytes(xp[4..8].try_into().unwrap()),
                u32::from_le_bytes(xp[8..12].try_into().unwrap()),
                Text::default(),
            )?;
            out.push(
                Kind::Experience,
                if xp[0] != 0 {
                    read::<i64>(memory, at(input.template_pointer, 0x538)?)
                        .ok_or("research XP modifier unreadable")?
                } else {
                    0
                },
                i64::from(xp[0]),
                0,
                0,
                Text::default(),
            )?;
            let ideas = array(&lists[..16], 8)?;
            let scripts = array(&lists[24..40], 40)?;
            let categories = array(&lists[48..64], 4)?;
            let idea_pointers = crate::governance_batch::Records::gather(
                memory,
                engine_native_memory::batch_fallback::Batch::GovernanceModifierIdeas,
                engine_native_memory::memory::RecordSources::Strided {
                    base: ideas.data,
                    stride: 8,
                    count: ideas.count as usize,
                },
                &crate::governance_batch::POINTER_LAYOUT,
            );
            let idea_targets = idea_pointers.targets(0);
            let idea_batch = crate::governance_batch::Records::gather(
                memory,
                engine_native_memory::batch_fallback::Batch::GovernanceModifierIdeas,
                engine_native_memory::memory::RecordSources::Pointers(&idea_targets),
                &IDEA_LAYOUT,
            );
            let idea_pointer_memory = idea_pointers.view(memory);
            let idea_memory = idea_batch.view(&idea_pointer_memory);
            pointer_pages(&idea_memory, ideas, |pointer| {
                let bytes = read::<[u8; 16]>(&idea_memory, at(pointer, 0x38)?)
                    .ok_or("research idea contribution unreadable")?;
                let raw = i64_at(&bytes, 0);
                if raw != 0 {
                    let idea = pointer_from(&bytes[8..])?;
                    if read::<u64>(&idea_memory, idea)
                        != Some(site(base, engine_native_parts::cidea::VTABLE_RVA)? as u64)
                    {
                        return Err("research idea identity mismatch");
                    }
                    out.push(
                        Kind::IdeaSource,
                        raw,
                        0,
                        0,
                        0,
                        text(&idea_memory, at(idea, engine_native_parts::cidea::TOKEN)?)?,
                    )?;
                }
                Ok(())
            })?;
            let script_pointers = crate::governance_batch::Records::gather(
                memory,
                engine_native_memory::batch_fallback::Batch::GovernanceModifierScripts,
                engine_native_memory::memory::RecordSources::Strided {
                    base: scripts.data,
                    stride: 40,
                    count: scripts.count as usize,
                },
                &crate::governance_batch::POINTER_LAYOUT,
            );
            let script_targets = script_pointers.targets(0);
            let script_batch = crate::governance_batch::Records::gather(
                memory,
                engine_native_memory::batch_fallback::Batch::GovernanceModifierScripts,
                engine_native_memory::memory::RecordSources::Pointers(&script_targets),
                &SCRIPT_LAYOUT,
            );
            let script_pointer_memory = script_pointers.view(memory);
            let script_memory = script_batch.view(&script_pointer_memory);
            for index in 0..scripts.count {
                let entry = scripts
                    .element(index, 40)
                    .ok_or("research script address overflow")?;
                let source = pointer(&script_memory, entry)?;
                let raw = read::<i64>(&script_memory, at(source, 0x30)?)
                    .ok_or("research script contribution unreadable")?;
                if raw != 0 {
                    out.push(
                        Kind::ScriptSource,
                        raw,
                        0,
                        0,
                        0,
                        text(&script_memory, at(entry, 8)?)?,
                    )?;
                }
            }
            let equipment = array(&equipment, 8)?;
            let license = if equipment.count == 0 {
                false
            } else {
                let roster = match licenses {
                    Some(roster) => roster,
                    None => {
                        let production =
                            pointer(memory, at(country.pointer, ccountry::PRODUCTION_STATUS)?)?;
                        let roster = array_at(memory, at(production, 0x1b0)?, 8)?;
                        licenses = Some(roster);
                        roster
                    }
                };
                licensed(memory, equipment, roster)?
            };
            out.value(Kind::License, if license { context.license } else { 0 })?;
            scientist_bonus(
                memory,
                base,
                country,
                countries,
                categories,
                &mut scientist,
                &mut out,
            )?;
            sharing.capture(memory, base, countries, input, &mut witnesses, &mut out)?;
        }
        Ok(())
    })();
    match result {
        Err(reason) => Outcome::Unavailable { reason },
        Ok(()) if out.omitted != 0 => Outcome::Truncated {
            len: out.len,
            unread_rows: out.omitted,
        },
        Ok(()) => Outcome::Rows { len: out.len },
    }
}

fn country_sources(
    memory: &impl Memory,
    base: usize,
    accumulator: usize,
    out: &mut Output<'_>,
) -> Result<(), &'static str> {
    struct Frame {
        accumulator: usize,
        children: Array,
        next: u32,
        bytes: [u8; 64 * 8],
    }
    fn frame(memory: &impl Memory, accumulator: usize) -> Result<Frame, &'static str> {
        Ok(Frame {
            accumulator,
            children: array_at(memory, at(accumulator, 0x18)?, 8)?,
            next: 0,
            bytes: [0; 64 * 8],
        })
    }
    let mut stack =
        crate::allocation::vec(1).map_err(|_| "research source stack allocation failed")?;
    stack.push(frame(memory, accumulator)?);
    while let Some(current) = stack.last_mut() {
        if current.children.count == 0 {
            let accumulator = current.accumulator;
            if stack.len() != 1 {
                let (raw, metadata) = modifier(
                    memory,
                    base,
                    accumulator,
                    crate::modifier_keys::RESEARCH_SPEED_FACTOR,
                )?;
                if raw != 0 {
                    out.modified(
                        Kind::CountrySource,
                        raw,
                        metadata,
                        0,
                        0,
                        text(memory, at(accumulator, 0x48)?)?,
                    )?;
                }
            }
            stack.pop();
            continue;
        }
        if current.next == current.children.count {
            stack.pop();
            continue;
        }
        let offset = current.next % 64;
        if offset == 0 {
            let count = (current.children.count - current.next).min(64);
            if !read_slice(
                memory,
                current
                    .children
                    .element(current.next, 8)
                    .ok_or("research pointer page overflow")?,
                &mut current.bytes[..count as usize * 8],
            ) {
                return Err("research modifier pointers unreadable");
            }
        }
        let child = at(
            pointer_from(&current.bytes[offset as usize * 8..offset as usize * 8 + 8])?,
            0x10,
        )?;
        current.next += 1;
        if stack.iter().any(|frame| frame.accumulator == child) {
            return Err("research modifier source cycle");
        }
        crate::allocation::reserve_push_batch(&mut stack, 1)
            .map_err(|_| "research source stack allocation failed")?;
        stack.push(frame(memory, child)?);
    }
    Ok(())
}

fn modifier(
    memory: &impl Memory,
    base: usize,
    accumulator: usize,
    key: u32,
) -> Result<(i64, Option<engine_native_parts::modifier_definition::Operands>), &'static str> {
    let entries = array_at(memory, accumulator, 16)?;
    let mut first = 0;
    let mut count = entries.count;
    while count != 0 {
        let half = count / 2;
        let index = first + half;
        let found = read::<u32>(
            memory,
            entries
                .element(index, 16)
                .ok_or("research modifier search overflow")?,
        )
        .ok_or("research modifier key unreadable")?;
        if found < key {
            first = index + 1;
            count -= half + 1;
        } else {
            count = half;
        }
    }
    if first == entries.count {
        return Ok((0, None));
    }
    let bytes = read::<[u8; 16]>(
        memory,
        entries
            .element(first, 16)
            .ok_or("research modifier search overflow")?,
    )
    .ok_or("research modifier value unreadable")?;
    if u32::from_le_bytes(bytes[..4].try_into().unwrap()) != key {
        return Ok((0, None));
    }
    let raw = i64_at(&bytes, 8);
    Ok((
        raw,
        Some(engine_native_parts::modifier_definition::operands(memory, base, key)?),
    ))
}

fn licensed(memory: &impl Memory, equipment: Array, licenses: Array) -> Result<bool, &'static str> {
    let mut found = false;
    pointer_pages(memory, equipment, |equipment| {
        if !found {
            pointer_pages(memory, licenses, |license| {
                if !found {
                    found = pointer(memory, at(license, 0x3f0)?)? == equipment;
                }
                Ok(())
            })?;
        }
        Ok(())
    })?;
    Ok(found)
}

#[derive(Clone, Copy)]
struct Scientist {
    data: usize,
    mask: u32,
    overflow: u8,
}
#[allow(clippy::too_many_arguments)]
fn scientist_bonus<M: Memory>(
    memory: &M,
    base: usize,
    country: engine_native_memory::country_identity::Country,
    countries: &ccountry::Reader<'_, M>,
    categories: Array,
    cached: &mut Option<Scientist>,
    out: &mut Output<'_>,
) -> Result<(), &'static str> {
    if categories.count == 0 {
        return out.value(Kind::Scientist, 0);
    }
    let header = match *cached {
        Some(v) => v,
        None => {
            let owner =
                crate::nproject_cprogram_status::from_country(memory, base, country, countries)?;
            let bytes = read::<[u8; 17]>(memory, at(owner, 0x88)?)
                .ok_or("research scientist map unreadable")?;
            let value = Scientist {
                data: pointer_from(&bytes[..8])?,
                mask: u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
                overflow: bytes[16],
            };
            if !value.mask.checked_add(1).is_some_and(u32::is_power_of_two) {
                return Err("research scientist map mask invalid");
            }
            *cached = Some(value);
            value
        }
    };
    let mut category_bytes = [0; 128 * 4];
    let mut first = 0;
    while first < categories.count {
        let count = (categories.count - first).min(128);
        if !read_slice(
            memory,
            categories
                .element(first, 4)
                .ok_or("research scientist category address overflow")?,
            &mut category_bytes[..count as usize * 4],
        ) {
            return Err("research scientist categories unreadable");
        }
        if first == 0 {
            out.value(Kind::Scientist, i64::from(categories.count))?;
        }
        for bytes in category_bytes[..count as usize * 4].chunks_exact(4) {
            let category = u32::from_le_bytes(bytes.try_into().unwrap());
            let mut hash = (category >> 16 ^ category).wrapping_mul(0x45d9f3b);
            hash = (hash >> 16 ^ hash).wrapping_mul(0x45d9f3b);
            hash = (hash >> 16 ^ hash) & header.mask;
            let mut found = false;
            for distance in 1..=u32::from(header.overflow) + 1 {
                let index = hash
                    .checked_add(distance - 1)
                    .ok_or("research scientist probe overflow")?;
                if index > header.mask + u32::from(header.overflow) {
                    break;
                }
                let bytes = read::<[u8; 48]>(memory, at(header.data, index as usize * 48)?)
                    .ok_or("research scientist bucket unreadable")?;
                if bytes[4] == 0 || (distance > 1 && distance > u32::from(bytes[4])) {
                    break;
                }
                if u32::from_le_bytes(bytes[8..12].try_into().unwrap()) == category {
                    let value = i64_at(&bytes, 16);
                    found = true;
                    out.push(
                        Kind::ScientistCategory,
                        value,
                        0,
                        0,
                        category,
                        Text::default(),
                    )?;
                    pointer_pages(memory, array(&bytes[24..40], 8)?, |source| {
                        let identity = read::<[u8; 16]>(memory, source)
                            .ok_or("research scientist identity unreadable")?;
                        if u64::from_le_bytes(identity[..8].try_into().unwrap())
                            != site(base, crate::cscientist::VTABLE_RVA)? as u64
                        {
                            return Err("research scientist identity mismatch");
                        }
                        let character = pointer_from(&identity[8..])?;
                        let identity = read::<[u8; 16]>(memory, character)
                            .ok_or("research scientist character unreadable")?;
                        if u64::from_le_bytes(identity[..8].try_into().unwrap())
                            != site(base, engine_native::ccharacter::VTABLE_RVA)? as u64
                            || u32::from_le_bytes(identity[8..12].try_into().unwrap())
                                != engine_native::ccharacter::REFERENCE_TYPE
                            || pointer(memory, at(character, engine_native::ccharacter::SCIENTIST)?)?
                                != source
                        {
                            return Err("research scientist character identity mismatch");
                        }
                        let id = u32::from_le_bytes(identity[12..16].try_into().unwrap());
                        let accumulator = at(source, 0x58)?;
                        let (raw, metadata) = modifier(
                            memory,
                            base,
                            accumulator,
                            crate::modifier_keys::SCIENTIST_RESEARCH_BONUS_FACTOR,
                        )?;
                        out.modified(
                            Kind::ScientistSource,
                            raw,
                            metadata,
                            category,
                            id,
                            text(memory, at(character, engine_native::ccharacter::TOKEN)?)?,
                        )
                    })?;
                    break;
                }
            }
            if !found {
                out.push(Kind::ScientistCategory, 0, 0, 0, category, Text::default())?;
            }
        }
        first += count;
    }
    Ok(())
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("research modifier address overflow")
}
fn site(base: usize, key: engine_sites::Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("research modifier address overflow")
}
fn pointer_from(bytes: &[u8]) -> Result<usize, &'static str> {
    usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
        .ok()
        .filter(|p| *p != 0)
        .ok_or("research modifier pointer invalid")
}
fn pointer(memory: &impl Memory, address: usize) -> Result<usize, &'static str> {
    pointer_from(&read::<[u8; 8]>(memory, address).ok_or("research modifier pointer unreadable")?)
}
fn array(bytes: &[u8], stride: usize) -> Result<Array, &'static str> {
    Array::from_header(bytes.try_into().unwrap(), stride).ok_or("research modifier array invalid")
}
fn array_at(memory: &impl Memory, address: usize, stride: usize) -> Result<Array, &'static str> {
    Array::read(memory, address, stride).ok_or("research modifier array unreadable")
}
fn i64_at(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
fn text(memory: &impl Memory, address: usize) -> Result<Text, &'static str> {
    Text::from_descriptor(memory, Descriptor::read(memory, address)?)
}
fn pointer_pages(
    memory: &impl Memory,
    array: Array,
    mut visit: impl FnMut(usize) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let mut bytes = [0; 64 * 8];
    let mut first = 0;
    while first < array.count {
        let count = (array.count - first).min(64);
        if !read_slice(
            memory,
            array
                .element(first, 8)
                .ok_or("research pointer page overflow")?,
            &mut bytes[..count as usize * 8],
        ) {
            return Err("research modifier pointers unreadable");
        }
        for p in bytes[..count as usize * 8].chunks_exact(8) {
            visit(pointer_from(p)?)?;
        }
        first += count;
    }
    Ok(())
}

#[cfg(test)]
mod serial;

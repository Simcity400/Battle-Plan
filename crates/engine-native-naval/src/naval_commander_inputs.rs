
use engine_native::cunit_leader::{CommanderLearningTrait, CommanderTrait};
use engine_native_memory::memory::Raw;
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native::{ccharacter, cunit_leader};
use engine_native_parts::{cnavy_leader, cunit_leader_skill, cunit_leader_trait};
use engine_sites::Rva;

pub const LEARNING_THRESHOLD_RVA: Rva = Rva(0x033e_4ce0);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CommanderPortraitModifier {
    pub source: u8,
    pub id: u32,
    pub value: i64,
    pub metadata_flags: u32,
    pub category_mask: u32,
    pub feature_selector: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CommanderPortraitTooltip {
    pub source: u8,
    pub bucket: u32,
    pub key: u32,
    pub text_start: usize,
    pub text_len: usize,
}

pub const SKILL_DATABASE_RVA: Rva = Rva(0x033d_d518);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PortraitEntry {
    pub role: u32,
    pub size: u32,
    pub is_set: u8,
    pub kind: u32,
    pub text_start: usize,
    pub text_len: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Presentation {
    pub character: u64,
    pub character_state: u8,
    pub character_country: u32,
    pub character_gender: u32,
    pub character_role_leader: u64,
    pub character_fallback_leader: u64,
    pub character_role_source: u8,
    pub fallback_text: [Vec<u8>; 3],
    pub fallback_text_len: [usize; 3],
    pub portraits: Vec<PortraitEntry>,
    pub portraits_len: usize,
    pub portrait_text: Vec<u8>,
    pub portrait_text_len: usize,
    pub role_traits: Vec<CommanderTrait>,
    pub role_traits_len: usize,
    pub role_trait_text: Vec<u8>,
    pub role_trait_text_len: usize,
}

impl Presentation {
    pub fn copied(&self) -> std::io::Result<Self> {
        use engine_allocation::copy_vec;
        let [first, second, third] = &self.fallback_text;
        Ok(Self {
            fallback_text: [copy_vec(first)?, copy_vec(second)?, copy_vec(third)?],
            portraits: copy_vec(&self.portraits)?,
            portrait_text: copy_vec(&self.portrait_text)?,
            role_traits: copy_vec(&self.role_traits)?,
            role_trait_text: copy_vec(&self.role_trait_text)?,
            ..*self
        })
    }

    pub fn allocated_bytes(&self) -> usize {
        self.fallback_text.iter().map(Vec::capacity).sum::<usize>()
            + self.portrait_text.capacity()
            + self.role_trait_text.capacity()
            + self.portraits.capacity() * size_of::<PortraitEntry>()
            + self.role_traits.capacity() * size_of::<CommanderTrait>()
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SkillDefinition {
    pub present: u8,
    pub valid: u8,
    pub level: i32,
    pub required_experience: i32,
    pub category: i32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Row {
    pub commander: u64,
    pub country: u32,
    pub category: i32,
    pub experience: i64,
    pub skills: [i32; 5],
    pub skill_definitions: [SkillDefinition; 5],
    pub next_skill: SkillDefinition,
    pub name: Vec<u8>,
    pub name_len: usize,
    pub description: Vec<u8>,
    pub description_len: usize,
    pub presentation: Presentation,
    pub traits: Vec<CommanderTrait>,
    pub traits_len: usize,
    pub trait_text: Vec<u8>,
    pub trait_text_len: usize,
    pub timed_traits: [usize; 2],
    pub learning_traits: Vec<CommanderLearningTrait>,
    pub learning_traits_len: usize,
    pub learning_text: Vec<u8>,
    pub learning_text_len: usize,
    pub learning_threshold: i64,
    pub modifiers: Vec<CommanderPortraitModifier>,
    pub modifiers_len: usize,
    pub modifier_feature_flags: u32,
    pub modifier_callback_kind: u8,
    pub tooltips: Vec<CommanderPortraitTooltip>,
    pub tooltips_len: usize,
    pub tooltip_text: Vec<u8>,
    pub tooltip_text_len: usize,
}

impl Row {
    pub fn allocated_bytes(&self) -> usize {
        self.name.capacity()
            + self.description.capacity()
            + self.trait_text.capacity()
            + self.learning_text.capacity()
            + self.tooltip_text.capacity()
            + self.presentation.allocated_bytes()
            + self.traits.capacity() * size_of::<CommanderTrait>()
            + self.learning_traits.capacity() * size_of::<CommanderLearningTrait>()
            + self.modifiers.capacity() * size_of::<CommanderPortraitModifier>()
            + self.tooltips.capacity() * size_of::<CommanderPortraitTooltip>()
    }
}

pub fn capture(
    memory: &impl Memory,
    base: usize,
    handle: u64,
    timed_traits: &mut Vec<cunit_leader::TimedTrait>,
    timed_len: &mut usize,
) -> Result<Row, &'static str> {
    if handle == 0 {
        return Ok(Row::default());
    }
    let r = Reader { memory, base };
    let pointer = engine_native_parts::creference_registry::resolve(memory, base, handle)?
        .ok_or("naval commander reference unresolved")?;
    r.vtable(pointer, cnavy_leader::VTABLE_RVA)?;
    if r.leader(pointer)? != handle {
        return Err("naval commander identity mismatch");
    }
    let mut row = Row {
        commander: handle,
        country: r.field(pointer, 0x120)?,
        category: r.field(pointer, cunit_leader::CATEGORY)?,
        experience: r.field(pointer, cunit_leader::EXPERIENCE)?,
        learning_threshold: r.field(r.global(LEARNING_THRESHOLD_RVA)?, 0)?,
        ..Row::default()
    };
    row.name_len = r.text(pointer, cunit_leader::NAME, &mut row.name)?;
    (row.modifier_feature_flags, row.modifier_callback_kind) = capture_modifier_gate(memory, base)?;
    row.description_len = r.text(pointer, 0x60, &mut row.description)?;
    row.presentation = capture_presentation(memory, base, pointer)?;
    for (index, offset) in [cunit_leader::SKILL, 0xf70, 0xf80, 0xf90, 0xfa0]
        .into_iter()
        .enumerate()
    {
        let definition = r.pointer(pointer, offset)?;
        row.skill_definitions[index] = r.skill(definition)?;
        if definition != 0 {
            r.accumulator(definition, 0x20, index as u8, &mut row)?;
        }
    }
    row.skills[0] = row.skill_definitions[0].level;
    for (index, offset) in [
        cnavy_leader::ATTACK,
        cnavy_leader::DEFENSE,
        cnavy_leader::MANEUVERING,
        cnavy_leader::COORDINATION,
    ]
    .into_iter()
    .enumerate()
    {
        row.skills[index + 1] = r.field(pointer, offset)?;
    }
    row.next_skill = r.next_skill(row.skills[0], row.category)?;
    (row.traits_len, row.trait_text_len) =
        r.traits(pointer, &mut row.traits, &mut row.trait_text)?;
    let timed = r.array(
        pointer,
        cunit_leader::TIMED_TRAIT_DATA,
        cunit_leader::TIMED_TRAIT_STRIDE,
    )?;
    let start = *timed_len;
    let end = start
        .checked_add(timed.count as usize)
        .ok_or("naval commander timed trait demand overflow")?;
    if timed_traits.len() < end {
        engine_allocation::resize_default(timed_traits, end)
            .map_err(|_| "collector allocation failed")?;
    }
    if let Err(reason) = cunit_leader::read_timed_traits(memory, base, timed, |value| {
        timed_traits[*timed_len] = value;
        *timed_len += 1;
    }) {
        *timed_len = start;
        return Err(reason);
    }
    row.timed_traits = [start, timed.count as usize];
    let learning = r.array(pointer, cunit_leader::TRAIT_EXPERIENCE_DATA, 16)?;
    engine_allocation::resize_default(&mut row.learning_traits, learning.count as usize)
        .map_err(|_| "commander learning allocation failed")?;
    row.learning_traits_len = learning.count as usize;
    for index in 0..learning.count {
        let entry = learning
            .element(index, 16)
            .ok_or("naval learning address overflow")?;
        let definition = r.pointer(entry, 0)?;
        r.vtable(definition, cunit_leader_trait::VTABLE_RVA)?;
        let start = row.learning_text_len;
        let len = r.text(
            definition,
            cunit_leader_trait::TOKEN,
            &mut row.learning_text,
        )?;
        row.learning_traits[index as usize] = CommanderLearningTrait {
            token_id: r.field(definition, cunit_leader_trait::TOKEN_ID)?,
            definition_ordinal: r.field(definition, cunit_leader_trait::ORDINAL)?,
            required_experience: r.field(definition, cunit_leader_trait::REQUIRED_EXPERIENCE)?,
            experience: r.field(entry, 8)?,
            token_start: start,
            token_len: len,
        };
        row.learning_text_len += len;
    }
    Ok(row)
}

pub fn capture_presentation(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Presentation, &'static str> {
    let r = Reader { memory, base };
    r.leader(pointer)?;
    let mut row = Presentation {
        character: r.field(pointer, cunit_leader::CHARACTER_TYPE)?,
        ..Presentation::default()
    };
    if row.character != 0 {
        row.character_state = 1;
        if let Some(character) = engine_native_parts::creference_registry::resolve(memory, base, row.character)? {
            r.vtable(character, ccharacter::VTABLE_RVA)?;
            if row.character as u32 != ccharacter::REFERENCE_TYPE
                || r.field::<u64>(character, ccharacter::TYPE)? != row.character
            {
                return Err("commander portrait character identity mismatch");
            }
            row.character_state = 2;
            row.character_country = r.field(character, ccharacter::COUNTRY)?;
            row.character_gender = r.field(character, 0x70)?;
            let live = (|| {
                let portraits = r.array(character, ccharacter::PORTRAIT_DATA, 0x38)?;
                engine_allocation::resize_default(&mut row.portraits, portraits.count as usize)
                    .map_err(|_| "commander portrait allocation failed")?;
                row.portraits_len = portraits.count as usize;
                for index in 0..portraits.count {
                    let entry = portraits
                        .element(index, 0x38)
                        .ok_or("commander portrait address overflow")?;
                    let start = row.portrait_text_len;
                    let len = r.text(entry, 0x10, &mut row.portrait_text)?;
                    row.portraits[index as usize] = PortraitEntry {
                        role: r.field(entry, 0)?,
                        size: r.field(entry, 4)?,
                        is_set: r.field(entry, 8)?,
                        kind: r.field(entry, 0xc)?,
                        text_start: start,
                        text_len: len,
                    };
                    row.portrait_text_len += len;
                }
                Ok(())
            })();
            let array = character.wrapping_add(ccharacter::PORTRAIT_DATA);
            engine_native::portrait_cache::settled(array, live, |image| {
                let count = engine_native_memory::cpdx_array::Array::from_header(image.header, 0x38)
                    .ok_or("commander array invalid")?
                    .count as usize;
                if count != image.entries().len() {
                    return Err("character portrait pre-image incomplete");
                }
                engine_allocation::resize_default(&mut row.portraits, count)
                    .map_err(|_| "commander portrait allocation failed")?;
                row.portraits_len = count;
                row.portrait_text.clear();
                row.portrait_text_len = 0;
                for (index, entry) in image.entries().iter().enumerate() {
                    let text = image.identity(index)?;
                    let start = row.portrait_text_len;
                    engine_allocation::resize(&mut row.portrait_text, start + text.len(), 0)
                        .map_err(|_| "commander text allocation failed")?;
                    row.portrait_text[start..].copy_from_slice(text);
                    let field =
                        |at: usize| u32::from_le_bytes(entry.head[at..at + 4].try_into().unwrap());
                    row.portraits[index] = PortraitEntry {
                        role: field(0),
                        size: field(4),
                        is_set: entry.head[8],
                        kind: field(0xc),
                        text_start: start,
                        text_len: text.len(),
                    };
                    row.portrait_text_len += text.len();
                }
                Ok(())
            })?;
            let direct = r.pointer(character, ccharacter::UNIT_LEADER)?;
            row.character_fallback_leader = r.field(character, 0xc8)?;
            let role = if direct != 0 {
                row.character_role_source = 1;
                Some(direct)
            } else {
                let role = engine_native_parts::creference_registry::resolve(
                    memory,
                    base,
                    row.character_fallback_leader,
                )?;
                if role.is_some() {
                    row.character_role_source = 2;
                }
                role
            };
            if let Some(role) = role {
                row.character_role_leader = r.leader(role)?;
                (row.role_traits_len, row.role_trait_text_len) =
                    r.traits(role, &mut row.role_traits, &mut row.role_trait_text)?;
            }
        }
    }
    if row.character_state != 2 {
        for (index, offset) in [0xc0, 0xe0, 0x100].into_iter().enumerate() {
            row.fallback_text_len[index] =
                r.text(pointer, offset, &mut row.fallback_text[index])?;
        }
    }
    Ok(row)
}

pub fn capture_next_skill(
    memory: &impl Memory,
    base: usize,
    level: i32,
    category: i32,
) -> Result<SkillDefinition, &'static str> {
    Reader { memory, base }.next_skill(level, category)
}

struct Reader<'a, M> {
    memory: &'a M,
    base: usize,
}
impl<M: Memory> Reader<'_, M> {
    fn field<T: Raw>(&self, p: usize, o: usize) -> Result<T, &'static str> {
        p.checked_add(o)
            .and_then(|a| read(self.memory, a))
            .ok_or("commander field unreadable or overflow")
    }
    fn pointer(&self, p: usize, o: usize) -> Result<usize, &'static str> {
        usize::try_from(self.field::<u64>(p, o)?).map_err(|_| "commander pointer overflow")
    }
    fn global(&self, key: Rva) -> Result<usize, &'static str> {
        engine_sites::at(self.base, key).ok_or("commander site withheld")
    }
    fn vtable(&self, p: usize, rva: Rva) -> Result<(), &'static str> {
        if engine_sites::at(self.base, rva).map(|v| v as u64) != Some(self.field(p, 0)?) {
            return Err("commander field unreadable or overflow");
        }
        Ok(())
    }
    fn leader(&self, p: usize) -> Result<u64, &'static str> {
        let vtable = self.field::<u64>(p, 0)?;
        if engine_sites::which(
            self.base,
            vtable as usize,
            &[
                cunit_leader::VTABLE_RVA,
                engine_native::carmy_leader::VTABLE_RVA,
                cnavy_leader::VTABLE_RVA,
                engine_native_parts::coperative_leader_layout::VTABLE_RVA,
            ],
        )
        .is_none()
        {
            return Err("commander leader vtable mismatch");
        }
        let handle = self.field::<u64>(p, cunit_leader::TYPE)?;
        if handle as u32 != cunit_leader::REFERENCE_TYPE || handle >> 32 == 0 {
            return Err("commander leader reference invalid");
        }
        Ok(handle)
    }
    fn array(&self, p: usize, o: usize, s: usize) -> Result<Array, &'static str> {
        p.checked_add(o)
            .and_then(|a| Array::read(self.memory, a, s))
            .ok_or("commander array invalid")
    }
    fn text(&self, p: usize, o: usize, output: &mut Vec<u8>) -> Result<usize, &'static str> {
        let a = p.checked_add(o).ok_or("commander text overflow")?;
        let h: [u8; 32] = self.field(a, 0)?;
        let len = usize::try_from(u64::from_le_bytes(h[16..24].try_into().unwrap()))
            .map_err(|_| "commander text length overflow")?;
        let cap = u64::from_le_bytes(h[24..].try_into().unwrap());
        if cap < 15 || len as u64 > cap || len > isize::MAX as usize {
            return Err("commander text header invalid");
        }
        let source = if cap == 15 {
            a
        } else {
            usize::try_from(u64::from_le_bytes(h[..8].try_into().unwrap()))
                .map_err(|_| "commander text pointer overflow")?
        };
        if source == 0 || self.field::<u8>(source, len)? != 0 {
            return Err("commander text unreadable");
        }
        let start = output.len();
        let end = start
            .checked_add(len)
            .ok_or("commander text length overflow")?;
        engine_allocation::resize(output, end, 0)
            .map_err(|_| "commander text allocation failed")?;
        if !engine_native_memory::read_slice(self.memory, source, &mut output[start..end]) {
            output.truncate(start);
            return Err("commander text unreadable");
        }
        Ok(len)
    }
    fn traits(
        &self,
        p: usize,
        output: &mut Vec<CommanderTrait>,
        text: &mut Vec<u8>,
    ) -> Result<(usize, usize), &'static str> {
        let a = self.array(p, cunit_leader::TRAIT_DATA, 8)?;
        engine_allocation::resize_default(output, a.count as usize)
            .map_err(|_| "commander trait allocation failed")?;
        let mut used = 0;
        for index in 0..a.count {
            let entry = a
                .element(index, 8)
                .ok_or("commander trait address overflow")?;
            let t = self.pointer(entry, 0)?;
            self.vtable(t, cunit_leader_trait::VTABLE_RVA)?;
            let len = self.text(t, cunit_leader_trait::TOKEN, text)?;
            output[index as usize] = CommanderTrait {
                token_id: self.field(t, cunit_leader_trait::TOKEN_ID)?,
                definition_ordinal: self.field(t, cunit_leader_trait::ORDINAL)?,
                kind: self.field(t, cunit_leader_trait::KIND)?,
                relevant: self.field(t, 0xa74)?,
                token_start: used,
                token_len: len,
            };
            used += len;
        }
        Ok((a.count as usize, used))
    }
    fn skill(&self, p: usize) -> Result<SkillDefinition, &'static str> {
        if p == 0 {
            return Ok(SkillDefinition::default());
        }
        self.vtable(p, cunit_leader_skill::VTABLE_RVA)?;
        Ok(SkillDefinition {
            present: 1,
            valid: self.field(p, cunit_leader_skill::VALID)?,
            level: self.field(p, cunit_leader_skill::LEVEL)?,
            required_experience: self.field(p, cunit_leader_skill::REQUIRED_EXPERIENCE)?,
            category: self.field(p, cunit_leader_skill::CATEGORY)?,
        })
    }
    fn next_skill(&self, level: i32, category: i32) -> Result<SkillDefinition, &'static str> {
        if !(0..4).contains(&category) {
            return Err("commander skill category invalid");
        }
        let db = self.pointer(self.global(SKILL_DATABASE_RVA)?, 0)?;
        let offset = 0xe8 + category as usize * 0x18;
        let a = self.array(db, offset, 8)?;
        if a.count == 0 {
            return Err("commander skill database bound invalid");
        }
        let next = level.wrapping_add(1);
        let index = if next > 0 && (next as u32) < a.count {
            next as u32
        } else {
            0
        };
        let p = self.pointer(
            a.element(index, 8)
                .ok_or("commander skill index overflow")?,
            0,
        )?;
        self.skill(p)
    }
    fn accumulator(
        &self,
        p: usize,
        offset: usize,
        source: u8,
        row: &mut Row,
    ) -> Result<(), &'static str> {
        let address = p
            .checked_add(offset)
            .ok_or("commander modifier address overflow")?;
        let a = self.array(address, 0, 16)?;
        let required = row
            .modifiers_len
            .checked_add(a.count as usize)
            .ok_or("commander modifier demand overflow")?;
        engine_allocation::resize_default(&mut row.modifiers, required)
            .map_err(|_| "commander modifier allocation failed")?;
        let mut previous = 0;
        for index in 0..a.count {
            let p = a
                .element(index, 16)
                .ok_or("commander modifier index overflow")?;
            let id = self.field(p, 0)?;
            if id == 0 || id <= previous {
                return Err("commander modifier keys invalid");
            }
            previous = id;
            let [metadata_flags, category_mask, feature_selector] =
                capture_modifier_metadata(self.memory, self.base, id)?;
            row.modifiers[row.modifiers_len] = CommanderPortraitModifier {
                source,
                id,
                value: self.field(p, 8)?,
                metadata_flags,
                category_mask,
                feature_selector,
            };
            row.modifiers_len += 1;
        }
        let h: [u8; 17] = self.field(address, 0x70)?;
        let data = usize::try_from(u64::from_le_bytes(h[..8].try_into().unwrap()))
            .map_err(|_| "commander tooltip pointer overflow")?;
        let count = u32::from_le_bytes(h[8..12].try_into().unwrap());
        let capacity = u32::from_le_bytes(h[12..16].try_into().unwrap());
        if count > capacity {
            return Err("commander tooltip capacity exceeded");
        }
        let required = row
            .tooltips_len
            .checked_add(count as usize)
            .ok_or("commander tooltip demand overflow")?;
        engine_allocation::resize_default(&mut row.tooltips, required)
            .map_err(|_| "commander tooltip allocation failed")?;
        if data == 0 {
            if count != 0 {
                return Err("commander live tooltip has null storage");
            }
        } else {
            let span = capacity
                .checked_add(h[16] as u32)
                .and_then(|n| n.checked_add(1))
                .ok_or("commander tooltip span overflow")?;
            let mut seen = 0;
            for bucket in 0..span {
                let entry = data
                    .checked_add(bucket as usize * 0x28)
                    .ok_or("commander tooltip address overflow")?;
                match self.field::<u8>(entry, 4)? {
                    0 => continue,
                    255 => return Err("commander tooltip premature sentinel"),
                    _ => {}
                }
                if seen == count {
                    return Err("commander tooltip occupied count mismatch");
                }
                seen += 1;
                let start = row.tooltip_text_len;
                let len = self.text(entry, 8, &mut row.tooltip_text)?;
                row.tooltips[row.tooltips_len] = CommanderPortraitTooltip {
                    source,
                    bucket,
                    key: self.field(entry, 0)?,
                    text_start: start,
                    text_len: len,
                };
                row.tooltips_len += 1;
                row.tooltip_text_len += len;
            }
            if seen != count {
                return Err("commander tooltip occupied count mismatch");
            }
        }
        Ok(())
    }
}

pub(crate) fn capture_modifier_metadata(
    memory: &impl Memory,
    base: usize,
    key: u32,
) -> Result<[u32; 3], &'static str> {
    use engine_native_parts::modifier_definition as md;
    let root = engine_sites::at(base, md::TABLE_RVA).ok_or("commander metadata root withheld")?;
    let table = read::<u64>(memory, root)
        .filter(|&p| p != 0)
        .ok_or("commander metadata root unreadable")?;
    let displacement = (key as i32 as isize)
        .checked_mul(md::STRIDE as isize)
        .ok_or("commander metadata index overflow")?;
    let address = usize::try_from(table)
        .ok()
        .and_then(|p| p.checked_add_signed(displacement))
        .and_then(|p| p.checked_add(md::FLAGS))
        .ok_or("commander metadata address overflow")?;
    let bytes = read::<[u8; 12]>(memory, address).ok_or("commander metadata unreadable")?;
    Ok(std::array::from_fn(|i| {
        u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
    }))
}

pub(crate) fn capture_modifier_gate(
    memory: &impl Memory,
    base: usize,
) -> Result<(u32, u8), &'static str> {
    use engine_native_parts::modifier_definition as md;
    let callback_address = engine_sites::at(base, md::ELIGIBILITY_CALLBACK_RVA)
        .ok_or("commander modifier callback withheld")?;
    let flags_address = engine_sites::at(base, md::TRADE_FEATURE_FLAGS_RVA)
        .ok_or("commander modifier flags withheld")?;
    let callback =
        read::<u64>(memory, callback_address).ok_or("commander modifier callback unreadable")?;
    let kind = if callback == 0 {
        0
    } else if engine_sites::at(base, md::ELIGIBILITY_FLAGS_GETTER_RVA).map(|p| p as u64)
        == Some(callback)
    {
        1
    } else {
        return Err("commander modifier callback unsupported");
    };
    let flags =
        read::<u32>(memory, flags_address).ok_or("commander modifier feature flags unreadable")?;
    Ok((flags, kind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    const LEADER: usize = 0x4000;
    const HANDLE: u64 = (0xf000_0001_u64 << 32) | cunit_leader::REFERENCE_TYPE as u64;
    struct Fixture {
        bytes: Vec<u8>,
        copies: Cell<usize>,
        bytes_read: Cell<usize>,
        reads: RefCell<Vec<(usize, usize)>>,
        fail: Option<usize>,
        callback: u64,
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            self.copies.set(self.copies.get() + 1);
            self.bytes_read.set(self.bytes_read.get() + out.len());
            self.reads.borrow_mut().push((address, out.len()));
            if self.fail == Some(address) {
                return false;
            }
            use engine_native_parts::modifier_definition as md;
            if address == md::TABLE_RVA.0 && out.len() == 8 {
                out.copy_from_slice(&0x200000_u64.to_le_bytes());
                return true;
            }
            if address == md::ELIGIBILITY_CALLBACK_RVA.0 && out.len() == 8 {
                out.copy_from_slice(&self.callback.to_le_bytes());
                return true;
            }
            if address == md::TRADE_FEATURE_FLAGS_RVA.0 && out.len() == 4 {
                out.copy_from_slice(&0x80000002_u32.to_le_bytes());
                return true;
            }
            if address == 0x200000 + 0x5f * md::STRIDE + md::FLAGS && out.len() == 12 {
                for (bytes, value) in out.chunks_exact_mut(4).zip([0x10_u32, u32::MAX, 2]) {
                    bytes.copy_from_slice(&value.to_le_bytes());
                }
                return true;
            }
            if (address == engine_native_parts::creference_registry::HIGH_ROOT_RVA.0
                || address
                    == engine_native_parts::creference_registry::LOW_ROOTS_RVA.0
                        + ccharacter::REFERENCE_TYPE as usize * 8)
                && out.len() == 8
            {
                out.copy_from_slice(&0x1000_u64.to_le_bytes());
                return true;
            }
            if address == SKILL_DATABASE_RVA.0 && out.len() == 8 {
                out.copy_from_slice(&0x1800_u64.to_le_bytes());
                return true;
            }
            if address == LEARNING_THRESHOLD_RVA.0 && out.len() == 8 {
                out.copy_from_slice(&100_i64.to_le_bytes());
                return true;
            }
            let Some(bytes) = self.bytes.get(address..address.saturating_add(out.len())) else {
                return false;
            };
            out.copy_from_slice(bytes);
            true
        }
    }
    impl Fixture {
        fn u64(&mut self, p: usize, v: u64) {
            self.bytes[p..p + 8].copy_from_slice(&v.to_le_bytes());
        }
        fn u32(&mut self, p: usize, v: u32) {
            self.bytes[p..p + 4].copy_from_slice(&v.to_le_bytes());
        }
        fn text(&mut self, p: usize, t: &[u8]) {
            self.bytes[p..p + t.len()].copy_from_slice(t);
            self.u64(p + 16, t.len() as u64);
            self.u64(p + 24, 15);
        }
        fn array(&mut self, p: usize, data: usize, count: u32) {
            self.u64(p, data as u64);
            self.u32(p + 8, count);
            self.u32(p + 12, count);
        }
    }
    fn fixture() -> Fixture {
        use engine_native_parts::creference_registry as registry;
        let mut f = Fixture {
            bytes: vec![0; 0x10000],
            copies: 0.into(),
            bytes_read: 0.into(),
            reads: RefCell::new(Vec::new()),
            fail: None,
            callback: engine_native_parts::modifier_definition::ELIGIBILITY_FLAGS_GETTER_RVA.0 as u64,
        };
        f.u64(0x1000 + registry::ENTRIES, 0x1100);
        f.u32(0x1000 + registry::COUNT, 1);
        f.u32(0x1000 + registry::MASK, 7);
        f.bytes[0x1000 + registry::OVERFLOW_DISTANCE] = 5;
        f.bytes[0x1100 + 13 * 24 + 4] = 255;
        let hash = HANDLE
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let slot = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
        let entry = 0x1100 + slot * 24;
        f.bytes[entry + 4] = 1;
        f.u64(entry + 8, HANDLE);
        f.u64(entry + 16, LEADER as u64);
        f.u64(LEADER, cnavy_leader::VTABLE_RVA.0 as u64);
        f.u64(LEADER + cunit_leader::TYPE, HANDLE);
        f.u32(LEADER + cunit_leader::CATEGORY, 2);
        f.u64(LEADER + cunit_leader::EXPERIENCE, i64::MIN as u64);
        f.text(LEADER + cunit_leader::NAME, &[b'n', 0xff]);
        for offset in [0x60, 0xc0, 0xe0, 0x100] {
            f.text(LEADER + offset, &[]);
        }
        for (index, offset) in [cunit_leader::SKILL, 0xf70, 0xf80, 0xf90, 0xfa0]
            .into_iter()
            .enumerate()
        {
            let skill = 0x6000 + index * 0x200;
            f.u64(LEADER + offset, skill as u64);
            f.u64(skill, cunit_leader_skill::VTABLE_RVA.0 as u64);
            f.bytes[skill + 8] = 1;
            f.u32(skill + cunit_leader_skill::LEVEL, 4);
        }
        for (index, offset) in [
            cnavy_leader::ATTACK,
            cnavy_leader::DEFENSE,
            cnavy_leader::MANEUVERING,
            cnavy_leader::COORDINATION,
        ]
        .into_iter()
        .enumerate()
        {
            f.u32(LEADER + offset, [i32::MIN, i32::MAX, -1, 256][index] as u32);
        }
        f.array(0x1800 + 0xe8 + 2 * 0x18, 0x1a00, 1);
        f.u64(0x1a00, 0x6a00);
        f.u64(0x6a00, cunit_leader_skill::VTABLE_RVA.0 as u64);
        f.array(0x6020, 0x7000, 1);
        f.u32(0x7000, 0x5f);
        f.u64(0x7008, i64::MAX as u64);
        f.array(LEADER + cunit_leader::TRAIT_DATA, 0x8000, 1);
        f.u64(0x8000, 0x8100);
        f.u64(0x8100, cunit_leader_trait::VTABLE_RVA.0 as u64);
        f.u32(0x8100 + cunit_leader_trait::TOKEN_ID, 55);
        f.text(0x8100 + cunit_leader_trait::TOKEN, b"navy_trait");
        f.u32(0x8100 + cunit_leader_trait::ORDINAL, 7);
        f.u32(0x8100 + cunit_leader_trait::KIND, 2);
        f.bytes[0x8100 + 0xa74] = 1;
        f.u32(0x8100 + cunit_leader_trait::REQUIRED_EXPERIENCE, 3);
        f.array(LEADER + cunit_leader::TRAIT_EXPERIENCE_DATA, 0x9000, 1);
        f.u64(0x9000, 0x8100);
        f.u64(0x9008, 321000);
        f
    }
    fn capture(memory: &impl Memory, base: usize, handle: u64) -> Result<Row, &'static str> {
        super::capture(memory, base, handle, &mut Vec::new(), &mut 0)
    }
    #[test]
    fn commander_text_accepts_native_long_names_descriptions_and_fallbacks() {
        let mut f = fixture();
        let mut at = f.bytes.len();
        let offsets = [cunit_leader::NAME, 0x60, 0xc0, 0xe0, 0x100];
        for (index, offset) in offsets.into_iter().enumerate() {
            let len = 257 + index * 5_000;
            f.bytes.resize(at + len + 1, 0);
            f.bytes[at..at + len].fill(b'a' + index as u8);
            f.bytes[at + 1] = 0xff;
            f.u64(LEADER + offset, at as u64);
            f.u64(LEADER + offset + 16, len as u64);
            f.u64(LEADER + offset + 24, len as u64);
            at += len + 1;
        }
        let row = capture(&f, 0, HANDLE).unwrap();
        assert_eq!((row.name_len, row.description_len), (257, 5_257));
        for (index, text) in [
            &row.name[..row.name_len],
            &row.description[..row.description_len],
        ]
        .into_iter()
        .chain(
            row.presentation
                .fallback_text
                .iter()
                .enumerate()
                .map(|(i, text)| &text[..row.presentation.fallback_text_len[i]]),
        )
        .enumerate()
        {
            assert_eq!(text.len(), 257 + index * 5_000);
            assert_eq!(text[0], b'a' + index as u8);
            assert_eq!(text[1], 0xff);
        }
        f.u64(LEADER + cunit_leader::NAME + 24, 256);
        assert_eq!(capture(&f, 0, HANDLE), Err("commander text header invalid"));
        f.u64(LEADER + cunit_leader::NAME + 24, 257);
        f.bytes[0x10000 + 257] = 1;
        assert_eq!(capture(&f, 0, HANDLE), Err("commander text unreadable"));
    }

    #[test]
    fn captures_hourly_components_traits_and_definition_operands_without_narrowing() {
        let f = fixture();
        let row = capture(&f, 0, HANDLE).unwrap();
        assert_eq!(row.skills, [4, i32::MIN, i32::MAX, -1, 256]);
        assert_eq!(row.experience, i64::MIN);
        assert_eq!(&row.name[..row.name_len], &[b'n', 0xff]);
        assert_eq!(row.traits_len, 1);
        assert_eq!(row.learning_traits[0].experience, 321000);
        assert_eq!(row.learning_traits[0].required_experience, 3);
        assert_eq!(row.modifiers_len, 1);
        assert_eq!(row.modifiers[0].value, i64::MAX);
        assert_eq!(row.modifiers[0].source, 0);
        assert_eq!(row.modifiers[0].metadata_flags, 0x10);
        assert_eq!(row.modifiers[0].category_mask, u32::MAX);
        assert_eq!(row.modifiers[0].feature_selector, 2);
        assert_eq!(
            (row.modifier_feature_flags, row.modifier_callback_kind),
            (0x80000002, 1)
        );
        assert_eq!((row.next_skill.present, row.next_skill.valid), (1, 0));
        let reads = f.reads.borrow();
        for address in [
            engine_native_parts::creference_registry::HIGH_ROOT_RVA.0,
            LEADER + cunit_leader::CHARACTER_TYPE,
            LEADER + cunit_leader::TRAIT_DATA,
            LEADER + cunit_leader::TRAIT_EXPERIENCE_DATA,
            SKILL_DATABASE_RVA.0,
            0x1800 + 0xe8 + 2 * 0x18,
            engine_native_parts::modifier_definition::TABLE_RVA.0,
            engine_native_parts::modifier_definition::ELIGIBILITY_CALLBACK_RVA.0,
            engine_native_parts::modifier_definition::TRADE_FEATURE_FLAGS_RVA.0,
        ] {
            assert_eq!(
                reads.iter().filter(|read| read.0 == address).count(),
                1,
                "source {address:x}"
            );
        }
        for offset in [cunit_leader::SKILL, 0xf70, 0xf80, 0xf90, 0xfa0] {
            assert_eq!(
                reads
                    .iter()
                    .filter(|read| **read == (LEADER + offset, 8))
                    .count(),
                1
            );
        }
        for offset in [cunit_leader::NAME, 0x60, 0xc0, 0xe0, 0x100] {
            assert_eq!(
                reads
                    .iter()
                    .filter(|read| **read == (LEADER + offset, 32))
                    .count(),
                1
            );
        }
    }
    #[test]
    fn malformed_traits_and_unresolved_identity_refuse_complete_context() {
        let mut f = fixture();
        f.u32(LEADER + cunit_leader::TRAIT_COUNT, 65);
        f.u32(LEADER + cunit_leader::TRAIT_CAPACITY, 65);
        assert_eq!(
            capture(&f, 0, HANDLE),
            Err("commander field unreadable or overflow")
        );
        let mut f = fixture();
        f.u64(LEADER, 0);
        assert!(capture(&f, 0, HANDLE).is_err());
        let before = f.copies.get();
        assert_eq!(capture(&f, 0, 0), Ok(Row::default()));
        assert_eq!(f.copies.get(), before);
    }

    #[test]
    fn timed_traits_keep_native_order_signed_days_and_live_demand() {
        const TIMED: usize = 0x10000;
        const COUNT: usize = 5_000;
        let mut f = fixture();
        f.bytes
            .resize(TIMED + COUNT * cunit_leader::TIMED_TRAIT_STRIDE, 0);
        f.array(LEADER + cunit_leader::TIMED_TRAIT_DATA, TIMED, COUNT as u32);
        for index in 0..COUNT {
            let entry = TIMED + index * cunit_leader::TIMED_TRAIT_STRIDE;
            f.u64(entry, 0x8100);
            f.u32(entry + 8, (index as i32 - 1) as u32);
        }
        let mut timed = Vec::new();
        let mut len = 0;
        let row = super::capture(&f, 0, HANDLE, &mut timed, &mut len).unwrap();
        assert_eq!(row.timed_traits, [0, COUNT]);
        assert_eq!(len, COUNT);
        for (index, value) in timed[..len].iter().enumerate() {
            assert_eq!((value.token_id, value.definition_ordinal), (55, 7));
            assert_eq!(value.remaining_days, index as i32 - 1);
        }
        let reads = f.reads.borrow();
        assert_eq!(
            reads
                .iter()
                .filter(|read| read.0 == LEADER + cunit_leader::TIMED_TRAIT_DATA)
                .count(),
            1
        );
        assert_eq!(
            reads
                .iter()
                .filter(|read| (TIMED..TIMED + COUNT * 16).contains(&read.0))
                .count(),
            COUNT.div_ceil(64)
        );
        drop(reads);
        let fail = TIMED + 64 * cunit_leader::TIMED_TRAIT_STRIDE;
        f.fail = Some(fail);
        f.reads.borrow_mut().clear();
        len = 0;
        assert_eq!(
            super::capture(&f, 0, HANDLE, &mut timed, &mut len),
            Err("unit leader timed trait entries unreadable")
        );
        assert_eq!(len, 0);
        assert_eq!(
            f.reads
                .borrow()
                .iter()
                .filter(|read| read.0 == fail)
                .count(),
            1
        );
        f.fail = None;
        f.u64(TIMED, 0x8d00);
        f.u64(0x8d00, 0);
        len = 0;
        assert_eq!(
            super::capture(&f, 0, HANDLE, &mut timed, &mut len),
            Err("unit leader timed trait vtable mismatch")
        );
        f.u32(
            LEADER + cunit_leader::TIMED_TRAIT_DATA + 12,
            COUNT as u32 + 1,
        );
        assert_eq!(
            super::capture(&f, 0, HANDLE, &mut timed, &mut len),
            Err("commander array invalid")
        );
        assert_eq!(len, 0);
    }

    #[test]
    fn metadata_and_callback_preserve_primary_values_and_fault_prefixes() {
        use engine_native_parts::modifier_definition as md;
        let address = 0x200000 + 0x5f * md::STRIDE + md::FLAGS;
        let f = fixture();
        assert_eq!(
            capture_modifier_metadata(&f, 0, 0x5f),
            Ok([0x10, u32::MAX, 2])
        );
        assert_eq!((f.copies.get(), f.bytes_read.get()), (2, 20));
        assert_eq!(*f.reads.borrow(), [(md::TABLE_RVA.0, 8), (address, 12)]);
        for (callback, result, calls, bytes) in [
            (
                md::ELIGIBILITY_FLAGS_GETTER_RVA.0 as u64,
                Ok((0x80000002, 1)),
                2,
                12,
            ),
            (0, Ok((0x80000002, 0)), 2, 12),
            (1, Err("commander modifier callback unsupported"), 1, 8),
        ] {
            let mut f = fixture();
            f.callback = callback;
            assert_eq!(capture_modifier_gate(&f, 0), result);
            assert_eq!((f.copies.get(), f.bytes_read.get()), (calls, bytes));
        }
        for (fail, calls, bytes, reason) in [
            (md::TABLE_RVA.0, 1, 8, "commander metadata root unreadable"),
            (address, 2, 20, "commander metadata unreadable"),
        ] {
            let mut f = fixture();
            f.fail = Some(fail);
            assert_eq!(capture_modifier_metadata(&f, 0, 0x5f), Err(reason));
            assert_eq!((f.copies.get(), f.bytes_read.get()), (calls, bytes));
            assert_eq!(f.reads.borrow().last().unwrap().0, fail);
        }
        for (fail, calls, bytes, reason) in [
            (
                md::ELIGIBILITY_CALLBACK_RVA.0,
                1,
                8,
                "commander modifier callback unreadable",
            ),
            (
                md::TRADE_FEATURE_FLAGS_RVA.0,
                2,
                12,
                "commander modifier feature flags unreadable",
            ),
        ] {
            let mut f = fixture();
            f.fail = Some(fail);
            assert_eq!(capture_modifier_gate(&f, 0), Err(reason));
            assert_eq!((f.copies.get(), f.bytes_read.get()), (calls, bytes));
            assert_eq!(f.reads.borrow().last().unwrap().0, fail);
        }
    }

    #[test]
    fn commander_collections_follow_every_native_occurrence() {
        const COUNT: usize = 65;
        let mut f = fixture();
        let traits = 0x10000;
        let learning = traits + COUNT * 8;
        f.bytes.resize(learning + COUNT * 16, 0);
        f.array(LEADER + cunit_leader::TRAIT_DATA, traits, COUNT as u32);
        f.array(
            LEADER + cunit_leader::TRAIT_EXPERIENCE_DATA,
            learning,
            COUNT as u32,
        );
        for index in 0..COUNT {
            f.u64(traits + index * 8, 0x8100);
            f.u64(learning + index * 16, 0x8100);
            f.u64(learning + index * 16 + 8, index as u64);
        }
        f.array(0x1800 + 0xe8 + 2 * 0x18, 0x1a00, COUNT as u32);
        f.u64(0x1a00 + 5 * 8, 0x6a00);
        let row = capture(&f, 0, HANDLE).unwrap();
        assert_eq!(row.traits_len, COUNT);
        assert_eq!(row.learning_traits_len, COUNT);
        assert!(row.traits.iter().all(|trait_| trait_.token_id == 55));
        for (index, trait_) in row.learning_traits.iter().enumerate() {
            assert_eq!((trait_.token_id, trait_.experience), (55, index as i64));
        }
        let mut role = Vec::new();
        let mut role_text = Vec::new();
        assert_eq!(
            Reader {
                memory: &f,
                base: 0
            }
            .traits(LEADER, &mut role, &mut role_text)
            .unwrap()
            .0,
            COUNT
        );
        f.u64(traits + (COUNT - 1) * 8, 0);
        assert_eq!(
            capture(&f, 0, HANDLE),
            Err("commander field unreadable or overflow")
        );
    }

    #[test]
    fn commander_tooltip_hash_uses_its_original_bucket_span_and_count() {
        const COUNT: usize = 5_001;
        let mut f = fixture();
        let data = 0x10000;
        f.bytes.resize(data + (COUNT + 1) * 0x28, 0);
        f.array(0x6000 + 0x20 + 0x70, data, COUNT as u32);
        for index in 0..COUNT {
            let entry = data + index * 0x28;
            f.u32(entry, index as u32 + 1);
            f.bytes[entry + 4] = 1;
            f.text(entry + 8, b"tip");
        }
        let row = capture(&f, 0, HANDLE).unwrap();
        assert_eq!(row.tooltips_len, COUNT);
        for (index, tooltip) in row.tooltips.iter().enumerate() {
            assert_eq!(tooltip.key, index as u32 + 1);
            assert_eq!(
                &row.tooltip_text[tooltip.text_start..tooltip.text_start + tooltip.text_len],
                b"tip"
            );
        }
        f.bytes[data + (COUNT - 1) * 0x28 + 4] = 255;
        assert_eq!(
            capture(&f, 0, HANDLE),
            Err("commander tooltip premature sentinel")
        );
    }

    #[test]
    fn commander_modifiers_preserve_every_native_sorted_entry() {
        const COUNT: usize = 1_001;
        let mut f = fixture();
        f.bytes.resize(0x10000 + COUNT * 16, 0);
        f.array(0x6020, 0x10000, COUNT as u32);
        for index in 0..COUNT {
            f.u32(0x10000 + index * 16, index as u32 + 1);
            f.u64(0x10008 + index * 16, index as u64);
        }
        struct Metadata<'a>(&'a Fixture);
        impl Memory for Metadata<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if (0x200000..0x200000 + 2_000 * engine_native_parts::modifier_definition::STRIDE)
                    .contains(&address)
                    && output.len() == 12
                {
                    output.fill(0);
                    true
                } else {
                    self.0.copy(address, output)
                }
            }
        }
        let row = capture(&Metadata(&f), 0, HANDLE).unwrap();
        assert_eq!(row.modifiers_len, COUNT);
        for (index, modifier) in row.modifiers.iter().enumerate() {
            assert_eq!(
                (modifier.id, modifier.value),
                (index as u32 + 1, index as i64)
            );
        }
        f.u32(0x10000 + (COUNT - 1) * 16, 1);
        assert_eq!(
            capture(&Metadata(&f), 0, HANDLE),
            Err("commander modifier keys invalid")
        );
    }

    #[test]
    fn character_portraits_and_role_traits_follow_native_headers() {
        use engine_native_parts::creference_registry as registry;
        const COUNT: usize = 5_001;
        const CHARACTER: usize = 0xa000;
        let character = (0xf000_0002_u64 << 32) | u64::from(ccharacter::REFERENCE_TYPE);
        let mut f = fixture();
        let traits = 0x10000 + COUNT * 0x38;
        f.bytes.resize(traits + COUNT * 8, 0);
        f.u32(0x1000 + registry::COUNT, 2);
        let hash = character
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let mut slot = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
        let mut distance = 1;
        while f.bytes[0x1100 + slot * 24 + 4] != 0 {
            slot += 1;
            distance += 1;
        }
        f.bytes[0x1100 + slot * 24 + 4] = distance;
        f.u64(0x1100 + slot * 24 + 8, character);
        f.u64(0x1100 + slot * 24 + 16, CHARACTER as u64);
        f.u64(LEADER + cunit_leader::CHARACTER_TYPE, character);
        f.u64(CHARACTER, ccharacter::VTABLE_RVA.0 as u64);
        f.u64(CHARACTER + ccharacter::TYPE, character);
        f.u64(CHARACTER + ccharacter::UNIT_LEADER, LEADER as u64);
        f.array(CHARACTER + ccharacter::PORTRAIT_DATA, 0x10000, COUNT as u32);
        f.array(LEADER + cunit_leader::TRAIT_DATA, traits, COUNT as u32);
        for index in 0..COUNT {
            f.u32(0x10000 + index * 0x38, index as u32);
            f.text(0x10010 + index * 0x38, b"portrait");
            f.u64(traits + index * 8, 0x8100);
        }
        let row = capture(&f, 0, HANDLE).unwrap();
        assert_eq!(row.presentation.portraits_len, COUNT);
        assert_eq!(row.presentation.role_traits_len, COUNT);
        for (index, portrait) in row.presentation.portraits.iter().enumerate() {
            assert_eq!(portrait.role, index as u32);
            assert_eq!(
                &row.presentation.portrait_text
                    [portrait.text_start..portrait.text_start + portrait.text_len],
                b"portrait"
            );
        }
        use engine_native::portrait_cache::{EntryImage, HolderImage, test_images};
        f.u32(CHARACTER + ccharacter::PORTRAIT_DATA + 12, COUNT as u32 + 1);
        assert!(capture(&f, 0, HANDLE).is_err());
        let entries: &'static [EntryImage] = Box::leak(Box::new([
            EntryImage {
                head: [7, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0],
                text: [0, 6],
            },
            EntryImage {
                head: [8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                text: [6, 0],
            },
        ]));
        let mut header = [0; 16];
        header[..8].copy_from_slice(&0x10000_u64.to_le_bytes());
        header[8..12].copy_from_slice(&2_u32.to_le_bytes());
        header[12..].copy_from_slice(&2_u32.to_le_bytes());
        let image = HolderImage::new(header, entries, b"before");
        test_images::set(CHARACTER + ccharacter::PORTRAIT_DATA, Some(image));
        let row = capture(&f, 0, HANDLE);
        test_images::set(0, None);
        let presentation = row.unwrap().presentation;
        assert_eq!(presentation.portraits_len, 2);
        let [first, second] = [presentation.portraits[0], presentation.portraits[1]];
        assert_eq!((first.role, first.size, first.is_set, first.kind), (7, 1, 1, 2));
        assert_eq!((second.role, second.text_start, second.text_len), (8, 6, 0));
        assert_eq!(
            &presentation.portrait_text[..presentation.portrait_text_len],
            b"before"
        );
        assert_eq!(presentation.role_traits_len, COUNT);
    }
}

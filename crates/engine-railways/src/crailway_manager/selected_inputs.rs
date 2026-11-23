
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read, read_slice, walk::Buffer};
use crate::cprovince_railway_info as info;
use engine_native::cprovince::definition as province_definition;

type Input<T> = Result<T, &'static str>;

fn level_header(memory: &impl Memory, node: usize, definition: usize) -> Input<Array> {
    let header = node
        .checked_add(info::LEVELS_BY_NEIGHBOR_INDEX)
        .and_then(|at| Array::read(memory, at, 4))
        .ok_or("selected railway level array unreadable or invalid")?;
    let neighbours = definition
        .checked_add(province_definition::NEIGHBOUR_COUNT)
        .and_then(|at| read::<i32>(memory, at))
        .ok_or("selected railway definition neighbour count unreadable")?;
    if neighbours < 0 || header.count != neighbours as u32 {
        return Err("selected railway level count differs from native definition neighbours");
    }
    Ok(header)
}

pub fn demand(memory: &impl Memory, base: usize, state: usize) -> Input<usize> {
    use super::{PROVINCE_INFO_TABLE, VTABLE_RVA};
    use engine_native::{cgame_state, cprovince};
    let word = |object: usize, offset: usize| {
        object
            .checked_add(offset)
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .ok_or("selected railway sizing pointer unreadable or overflow")
    };
    let identity = |pointer: usize, key: Rva| -> Input<()> {
        let expected = engine_sites::at(base, key)
            .ok_or("selected railway sizing module address overflow")?;
        if pointer == 0 || read::<u64>(memory, pointer) != Some(expected as u64) {
            return Err("selected railway sizing identity unreadable or unsupported");
        }
        Ok(())
    };
    let manager = word(state, cgame_state::RAIL_MANAGER)?;
    identity(manager, VTABLE_RVA)?;
    let array = |object: usize, offset: usize| {
        object
            .checked_add(offset)
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("selected railway sizing array unreadable or invalid")
    };
    let rail = array(manager, PROVINCE_INFO_TABLE)?;
    let provinces = array(state, cgame_state::PROVINCES)?;
    if provinces.count < 2 || rail.count != provinces.count {
        return Err("rail table extent differs from province table");
    }
    let mut required = 0_usize;
    for slot in 0..rail.count {
        let node = word(
            rail.element(slot, 8)
                .ok_or("selected railway sizing slot address overflow")?,
            0,
        )?;
        if node == 0 {
            continue;
        }
        identity(node, info::VTABLE_RVA)?;
        let province = word(
            provinces
                .element(slot, 8)
                .ok_or("selected railway sizing province address overflow")?,
            0,
        )?;
        identity(province, cprovince::VTABLE_RVA)?;
        let definition = word(province, cprovince::DEFINITION)?;
        let id = |object: usize, offset: usize| {
            object
                .checked_add(offset)
                .and_then(|at| read::<u32>(memory, at))
        };
        if definition == 0
            || id(province, cprovince::ID) != Some(slot)
            || id(definition, province_definition::ID) != Some(slot)
            || word(node, info::PROVINCE_RECORD)? != definition
            || Some(word(node, info::BUILDING_STATUS)?)
                != province.checked_add(cprovince::BUILDING_STATUS)
        {
            return Err("selected railway sizing province identity or backlink mismatch");
        }
        required = required
            .checked_add(level_header(memory, node, definition)?.count as usize)
            .ok_or("selected railway level demand overflow")?;
    }
    Ok(required)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LevelSpan {
    pub first: usize,
    pub len: usize,
    pub positive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionRow {
    pub levels: Input<LevelSpan>,
    pub pair_count: Input<i32>,
    pub building_level: Input<Option<Option<i16>>>,
}

impl Default for SelectionRow {
    fn default() -> Self {
        Self {
            levels: Err("selected railway levels not acquired"),
            pair_count: Err("selected railway pair count not acquired"),
            building_level: Err("selected railway building level not acquired"),
        }
    }
}

pub struct Levels<'a> {
    pub values: &'a mut Vec<i32>,
    pub len: usize,
    pub required: usize,
}

pub(super) struct LevelBuffer<'a> {
    pub values: Buffer<'a, i32>,
    pub len: usize,
    pub required: usize,
}

impl LevelBuffer<'_> {
    pub(super) fn read(
        &mut self,
        memory: &impl Memory,
        node: usize,
        definition: usize,
    ) -> Input<LevelSpan> {
        let header = level_header(memory, node, definition)?;
        let count = header.count as usize;
        self.required = self
            .required
            .checked_add(count)
            .ok_or("selected railway level demand overflow")?;
        let end = self
            .len
            .checked_add(count)
            .ok_or("selected railway level arena overflow")?;
        if !self.values.ensure(end)? {
            return Err("selected railway level capacity insufficient");
        }
        let mut bytes = [0_u8; 256];
        let mut positive = false;
        for first in (0..count).step_by(bytes.len() / 4) {
            let take = (count - first).min(bytes.len() / 4);
            let at = header
                .element(first as u32, 4)
                .ok_or("selected railway level address overflow")?;
            if !read_slice(memory, at, &mut bytes[..take * 4]) {
                return Err("selected railway indexed levels unreadable");
            }
            for (index, bytes) in bytes[..take * 4].chunks_exact(4).enumerate() {
                let level = i32::from_le_bytes(bytes.try_into().unwrap());
                positive |= level > 0;
                self.values[self.len + first + index] = level;
            }
        }
        let span = LevelSpan {
            first: self.len,
            len: count,
            positive,
        };
        self.len = end;
        Ok(span)
    }

    pub(super) fn append(&mut self, values: &[i32]) -> Input<LevelSpan> {
        let count = values.len();
        self.required = self
            .required
            .checked_add(count)
            .ok_or("selected railway level demand overflow")?;
        let end = self
            .len
            .checked_add(count)
            .ok_or("selected railway level arena overflow")?;
        if !self.values.ensure(end)? {
            return Err("selected railway level capacity insufficient");
        }
        self.values[self.len..end].copy_from_slice(values);
        let span = LevelSpan {
            first: self.len,
            len: count,
            positive: values.iter().any(|&level| level > 0),
        };
        self.len = end;
        Ok(span)
    }
}

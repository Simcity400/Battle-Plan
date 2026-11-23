use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native::{ccountry, cgame_state};
use engine_sites::Rva;

pub const PENDING_STRIDE: usize = 0x38;
pub const PENDING_INSTANCE: usize = 0;
pub const PENDING_DEFINITION: usize = 8;
pub const PENDING_SCOPE: usize = 0x10;
pub const PENDING_ORIGINATOR: usize = 0x18;
pub const PENDING_DUE: usize = 0x28;
pub const SET_COUNT: usize = 0;
const SET_BUCKET_COUNT: usize = 4;
const SET_BUCKETS: usize = 8;
const NODE_DEFINITION: usize = 0;
const NODE_NEXT: usize = 8;
pub const DELAYED_VTABLE_RVA: Rva = Rva(0x0286_4858);
pub const DELAYED_DEFINITION: usize = 8;
pub const DELAYED_SCOPE: usize = 0x10;
pub const DELAYED_ORIGINATOR: usize = 0xc0;
pub const DELAYED_REMAINING: usize = 0xc4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delayed {
    pub pointer: usize,
    pub definition: usize,
    pub scope: usize,
    pub originator: u32,
    pub remaining: i32,
}
pub fn delayed_array(memory: &impl Memory, country: usize) -> Result<Array, &'static str> {
    Array::read(memory, at(country, ccountry::DELAYED_EVENTS)?, 8)
        .ok_or("country delayed-event array unreadable or invalid")
}
pub fn delayed_at(
    memory: &impl Memory,
    base: usize,
    array: Array,
    ordinal: u32,
) -> Result<Delayed, &'static str> {
    let pointer = word::<u64>(
        memory,
        array
            .element(ordinal, 8)
            .ok_or("delayed-event ordinal outside native array")?,
        0,
    )? as usize;
    if word::<u64>(memory, pointer, 0)?
        != engine_sites::at(base, DELAYED_VTABLE_RVA).ok_or("script registry address overflow")?
            as u64
    {
        return Err("delayed-event vtable differs");
    }
    Ok(Delayed {
        pointer,
        definition: word::<u64>(memory, pointer, DELAYED_DEFINITION)? as usize,
        scope: at(pointer, DELAYED_SCOPE)?,
        originator: word(memory, pointer, DELAYED_ORIGINATOR)?,
        remaining: word(memory, pointer, DELAYED_REMAINING)?,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pending {
    pub instance: u32,
    pub definition: usize,
    pub scope: usize,
    pub originator: u32,
    pub due_hour: i32,
}
fn at(object: usize, offset: usize) -> Result<usize, &'static str> {
    object
        .checked_add(offset)
        .ok_or("script registry address overflow")
}
fn word<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(object, offset)?).ok_or("script registry operand unreadable")
}
pub fn pending_array(memory: &impl Memory, state: usize) -> Result<Array, &'static str> {
    Array::read(
        memory,
        at(state, cgame_state::PENDING_EVENTS)?,
        PENDING_STRIDE,
    )
    .ok_or("pending event array unreadable or invalid")
}
pub fn pending_at(
    memory: &impl Memory,
    array: Array,
    ordinal: u32,
) -> Result<Pending, &'static str> {
    let entry = array
        .element(ordinal, PENDING_STRIDE)
        .ok_or("pending event ordinal outside native array")?;
    Ok(Pending {
        instance: word(memory, entry, PENDING_INSTANCE)?,
        definition: word::<u64>(memory, entry, PENDING_DEFINITION)? as usize,
        scope: word::<u64>(memory, entry, PENDING_SCOPE)? as usize,
        originator: word(memory, entry, PENDING_ORIGINATOR)?,
        due_hour: word(memory, entry, PENDING_DUE)?,
    })
}
pub fn pending_last(
    memory: &impl Memory,
    state: usize,
    instance: u32,
) -> Result<Pending, &'static str> {
    let array = pending_array(memory, state)?;
    let row = pending_at(
        memory,
        array,
        array
            .count
            .checked_sub(1)
            .ok_or("pending event append is empty")?,
    )?;
    if row.instance != instance {
        return Err("pending event append instance differs");
    }
    Ok(row)
}
pub fn walk_pending(
    memory: &impl Memory,
    state: usize,
    mut visit: impl FnMut(Pending) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let array = pending_array(memory, state)?;
    for ordinal in 0..array.count {
        visit(pending_at(memory, array, ordinal)?)?;
    }
    if pending_array(memory, state)? != array {
        return Err("pending event baseline changed during acquisition");
    }
    Ok(())
}
pub fn walk_fired(
    memory: &impl Memory,
    state: usize,
    mut visit: impl FnMut(usize) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let set = at(state, cgame_state::FIRED_EVENTS)?;
    let count = word::<u32>(memory, set, SET_COUNT)?;
    let buckets = word::<u32>(memory, set, SET_BUCKET_COUNT)?;
    let data = word::<u64>(memory, set, SET_BUCKETS)? as usize;
    if count != 0 && (buckets == 0 || data == 0) {
        return Err("fired-event set header invalid");
    }
    let mut seen = 0_u32;
    for bucket in 0..buckets {
        let offset = (bucket as usize)
            .checked_mul(8)
            .ok_or("fired-event bucket address overflow")?;
        let mut node = word::<u64>(memory, data, offset)? as usize;
        while node != 0 {
            seen = seen
                .checked_add(1)
                .filter(|seen| *seen <= count)
                .ok_or("fired-event set cycle or count differs")?;
            visit(word::<u64>(memory, node, NODE_DEFINITION)? as usize)?;
            node = word::<u64>(memory, node, NODE_NEXT)? as usize;
        }
    }
    if seen != count
        || word::<u32>(memory, set, SET_COUNT)? != count
        || word::<u64>(memory, set, SET_BUCKETS)? as usize != data
        || word::<u32>(memory, set, SET_BUCKET_COUNT)? != buckets
    {
        return Err("fired-event baseline count or header differs");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::BTreeMap};
    #[derive(Default)]
    struct Image {
        bytes: BTreeMap<usize, u8>,
        reads: RefCell<Vec<usize>>,
    }
    impl Image {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            for (n, byte) in bytes.iter().enumerate() {
                self.bytes.insert(at + n, *byte);
            }
        }
        fn array(&mut self, at: usize, data: usize, count: u32) {
            self.put(at, &(data as u64).to_le_bytes());
            self.put(at + 8, &count.to_le_bytes());
            self.put(at + 12, &count.to_le_bytes());
        }
        fn pending(&mut self, at: usize, id: u32) {
            self.put(at, &[0; PENDING_STRIDE]);
            self.put(at, &id.to_le_bytes());
            self.put(at + PENDING_DEFINITION, &99_u64.to_le_bytes());
            self.put(at + PENDING_SCOPE, &88_u64.to_le_bytes());
            self.put(at + PENDING_DUE, &(-25_i32).to_le_bytes());
        }
    }
    impl Memory for Image {
        fn copy(&self, at: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push(at);
            for (n, byte) in output.iter_mut().enumerate() {
                let Some(value) = self.bytes.get(&(at + n)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }
    #[test]
    fn registry_append_demands_only_the_last_entry() {
        let mut image = Image::default();
        let state = 0x1000;
        let data = 0x10000;
        image.array(state + cgame_state::PENDING_EVENTS, data, 3);
        let last = data + 2 * PENDING_STRIDE;
        image.pending(last, 4);
        let row = pending_last(&image, state, 4).unwrap();
        assert_eq!(
            (row.instance, row.definition, row.scope, row.due_hour),
            (4, 99, 88, -25)
        );
        assert!(
            image
                .reads
                .borrow()
                .iter()
                .all(|at| *at == state + cgame_state::PENDING_EVENTS
                    || (*at >= last && *at < last + PENDING_STRIDE))
        );
        assert!(pending_last(&image, state, 3).is_err());
    }
    #[test]
    fn registry_baseline_preserves_duplicate_members_and_refuses_cycles() {
        let mut image = Image::default();
        let state = 0x1000;
        let set = state + cgame_state::FIRED_EVENTS;
        image.put(set, &2_u32.to_le_bytes());
        image.put(set + 4, &1_u32.to_le_bytes());
        image.put(set + 8, &0x2000_u64.to_le_bytes());
        image.put(0x2000, &0x3000_u64.to_le_bytes());
        image.put(0x3000, &77_u64.to_le_bytes());
        image.put(0x3008, &0x4000_u64.to_le_bytes());
        image.put(0x4000, &77_u64.to_le_bytes());
        image.put(0x4008, &0_u64.to_le_bytes());
        let mut members = Vec::new();
        walk_fired(&image, state, |row| {
            members.push(row);
            Ok(())
        })
        .unwrap();
        assert_eq!(members, [77, 77]);
        image.put(0x4008, &0x3000_u64.to_le_bytes());
        assert!(walk_fired(&image, state, |_| Ok(())).is_err());
    }
}

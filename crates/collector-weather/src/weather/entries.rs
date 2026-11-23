use engine_native_memory::Memory;
use engine_native_parts::modifier_accumulator;
use engine_native::cweather_manager::province_weather as sweather_per_province;

pub type Entry = (u32, i64);

pub type ProvinceModifiers = Spans<Entry>;

pub type ProvinceStatics = Spans<u8>;

pub type CustomSea = Spans<i16>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spans<T> {
    spans: Vec<Option<(u32, u32)>>,
    entries: Vec<T>,
}

impl<T: Copy> Spans<T> {
    pub fn clear(&mut self) {
        self.spans.clear();
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.spans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    pub fn allocated_bytes(&self) -> usize {
        self.spans.capacity() * size_of::<Option<(u32, u32)>>()
            + self.entries.capacity() * size_of::<T>()
    }

    pub fn get(&self, slot: usize) -> Option<&[T]> {
        let (start, len) = (*self.spans.get(slot)?)?;
        Some(&self.entries[start as usize..(start + len) as usize])
    }

    pub fn complete(&self) -> bool {
        self.spans.iter().all(Option::is_some)
    }

    pub fn reserve_slots(&mut self, records: usize) {
        let _ = engine_allocation::reserve_exact(&mut self.spans, records);
    }

    pub fn push(&mut self, entries: Option<&[T]>) -> bool {
        if engine_allocation::reserve(&mut self.spans, 1).is_err() {
            return false;
        }
        let Some(entries) = entries else {
            self.spans.push(None);
            return true;
        };
        if engine_allocation::reserve(&mut self.entries, entries.len()).is_err() {
            self.spans.push(None);
            return false;
        }
        let (Ok(start), Ok(len)) = (
            u32::try_from(self.entries.len()),
            u32::try_from(entries.len()),
        ) else {
            self.spans.push(None);
            return false;
        };
        self.entries.extend_from_slice(entries);
        self.spans.push(Some((start, len)));
        true
    }

    pub fn copy_from(&mut self, other: &Self) -> bool {
        self.clear();
        let fits = |have: usize, need: usize| have >= need;
        if !fits(self.spans.capacity(), other.spans.len())
            && engine_allocation::reserve_exact(&mut self.spans, other.spans.len()).is_err()
        {
            return false;
        }
        if !fits(self.entries.capacity(), other.entries.len())
            && engine_allocation::reserve_exact(&mut self.entries, other.entries.len()).is_err()
        {
            return false;
        }
        self.spans.extend_from_slice(&other.spans);
        self.entries.extend_from_slice(&other.entries);
        true
    }
}

pub fn read(
    memory: &impl Memory,
    array: engine_native_memory::cpdx_array::Array,
    slot: u32,
    out: &mut Vec<Entry>,
) -> bool {
    out.clear();
    let Some(address) = array
        .element(slot, sweather_per_province::STRIDE)
        .and_then(|record| record.checked_add(sweather_per_province::MODIFIERS))
    else {
        return false;
    };
    let mut overflow = false;
    let read = modifier_accumulator::entries_at(memory, address, &mut |key, value| {
        if engine_allocation::reserve(out, 1).is_err() {
            overflow = true;
            return;
        }
        out.push((key, value));
    });
    if read.is_err() || overflow {
        out.clear();
        return false;
    }
    true
}

pub fn statics(mut list: &[u8]) -> impl Iterator<Item = (&[u8], i32)> {
    std::iter::from_fn(move || {
        let current = list;
        let days = i32::from_le_bytes(current.get(..4)?.try_into().ok()?);
        let len = u32::from_le_bytes(current.get(4..8)?.try_into().ok()?) as usize;
        let name = current.get(8..8 + len)?;
        list = &current[8 + len..];
        Some((name, days))
    })
}

pub fn read_statics(
    memory: &impl Memory,
    header: [u8; 16],
    definition_vtable: Option<usize>,
    out: &mut Vec<u8>,
) -> bool {
    out.clear();
    let (Some(list), Some(vtable)) = (
        sweather_per_province::static_modifier_list(header),
        definition_vtable,
    ) else {
        return false;
    };
    if sweather_per_province::read_static_modifiers(memory, list, vtable, out).is_err() {
        out.clear();
        return false;
    }
    true
}

use super::{DETAIL_ARENA_CAPACITY, EntryOutput};
use engine_native_memory::{Memory, cpdx_array::Array, read};

pub(crate) fn append_entries<T>(
    memory: &impl Memory,
    array: Array,
    stride: usize,
    mut output: EntryOutput<'_, T>,
    length: &mut usize,
    mut decode: impl FnMut(usize) -> Result<T, &'static str>,
    indirect: bool,
) -> Result<[usize; 2], &'static str> {
    let count = array.count as usize;
    let end = length
        .checked_add(count)
        .ok_or("naval detail required capacity overflow")?;
    let span = [*length, count];
    match &mut output {
        EntryOutput::Owned(rows) => {
            rows.truncate(*length);
            engine_allocation::reserve_push_batch(rows, count)
                .map_err(|_| "naval detail allocation failed")?;
        }
        EntryOutput::Slice(rows) if end > rows.len() => return Err(DETAIL_ARENA_CAPACITY),
        EntryOutput::Slice(_) => (),
    }
    for index in 0..array.count {
        let entry = array
            .element(index, stride)
            .ok_or("naval detail nested element overflow")?;
        let entry = if indirect {
            read::<u64>(memory, entry).ok_or("naval detail nested pointer unreadable")? as usize
        } else {
            entry
        };
        let row = decode(entry)?;
        match &mut output {
            EntryOutput::Owned(rows) => rows.push(row),
            EntryOutput::Slice(rows) => rows[*length + index as usize] = row,
        }
    }
    *length = end;
    Ok(span)
}

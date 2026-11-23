use super::*;

pub fn walk_slots_buffer(
    memory: &impl Memory,
    base: usize,
    country_pointer: usize,
    country: u32,
    mut output: engine_native_memory::walk::Buffer<'_, cresearch_slot::Row>,
) -> Outcome {
    let result = (|| {
        let status = status(memory, base, country_pointer)?;
        let array = Array::read(
            memory,
            status
                .checked_add(RESEARCH_SLOTS_DATA)
                .ok_or("research slots address overflow")?,
            POINTER_STRIDE,
        )
        .ok_or("research slots header invalid")?;
        output.ensure(array.count as usize)?;
        let len = output.len().min(array.count as usize);
        for (index, row) in output[..len].iter_mut().enumerate() {
            let pointer = read::<u64>(
                memory,
                array
                    .element(index as u32, POINTER_STRIDE)
                    .ok_or("research slots address overflow")?,
            )
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("research slot pointer unreadable or null")?;
            *row = cresearch_slot::row(memory, base, pointer, status, country)?;
            if row.index >= array.count {
                return Err("research slot index outside roster");
            }
        }
        output[..len].sort_unstable_by_key(|row| row.index);
        if output[..len]
            .windows(2)
            .any(|pair| pair[0].index == pair[1].index)
        {
            return Err("duplicate research slot index");
        }
        let unread_rows = array.count - len as u32;
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

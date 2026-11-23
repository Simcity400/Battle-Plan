
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029f_1860);
pub const RESEARCHED_DATA: usize = 0x40;
pub const TECHNOLOGIES_DATA: usize = 0x88;
pub const RESEARCH_SLOTS_DATA: usize = 0xa0;
pub const RESEARCH_SLOTS_CAPACITY: usize = 0xa8;
pub const RESEARCH_SLOTS_COUNT: usize = 0xac;
pub const LIMITED_BONUSES: usize = 0xe8;
pub const COST_REDUCTIONS: usize = 0x100;
pub const NEXT_BONUS_ID: usize = 0x13c;
pub const COUNTRY: usize = 0x130;
pub const LAND_DOCTRINE_LEVEL: usize = 0x140;
pub const POINTER_STRIDE: usize = 8;

pub const FRESH_INIT_RVA: Rva = Rva(0x00f6_0650);
pub const FRESH_INIT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x56, 0x57, 0x41, 0x54, 0x41, 0x56,
];
pub const RESEARCH_COMPLETED_RVA: Rva = Rva(0x00f6_0950);
pub const RESEARCH_COMPLETED_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55,
];
pub const REBUILD_OR_TRANSFER_A_RVA: Rva = Rva(0x00f5_ede0);
pub const REBUILD_OR_TRANSFER_B_RVA: Rva = Rva(0x00f5_ca80);

use engine_native_memory::{Memory, cpdx_array::Array, read, read_slice, walk::Outcome};
use engine_native::ccountry;
use crate::{cresearch_slot, ctechnology, ctechnology_template};

fn status(memory: &impl Memory, base: usize, country: usize) -> Result<usize, &'static str> {
    let raw = copy_status(memory, country)?;
    decode_status(base, country, &raw)
}

pub struct StatusRaw {
    pub pointer: usize,
    pub vtable: [u8; 8],
    pub backlink: [u8; 8],
    pub scattered: usize,
}

pub fn copy_status(memory: &impl Memory, country: usize) -> Result<StatusRaw, &'static str> {
    let address = country
        .checked_add(ccountry::TECHNOLOGY_STATUS)
        .ok_or("research country address overflow")?;
    let pointer = read::<u64>(memory, address)
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("country technology status unreadable or null")?;
    let mut vtable = [0; 8];
    let mut backlink = [0; 8];
    let scattered = memory.copy_scattered_prefetched(
        [
            pointer,
            pointer
                .checked_add(COUNTRY)
                .ok_or("technology status address overflow")?,
        ],
        &mut [&mut vtable, &mut backlink],
    );
    Ok(StatusRaw {
        pointer,
        vtable,
        backlink,
        scattered,
    })
}

pub fn decode_status(base: usize, country: usize, raw: &StatusRaw) -> Result<usize, &'static str> {
    if raw.scattered != 2 {
        return Err("technology status identity unreadable");
    }
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64;
    if u64::from_le_bytes(raw.vtable) != expected {
        return Err("technology status vtable mismatch");
    }
    if u64::from_le_bytes(raw.backlink) != country as u64 {
        return Err("technology status country backlink mismatch");
    }
    Ok(raw.pointer)
}

pub fn land_doctrine_level(
    memory: &impl Memory,
    base: usize,
    country: usize,
) -> Result<i32, &'static str> {
    let pointer = status(memory, base, country)?;
    read::<i32>(
        memory,
        pointer
            .checked_add(LAND_DOCTRINE_LEVEL)
            .ok_or("land doctrine address overflow")?,
    )
    .ok_or("land doctrine level unreadable")
}

pub fn walk_technologies(
    memory: &impl Memory,
    base: usize,
    country_pointer: usize,
    country: u32,
    output: (&mut Vec<ctechnology::Row>, &mut Vec<usize>),
    start: usize,
    definitions: &mut ctechnology_template::Dictionary<'_>,
) -> Outcome {
    let (output, source_pointers) = output;
    assert_eq!(
        source_pointers.len(),
        output.len(),
        "technology pointer scratch must match row capacity"
    );
    let result = (|| {
        let status = status(memory, base, country_pointer)?;
        let array = Array::read(
            memory,
            status
                .checked_add(TECHNOLOGIES_DATA)
                .ok_or("technology roster address overflow")?,
            POINTER_STRIDE,
        )
        .ok_or("technology roster header invalid")?;
        start
            .checked_add(array.count as usize)
            .ok_or("technology row count overflow")?;
        if start > output.len() {
            return Err("technology row start exceeds storage");
        }
        definitions.begin_roster();
        let mut len = 0;
        let mut first = 0;
        let mut pointers = [0_u8; ctechnology::BATCH * POINTER_STRIDE];
        let mut selected = [0; ctechnology::BATCH];
        let mut selected_indices = [0; ctechnology::BATCH];
        let mut instance_bytes = [0; ctechnology::BATCH_BYTES];
        while first < array.count {
            let count = (array.count - first).min(ctechnology::BATCH as u32);
            let address = array
                .element(first, POINTER_STRIDE)
                .ok_or("technology roster address overflow")?;
            if !read_slice(
                memory,
                address,
                &mut pointers[..count as usize * POINTER_STRIDE],
            ) {
                return Err("technology roster pointers unreadable");
            }
            let mut selected_len = 0;
            for (ordinal, bytes) in pointers[..count as usize * POINTER_STRIDE]
                .chunks_exact(POINTER_STRIDE)
                .enumerate()
            {
                let pointer = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                    .map_err(|_| "technology pointer overflow")?;
                if pointer == 0 {
                    continue;
                }
                ctechnology::hint(pointer);
                selected[selected_len] = pointer;
                selected_indices[selected_len] = i32::try_from(first as usize + ordinal + 1)
                    .map_err(|_| "technology database index overflow")?;
                selected_len += 1;
            }
            if selected_len != 0 {
                let copied = ctechnology::copy_batch(
                    memory,
                    &selected[..selected_len],
                    &mut instance_bytes,
                );
                let done = (copied / ctechnology::FIELDS.len()).min(selected_len);
                for index in done..selected_len {
                    let pointer = selected[index];
                    let offset = index * ctechnology::INSTANCE_BYTES;
                    if let Some(previous) =
                        selected[..index].iter().position(|&seen| seen == pointer)
                    {
                        let src = previous * ctechnology::INSTANCE_BYTES;
                        instance_bytes
                            .copy_within(src..src + ctechnology::INSTANCE_BYTES, offset);
                    } else {
                        let out: &mut [u8; ctechnology::INSTANCE_BYTES] =
                            (&mut instance_bytes[offset..offset + ctechnology::INSTANCE_BYTES])
                                .try_into()
                                .unwrap();
                        if ctechnology::copy_one(memory, pointer, out)
                            != ctechnology::FIELDS.len()
                        {
                            return Err("technology fields unreadable");
                        }
                    }
                }
                let required = start + len + selected_len;
                if required > output.len() {
                    let additional = required - output.len();
                    crate::allocation::reserve(output, additional)
                        .map_err(|_| "technology row allocation failed")?;
                    let additional = required - source_pointers.len();
                    crate::allocation::reserve(source_pointers, additional)
                        .map_err(|_| "technology pointer allocation failed")?;
                    output.resize(required, ctechnology::Row::default());
                    source_pointers.resize(required, 0);
                }
                let batch_start = len;
                ctechnology::decode_batch(
                    base,
                    status,
                    country,
                    &instance_bytes[..selected_len * ctechnology::INSTANCE_BYTES],
                    |mut instance| {
                        instance.row.database_index = selected_indices[len - batch_start];
                        let handle = instance.row.technology_handle;
                        definitions.admit_instance(
                            memory,
                            selected[len - batch_start],
                            instance.template,
                            handle,
                            instance.row.database_index,
                            &output[start..start + len],
                        )?;
                        source_pointers[start + len] = selected[len - batch_start];
                        output[start + len] = instance.row;
                        len += 1;
                        Ok(())
                    },
                )?;
            }
            first += count;
        }
        let output = &mut output[start..start + len];
        let source_pointers = &source_pointers[start..start + len];
        let researched = Array::read(
            memory,
            status
                .checked_add(RESEARCHED_DATA)
                .ok_or("researched roster address overflow")?,
            POINTER_STRIDE,
        )
        .ok_or("researched roster header invalid")?;
        if researched.count > array.count {
            return Err("researched roster exceeds all-technology roster");
        }
        let mut previous = None;
        first = 0;
        while first < researched.count {
            let count = (researched.count - first).min(ctechnology::BATCH as u32);
            if !read_slice(
                memory,
                researched
                    .element(first, POINTER_STRIDE)
                    .ok_or("researched roster address overflow")?,
                &mut pointers[..count as usize * POINTER_STRIDE],
            ) {
                return Err("researched technology pointers unreadable");
            }
            let mut retained = [None; ctechnology::BATCH];
            let mut selected_len = 0;
            for (row, bytes) in retained[..count as usize]
                .iter_mut()
                .zip(pointers[..count as usize * POINTER_STRIDE].chunks_exact(POINTER_STRIDE))
            {
                let pointer = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                    .ok()
                    .filter(|&p| p != 0)
                    .ok_or("researched technology pointer unreadable or null")?;
                *row = definitions.instance_pointer_row(pointer).or_else(|| {
                    source_pointers[..len]
                        .iter()
                        .position(|&retained| retained == pointer)
                });
                if row.is_none() {
                    selected[selected_len] = pointer;
                    selected_len += 1;
                }
            }
            if selected_len != 0 {
                let copied = ctechnology::copy_identities(
                    memory,
                    &selected[..selected_len],
                    &mut instance_bytes,
                );
                let done = (copied / ctechnology::IDENTITY_FIELDS.len()).min(selected_len);
                for acquired in done..selected_len {
                    let pointer = selected[acquired];
                    let start = acquired * ctechnology::IDENTITY_BYTES;
                    if let Some(previous) =
                        selected[..acquired].iter().position(|&seen| seen == pointer)
                    {
                        let src = previous * ctechnology::IDENTITY_BYTES;
                        instance_bytes
                            .copy_within(src..src + ctechnology::IDENTITY_BYTES, start);
                    } else {
                        let out: &mut [u8; ctechnology::IDENTITY_BYTES] =
                            (&mut instance_bytes[start..start + ctechnology::IDENTITY_BYTES])
                                .try_into()
                                .unwrap();
                        if ctechnology::copy_identity_one(memory, pointer, out)
                            != ctechnology::IDENTITY_FIELDS.len()
                        {
                            return Err("technology fields unreadable");
                        }
                    }
                }
            }
            let expected = engine_sites::at(base, ctechnology::VTABLE_RVA)
                .ok_or("module address overflow")? as u64;
            let mut acquired = 0;
            for retained in &retained[..count as usize] {
                let handle = if let Some(index) = *retained {
                    output[index].technology_handle
                } else {
                    let start = acquired * ctechnology::IDENTITY_BYTES;
                    acquired += 1;
                    let identity = ctechnology::decode_identity(
                        expected,
                        status,
                        &instance_bytes[start..start + ctechnology::IDENTITY_BYTES],
                    )?;
                    identity.handle
                };
                if previous.is_some_and(|previous: u32| previous as i32 >= handle as i32) {
                    return Err("researched technology roster is not strictly ordered");
                }
                previous = Some(handle);
                match *retained {
                    Some(index) => {
                        output[index].researched = 1;
                    }
                    None => {
                        return Err(
                            if output[..len]
                                .iter()
                                .any(|row| row.technology_handle == handle)
                            {
                                "researched technology pointer disagrees with country roster"
                            } else {
                                "researched technology missing from country roster"
                            },
                        );
                    }
                }
            }
            first += count;
        }
        Ok(Outcome::Rows { len })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

pub fn count_slots(memory: &impl Memory, base: usize, country_pointer: usize, country: u32) -> Outcome {
    walk_slots_buffer(memory, base, country_pointer, country, engine_native_memory::walk::Buffer::Count)
}

pub fn walk_slots(
    memory: &impl Memory,
    base: usize,
    country_pointer: usize,
    country: u32,
    output: &mut Vec<cresearch_slot::Row>,
) -> Outcome {
    walk_slots_buffer(
        memory,
        base,
        country_pointer,
        country,
        engine_native_memory::walk::Buffer::Growing(output),
    )
}

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
        let batches = crate::governance_batch::Assignments::gather(memory, array, len, false);
        let batched = batches.view(memory);
        let memory = &batched;
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

#[cfg(test)]
#[path = "ctechnology_status/border/tests.rs"]
mod border_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "ctechnology_status/serial_slots.rs"]
mod serial_slots;

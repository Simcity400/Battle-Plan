pub mod region_weather;
pub mod province_weather;

use crate::{
    Memory,
    cgame_state,
    cpdx_array::Array,
    memory::{FieldBatch, RecordField, RecordLayout, RecordSources},
    read,
    cweather_manager::{
        province_weather as sweather_per_province,
        region_weather as sweather_per_region,
    },
    walk::Outcome,
};
use engine_sites::Rva;

mod display_grid;
pub use display_grid::{DisplayGrid, display_grid_size, read_display_grid};

pub const VTABLE_RVA: Rva = Rva(0x029f_4138);
pub const PROVINCES: usize = 0x10;
pub const PROVINCES_COUNT: usize = 0x1c;
pub const REGIONS: usize = 0x40;
pub const REGIONS_COUNT: usize = 0x4c;
pub const UPDATE_CURSOR: usize = 0x58;
pub const PROVINCES_PER_UPDATE: usize = 0x2f0;
pub const REGION_CURSOR: usize = 0x5c;
pub const REGIONS_PER_UPDATE: usize = 0x2f4;
pub const SEED: usize = 0x308;
pub const MAX_SNOW: usize = 0x1f8;
pub const DISPLAY_SNOW: usize = 0x310;
pub const DISPLAY_MUD: usize = 0x328;
pub const DISPLAY_REFRESH_INTERVAL: usize = 0x300;
pub const DISPLAY_REFRESH_IDENTITY: usize = 0x340;
pub const DISPLAY_GRID_SNOW: usize = 0x388;
#[cfg(any(test, feature = "test-support"))]
pub const DISPLAY_GRID_MUD: usize = 0x390;
pub const DISPLAY_WORK_COUNT: usize = 0x3b4;
pub const DISPLAY_WORK_QUEUE: usize = 0x3a8;
pub const DISPLAY_WORK_CAPACITY: usize = 0x3b0;
pub const PROVINCE_CLOCKS: usize = 0x370;
pub const DAYLIGHT_DIRECTION: usize = 0x350;
pub const INITIAL_MODE: usize = 0x348;
pub const SUSPEND_FLAGS: usize = 0x349;
pub const HOURLY_UPDATE_RVA: Rva = Rva(0x00f9_f440);

pub struct Reader<'a, M> {
    memory: &'a M,
    base: usize,
    pointer: usize,
    movement_provinces: MovementHeader,
    movement_regions: MovementHeader,
    clocks: std::cell::Cell<Option<Result<Array, &'static str>>>,
}

#[derive(Default)]
struct MovementHeader {
    count: std::cell::Cell<Option<Result<i32, &'static str>>>,
    data: std::cell::Cell<Option<Result<usize, &'static str>>>,
}

fn original_attempt<T: Copy>(
    memo: &std::cell::Cell<Option<Result<T, &'static str>>>,
    observe: impl FnOnce() -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    if let Some(original) = memo.get() {
        return original;
    }
    let original = observe();
    memo.set(Some(original));
    original
}

impl<'a, M: Memory> Reader<'a, M> {
    pub fn new(memory: &'a M, base: usize, game_state: usize) -> Result<Self, &'static str> {
        let address = game_state
            .checked_add(cgame_state::WEATHER_MANAGER)
            .ok_or("weather manager address overflow")?;
        let pointer = read::<u64>(memory, address)
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|pointer| *pointer != 0)
            .ok_or("weather manager pointer unreadable or absent")?;
        Self::with_manager(memory, base, pointer)
    }

    fn with_manager(memory: &'a M, base: usize, pointer: usize) -> Result<Self, &'static str> {
        let vtable = engine_sites::at(base, VTABLE_RVA).ok_or("weather manager vtable withheld")?;
        if read::<u64>(memory, pointer) != Some(vtable as u64) {
            return Err("weather manager vtable mismatch");
        }
        Ok(Self {
            memory,
            base,
            pointer,
            movement_provinces: MovementHeader::default(),
            movement_regions: MovementHeader::default(),
            clocks: std::cell::Cell::new(None),
        })
    }

    pub fn max_snow(&self) -> Result<i64, &'static str> {
        self.pointer
            .checked_add(MAX_SNOW)
            .and_then(|address| read::<i64>(self.memory, address))
            .ok_or("weather snow ceiling unreadable")
    }

    pub fn temperature_words(&self) -> Result<(i32, i32, u32), &'static str> {
        let word = |offset: usize| {
            self.pointer
                .checked_add(offset)
                .and_then(|address| read::<u32>(self.memory, address))
                .ok_or("weather temperature pass words unreadable")
        };
        Ok((
            word(UPDATE_CURSOR)? as i32,
            word(PROVINCES_PER_UPDATE)? as i32,
            word(SEED)?,
        ))
    }

    pub fn suspend_flags(&self) -> Result<[u8; 2], &'static str> {
        let address = self
            .pointer
            .checked_add(SUSPEND_FLAGS)
            .ok_or("weather suspension address overflow")?;
        let mut flags = [0; 2];
        if !crate::read_slice(self.memory, address, &mut flags) {
            return Err("weather suspension flags unreadable");
        }
        Ok(flags)
    }

    pub fn daylight_direction(&self) -> Result<[i64; 3], &'static str> {
        self.pointer
            .checked_add(DAYLIGHT_DIRECTION)
            .and_then(|address| read::<[u8; 24]>(self.memory, address))
            .map(|bytes| {
                std::array::from_fn(|index| {
                    i64::from_le_bytes(bytes[index * 8..index * 8 + 8].try_into().unwrap())
                })
            })
            .ok_or("weather daylight direction unreadable")
    }

    pub fn daylight_feather(&self) -> Result<[i64; 2], &'static str> {
        let word = |key| {
            engine_sites::at(self.base, key)
                .and_then(|address| read::<i64>(self.memory, address))
                .ok_or("held weather daylight feather unreadable")
        };
        Ok([
            word(crate::air_fuel_defines::CONTEXT_SUBTRACT_RVA)?,
            word(crate::air_fuel_defines::CONTEXT_SCALE_RVA)?,
        ])
    }

    pub fn region_count(&self) -> Result<usize, &'static str> {
        self.pointer
            .checked_add(REGIONS)
            .and_then(|address| Array::read(self.memory, address, sweather_per_region::STRIDE))
            .map(|array| array.count as usize)
            .ok_or("weather region array unreadable or invalid")
    }

    pub fn province_count(&self) -> Result<usize, &'static str> {
        self.pointer
            .checked_add(PROVINCES)
            .and_then(|address| Array::read(self.memory, address, sweather_per_province::STRIDE))
            .map(|array| array.count as usize)
            .ok_or("weather province array unreadable or invalid")
    }

    pub fn regions(&self, output: &mut [sweather_per_region::Row]) -> Outcome {
        let max_snow = match self.max_snow() {
            Ok(value) => value,
            Err(reason) => return Outcome::Unavailable { reason },
        };
        let Some(array) = self
            .pointer
            .checked_add(REGIONS)
            .and_then(|address| Array::read(self.memory, address, sweather_per_region::STRIDE))
        else {
            return Outcome::Unavailable {
                reason: "weather region array unreadable or invalid",
            };
        };
        let outcome = sweather_per_region::walk(self.memory, self.base, array, output);
        for row in &mut output[..outcome.len()] {
            row.max_snow = max_snow;
        }
        outcome
    }

    pub fn custom_sea_into(&self, region_id: i32, output: &mut [i16]) -> Result<(), &'static str> {
        let count = output.len();
        if count == 0 {
            return Ok(());
        }
        let record = self.movement_record(region_id, true)?;
        if record == 0 {
            return Err("naval custom sea weather region absent");
        }
        let data = read::<u64>(self.memory, record + 0x30)
            .ok_or("naval custom sea values pointer unreadable")? as usize;
        let len = count
            .checked_mul(2)
            .ok_or("naval custom sea length overflow")?;
        let bytes =
            unsafe { std::slice::from_raw_parts_mut(output.as_mut_ptr().cast::<u8>(), len) };
        if data == 0 || !crate::read_slice(self.memory, data, bytes) {
            return Err("naval custom sea values unreadable");
        }
        Ok(())
    }

    pub fn custom_sea(&self, region_id: i32, count: u32) -> Result<Vec<i16>, &'static str> {
        let mut values = crate::allocation::filled(count as usize, 0)
            .map_err(|_| "naval custom sea allocation failed")?;
        self.custom_sea_into(region_id, &mut values)?;
        Ok(values)
    }

    pub fn display_arrays(&self) -> Result<[Array; 2], &'static str> {
        let header = |offset| {
            self.pointer
                .checked_add(offset)
                .and_then(|address| Array::read(self.memory, address, 1))
                .ok_or("weather display array unreadable or invalid")
        };
        Ok([header(DISPLAY_SNOW)?, header(DISPLAY_MUD)?])
    }

    pub fn province_clocks(&self) -> Result<Array, &'static str> {
        original_attempt(&self.clocks, || {
            self.pointer
                .checked_add(PROVINCE_CLOCKS)
                .and_then(|address| {
                    Array::read(self.memory, address, sweather_per_province::CLOCK_STRIDE)
                })
                .ok_or("weather clock array unreadable or invalid")
        })
    }

    pub fn provinces(&self, output: &mut [sweather_per_province::Row]) -> Outcome {
        let Some(array) = self
            .pointer
            .checked_add(PROVINCES)
            .and_then(|address| Array::read(self.memory, address, sweather_per_province::STRIDE))
        else {
            return Outcome::Unavailable {
                reason: "weather province array unreadable or invalid",
            };
        };
        let outcome = sweather_per_province::walk(self.memory, self.base, array, output);
        if matches!(outcome, Outcome::Unavailable { .. }) {
            return outcome;
        }
        let len = outcome.len();
        match self.display_arrays().and_then(|arrays| {
            sweather_per_province::fill_display(
                self.memory,
                arrays,
                array.count,
                &mut output[..len],
            )?;
            sweather_per_province::fill_clocks(
                self.memory,
                self.province_clocks()?,
                array.count,
                &mut output[..len],
            )
        }) {
            Ok(()) => outcome,
            Err(reason) => Outcome::Unavailable { reason },
        }
    }
}

pub fn movement_modifiers(
    memory: &impl Memory,
    base: usize,
    state: usize,
    province: usize,
) -> Result<(usize, usize), &'static str> {
    let reader = Reader::new(memory, base, state)?;
    let fields = FieldBatch::gather(
        memory,
        movement_fields(
            province,
            [
                (crate::cprovince::ID, 4),
                (crate::cprovince::STRATEGIC_REGION, 8),
            ],
        ),
    );
    let province_id = movement_batched::<i32, 2>(memory, &fields, 0, province)?;
    let province_record = reader.movement_record(province_id, false)?;
    let province_accumulator = if province_record == 0 {
        0
    } else {
        province_record
            .checked_add(sweather_per_province::MODIFIERS)
            .ok_or("movement weather address overflow")?
    };
    let region = movement_batched::<u64, 2>(memory, &fields, 1, province)?;
    let region_accumulator = if region == 0 {
        0
    } else {
        let region = usize::try_from(region).map_err(|_| "movement region pointer invalid")?;
        let expected = engine_sites::at(base, crate::cstrategic_region::VTABLE_RVA)
            .ok_or("movement region vtable withheld")?;
        let fields = FieldBatch::gather(
            memory,
            movement_fields(region, [(0, 8), (crate::cstrategic_region::ID, 4)]),
        );
        if movement_batched::<u64, 2>(memory, &fields, 0, region)? != expected as u64 {
            return Err("movement region vtable mismatch");
        }
        let id = movement_batched::<i32, 2>(memory, &fields, 1, region)?;
        let record = reader.movement_record(id, true)?;
        if record == 0 {
            0
        } else {
            record
                .checked_add(sweather_per_region::MODIFIERS)
                .ok_or("movement weather address overflow")?
        }
    };
    Ok((province_accumulator, region_accumulator))
}

impl<M: Memory> Reader<'_, M> {
    pub(crate) fn movement_record(&self, id: i32, region: bool) -> Result<usize, &'static str> {
        if id < 0 {
            return Ok(0);
        }
        let (data_offset, count_offset, stride, vtable_rva, id_offset, header) = if region {
            (
                REGIONS,
                REGIONS_COUNT,
                sweather_per_region::STRIDE,
                sweather_per_region::VTABLE_RVA,
                sweather_per_region::ID,
                &self.movement_regions,
            )
        } else {
            (
                PROVINCES,
                PROVINCES_COUNT,
                sweather_per_province::STRIDE,
                sweather_per_province::VTABLE_RVA,
                sweather_per_province::ID,
                &self.movement_provinces,
            )
        };
        let count = original_attempt(&header.count, || {
            movement_field::<i32>(self.memory, self.pointer, count_offset)
        })?;
        if id >= count {
            return Ok(0);
        }
        let data = original_attempt(&header.data, || {
            let data = movement_field::<u64>(self.memory, self.pointer, data_offset)?;
            let data = usize::try_from(data).map_err(|_| "movement weather pointer invalid")?;
            if data == 0 {
                return Err("movement weather array absent");
            }
            Ok(data)
        })?;
        let record = (id as usize)
            .checked_mul(stride)
            .and_then(|offset| data.checked_add(offset))
            .ok_or("movement weather address overflow")?;
        let expected =
            engine_sites::at(self.base, vtable_rva).ok_or("movement weather vtable withheld")?;
        let fields = FieldBatch::gather(
            self.memory,
            movement_fields(record, [(0, 8), (id_offset, 4)]),
        );
        if movement_batched::<u64, 2>(self.memory, &fields, 0, record)? != expected as u64 {
            return Err("movement weather row vtable mismatch");
        }
        if movement_batched::<i32, 2>(self.memory, &fields, 1, record)? != id {
            return Err("movement weather row id mismatch");
        }
        Ok(record)
    }
}

fn movement_fields<const N: usize>(
    pointer: usize,
    fields: [(usize, usize); N],
) -> [(usize, usize); N] {
    fields.map(|(offset, len)| {
        (
            pointer
                .checked_add(offset)
                .filter(|_| pointer != 0)
                .unwrap_or(0),
            len,
        )
    })
}

fn movement_batched<T: crate::memory::Raw, const N: usize>(
    memory: &impl Memory,
    fields: &FieldBatch<N>,
    index: usize,
    pointer: usize,
) -> Result<T, &'static str> {
    if pointer == 0 {
        return Err("movement weather source absent");
    }
    fields
        .get(memory, index)
        .ok_or("movement weather field unreadable")
}

fn movement_field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    if pointer == 0 {
        return Err("movement weather source absent");
    }
    pointer
        .checked_add(offset)
        .and_then(|address| read(memory, address))
        .ok_or("movement weather field unreadable")
}

pub(crate) const DENSE_BATCH: usize = 64;
pub(crate) const DENSE_RECORD_BYTES: usize = 64;

pub(crate) fn walk_dense_records<R>(
    memory: &impl Memory,
    array: Array,
    stride: usize,
    fields: &[RecordField],
    record_stride: usize,
    output: &mut [R],
    decode: impl FnMut(u32, &[u8], usize) -> Result<R, &'static str>,
) -> Outcome {
    let view = (|| {
        if record_stride > DENSE_RECORD_BYTES {
            return None;
        }
        let len = (array.count as usize).min(output.len());
        let field_end = fields.iter().try_fold(0_usize, |end, field| {
            if field.len == 0 {
                Some(end)
            } else {
                Some(end.max(field.source_offset.checked_add(field.len)?))
            }
        })?;
        if field_end == 0 || field_end > stride {
            return None;
        }
        let size = len
            .checked_sub(1)?
            .checked_mul(stride)?
            .checked_add(field_end)?;
        memory.immutable_view(array.data, size)
    })();
    if let Some(view) = view {
        return walk_dense_records_from(
            &view,
            array,
            stride,
            fields,
            record_stride,
            output,
            decode,
        );
    }
    walk_dense_records_from(memory, array, stride, fields, record_stride, output, decode)
}

fn walk_dense_records_from<R>(
    memory: &impl Memory,
    array: Array,
    stride: usize,
    fields: &[RecordField],
    record_stride: usize,
    output: &mut [R],
    mut decode: impl FnMut(u32, &[u8], usize) -> Result<R, &'static str>,
) -> Outcome {
    if record_stride > DENSE_RECORD_BYTES {
        return Outcome::Unavailable {
            reason: "weather record layout exceeds batch storage",
        };
    }
    let len = (array.count as usize).min(output.len());
    let layout = RecordLayout::new(fields, record_stride);
    let mut pointers = [0; DENSE_BATCH];
    let mut bytes = [0; DENSE_BATCH * DENSE_RECORD_BYTES];
    let mut first = 0;
    while first < len {
        let count = (len - first).min(DENSE_BATCH);
        let mut overflow = None;
        let sources = if array.element((first + count - 1) as u32, stride).is_some() {
            RecordSources::Strided {
                base: array.element(first as u32, stride).unwrap(),
                stride,
                count,
            }
        } else {
            for (slot, pointer) in pointers[..count].iter_mut().enumerate() {
                match array.element((first + slot) as u32, stride) {
                    Some(address) => *pointer = address,
                    None => {
                        overflow = Some(slot);
                        break;
                    }
                }
            }
            RecordSources::Pointers(&pointers[..overflow.unwrap_or(count)])
        };
        let staged = overflow.unwrap_or(count);
        let completed = layout.as_ref().map_or(0, |layout| {
            memory.copy_records(sources, layout, &mut bytes[..staged * record_stride])
        });
        for slot in 0..count {
            if overflow == Some(slot) {
                return Outcome::Unavailable {
                    reason: "weather row address overflow",
                };
            }
            let copied = completed
                .saturating_sub(slot * fields.len())
                .min(fields.len());
            let record = &bytes[slot * record_stride..(slot + 1) * record_stride];
            match decode((first + slot) as u32, record, copied) {
                Ok(row) => output[first + slot] = row,
                Err(reason) => return Outcome::Unavailable { reason },
            }
        }
        first += count;
    }
    let unread_rows = array.count - len as u32;
    if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    }
}

#[cfg(test)]
mod tests;

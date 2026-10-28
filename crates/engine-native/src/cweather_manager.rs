//! Weather simulation arrays and independent graphics-refresh controls.
//! Hourly weather jobs finish before capture and display-byte publication follows
//! the simulation joins. Graphics copying separately requires zero outstanding
//! work and an unchanged refresh token; the timestamp alone proves no completion.

use crate::{
    Memory, cgame_state,
    cpdx_array::Array,
    memory::{FieldBatch, RecordField, RecordLayout, RecordSources},
    read, sweather_per_province, sweather_per_region,
    walk::Outcome,
};

mod display_grid;
pub use display_grid::{DisplayGrid, display_grid_size, read_display_grid};

/// CWeatherManager vtable.
pub const VTABLE_RVA: usize = 0x029f_3fd8;
/// CWeatherManager contiguous SWeatherPerProvince array header.
pub const PROVINCES: usize = 0x10;
/// CWeatherManager province array count, i32.
pub const PROVINCES_COUNT: usize = 0x1c;
/// CWeatherManager contiguous SWeatherPerRegion array header.
pub const REGIONS: usize = 0x40;
/// CWeatherManager region array count, i32.
pub const REGIONS_COUNT: usize = 0x4c;
/// Province update cursor, i32: the weather update pass visits the window from it and stores where
/// it stopped, wrapping to zero at the province count (`PROVINCES_COUNT`).
pub const UPDATE_CURSOR: usize = 0x58;
/// `provinces_per_update`, i32, which the content loader loads from
/// common/weather.txt; an ordinary weather update pass visits that many slots.
pub const PROVINCES_PER_UPDATE: usize = 0x2f0;
/// Region update cursor, i32: the weather update pass visits the region window from it and wraps it
/// to zero at the region count (`REGIONS_COUNT`).
pub const REGION_CURSOR: usize = 0x5c;
/// `regions_per_update`, i32, loaded by the content loader beside
/// [`PROVINCES_PER_UPDATE`]; an ordinary weather update pass visits that many
/// region slots.
pub const REGIONS_PER_UPDATE: usize = 0x2f4;
/// Weather random seed, u32: the hourly update adds a synchronized 31-bit
/// random integer before the pass, and province weather calculation hashes each
/// province id plus it.
pub const SEED: usize = 0x308;
/// Weather manager snow ceiling, native i64 fixed point; province and region
/// weather calculations read all eight bytes. Construction zeroes it and only
/// the content loader, reached from load initialization, writes it.
pub const MAX_SNOW: usize = 0x1f8;
/// Display-snow byte array indexed by province id and sized to the live
/// province count. Changed weather updates the bytes, then the graphics grid
/// averages them.
pub const DISPLAY_SNOW: usize = 0x310;
/// CWeatherManager display mud bytes (0 or 255), the same layout as [`DISPLAY_SNOW`].
pub const DISPLAY_MUD: usize = 0x328;
/// Content-loaded fixed5 graphics refresh interval.
pub const DISPLAY_REFRESH_INTERVAL: usize = 0x300;
/// Graphics refresh-start f64 bits, stored before rebuilding.
pub const DISPLAY_REFRESH_IDENTITY: usize = 0x340;
/// Current snow/mud grid pointers, sized to the live quarter-map dimensions.
pub const DISPLAY_GRID_SNOW: usize = 0x388;
#[cfg(any(test, feature = "test-support"))]
pub const DISPLAY_GRID_MUD: usize = 0x390;
/// Pending graphics cells, i32; remains nonzero until threaded rebuilding finishes.
pub const DISPLAY_WORK_COUNT: usize = 0x3b4;
/// Province day/night array, sized to the live weather province count.
pub const PROVINCE_CLOCKS: usize = 0x370;
/// Held seasonal direction used by regional day/night forecasts.
pub const DAYLIGHT_DIRECTION: usize = 0x350;
/// Initial-mode byte: initialization sets it before the hourly update, whose weather update pass
/// then visits every province and region in as many passes as its stored pass count; the hourly
/// update clears it afterward.
pub const INITIAL_MODE: usize = 0x348;
/// Hourly-update gates: weather updating tests both bytes from `SUSPEND_FLAGS` before advancing
/// weather and day/night. The weather-simulation reset command can change the second.
pub const SUSPEND_FLAGS: usize = 0x349;
/// CWeatherManager hourly update.
pub const HOURLY_UPDATE_RVA: usize = 0x00f9_f3d0;

/// An authenticated manager for one callback. Each table reads its own header,
/// so failure of one weather array does not make the other table unavailable.
pub struct Reader<'a, M> {
    memory: &'a M,
    base: usize,
    pointer: usize,
    // Movement selectors share these original header attempts for this Reader's
    // callback lifetime. A second region cannot retry a failed common count or
    // data pointer. The dense table walkers retain their independent headers.
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

    /// Authenticate an already read, nonnull manager pointer.
    fn with_manager(memory: &'a M, base: usize, pointer: usize) -> Result<Self, &'static str> {
        let vtable = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")?;
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

    /// The snow ceiling, which every region row carries.
    pub fn max_snow(&self) -> Result<i64, &'static str> {
        self.pointer
            .checked_add(MAX_SNOW)
            .and_then(|address| read::<i64>(self.memory, address))
            .ok_or("weather snow ceiling unreadable")
    }

    /// The update cursor, window and seed the hourly temperature pass reads.
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

    /// Stored simulation gates, read together without normalising their bytes.
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

    /// Only held/suspended clocks need this observation. Ordinary updated
    /// direction derives from the hour without acquiring another field.
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

    /// The native feather words may retain initialization while updates are
    /// suspended; only an ordinary completed update proves content derivation.
    pub fn daylight_feather(&self) -> Result<[i64; 2], &'static str> {
        let word = |offset| {
            self.base
                .checked_add(offset)
                .and_then(|address| read::<i64>(self.memory, address))
                .ok_or("held weather daylight feather unreadable")
        };
        Ok([
            word(crate::air_fuel_defines::CONTEXT_SUBTRACT_RVA)?,
            word(crate::air_fuel_defines::CONTEXT_SCALE_RVA)?,
        ])
    }

    /// Attach sizing only, using the live signed province and region counts
    /// with their respective array strides.
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

    /// Select original region root once.
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

    /// Active i16 regional vector, bounded by the authenticated strategic
    /// template's count. Reached only for a live naval combat's region.
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
        // i16 has alignment 2 and the Windows game target is little endian.
        // The arena is owned and sized before entering the hourly block.
        let bytes =
            unsafe { std::slice::from_raw_parts_mut(output.as_mut_ptr().cast::<u8>(), len) };
        if data == 0 || !crate::read_slice(self.memory, data, bytes) {
            return Err("naval custom sea values unreadable");
        }
        Ok(())
    }

    /// Convenience wrapper for fixture readers; the live capture uses the
    /// bounded pack arena above and performs no allocation in the block.
    pub fn custom_sea(&self, region_id: i32, count: u32) -> Result<Vec<i16>, &'static str> {
        let mut values = crate::allocation::filled(count as usize, 0)
            .map_err(|_| "naval custom sea allocation failed")?;
        self.custom_sea_into(region_id, &mut values)?;
        Ok(values)
    }

    /// The display snow and mud array headers.
    pub fn display_arrays(&self) -> Result<[Array; 2], &'static str> {
        let header = |offset| {
            self.pointer
                .checked_add(offset)
                .and_then(|address| Array::read(self.memory, address, 1))
                .ok_or("weather display array unreadable or invalid")
        };
        Ok([header(DISPLAY_SNOW)?, header(DISPLAY_MUD)?])
    }

    /// Stored day/night rows, joined by the ordinary weather array's slot.
    /// Suspension flags can hold their values, including a reset local hour zero.
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

    /// Every province's snow and display bytes.
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

/// Passive division movement-weather inputs. Zero means the indexed getter
/// has no modifier, distinct from read failure. Region weather uses the authored
/// region id rather than its runtime roster slot.
pub fn movement_modifiers(
    memory: &impl Memory,
    base: usize,
    state: usize,
    province: usize,
) -> Result<(usize, usize), &'static str> {
    let reader = Reader::new(memory, base, state)?;
    // The province's id and region pointer are one guarded call.
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
        let expected = base
            .checked_add(crate::cstrategic_region::VTABLE_RVA)
            .ok_or("movement region vtable overflow")?;
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
    /// Authenticate a row within the signed bounds, without walking the dense
    /// array or reading unrelated weather fields.
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
        // The data pointer is read only for an admitted id, as main reads it;
        // the row's vtable and id are one guarded call.
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
        let expected = self
            .base
            .checked_add(vtable_rva)
            .ok_or("movement weather vtable overflow")?;
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

/// Gather fields of the object at `pointer`; a null object or overflowing
/// field fails at that field.
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

/// A gathered field, consulted exactly as `movement_field` would read it.
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

/// Rows per batched dense gather and the exact bytes reserved for each record.
pub(crate) const DENSE_BATCH: usize = 64;
pub(crate) const DENSE_RECORD_BYTES: usize = 64;

/// Weather arrays store rows directly, including id zero. Their validated count
/// supplies the exact number of rows omitted when the caller's buffer is full.
///
/// Dense weather rows share one exact field layout, so a whole batch crosses
/// the guarded boundary once instead of once per row. Decoding still runs per
/// row in slot order with that row's own copied-field prefix, so every value,
/// validator and first-failure reason is the one a per-row gather produced.
/// A decode failure stops the walk exactly where it did before; the only
/// difference is that the batch's later rows were already copied.
pub(crate) fn walk_dense_records<R>(
    memory: &impl Memory,
    array: Array,
    stride: usize,
    fields: &[RecordField],
    record_stride: usize,
    output: &mut [R],
    decode: impl FnMut(u32, &[u8], usize) -> Result<R, &'static str>,
) -> Outcome {
    // Only the admitted rows' exact fields need a view. Both weather layouts
    // leave trailing allocation padding, so this is a proper subrange even
    // when every row fits. Recorded declines aliases and overriding spans;
    // live/recording sources retain the original guarded-read path.
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

/// Both sources use the same batch scheduling, field-prefix validation and
/// row decoder. Borrowing owned bytes changes no capacity or outcome rule.
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
            // A valid last address proves the preceding nonnegative offsets.
            // Keep the native array strided so the guarded reader validates its
            // envelope once, without rebuilding and checking every row pointer.
            RecordSources::Strided {
                base: array.element(first as u32, stride).unwrap(),
                stride,
                count,
            }
        } else {
            // Hand-constructed or overflowing arrays retain the old exact
            // readable prefix and the row-address error at its original slot.
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

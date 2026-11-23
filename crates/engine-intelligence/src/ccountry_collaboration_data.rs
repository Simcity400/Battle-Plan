
use engine_sites::Rva;
use engine_native_memory::Memory;

pub const VTABLE_RVA: Rva = Rva(0x02a3_76f8);
pub const TARGET_COUNTRY: usize = 0x08;
pub const PROGRESS: usize = 0x10;
pub const FIXED_POINT_SCALE: i64 = 100_000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub source: u32,
    pub target: u32,
    pub progress: i64,
}

pub(crate) fn read_row(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    source: u32,
) -> Result<Row, &'static str> {
    let vtable = engine_sites::at(base, VTABLE_RVA)
        .ok_or("module address overflow")?;
    let target_address = pointer
        .checked_add(TARGET_COUNTRY)
        .ok_or("collaboration relation address overflow")?;
    let progress_address = pointer
        .checked_add(PROGRESS)
        .ok_or("collaboration relation address overflow")?;
    let mut actual_vtable = [0; 8];
    let mut target = [0; 4];
    let mut progress = [0; 8];
    let completed = memory.copy_scattered(
        [pointer, target_address, progress_address],
        &mut [&mut actual_vtable, &mut target, &mut progress],
    );
    if completed < 1 || u64::from_le_bytes(actual_vtable) != vtable as u64 {
        return Err("collaboration relation vtable mismatch or unreadable");
    }
    if completed < 2 {
        return Err("collaboration target unreadable");
    }
    if completed < 3 {
        return Err("collaboration progress unreadable");
    }
    Ok(Row {
        source,
        target: u32::from_le_bytes(target),
        progress: i64::from_le_bytes(progress),
    })
}

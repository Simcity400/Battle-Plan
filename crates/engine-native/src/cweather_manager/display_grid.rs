//! Live snow/mud display-grid boundary, including attachment after a full rebuild.
//! A changed province queues its cells clamped one short of the grid, so an ordinary
//! weather change never reaches the last row and column; only a whole-grid mark
//! publishes every cell, and only that can move the boundary. No load history is
//! needed. The refresh token is stored before copying and rebuilding; the queued
//! count clears only after job joins. Read the token, require no whole-grid mark
//! queued, select and copy, require it again, then compare the token. Equality alone
//! can admit a running rebuild. Never retry or wait.

use super::*;

/// Positive live map dimensions quartered for the display grid.
pub fn display_grid_size(memory: &impl Memory, base: usize) -> Result<[u32; 2], &'static str> {
    let map = engine_sites::at(base, crate::cmap::SINGLETON_RVA)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("weather display map absent or unreadable")?;
    let address = map
        .checked_add(crate::cmap::WIDTH)
        .ok_or("weather display dimensions overflow")?;
    let mut bytes = [0; 8];
    if !crate::read_slice(memory, address, &mut bytes) {
        return Err("weather display dimensions unreadable");
    }
    let dimensions = [
        i32::from_le_bytes(bytes[..4].try_into().unwrap()),
        i32::from_le_bytes(bytes[4..].try_into().unwrap()),
    ];
    if dimensions.iter().any(|&size| size / 4 <= 0) {
        return Err("weather display grid has no cells");
    }
    Ok(dimensions.map(|size| (size / 4) as u32))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayGrid {
    pub size: [u32; 2],
    pub refresh_interval: i64,
    /// An equality token only, never a replay or viewer animation clock.
    pub refresh_identity: u64,
    manager: usize,
    planes: [usize; 2],
}

impl DisplayGrid {
    pub fn edge_bytes(&self) -> Result<usize, &'static str> {
        (self.size[0] as usize)
            .checked_add(self.size[1] as usize)
            .and_then(|cells| cells.checked_sub(1))
            .and_then(|cells| cells.checked_mul(2))
            .ok_or("weather display edge length overflow")
    }

    /// Planar snow then mud. Each plane is the right column (south first,
    /// excluding its northern corner), then the complete northern row.
    /// Only these bytes are read; no whole grid or pre-attach baseline.
    pub fn read_edges(&self, memory: &impl Memory, output: &mut [u8]) -> Result<(), &'static str> {
        let required = self.edge_bytes()?;
        let output = output
            .get_mut(..required)
            .ok_or("weather display edge capacity exhausted")?;
        let [width, height] = self.size.map(|size| size as usize);
        let cells = required / 2;
        const FIELD: [RecordField; 1] = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 1,
        }];
        const LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&FIELD, 1).expect("valid edge layout");
        for (plane, bytes) in self.planes.into_iter().zip(output.chunks_exact_mut(cells)) {
            let column = plane
                .checked_add(width - 1)
                .ok_or("weather display edge address overflow")?;
            if memory.copy_records(
                RecordSources::Strided {
                    base: column,
                    stride: width,
                    count: height - 1,
                },
                &LAYOUT,
                &mut bytes[..height - 1],
            ) != height - 1
            {
                return Err("weather display edge column unreadable");
            }
            let row = width
                .checked_mul(height - 1)
                .and_then(|offset| plane.checked_add(offset))
                .ok_or("weather display edge address overflow")?;
            if !crate::read_slice(memory, row, &mut bytes[height - 1..]) {
                return Err("weather display edge row unreadable");
            }
        }
        require_boundary_unqueued(memory, self.manager, self.size)?;
        if read_display_identity(memory, self.manager)? != self.refresh_identity {
            return Err("weather display refreshed during boundary copy");
        }
        Ok(())
    }
}

impl<M: Memory> Reader<'_, M> {
    pub fn display_grid(&self) -> Result<DisplayGrid, &'static str> {
        read_display_grid(self.memory, self.base, self.pointer)
    }
}

fn read_display_identity(memory: &impl Memory, manager: usize) -> Result<u64, &'static str> {
    movement_field(memory, manager, DISPLAY_REFRESH_IDENTITY)
        .map_err(|_| "weather display refresh identity unreadable")
}

/// Refuses a queue that can still move the boundary: a whole-grid mark, which holds the
/// last column's first index at that same position. A changed province never queues a
/// last-column cell, however often its box repeats cells, so any other queue is interior.
fn require_boundary_unqueued(
    memory: &impl Memory,
    manager: usize,
    size: [u32; 2],
) -> Result<(), &'static str> {
    let queued: i32 = movement_field(memory, manager, DISPLAY_WORK_COUNT)
        .map_err(|_| "weather display work count unreadable")?;
    let corner = size[0] as usize - 1;
    let queued = usize::try_from(queued).map_err(|_| "weather display work count invalid")?;
    if queued <= corner {
        return Ok(());
    }
    // The queue's storage (pointer and capacity, one read) must hold the count read above, the
    // entry is read only inside it, and the storage is read again unchanged: a count that ran
    // ahead of the storage it was read with, or storage moved during the read, refuses rather
    // than reads past it.
    let storage = || movement_field::<[u8; 12]>(memory, manager, DISPLAY_WORK_QUEUE).map_err(|_| "weather display work queue unreadable");
    let held = storage()?;
    let pointer = u64::from_le_bytes(held[..8].try_into().unwrap());
    let capacity = i32::from_le_bytes(held[DISPLAY_WORK_CAPACITY - DISPLAY_WORK_QUEUE..][..4].try_into().unwrap());
    if usize::try_from(capacity).map_or(true, |capacity| queued > capacity) || pointer == 0 {
        return Err("weather display work queue inconsistent");
    }
    let entry: i32 = usize::try_from(pointer)
        .ok()
        .and_then(|queue| queue.checked_add(corner.checked_mul(4)?))
        .and_then(|address| read(memory, address))
        .ok_or("weather display work queue unreadable")?;
    if storage()? != held {
        return Err("weather display work queue moved during its read");
    }
    if usize::try_from(entry) == Ok(corner) {
        return Err("weather display whole-grid rebuild queued");
    }
    Ok(())
}

/// Exact header read for an already authenticated manager (the raw copy keeps
/// its successful root gather instead of observing that pointer/vtable twice).
pub fn read_display_grid(
    memory: &impl Memory,
    base: usize,
    manager: usize,
) -> Result<DisplayGrid, &'static str> {
    let size = display_grid_size(memory, base)?;
    let extent = (size[0] as usize)
        .checked_mul(size[1] as usize)
        .ok_or("weather display grid extent overflow")?;
    // Keep these separate and ordered: the timestamp is stored before work
    // finishes, so selecting it in a header batch cannot certify completion.
    let refresh_identity = read_display_identity(memory, manager)?;
    require_boundary_unqueued(memory, manager, size)?;
    let fields = FieldBatch::gather(
        memory,
        movement_fields(
            manager,
            [(DISPLAY_REFRESH_INTERVAL, 8), (DISPLAY_GRID_SNOW, 16)],
        ),
    );
    let refresh_interval = fields
        .get::<i64>(memory, 0)
        .ok_or("weather display refresh interval unreadable")?;
    let bytes = fields
        .get::<[u8; 16]>(memory, 1)
        .ok_or("weather display grid pointers unreadable")?;
    let pointers = [
        u64::from_le_bytes(bytes[..8].try_into().unwrap()),
        u64::from_le_bytes(bytes[8..].try_into().unwrap()),
    ];
    let mut planes = [0; 2];
    for (plane, pointer) in planes.iter_mut().zip(pointers) {
        *plane = usize::try_from(pointer)
            .ok()
            .filter(|&p| p != 0 && p.checked_add(extent).is_some())
            .ok_or("weather display grid absent or invalid")?;
    }
    Ok(DisplayGrid {
        size,
        refresh_interval,
        refresh_identity,
        manager,
        planes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::BTreeMap};

    #[derive(Default)]
    struct Source {
        bytes: BTreeMap<usize, u8>,
        reads: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }
    impl Source {
        fn put(&mut self, address: usize, bytes: &[u8]) {
            self.bytes.extend(
                bytes
                    .iter()
                    .enumerate()
                    .map(|(index, &byte)| (address + index, byte)),
            );
        }
        fn grid() -> Self {
            let mut source = Self::default();
            source.put(crate::cmap::SINGLETON_RVA.0, &0x2800_u64.to_le_bytes());
            source.put(0x2800 + crate::cmap::WIDTH, &12_i32.to_le_bytes());
            source.put(0x2800 + crate::cmap::HEIGHT, &8_i32.to_le_bytes());
            source.put(0x2000, &(VTABLE_RVA.0 as u64).to_le_bytes());
            source.put(
                0x2000 + DISPLAY_REFRESH_INTERVAL,
                &(-9_007_199_254_740_993_i64).to_le_bytes(),
            );
            source.put(
                0x2000 + DISPLAY_REFRESH_IDENTITY,
                &22.25_f64.to_bits().to_le_bytes(),
            );
            source.put(0x2000 + DISPLAY_WORK_COUNT, &0_i32.to_le_bytes());
            source.put(0x2000 + DISPLAY_GRID_SNOW, &0x3000_u64.to_le_bytes());
            source.put(0x2000 + DISPLAY_GRID_MUD, &0x3100_u64.to_le_bytes());
            source.put(0x2000 + DISPLAY_WORK_QUEUE, &0x3200_u64.to_le_bytes());
            source.put(0x3000, &[0, 1, 2, 3, 4, 5]);
            source.put(0x3100, &[20, 21, 22, 23, 24, 25]);
            source
        }
    }
    impl Memory for Source {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, destination.len()));
            if self.fault.is_some_and(|fault| {
                (address..address.saturating_add(destination.len())).contains(&fault)
            }) {
                return false;
            }
            for (index, byte) in destination.iter_mut().enumerate() {
                let Some(&value) = self.bytes.get(&(address + index)) else {
                    return false;
                };
                *byte = value;
            }
            true
        }
    }

    #[test]
    fn weather_display_edges_attach_after_rebuild_and_read_only_the_live_boundary() {
        let source = Source::grid();
        let reader = Reader::with_manager(&source, 0, 0x2000).unwrap();
        let grid = reader.display_grid().unwrap();
        assert_eq!(grid.size, [3, 2]);
        assert_eq!(grid.refresh_interval, -9_007_199_254_740_993);
        assert_eq!(grid.refresh_identity, 22.25_f64.to_bits());
        assert_eq!(grid.edge_bytes(), Ok(8));
        let mut edges = [0; 8];
        grid.read_edges(&source, &mut edges).unwrap();
        assert_eq!(edges, [2, 3, 4, 5, 22, 23, 24, 25]);
        assert!(
            !source
                .reads
                .borrow()
                .iter()
                .any(|&(address, _)| address == 0x3000 || address == 0x3100),
            "no interior read and no startup baseline"
        );
        assert_eq!(
            grid.read_edges(&source, &mut edges[..7]),
            Err("weather display edge capacity exhausted")
        );
        // Counts come from the live map, including odd-sized mod rasters.
        let mut odd = Source::grid();
        odd.put(0x2800 + crate::cmap::WIDTH, &18_i32.to_le_bytes());
        odd.put(0x2800 + crate::cmap::HEIGHT, &10_i32.to_le_bytes());
        assert_eq!(display_grid_size(&odd, 0), Ok([4, 2]));
        odd.put(0x2800 + crate::cmap::HEIGHT, &3_i32.to_le_bytes());
        assert_eq!(
            display_grid_size(&odd, 0),
            Err("weather display grid has no cells")
        );
    }

    #[test]
    fn weather_display_edges_refuse_each_failed_plane_without_retrying() {
        for fault in [0x3002, 0x3004, 0x3102, 0x3104] {
            let mut source = Source::grid();
            source.fault = Some(fault);
            let reader = Reader::with_manager(&source, 0, 0x2000).unwrap();
            assert!(
                reader
                    .display_grid()
                    .unwrap()
                    .read_edges(&source, &mut [0; 8])
                    .is_err()
            );
            assert_eq!(
                source
                    .reads
                    .borrow()
                    .iter()
                    .filter(|&&(address, len)| (address..address + len).contains(&fault))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn weather_display_completion_guard_orders_token_work_edges_work_token() {
        let source = Source::grid();
        let reader = Reader::with_manager(&source, 0, 0x2000).unwrap();
        source.reads.borrow_mut().clear();
        let grid = reader.display_grid().unwrap();
        grid.read_edges(&source, &mut [0; 8]).unwrap();
        let reads = source.reads.borrow();
        let token = (0x2000 + DISPLAY_REFRESH_IDENTITY, 8);
        let work = (0x2000 + DISPLAY_WORK_COUNT, 4);
        // The grid's size is read first; then the token and the work count open the read,
        // before any edge cell of the planes (0x3000 to 0x4000).
        let opened = reads.iter().position(|&read| read == token).expect("the token is read");
        assert_eq!(&reads[opened..opened + 2], &[token, work]);
        assert!(reads[..opened].iter().all(|&(address, _)| !(0x3000..0x4000).contains(&address)), "no edge before the token: {reads:?}");
        assert_eq!(&reads[reads.len() - 2..], &[work, token]);
        assert_eq!(reads.iter().filter(|&&read| read == token).count(), 2);
        assert_eq!(reads.iter().filter(|&&read| read == work).count(), 2);
    }

    #[test]
    fn weather_display_completion_guard_refuses_only_a_queued_whole_grid_mark_before_edges() {
        let queued = |entries: &[i32]| {
            let mut source = Source::grid();
            source.put(
                0x2000 + DISPLAY_WORK_COUNT,
                &(entries.len() as i32).to_le_bytes(),
            );
            source.put(
                0x2000 + DISPLAY_WORK_CAPACITY,
                &(entries.len() as i32).to_le_bytes(),
            );
            for (index, entry) in entries.iter().enumerate() {
                source.put(0x3200 + index * 4, &entry.to_le_bytes());
            }
            source
        };
        // Changed provinces queue interior cells only, repeating the cells their boxes
        // share, so the count can pass the grid's six cells and the boundary still reads.
        for entries in [&[1][..], &[0, 1, 1, 0, 1], &[0, 1, 0, 1, 1, 0, 0]] {
            let source = queued(entries);
            let reader = Reader::with_manager(&source, 0, 0x2000).unwrap();
            let mut edges = [0; 8];
            reader
                .display_grid()
                .unwrap()
                .read_edges(&source, &mut edges)
                .unwrap();
            assert_eq!(edges, [2, 3, 4, 5, 22, 23, 24, 25]);
        }
        // A whole-grid mark queues every index in order; only it reaches the last column.
        let mark = queued(&[0, 1, 2, 3, 4, 5]);
        let reader = Reader::with_manager(&mark, 0, 0x2000).unwrap();
        assert_eq!(
            reader.display_grid(),
            Err("weather display whole-grid rebuild queued")
        );
        assert!(
            !mark
                .reads
                .borrow()
                .iter()
                .any(|&(address, _)| (0x3000..0x3200).contains(&address))
        );
        // A count past the capacity it was read with (a header torn while the queue grows)
        // refuses without reading an entry past the storage.
        let mut torn = queued(&[0, 1, 0, 1, 1, 0, 0]);
        torn.put(0x2000 + DISPLAY_WORK_CAPACITY, &4_i32.to_le_bytes());
        let reader = Reader::with_manager(&torn, 0, 0x2000).unwrap();
        assert_eq!(
            reader.display_grid(),
            Err("weather display work queue inconsistent")
        );
        assert!(
            !torn
                .reads
                .borrow()
                .iter()
                .any(|&(address, _)| (0x3200..0x3300).contains(&address))
        );
        let mut negative = Source::grid();
        negative.put(0x2000 + DISPLAY_WORK_COUNT, &(-1_i32).to_le_bytes());
        let reader = Reader::with_manager(&negative, 0, 0x2000).unwrap();
        assert_eq!(
            reader.display_grid(),
            Err("weather display work count invalid")
        );
        let mut unreadable = queued(&[0, 1, 0, 1]);
        unreadable.fault = Some(0x3208);
        let reader = Reader::with_manager(&unreadable, 0, 0x2000).unwrap();
        assert_eq!(
            reader.display_grid(),
            Err("weather display work queue unreadable")
        );
        let mut source = Source::grid();
        source.fault = Some(0x2000 + DISPLAY_WORK_COUNT);
        let reader = Reader::with_manager(&source, 0, 0x2000).unwrap();
        assert_eq!(
            reader.display_grid(),
            Err("weather display work count unreadable")
        );
        assert_eq!(
            source
                .reads
                .borrow()
                .iter()
                .filter(|&&(address, _)| address == 0x2000 + DISPLAY_WORK_COUNT)
                .count(),
            1
        );
    }
}

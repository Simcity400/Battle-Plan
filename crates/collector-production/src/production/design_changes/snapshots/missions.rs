use super::*;
use std::ops::Range;

pub(super) struct Proof {
    invariant: Vec<usize>,
    root: usize,
    width: usize,
    #[cfg(test)]
    pub(super) key_reads: std::cell::Cell<usize>,
}
fn overlaps(regions: &[Range<usize>], address: usize, len: usize) -> bool {
    let first = regions.partition_point(|region| region.end <= address);
    regions
        .get(first)
        .is_some_and(|region| region.start < address + len)
}
fn same_missions(
    memory: &impl Memory,
    pointer: usize,
    expected: &[rows::VariantMissionStats],
    mut read: impl FnMut(usize, usize) -> Option<()>,
    mut region: impl FnMut(Range<usize>) -> Option<()>,
) -> Option<bool> {
    let address = pointer.checked_add(native::MISSION_GROUP_DATA)?;
    let missions = Array::read(memory, address, 40)?;
    if missions.count as usize != expected.len() {
        return Some(false);
    }
    if missions.count != 0 {
        region(missions.data..missions.data.checked_add(expected.len().checked_mul(40)?)?)?;
    }
    for (ordinal, expected) in expected.iter().enumerate() {
        let mission = missions.element(ordinal as u32, 40)?;
        read(mission, 4)?;
        if engine_native::read::<u32>(memory, mission)? != expected.mask {
            return Some(false);
        }
        let header = mission.checked_add(16)?;
        read(header, 16)?;
        let stats = Array::read(memory, header, 16)?;
        if stats.count as usize != expected.stats.len() {
            return Some(false);
        }
        if stats.count != 0 {
            region(
                stats.data
                    ..stats
                        .data
                        .checked_add(expected.stats.len().checked_mul(16)?)?,
            )?;
        }
        for (ordinal, expected) in expected.stats.iter().enumerate() {
            let address = stats.element(ordinal as u32, 16)?;
            read(address, 16)?;
            let bytes = engine_native::read::<[u8; 16]>(memory, address)?;
            if u32::from_le_bytes(bytes[..4].try_into().ok()?) != expected.index
                || i64::from_le_bytes(bytes[8..].try_into().ok()?) != expected.value
            {
                return Some(false);
            }
        }
    }
    Some(true)
}
pub(super) fn model_key(
    memory: &impl Memory,
    pointer: usize,
    spans: &[collector_raw::raw::Span],
) -> Option<u64> {
    let missions = Array::read(memory, pointer.checked_add(native::MISSION_GROUP_DATA)?, 40)?;
    if missions.count == 0 {
        return None;
    }
    let mut regions = engine_allocation::vec(missions.count as usize + 1).ok()?;
    regions.push(
        missions.data
            ..missions
                .data
                .checked_add((missions.count as usize).checked_mul(40)?)?,
    );
    let mut hash = DefaultHasher::new();
    pointer.hash(&mut hash);
    engine_native::read::<[u8; 16]>(memory, pointer.checked_add(native::BONUS_SOURCES)?)?
        .hash(&mut hash);
    missions.count.hash(&mut hash);
    for ordinal in 0..missions.count {
        let mission = missions.element(ordinal, 40)?;
        engine_native::read::<u32>(memory, mission)?.hash(&mut hash);
        let stats = Array::read(memory, mission.checked_add(16)?, 16)?;
        stats.count.hash(&mut hash);
        if stats.count != 0 {
            regions.push(
                stats.data
                    ..stats
                        .data
                        .checked_add((stats.count as usize).checked_mul(16)?)?,
            );
        }
        for ordinal in 0..stats.count {
            let bytes = engine_native::read::<[u8; 16]>(memory, stats.element(ordinal, 16)?)?;
            hash.write(&bytes[..4]);
            hash.write(&bytes[8..]);
        }
    }
    regions.sort_unstable_by_key(|region| region.start);
    if regions.windows(2).any(|pair| pair[0].end > pair[1].start) {
        return None;
    }
    for span in spans {
        let first = regions.partition_point(|region| region.end <= span.addr);
        if regions.get(first).is_some_and(|region| {
            region.start <= span.addr
                && span
                    .addr
                    .checked_add(span.len as usize)
                    .is_some_and(|end| end <= region.end)
        }) {
            continue;
        }
        // Only mission storage may relocate within one invariant read layout.
        (span.addr, span.len, span.ok, span.bulk).hash(&mut hash);
    }
    Some(hash.finish())
}
impl Proof {
    pub fn build(
        memory: &impl Memory,
        witness: &witness::Witness,
        pointer: usize,
        expected: &[rows::VariantMissionStats],
    ) -> Option<Self> {
        if expected.is_empty() {
            return None;
        }
        let address = pointer.checked_add(native::MISSION_GROUP_DATA)?;
        let mut regions = engine_allocation::vec(1).ok()?;
        regions.push(address..address.checked_add(16)?);
        let mut reads = Vec::new();
        if !same_missions(
            memory,
            pointer,
            expected,
            |address, len| {
                engine_allocation::reserve(&mut reads, 1).ok()?;
                reads.push((address, len, false));
                Some(())
            },
            |region| {
                engine_allocation::reserve(&mut regions, 1).ok()?;
                regions.push(region);
                Some(())
            },
        )? {
            return None;
        }
        regions.sort_unstable_by_key(|region| region.start);
        // Aliased mission storage cannot hide an invariant use of the same bytes.
        if regions.windows(2).any(|pair| pair[0].end > pair[1].start) {
            return None;
        }
        reads.sort_unstable_by_key(|&(address, len, _)| (address, len));
        let mut invariant = engine_allocation::vec(witness.requests.len()).ok()?;
        let mut root = None;
        let mut width = 0;
        for (index, request) in witness.requests.iter().enumerate() {
            let len = request.bytes.len();
            if request.address == address && len == 128 {
                if root.replace(index).is_some() {
                    return None;
                }
            } else if overlaps(&regions, request.address, len) {
                let found = reads
                    .binary_search_by_key(&(request.address, len), |&(address, len, _)| {
                        (address, len)
                    })
                    .ok()?;
                if std::mem::replace(&mut reads[found].2, true) {
                    return None;
                }
                continue;
            }
            width = width.max(len);
            invariant.push(index);
        }
        if reads.iter().any(|read| !read.2) {
            return None;
        }
        Some(Self {
            invariant,
            root: root?,
            width,
            #[cfg(test)]
            key_reads: std::cell::Cell::new(0),
        })
    }
    pub fn same_reads(
        &self,
        witness: &witness::Witness,
        other: &Self,
        other_witness: &witness::Witness,
    ) -> bool {
        self.invariant
            .iter()
            .map(|&index| {
                let request = &witness.requests[index];
                (request.address, request.bytes.len(), index == self.root)
            })
            .eq(other.invariant.iter().map(|&index| {
                let request = &other_witness.requests[index];
                (request.address, request.bytes.len(), index == other.root)
            }))
    }
    fn invariant_bytes<'a>(&self, index: usize, bytes: &'a [u8]) -> &'a [u8] {
        if index == self.root {
            &bytes[16..]
        } else {
            bytes
        }
    }
    pub fn key(
        &self,
        memory: &impl Memory,
        witness: &witness::Witness,
        model_key: u64,
        buffer: &mut Vec<u8>,
    ) -> Option<u64> {
        engine_allocation::reserve(buffer, self.width.saturating_sub(buffer.len())).ok()?;
        buffer.resize(self.width, 0);
        let mut hash = DefaultHasher::new();
        model_key.hash(&mut hash);
        self.invariant.len().hash(&mut hash);
        for &index in &self.invariant {
            let request = &witness.requests[index];
            (request.address, request.bytes.len(), index == self.root).hash(&mut hash);
            let actual = &mut buffer[..request.bytes.len()];
            #[cfg(test)]
            self.key_reads.set(self.key_reads.get() + 1);
            if !memory.copy(request.address, actual) {
                return None;
            }
            hash.write(self.invariant_bytes(index, actual));
        }
        Some(hash.finish())
    }
    pub fn equal(
        &self,
        memory: &impl Memory,
        witness: &witness::Witness,
        pointer: usize,
        expected: &[rows::VariantMissionStats],
        buffer: &mut Vec<u8>,
    ) -> Option<bool> {
        engine_allocation::reserve(buffer, self.width.saturating_sub(buffer.len())).ok()?;
        buffer.resize(self.width, 0);
        for &index in &self.invariant {
            let request = &witness.requests[index];
            let actual = &mut buffer[..request.bytes.len()];
            if !memory.copy(request.address, actual) {
                return Some(false);
            }
            let expected = &witness.bytes[request.bytes.clone()];
            if self.invariant_bytes(index, actual) != self.invariant_bytes(index, expected) {
                return Some(false);
            }
        }
        // Rebuilds replace nested mission allocations; their addresses are not design operands.
        same_missions(memory, pointer, expected, |_, _| Some(()), |_| Some(()))
    }
}

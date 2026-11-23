use engine_native_memory::{Memory, cpdx_array::Array, read};
use crate::supply_parent_map::Map;
use engine_sites::Rva;

pub const ROUTE_MAP: usize = 0xf0;
pub const OUTER_STRIDE: usize = 0x30;
pub const INNER_HEADER: usize = 0x18;
pub const INNER_STRIDE: usize = 0xa0;
pub const VALUE: usize = 0x10;
pub const PATH: usize = 0;
pub const FROM: usize = 0x80;
pub const TO: usize = 0x82;
pub const BREAK: usize = 0x87;
pub const EMPTY_RVA: Rva = Rva(0x033e_af50);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub from: u16,
    pub to: u16,
    pub break_after: u8,
    pub start: usize,
    pub len: usize,
}

#[derive(Default)]
pub(crate) struct Memo {
    edges: std::collections::HashMap<(u32, u32), Row>,
    worker: bool,
    pub(crate) inner: InnerMemo,
}

impl Memo {
    pub fn try_with_scratch() -> std::io::Result<Self> {
        Ok(Self {
            worker: true,
            inner: InnerMemo::try_with_scratch()?,
            ..Self::default()
        })
    }

    pub fn begin(&mut self) {
        self.edges.clear();
        self.inner.begin();
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.edges.capacity() * std::mem::size_of::<((u32, u32), Row)>()
            + self.inner.allocated_bytes()
    }

    pub(crate) fn find(&self, parent: u32, province: u32) -> (Option<Row>, bool) {
        (self.edges.get(&(parent, province)).copied(), self.worker)
    }

    pub(crate) fn insert(
        &mut self,
        admit: bool,
        parent: u32,
        province: u32,
        row: Row,
    ) -> Result<(), &'static str> {
        if admit {
            engine_allocation::reserve_map(&mut self.edges, 1)
                .map_err(|_| "supply edge memo allocation failed")?;
            self.edges.insert((parent, province), row);
        }
        Ok(())
    }
}

pub fn read_outer(memory: &impl Memory, root: usize) -> Result<Map, &'static str> {
    Map::read(memory, root + ROUTE_MAP, OUTER_STRIDE, 4)
}

#[derive(Default)]
pub struct InnerMemo {
    parent: Option<u32>,
    map: Option<Map>,
    parents: std::collections::HashMap<u32, Option<Map>>,
    worker: bool,
    empty_identity: Option<[u8; 8]>,
}

impl InnerMemo {
    pub fn try_with_scratch() -> std::io::Result<Self> {
        Ok(Self {
            worker: true,
            ..Self::default()
        })
    }

    pub fn begin(&mut self) {
        self.parents.clear();
        self.parent = None;
        self.map = None;
        self.empty_identity = None;
    }

    fn allocated_bytes(&self) -> usize {
        self.parents.capacity() * std::mem::size_of::<(u32, Option<Map>)>()
    }

    fn resolve(
        &mut self,
        memory: &impl Memory,
        outer: &Map,
        parent: u32,
    ) -> Result<Option<Map>, &'static str> {
        if self.parent == Some(parent) {
            return Ok(self.map);
        }
        if let Some(map) = self.parents.get(&parent).copied() {
            self.parent = Some(parent);
            self.map = map;
            return Ok(map);
        }
        let map = match outer.find(memory, parent)? {
            Some(bucket) => Some(Map::read(memory, bucket + INNER_HEADER, INNER_STRIDE, 4)?),
            None => None,
        };
        if self.worker {
            engine_allocation::reserve_map(&mut self.parents, 1)
                .map_err(|_| "supply parent memo allocation failed")?;
            self.parents.insert(parent, map);
        }
        self.parent = Some(parent);
        self.map = map;
        Ok(map)
    }
}

pub fn read_edge_buffer(
    memory: &impl Memory,
    base: usize,
    outer: &Map,
    inner: &mut InnerMemo,
    parent: u32,
    province: u32,
    mut output: engine_native_memory::walk::Buffer<'_, u32>,
) -> Result<(Row, usize), &'static str> {
    let empty = engine_sites::at(base, EMPTY_RVA).ok_or("empty supply route withheld")?;
    let mut pointer = empty;
    if let Some(map) = inner.resolve(memory, outer, parent)?
        && let Some(bucket) = map.find(memory, province)?
    {
        pointer = bucket + VALUE;
    }
    let identity = if pointer == empty {
        if let Some(identity) = inner.empty_identity {
            identity
        } else {
            let identity = read::<[u8; 8]>(memory, pointer + FROM)
                .ok_or("supply route identity unreadable")?;
            inner.empty_identity = Some(identity);
            identity
        }
    } else {
        read::<[u8; 8]>(memory, pointer + FROM).ok_or("supply route identity unreadable")?
    };
    let from = u16::from_le_bytes(identity[..2].try_into().unwrap());
    let to = u16::from_le_bytes(identity[2..4].try_into().unwrap());
    let mut row = Row {
        from,
        to,
        break_after: identity[BREAK - FROM],
        ..Row::default()
    };
    let required = if from == 0 {
        0
    } else {
        if u32::from(from) != parent || u32::from(to) != province {
            return Err("supply route endpoints differ from lookup");
        }
        let path = Array::read(memory, pointer + PATH, 4).ok_or("supply route path invalid")?;
        output.ensure(path.count as usize)?;
        if output.len() >= path.count as usize {
            engine_native_memory::cpdx_array::copy_u32s(
                memory,
                path,
                &mut output[..path.count as usize],
                "supply route provinces unreadable",
            )?;
            row.len = path.count as usize;
        }
        path.count as usize
    };
    Ok((row, required))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_memo_retains_original_edges_and_clears_each_walk() {
        let mut memo = Memo::try_with_scratch().unwrap();
        memo.begin();
        for province in 1..=65_u32 {
            let (cached, admit) = memo.find(3, province);
            assert!(cached.is_none());
            let row = Row {
                from: 3,
                start: province as usize,
                len: 3,
                ..Row::default()
            };
            memo.insert(admit, 3, province, row).unwrap();
            assert_eq!(memo.find(3, province).0, Some(row));
        }
        let bytes = memo.allocated_bytes();
        memo.begin();
        assert!(memo.find(3, 65).0.is_none());
        assert_eq!(memo.allocated_bytes(), bytes);
        assert_eq!(Memo::default().find(3, 1), (None, false));
        assert_eq!(Memo::default().allocated_bytes(), 0);
    }

    #[test]
    fn parent_memo_retains_observations_and_reauthenticates_each_walk() {
        let mut bytes = [0; 1024];
        bytes[..8].copy_from_slice(&0x1100_u64.to_le_bytes());
        bytes[12..16].copy_from_slice(&7_u32.to_le_bytes());
        bytes[16] = 4;
        bytes[0x100 + 12 * OUTER_STRIDE + 4] = 255;
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let outer = Map::read(&memory, 0x1000, OUTER_STRIDE, 4).unwrap();
        let mut memo = InnerMemo::try_with_scratch().unwrap();
        for parent in 1..=65 {
            assert!(memo.resolve(&memory, &outer, parent).unwrap().is_none());
        }
        assert_eq!(memo.parents.len(), 65);
        let bytes = memo.allocated_bytes();
        memo.begin();
        assert!(memo.parents.is_empty());
        assert!(memo.parent.is_none());
        assert!(memo.empty_identity.is_none());
        assert!(memo.resolve(&memory, &outer, 1).unwrap().is_none());
        assert_eq!(memo.parents.len(), 1);
        assert_eq!(memo.allocated_bytes(), bytes);
        let mut vacant = InnerMemo::default();
        assert!(vacant.resolve(&memory, &outer, 1).unwrap().is_none());
        assert_eq!(vacant.allocated_bytes(), 0);
    }
}

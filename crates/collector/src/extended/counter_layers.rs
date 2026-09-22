//! Hourly snapshot of retained counter collection and first-fit source order.
use crate::{
    sampling::qpc,
    spooling::table,
    table_capture::{Arena, CaptureRows, grow_vec},
};
use engine::{
    Memory, cmap_icon_group, cunits_stack_map_icon, map_icon_group_collection as native,
    walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{CounterGroup, CounterLayerRow, CounterSource, TableId, tables},
};
pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::CounterLayers];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    arenas: [usize; 4],
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 34,
            arenas: [4096, 4096, 8192, 32768],
        }
    }
}
pub struct Captured {
    rows: CaptureRows<native::Row>,
    sources: Arena<cunits_stack_map_icon::SourceRow>,
    groups: Arena<cmap_icon_group::OrderRow>,
    members: Arena<u32>,
    units: Arena<u64>,
    index: Vec<(usize, u32)>,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            arenas: [0; 4],
        })
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(c.rows),
            sources: Arena::new(c.arenas[0]),
            groups: Arena::new(c.arenas[1]),
            members: Arena::new(c.arenas[2]),
            units: Arena::new(c.arenas[3]),
            index: vec![(0, 0); c.arenas[0]],
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.sources.reset();
        self.groups.reset();
        self.members.reset();
        self.units.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.sources.allocated_bytes()
            + self.groups.allocated_bytes()
            + self.members.allocated_bytes()
            + self.units.allocated_bytes()
            + self.index.capacity() * std::mem::size_of::<(usize, u32)>()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        self.rows.grow(&mut c.rows);
        self.sources.grow(&mut c.arenas[0]);
        self.groups.grow(&mut c.arenas[1]);
        self.members.grow(&mut c.arenas[2]);
        self.units.grow(&mut c.arenas[3]);
        grow_vec(&mut self.index, c.arenas[0]);
    }
    fn read(&mut self, memory: &impl Memory, base: usize, state: usize) -> native::OrderWalk {
        native::walk_order(
            memory,
            base,
            state,
            native::OrderBuffers {
                rows: &mut self.rows.rows,
                sources: &mut self.sources.first,
                groups: &mut self.groups.first,
                members: &mut self.members.first,
                units: &mut self.units.first,
                index: &mut self.index,
            },
        )
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        let captured = self.read(memory, base, state);
        self.rows.outcome = captured.outcome;
        self.sources.len = captured.lengths[0];
        self.sources.required = captured.required[0];
        self.groups.len = captured.lengths[1];
        self.groups.required = captured.required[1];
        self.members.len = captured.lengths[2];
        self.members.required = captured.required[2];
        self.units.len = captured.lengths[3];
        self.units.required = captured.required[3];
        [qpc().saturating_sub(begin).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::CounterLayers, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| CounterLayerRow {
                collection: r.collection,
                bucket: r.bucket,
                cursor: r.cursor,
                viewing_country: r.viewing_country,
                sources: self.sources.first[r.sources[0]..r.sources[0] + r.sources[1]]
                    .iter()
                    .map(|s| CounterSource {
                        province: s.province,
                        stack: s.stack,
                        units: self.units.first[s.units[0]..s.units[0] + s.units[1]].to_vec(),
                    })
                    .collect(),
                groups: self.groups.first[r.groups[0]..r.groups[0] + r.groups[1]]
                    .iter()
                    .map(|g| CounterGroup {
                        sources: self.members.first[g.sources[0]..g.sources[0] + g.sources[1]]
                            .to_vec(),
                        units: self.units.first[g.units[0]..g.units[0] + g.units[1]].to_vec(),
                    })
                    .collect(),
            },
        )]
    }
}

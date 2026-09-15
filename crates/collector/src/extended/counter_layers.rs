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
    pub fn with_capacities(c: &Capacities, s: &[u16]) -> Self {
        assert_eq!(s.len(), N);
        Self {
            rows: CaptureRows::new(c.rows, s[0]),
            sources: Arena::new(c.arenas[0], s[0]),
            groups: Arena::new(c.arenas[1], s[0]),
            members: Arena::new(c.arenas[2], s[0]),
            units: Arena::new(c.arenas[3], s[0]),
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
    pub fn grow(&mut self, c: &mut Capacities, s: &[u16]) {
        self.rows.grow(&mut c.rows, s[0]);
        self.sources.grow(&mut c.arenas[0], s[0]);
        self.groups.grow(&mut c.arenas[1], s[0]);
        self.members.grow(&mut c.arenas[2], s[0]);
        self.units.grow(&mut c.arenas[3], s[0]);
        grow_vec(&mut self.index, c.arenas[0]);
    }
    fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        second: bool,
    ) -> native::OrderWalk {
        native::walk_order(
            memory,
            base,
            state,
            native::OrderBuffers {
                rows: if second {
                    &mut self.rows.second
                } else {
                    &mut self.rows.rows
                },
                sources: if second {
                    &mut self.sources.second
                } else {
                    &mut self.sources.first
                },
                groups: if second {
                    &mut self.groups.second
                } else {
                    &mut self.groups.first
                },
                members: if second {
                    &mut self.members.second
                } else {
                    &mut self.members.first
                },
                units: if second {
                    &mut self.units.second
                } else {
                    &mut self.units.first
                },
                index: &mut self.index,
            },
        )
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        let begin = qpc();
        let first = self.read(memory, base, state, false);
        self.rows.outcome = first.outcome;
        self.rows.checked = needed[0];
        self.sources.len = first.lengths[0];
        self.sources.required = first.required[0];
        self.groups.len = first.lengths[1];
        self.groups.required = first.required[1];
        self.members.len = first.lengths[2];
        self.members.required = first.required[2];
        self.units.len = first.lengths[3];
        self.units.required = first.required[3];
        let matched = if needed[0] {
            let second = self.read(memory, base, state, true);
            let rows = self.rows.compare(second.outcome);
            let arenas = [
                self.sources.compare(second.lengths[0], second.required[0]),
                self.groups.compare(second.lengths[1], second.required[1]),
                self.members.compare(second.lengths[2], second.required[2]),
                self.units.compare(second.lengths[3], second.required[3]),
            ];
            self.rows.mismatches += arenas.iter().filter(|&&v| !v).count();
            rows && arenas.into_iter().all(|v| v)
        } else {
            false
        };
        ([matched], [qpc().saturating_sub(begin).max(0) as u64])
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "counter layers changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
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

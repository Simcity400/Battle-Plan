use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows, grow_capacity},
    table_conversion::{table, unavailable},
};
use engine_allocation::{copy_vec, owned_vec};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native_land::battle_sounds as native;
use replay_data::{rows::BattleSoundRow, rows::TableId, rows::tables};
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::BattleSounds];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    battles: usize,
    provinces: usize,
}

collector_primitives::capacity_max!(Capacities { battles, provinces });

impl Capacities {
    pub fn raise(&mut self, demand: native::Demand) {
        grow_capacity(&mut self.battles, demand.battles);
        grow_capacity(&mut self.provinces, demand.provinces);
    }
}

pub struct Captured {
    rows: CaptureRows<native::Row>,
    provinces: Arena<u32>,
}

impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities::default())
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(c.battles),
            provinces: Arena::new(c.provinces),
        }
    }

    pub fn reset(&mut self) {
        self.rows.reset();
        self.provinces.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.provinces.allocated_bytes()
    }

    pub fn grow(&mut self, c: &mut Capacities) {
        self.rows.grow(&mut c.battles);
        self.provinces.grow(&mut c.provinces);
    }

    pub fn capture(&mut self, memory: &impl Memory, base: usize, _state: usize) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        self.reset();
        let walk =
            native::walk(memory, base, &mut self.rows.rows, &mut self.provinces.first);
        self.rows.outcome = walk.outcome;
        self.provinces.len = walk.provinces;
        self.provinces.required = walk.required.provinces;
        [qpc().saturating_sub(begin).max(0) as u64]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        let outcome = self.rows.outcome;
        let Some(provinces) = self.provinces.first.get(..self.provinces.len) else {
            return [unavailable("battle sound member arena length invalid")];
        };
        let Some(rows) = self.rows.rows.get(..outcome.len()) else {
            return [unavailable("walker row count exceeds allocated buffer")];
        };
        if rows.iter().any(|row| {
            row.provinces_start
                .checked_add(row.provinces_len)
                .is_none_or(|end| end > provinces.len())
        }) {
            return [unavailable(
                "battle sound members lie outside the captured arena",
            )];
        }
        [table::<tables::BattleSounds, _>(outcome, rows, |row| {
            BattleSoundRow {
                ordinal: row.ordinal,
                center_x: row.center_x,
                center_z: row.center_z,
                radius: row.radius,
                provinces: owned_vec(copy_vec(
                    &provinces[row.provinces_start..row.provinces_start + row.provinces_len],
                )),
                sounding: row.sounding,
                object: row.object,
            }
        })]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn the_list_publishes_in_order_from_empty_storage() {
        let base = 0x10;
        let manager = 0x100;
        let nodes = 0x200;
        let list = 0x300;
        let battles = [0x400, 0x480];
        let members = 0x600;
        let province = |id: usize| 0x800 + id * 0x100;
        let mut bytes = vec![0; 0x1000];
        let mut put = |at: usize, value: &[u8]| bytes[at..at + value.len()].copy_from_slice(value);
        let header = |data: usize, count: u32| {
            let mut header = [0; 16];
            header[..8].copy_from_slice(&(data as u64).to_le_bytes());
            header[8..12].copy_from_slice(&count.to_le_bytes());
            header[12..].copy_from_slice(&count.to_le_bytes());
            header
        };
        let global = base + native::MANAGER_RVA.0;
        let pointer = (manager as u64).to_le_bytes();
        put(
            manager + native::MANAGER_TREE + native::TREE_NODES,
            &header(nodes, 1),
        );
        put(nodes + native::NODE_BATTLES, &header(list, 2));
        put(list, &(battles[0] as u64).to_le_bytes());
        put(list + 8, &(battles[1] as u64).to_le_bytes());
        for (index, &battle) in battles.iter().enumerate() {
            put(battle, &(index as f32 + 1.0).to_bits().to_le_bytes());
            put(
                battle + native::BATTLE_RADIUS,
                &9.0_f32.to_bits().to_le_bytes(),
            );
            put(
                battle + native::BATTLE_PROVINCES,
                &header(members + index * 0x10, 1),
            );
            put(
                members + index * 0x10,
                &(province(index + 1) as u64).to_le_bytes(),
            );
            put(
                province(index + 1),
                &((base + engine_native::cprovince::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(
                province(index + 1) + engine_native::cprovince::ID,
                &(index as i32 + 5).to_le_bytes(),
            );
        }
        put(battles[1] + native::BATTLE_SOUND, &7_u64.to_le_bytes());
        struct Split<'a> {
            low: SliceMemory<'a>,
            global: SliceMemory<'a>,
        }
        impl Memory for Split<'_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                self.global.copy(at, out) || self.low.copy(at, out)
            }
        }
        let memory = Split {
            low: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            global: SliceMemory {
                base: global,
                bytes: &pointer,
            },
        };
        let mut capacities = Capacities::default();
        let mut pack = Captured::with_capacities(&capacities);
        assert_eq!(pack.allocated_bytes(), 0);
        pack.capture(&memory, base, 0);
        assert_eq!(pack.outcomes(), [Outcome::Rows { len: 2 }]);
        let [TableView::Complete(rows)] = pack.tables() else {
            panic!("the battle list was not published");
        };
        let rows = rows.rows::<tables::BattleSounds>().unwrap();
        assert_eq!(
            rows,
            [
                BattleSoundRow {
                    ordinal: 0,
                    center_x: 1.0_f32.to_bits(),
                    center_z: 0,
                    radius: 9.0_f32.to_bits(),
                    provinces: vec![5],
                    sounding: false,
                    object: battles[0] as u64,
                },
                BattleSoundRow {
                    ordinal: 1,
                    center_x: 2.0_f32.to_bits(),
                    center_z: 0,
                    radius: 9.0_f32.to_bits(),
                    provinces: vec![6],
                    sounding: true,
                    object: battles[1] as u64,
                },
            ]
        );
        pack.grow(&mut capacities);
        assert!(capacities.battles >= 2 && capacities.provinces >= 2);
        let unreadable = SliceMemory {
            base: 0,
            bytes: &[],
        };
        pack.capture(&unreadable, base, 0);
        assert!(matches!(pack.tables()[0], TableView::Unavailable { .. }));
    }
}

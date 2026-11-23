use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
    table_conversion::table,
};
use engine_allocation::{copy_vec, owned_vec};
use engine_native_memory::{Memory, walk::Outcome};
use engine_scripts::saved_event_targets as native;
use replay_data::{rows::GlobalSavedEventTargetRow, rows::TableId, rows::tables};
use replay_frame::frame::TableView;
pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::GlobalSavedEventTargets];
pub type Capacities = native::Demand;
pub struct Captured {
    rows: CaptureRows<native::Row>,
    text: Arena<u8>,
    required: native::Demand,
}
impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities::default())
    }
    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(c.rows),
            text: Arena::new(c.bytes),
            required: Default::default(),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.text.reset();
        self.required = Default::default();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.text.allocated_bytes()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        grow_capacity(&mut c.rows, self.required.rows);
        grow_capacity(&mut c.bytes, self.required.bytes);
        self.rows.grow(&mut c.rows);
        grow_vec(&mut self.text.first, c.bytes);
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        self.reset();
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        let result = native::walk(
            memory,
            base,
            state,
            &mut self.rows.rows,
            &mut self.text.first,
        );
        self.rows.outcome = result.outcome;
        self.text.len = result.bytes;
        self.text.required = result.required.bytes;
        self.required = result.required;
        [qpc().saturating_sub(begin).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::GlobalSavedEventTargets, _>(
            self.rows.outcome,
            &self.rows.rows,
            |r| GlobalSavedEventTargetRow {
                ordinal: r.ordinal,
                name: owned_vec(copy_vec(
                    &self.text.first[r.name_start..r.name_start + r.name_len],
                )),
                state: r.state,
                country: r.country,
                character: r.character,
                live: r.live,
                character_leader: r.character_leader,
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    struct Fixture(BTreeMap<usize, Vec<u8>>);
    impl Memory for Fixture {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            let Some((&start, b)) = self.0.range(..=at).next_back() else {
                return false;
            };
            let Some(b) = b.get(at - start..at - start + out.len()) else {
                return false;
            };
            out.copy_from_slice(b);
            true
        }
    }
    #[test]
    fn presized_registry_and_growth_keep_every_target() {
        let n = 5usize;
        let state = 0x1000;
        let data = 0x10000;
        let names = 0x100000;
        let header = |ptr: usize, count: usize| {
            let mut h = vec![0; 16];
            h[..8].copy_from_slice(&(ptr as u64).to_le_bytes());
            h[8..12].copy_from_slice(&(count as u32).to_le_bytes());
            h[12..].copy_from_slice(&(count as u32).to_le_bytes());
            h
        };
        let mut f = Fixture(BTreeMap::new());
        f.0.insert(state + native::IN_GAME, vec![1]);
        f.0.insert(state + native::REGISTRY, header(data, n));
        f.0.insert(native::NAMES_RVA.0, header(names, n + 1));
        let mut registry = vec![0; n * native::STRIDE];
        let mut strings = vec![0; (n + 1) * 32];
        let mut expected_bytes = 0;
        strings[24..32].copy_from_slice(&15u64.to_le_bytes());
        for i in 0..n {
            let at = i * native::STRIDE;
            registry[at..at + 8].copy_from_slice(&(native::VTABLE_RVA.0 as u64).to_le_bytes());
            registry[at + 12..at + 16].copy_from_slice(&((i + 1) as i32).to_le_bytes());
            registry[at + 0x68..at + 0x6a].copy_from_slice(&((i + 1) as u16).to_le_bytes());
            let name = format!("country_{i}");
            let at = (i + 1) * 32;
            strings[at..at + name.len()].copy_from_slice(name.as_bytes());
            strings[at + 16..at + 24].copy_from_slice(&(name.len() as u64).to_le_bytes());
            strings[at + 24..at + 32].copy_from_slice(&15u64.to_le_bytes());
            expected_bytes += name.len();
        }
        f.0.insert(data, registry);
        f.0.insert(names, strings);
        let d = native::demand(&f, 0, state).unwrap();
        assert_eq!(
            d,
            native::Demand {
                rows: n,
                bytes: expected_bytes
            }
        );
        let mut caps = Capacities::default();
        caps.raise(d);
        let mut pack = Captured::with_capacities(&caps);
        pack.capture(&f, 0, state);
        assert_eq!(pack.outcomes(), [Outcome::Rows { len: n }]);
        let [TableView::Complete(rows)] = pack.tables() else {
            panic!("not complete")
        };
        assert_eq!(
            rows.rows::<tables::GlobalSavedEventTargets>()
                .unwrap()
                .last()
                .unwrap()
                .country,
            5
        );
        let mut small = Captured::vacant();
        assert_eq!(
            native::count(&f, 0, state).outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: n as u32
            }
        );
        small.capture(&f, 0, state);
        assert_eq!(small.outcomes(), [Outcome::Rows { len: n }]);
        small.grow(&mut Capacities::default());
        small.capture(&f, 0, state);
        assert_eq!(small.outcomes(), [Outcome::Rows { len: n }]);
    }
}

#![allow(dead_code)]
use engine_focus::cnational_focus_progress as focus;
use engine_native_memory::Memory;
use engine_native::{ccountry, cgame_state};
use engine_native_parts::modifier_definition;
use engine_native_memory::memory::{RecordLayout, RecordSources};
use engine_research::{
    climited_use_tech_bonus as bonus, cresearch_slot as slot, ctechnology as tech,
    ctechnology_status as status, ctechnology_template as template,
};
use engine_sites::Rva;
use std::cell::{Cell, RefCell};

pub const GAME: usize = 0x1000;
pub const IMAGE_START: usize = modifier_definition::TABLE_RVA.0 & !0xffff;
pub const IMAGE_LEN: usize =
    (tech::BASE_COST_SCALE_RVA.0 + size_of::<i64>()).next_multiple_of(0x10000) - IMAGE_START;
const STRIDE: usize = 0x8000;
pub const STATUS: usize = 0x1800;
pub const SLOTS: usize = 0x1980;
pub const SLOT: usize = 0x1a00;
pub const TECH: usize = 0x2000;
pub const TEMPLATE: usize = 0x3000;
pub const FOCUS: usize = 0x6000;
const _: () = {
    assert!(ccountry::FOCUS_PROGRESS + 8 <= STATUS);
    assert!(STATUS + status::COUNTRY + 8 <= SLOTS);
    assert!(SLOTS + 3 * 8 <= SLOT);
    assert!(SLOT + 2 * 0x80 + slot::INDEX + 4 <= TECH);
};

pub struct World {
    heap: Vec<u8>,
    image: Vec<u8>,
}
impl World {
    pub fn country(id: u32) -> usize {
        let origin = 0x10000 + (id as usize - 1) * STRIDE;
        if origin + STRIDE > IMAGE_START {
            origin + IMAGE_LEN + STRIDE
        } else {
            origin
        }
    }
    pub fn new(countries: u32) -> Self {
        let tables = Self::country(countries + 1);
        let mut world = Self {
            heap: vec![0; tables + (countries as usize + 1) * 12],
            image: vec![0; IMAGE_LEN],
        };
        world.array(GAME + cgame_state::COUNTRIES, tables, countries + 1);
        let links = tables + (countries as usize + 1) * 8;
        world.array(GAME + cgame_state::COUNTRY_LINKS, links, countries + 1);
        world.image_put(tech::BASE_COST_SCALE_RVA, &130_000_i64.to_le_bytes());
        world.image_put(
            tech::BASE_YEAR_AHEAD_PENALTY_FACTOR_RVA,
            &175_000_i64.to_le_bytes(),
        );
        world.image_put(modifier_definition::TABLE_RVA, &0x4000_u64.to_le_bytes());
        for id in 1..=countries {
            let c = Self::country(id);
            world.put(tables + id as usize * 8, (c as u64).to_le_bytes());
            world.put(links + id as usize * 4, id.to_le_bytes());
            world.put(c, (ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
            world.put(c + ccountry::ID, id.to_le_bytes());
            world.put(c + ccountry::OWNED_STATE_COUNT, 1_u32.to_le_bytes());
            world.pointer(c + ccountry::TECHNOLOGY_STATUS, c + STATUS);
            world.put(c + STATUS, (status::VTABLE_RVA.0 as u64).to_le_bytes());
            world.pointer(c + STATUS + status::COUNTRY, c);
            world.array(c + STATUS + status::RESEARCH_SLOTS_DATA, c + SLOTS, 3);
            world.array(c + STATUS + status::TECHNOLOGIES_DATA, c + 0x5400, 2);
            world.array(c + STATUS + status::RESEARCHED_DATA, c + 0x5500, 1);
            world.pointer(c + 0x5500, c + TECH);
            for (position, index) in [2_u32, 1, 0].into_iter().enumerate() {
                let s = c + SLOT + position * 0x80;
                world.pointer(c + SLOTS + position * 8, s);
                world.put(s, (slot::VTABLE_RVA.0 as u64).to_le_bytes());
                world.pointer(s + slot::STATUS, c + STATUS);
                world.put(s + slot::INDEX, index.to_le_bytes());
                world.put(
                    s + slot::BANKED_DAYS,
                    (100_000_i64 * i64::from(id)).to_le_bytes(),
                );
                world.put(s + slot::APPLIED_DAYS, (-17_i64).to_le_bytes());
                world.put(s + slot::RESEARCH_SPEED, (150_001_i64).to_le_bytes());
                if position == 1 {
                    world.put(s + slot::TECHNOLOGY_HANDLE, slot::IDLE_HANDLE.to_le_bytes());
                    continue;
                }
                let ordinal = position / 2;
                let t = c + TECH + ordinal * 0x400;
                let definition = c + TEMPLATE + ordinal * 0x800;
                let handle = 7 + ordinal as u32;
                world.put(s + slot::TECHNOLOGY_HANDLE, handle.to_le_bytes());
                world.pointer(s + slot::TECHNOLOGY, t);
                world.pointer(c + 0x5400 + ordinal * 8, t);
                world.put(t, (tech::VTABLE_RVA.0 as u64).to_le_bytes());
                world.put(t + tech::DEFINITION_INDEX, handle.to_le_bytes());
                world.pointer(t + tech::STATUS, c + STATUS);
                world.pointer(t + tech::TEMPLATE, definition);
                world.put(
                    t + tech::CLASSIFIER,
                    (if ordinal == 0 {
                        tech::CLASSIFIER_COMPLETED
                    } else {
                        tech::CLASSIFIER_ACTIVE
                    })
                    .to_le_bytes(),
                );
                world.put(t + tech::LEVEL, (ordinal as i32 + 1).to_le_bytes());
                world.put(
                    t + tech::LIVE_PROGRESS,
                    (500_003_i64 + i64::from(id)).to_le_bytes(),
                );
                world.put(t + tech::AHEAD_REDUCTION_YEARS, (-25_001_i64).to_le_bytes());
                world.put(t + tech::APPLIED_SPEED_BONUS, 100_003_i64.to_le_bytes());
                world.array(t + tech::APPLIED_BONUS_IDS, c + 0x4100 + ordinal * 4, 1);
                world.put(c + 0x4100 + ordinal * 4, (-7_i32).to_le_bytes());
                world.put(t + 0x190, (-5_003_i64).to_le_bytes());
                world.put(t + 0x1a0, 7_001_i64.to_le_bytes());
                world.put(
                    definition + template::DATABASE_INDEX,
                    (ordinal as i32 + 1).to_le_bytes(),
                );
                world.put(
                    definition + template::TECHNOLOGY_HANDLE,
                    handle.to_le_bytes(),
                );
                world.put(
                    definition + template::YEAR,
                    (1936_i32 + ordinal as i32).to_le_bytes(),
                );
                world.put(definition + template::MAX_LEVEL, 3_i32.to_le_bytes());
                world.put(
                    definition + template::RESEARCH_COST,
                    2_000_003_i64.to_le_bytes(),
                );
                world.text(
                    definition + template::ID,
                    if ordinal == 0 {
                        b"mod_tech_a"
                    } else {
                        b"mod_tech_b"
                    },
                );
            }
            world.array(c + STATUS + status::LIMITED_BONUSES, c + 0x4200, 1);
            world.pointer(c + 0x4200, c + 0x4400);
            world.put(c + 0x4400, (bonus::VTABLE_RVA.0 as u64).to_le_bytes());
            world.put(c + 0x4400 + bonus::ID, (-7_i32).to_le_bytes());
            world.text(c + 0x4400 + bonus::NAME, b"mod_bonus");
            world.put(c + 0x4400 + bonus::SPEED_BONUS, (-30_003_i64).to_le_bytes());
            world.put(
                c + 0x4400 + bonus::AHEAD_REDUCTION,
                200_001_i64.to_le_bytes(),
            );
            let accumulator = c + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR;
            world.array(accumulator, c + 0x5000, 1);
            world.put(
                c + 0x5000,
                engine_executable::modifier_keys::RESEARCH_SPEED_FACTOR.to_le_bytes(),
            );
            world.put(c + 0x5008, 15_003_i64.to_le_bytes());
            world.pointer(c + ccountry::FOCUS_PROGRESS, c + FOCUS);
            world.pointer(c + ccountry::FOCUS_TREE, c + 0x6100);
            world.put(c + FOCUS, (focus::VTABLE_RVA.0 as u64).to_le_bytes());
            world.pointer(c + FOCUS + focus::CURRENT, c + 0x6200);
            world.pointer(c + FOCUS + focus::CONTINUOUS, c + 0x6300);
            world.put(c + FOCUS + focus::PROGRESS, (-125_003_i64).to_le_bytes());
            world.heap[c + FOCUS + focus::PAUSED] = 3;
            world.array(c + FOCUS + focus::COMPLETED_DATA, c + 0x6400, 3);
            world.text(
                c + 0x6100 + engine_focus::cnational_focus_tree::TOKEN,
                b"mod_tree",
            );
            world.text(
                c + 0x6200 + engine_focus::cnational_focus::TOKEN,
                b"mod_current",
            );
            world.text(
                c + 0x6300 + engine_focus::ccontinuous_national_focus::TOKEN,
                b"mod_continuous",
            );
            for i in 0..3 {
                let definition = c + 0x6500 + i * 0x100;
                world.pointer(c + 0x6400 + i * 8, definition);
                if i == 2 {
                    let string = definition + engine_focus::cnational_focus::TOKEN;
                    let token = b"mod_completed_heap_token";
                    world.pointer(string, c + 0x6900);
                    world.put(string + 16, (token.len() as u64).to_le_bytes());
                    world.put(string + 24, (token.len() as u64).to_le_bytes());
                    world.heap[c + 0x6900..c + 0x6900 + token.len()].copy_from_slice(token);
                } else {
                    world.text(
                        definition + engine_focus::cnational_focus::TOKEN,
                        b"mod_completed",
                    );
                }
            }
        }
        world
    }
    pub fn put<const N: usize>(&mut self, at: usize, bytes: [u8; N]) {
        self.heap[at..at + N].copy_from_slice(&bytes);
    }
    pub fn pointer(&mut self, at: usize, pointer: usize) {
        self.put(at, (pointer as u64).to_le_bytes());
    }
    pub fn array(&mut self, at: usize, pointer: usize, count: u32) {
        self.pointer(at, pointer);
        self.put(at + 8, count.to_le_bytes());
        self.put(at + 12, count.to_le_bytes());
    }
    pub fn text(&mut self, at: usize, bytes: &[u8]) {
        assert!(bytes.len() <= 15);
        self.heap[at..at + 32].fill(0);
        self.heap[at..at + bytes.len()].copy_from_slice(bytes);
        self.put(at + 16, (bytes.len() as u64).to_le_bytes());
        self.put(at + 24, 15_u64.to_le_bytes());
    }
    fn image_put(&mut self, key: Rva, bytes: &[u8]) {
        let at = key.0;
        self.image[at - IMAGE_START..at - IMAGE_START + bytes.len()].copy_from_slice(bytes);
    }
}
impl Memory for World {
    fn copy(&self, address: usize, out: &mut [u8]) -> bool {
        let image = address
            .checked_sub(IMAGE_START)
            .and_then(|at| self.image.get(at..at.checked_add(out.len())?));
        let bytes = image.or_else(|| self.heap.get(address..address.checked_add(out.len())?));
        if let Some(bytes) = bytes {
            out.copy_from_slice(bytes);
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub reader: &'static str,
    pub kind: &'static str,
}
pub struct FallbackCounts {
    before: Cell<[u64; 12]>,
}
impl FallbackCounts {
    const BATCHES: [engine_native_memory::batch_fallback::Batch; 12] = [
        engine_native_memory::batch_fallback::Batch::GovernanceSlotPointers,
        engine_native_memory::batch_fallback::Batch::GovernanceSlots,
        engine_native_memory::batch_fallback::Batch::GovernanceAssignedTechnologies,
        engine_native_memory::batch_fallback::Batch::GovernanceAssignedTemplates,
        engine_native_memory::batch_fallback::Batch::GovernanceModifierTechnologies,
        engine_native_memory::batch_fallback::Batch::GovernanceModifierIdeas,
        engine_native_memory::batch_fallback::Batch::GovernanceModifierScripts,
        engine_native_memory::batch_fallback::Batch::GovernanceFocusProgress,
        engine_native_memory::batch_fallback::Batch::GovernanceFocusPointers,
        engine_native_memory::batch_fallback::Batch::GovernanceFocusDefinitions,
        engine_native_memory::batch_fallback::Batch::GovernanceResearchDrift,
        engine_native_memory::batch_fallback::Batch::GovernanceTechnologyDrift,
    ];
    fn snapshot() -> [u64; 12] {
        Self::BATCHES.map(engine_native_memory::batch_fallback::count)
    }
    fn new() -> Self {
        Self {
            before: Cell::new(Self::snapshot()),
        }
    }
    fn clear(&self) {
        self.before.set(Self::snapshot());
    }
    pub fn witnesses(&self) -> Vec<&'static str> {
        let before = self.before.get();
        Self::BATCHES
            .into_iter()
            .enumerate()
            .flat_map(|(i, batch)| {
                std::iter::repeat_n(
                    batch.name(),
                    (engine_native_memory::batch_fallback::count(batch) - before[i]) as usize,
                )
            })
            .collect()
    }
}
pub struct Trace<'a, M> {
    pub source: &'a M,
    pub reader: Cell<&'static str>,
    pub calls: RefCell<Vec<Call>>,
    pub operands: RefCell<Vec<(usize, usize)>>,
    pub fallbacks: FallbackCounts,
    pub fault: Option<usize>,
    pub remaining: Cell<Option<u32>>,
    gather_only: bool,
}
impl<'a, M: Memory> Trace<'a, M> {
    pub fn new(source: &'a M, reader: &'static str) -> Self {
        Self {
            source,
            reader: Cell::new(reader),
            calls: RefCell::new(Vec::new()),
            operands: RefCell::new(Vec::new()),
            fallbacks: FallbackCounts::new(),
            fault: None,
            remaining: Cell::new(None),
            gather_only: false,
        }
    }
    pub fn fault(source: &'a M, reader: &'static str, address: usize, once: bool) -> Self {
        let mut trace = Self::new(source, reader);
        trace.fault = Some(address);
        trace.remaining.set(once.then_some(1));
        trace
    }
    pub fn count(&self) -> usize {
        self.calls.borrow().len()
    }
    pub fn gather_fault(source: &'a M, reader: &'static str, address: usize) -> Self {
        let mut trace = Self::fault(source, reader, address, true);
        trace.gather_only = true;
        trace
    }
    pub fn clear(&self) {
        self.calls.borrow_mut().clear();
        self.operands.borrow_mut().clear();
        self.fallbacks.clear();
    }
    fn call(&self, kind: &'static str) {
        self.calls.borrow_mut().push(Call {
            reader: self.reader.get(),
            kind,
        });
    }
    fn field(&self, address: usize, out: &mut [u8], gathered: bool) -> bool {
        self.operands.borrow_mut().push((address, out.len()));
        if self
            .fault
            .is_some_and(|at| address <= at && at < address.saturating_add(out.len()))
            && self.remaining.get() != Some(0)
            && (!self.gather_only || gathered)
        {
            if let Some(left) = self.remaining.get() {
                self.remaining.set(Some(left - 1));
            }
            out.fill(255);
            return false;
        }
        self.source.copy(address, out)
    }
}
impl<M: Memory> Memory for Trace<'_, M> {
    fn copy(&self, address: usize, out: &mut [u8]) -> bool {
        self.call("copy");
        self.field(address, out, false)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        out: &mut [&mut [u8]; N],
    ) -> usize {
        self.call("copy_scattered");
        for (i, (&at, out)) in addresses.iter().zip(out).enumerate() {
            if !out.is_empty() && !self.field(at, out, false) {
                return i;
            }
        }
        N
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        out: &mut [u8],
    ) -> usize {
        self.call("copy_records");
        struct Fields<'a, 'b, M>(&'a Trace<'b, M>);
        impl<M: Memory> Memory for Fields<'_, '_, M> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                self.0.field(at, out, true)
            }
        }
        Fields(self).copy_records(sources, layout, out)
    }
}

pub fn addresses(trace: &Trace<'_, impl Memory>) -> Vec<usize> {
    let mut addresses = trace
        .operands
        .borrow()
        .iter()
        .flat_map(|&(at, len)| at..at + len)
        .collect::<Vec<_>>();
    addresses.sort_unstable();
    addresses.dedup();
    addresses
}
pub fn never_harsher(serial: engine_native_memory::walk::Outcome, batched: engine_native_memory::walk::Outcome) {
    use engine_native_memory::walk::Outcome;
    match serial {
        Outcome::Rows { len } => assert_eq!(batched, Outcome::Rows { len }),
        Outcome::Truncated { len, unread_rows } => match batched {
            Outcome::Rows { len: new } => assert!(new >= len),
            Outcome::Truncated {
                len: new,
                unread_rows: unread,
            } => {
                assert!(new >= len);
                assert!(unread <= unread_rows);
            }
            other => panic!("batch withheld more: {serial:?} -> {other:?}"),
        },
        Outcome::Unavailable { .. } => {}
    }
}

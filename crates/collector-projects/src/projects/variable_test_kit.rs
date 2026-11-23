use super::*;
use std::cell::Cell;

pub struct Fixture {
    pub pointer: usize,
    bytes: std::cell::RefCell<Vec<u8>>,
    pub reads: Cell<usize>,
    pub refuse: Cell<bool>,
}
impl Fixture {
    pub fn new(pointer: usize, seed: u32) -> Self {
        let this = Self {
            pointer,
            bytes: std::cell::RefCell::new(vec![0; 0x3000]),
            reads: Cell::new(0),
            refuse: Cell::new(false),
        };
        this.replace(seed);
        this
    }
    pub fn written(&self, seed: u32) {
        let writing = collector_observers::project_variable_changes::writing(
            self.pointer + nproject_cproject::VARIABLES,
        );
        self.replace(seed);
        drop(writing);
    }
    pub fn replace(&self, seed: u32) {
        use engine_scripts::{cvariables as variables, variable_hash_node as node};
        let mut bytes = self.bytes.borrow_mut();
        bytes.fill(0);
        let put64 = |bytes: &mut [u8], at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        };
        let store = nproject_cproject::VARIABLES;
        put64(&mut bytes, store, variables::VTABLE_RVA.0 as u64);
        bytes[store + variables::SEED..store + variables::SEED + 4]
            .copy_from_slice(&seed.to_le_bytes());
        put64(
            &mut bytes,
            store + variables::STORAGE,
            (self.pointer + 0x1000) as u64,
        );
        bytes[store + variables::ENTRY_COUNT..store + variables::ENTRY_COUNT + 4]
            .copy_from_slice(&3_u32.to_le_bytes());
        bytes[store + variables::BUCKET_MASK..store + variables::BUCKET_MASK + 4]
            .copy_from_slice(&7_u32.to_le_bytes());
        bytes[store + variables::PROBE_TAIL] = 5;
        bytes[0x1000 + 13 * node::STRIDE + node::DISTANCE] = 255;
        let names: [&[u8]; 3] = [b"\xff\0x", b"long_variable_key_bytes", b""];
        for (index, name) in names.into_iter().enumerate() {
            let at = 0x1000 + index * node::STRIDE;
            bytes[at..at + 4].copy_from_slice(&(index as u32).to_le_bytes());
            bytes[at + node::DISTANCE] = 1;
            if name.len() <= 15 {
                bytes[at + node::KEY..at + node::KEY + name.len()].copy_from_slice(name);
                put64(&mut bytes, at + 0x20, 15);
            } else {
                put64(&mut bytes, at + node::KEY, (self.pointer + 0x2000) as u64);
                bytes[0x2000..0x2000 + name.len()].copy_from_slice(name);
                put64(&mut bytes, at + 0x20, name.len() as u64);
            }
            put64(&mut bytes, at + 0x18, name.len() as u64);
            let value = [i64::MIN + i64::from(seed), i64::MAX - i64::from(seed), -7][index];
            put64(&mut bytes, at + node::VALUE, value as u64);
        }
        bytes[0x1000 + node::KEY] = seed as u8;
        bytes[0x2000] = seed as u8;
    }
    pub fn row(&self) -> nproject_cproject::Row {
        nproject_cproject::Row {
            owner: 5_000,
            local_id: 9,
            source_pointer: self.pointer,
            ..Default::default()
        }
    }
}
impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.set(self.reads.get() + 1);
        !self.refuse.get()
            && engine_native_memory::memory::SliceMemory {
                base: self.pointer,
                bytes: &self.bytes.borrow(),
            }
            .copy(address, destination)
    }
}

pub fn acquire(captured: &mut Captured, memory: &Fixture, force: bool) {
    let row = memory.row();
    captured.projects.rows = vec![row];
    captured.projects.outcome = Outcome::Rows { len: 1 };
    captured
        .project_runtime
        .acquire(memory, 0, &[row], force, false, None);
}

pub fn pending(captured: &Captured) -> usize {
    captured.project_runtime.pending_count()
}

pub fn reference(memory: &Fixture) -> Vec<u8> {
    let mut captured = Captured::vacant();
    let row = memory.row();
    captured.projects.rows = vec![row];
    captured.projects.outcome = Outcome::Rows { len: 1 };
    captured
        .project_runtime
        .read(memory, 0, &[row], true, false, None);
    encoded(&captured)
}

pub fn encoded(captured: &Captured) -> Vec<u8> {
    let TableView::Complete(table) = captured.tables().into_iter().nth(1).unwrap() else {
        panic!("complete project table");
    };
    let mut bytes = Vec::new();
    table.encode_rows(&mut bytes).unwrap();
    bytes
}

pub fn variables(captured: &Captured) -> Option<std::sync::Arc<rows::ProjectVariableStore>> {
    let rows::Recorded::Value(runtime) = captured.project_runtime.value(&captured.projects.rows[0])
    else {
        panic!("runtime");
    };
    runtime.variables
}

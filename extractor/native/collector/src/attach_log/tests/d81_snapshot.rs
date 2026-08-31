use super::*;
use crate::attach_log::d81_snapshot_memory::{
    Domain81Memory, Domain81SnapshotPlan, Domain81TracingMemory,
};
use d81_rail_change_gate_lane::gate::{FullCopyReason, RailWitness};
use std::cell::RefCell;
use std::collections::BTreeMap;

const ROOT: u64 = 0x1000;
const OBJECT: u64 = 0x2000;

struct FixtureMemory(RefCell<BTreeMap<u64, u8>>);

impl FixtureMemory {
    fn new(value: u32) -> Self {
        let memory = Self(RefCell::new(BTreeMap::new()));
        memory.write(ROOT, &OBJECT.to_le_bytes());
        memory.write(OBJECT, &value.to_le_bytes());
        memory
    }

    fn write(&self, address: u64, bytes: &[u8]) {
        let mut memory = self.0.borrow_mut();
        for (offset, value) in bytes.iter().copied().enumerate() {
            memory.insert(address + offset as u64, value);
        }
    }
}

impl Domain81Memory for FixtureMemory {
    fn read_into(&self, address: u64, out: &mut [u8]) -> bool {
        let memory = self.0.borrow();
        for (offset, slot) in out.iter_mut().enumerate() {
            let Some(value) = memory.get(&(address + offset as u64)) else {
                return false;
            };
            *slot = *value;
        }
        true
    }
}

fn payload(memory: &dyn Domain81Memory) -> Option<Vec<u8>> {
    let mut pointer = [0; 8];
    memory.read_into(ROOT, &mut pointer).then_some(())?;
    let pointer = u64::from_le_bytes(pointer);
    let mut value = [0; 4];
    memory.read_into(pointer, &mut value).then_some(())?;
    let mut payload = pointer.to_le_bytes().to_vec();
    payload.extend_from_slice(&value);
    Some(payload)
}

fn armed_plan(memory: &FixtureMemory) -> Domain81SnapshotPlan {
    let tracing = Domain81TracingMemory::new(memory);
    assert!(payload(&tracing).is_some());
    Domain81SnapshotPlan::from_trace(tracing.into_trace()).expect("bounded fixture plan")
}

#[test]
fn a_proven_rail_signal_forces_the_next_snapshot_and_the_snapshot_is_walked() {
    let memory = FixtureMemory::new(7);
    let mut plan = armed_plan(&memory);
    memory.write(OBJECT, &9u32.to_le_bytes());

    let witness = RailWitness {
        epoch: 12,
        quiet: true,
    };
    assert_eq!(
        d81_classify_snapshot_capture(FullCopyReason::None, Some(10), witness, witness),
        D81SnapshotCaptureCause::RailWritten
    );

    let receipt = plan.capture_with(|address, out| memory.read_into(address, out));
    assert_eq!(receipt.failure_region_ordinal, None);
    let snapshot = plan.view();
    assert_eq!(payload(&snapshot), payload(&memory));
    assert_eq!(payload(&snapshot).unwrap()[8..], 9u32.to_le_bytes());
    memory.write(OBJECT, &11u32.to_le_bytes());
    assert_eq!(payload(&snapshot).unwrap()[8..], 9u32.to_le_bytes());
    assert_eq!(payload(&memory).unwrap()[8..], 11u32.to_le_bytes());
}

#[test]
fn no_signal_still_snapshots_because_whole_d81_mutator_coverage_is_incomplete() {
    let memory = FixtureMemory::new(42);
    let mut plan = armed_plan(&memory);
    let witness = RailWitness {
        epoch: 20,
        quiet: true,
    };
    assert_eq!(
        d81_classify_snapshot_capture(FullCopyReason::None, Some(20), witness, witness),
        D81SnapshotCaptureCause::MutatorCoverageIncomplete
    );

    let receipt = plan.capture_with(|address, out| memory.read_into(address, out));
    assert_eq!(receipt.copied_region_count, receipt.region_count);
    assert_eq!(payload(&plan.view()), payload(&memory));
}

#[test]
fn an_inflight_writer_refuses_instead_of_publishing_a_snapshot() {
    let before = RailWitness {
        epoch: 30,
        quiet: false,
    };
    let after = RailWitness {
        epoch: 31,
        quiet: false,
    };
    assert_eq!(
        d81_classify_snapshot_capture(FullCopyReason::None, Some(30), before, after),
        D81SnapshotCaptureCause::WriterInFlight
    );
}

#[test]
fn the_shipped_d81_bracket_calls_only_the_preallocated_capture() {
    let source = super::collector_source_lf();
    let start = source
        .find("let rail_replay_state_handle = rail_replay_state_live_enabled.then(||")
        .expect("d81 scheduling block");
    let end = source[start..]
        .find("let nuke_stockpile_handle")
        .map(|offset| start + offset)
        .expect("next scheduling block");
    let block = &source[start..end];
    assert!(block.contains("capture_compact_rail_replay_state_snapshot("));
    assert!(!block.contains("read_compact_rail_replay_state("));
}

#[test]
fn the_outside_bracket_decoder_is_scoped_to_snapshot_memory() {
    let source = super::collector_source_lf();
    let body = source
        .split_once("fn finish_compact_rail_replay_state_snapshot(")
        .expect("snapshot finish")
        .1
        .split_once("\nfn ")
        .expect("end of snapshot finish")
        .0;
    assert_eq!(body.matches("with_d81_memory_override(&view").count(), 2);
    assert!(body.contains("read_compact_rail_replay_state("));
    assert!(body.contains("d81_finish_rail_replay_state(observed, geometry)"));
    assert!(!body.contains("D81LiveMemory"));
}

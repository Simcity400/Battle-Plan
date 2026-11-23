use engine_native::ctask_force::TaskForceStats;
use super::*;
use std::cell::Cell;

struct Words {
    attempts: [Cell<usize>; 4],
    failed: Option<usize>,
}

impl Memory for Words {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let Some(index) = [0x1138, 0x1148, 0x1150, 0x1160]
            .iter()
            .position(|&at| at == address)
        else {
            panic!("unexpected read");
        };
        self.attempts[index].set(self.attempts[index].get() + 1);
        if self.failed == Some(index) {
            return false;
        }
        destination.copy_from_slice(&[i64::MIN, -7, i64::MAX, 0][index].to_le_bytes());
        true
    }
}

#[test]
fn task_force_stat_words_borrow_one_original_selector_and_keep_signed_values() {
    let memory = Words {
        attempts: std::array::from_fn(|_| Cell::new(0)),
        failed: None,
    };
    let stats = TaskForceStats::capture(&memory, Ok(0x1000));
    assert_eq!(
        stats,
        TaskForceStats {
            detection_138: Ok(i64::MIN),
            visibility_148: Ok(-7),
            visibility_150: Ok(i64::MAX),
            carrier_detection_160: Ok(0)
        }
    );
    assert!(memory.attempts.iter().all(|attempt| attempt.get() == 1));
}

#[test]
fn task_force_stat_field_faults_are_independent_and_original_root_failure_is_sticky() {
    let memory = Words {
        attempts: std::array::from_fn(|_| Cell::new(0)),
        failed: Some(1),
    };
    let stats = TaskForceStats::capture(&memory, Err("original stat selector refused"));
    assert_eq!(stats.detection_138, Err("original stat selector refused"));
    assert_eq!(stats.visibility_148, stats.detection_138);
    assert_eq!(stats.visibility_150, stats.detection_138);
    assert_eq!(stats.carrier_detection_160, stats.detection_138);
    assert!(memory.attempts.iter().all(|attempt| attempt.get() == 0));
    let stats = TaskForceStats::capture(&memory, Ok(0x1000));
    assert_eq!(
        stats.visibility_148,
        Err("selected transport task-force stat word unreadable")
    );
    assert_eq!(
        (
            stats.detection_138,
            stats.visibility_150,
            stats.carrier_detection_160
        ),
        (Ok(i64::MIN), Ok(i64::MAX), Ok(0))
    );
    assert!(memory.attempts.iter().all(|attempt| attempt.get() == 1));
}

#[test]
fn spotting_contributor_does_not_read_unreached_visibility_words() {
    let memory = Words {
        attempts: std::array::from_fn(|_| Cell::new(0)),
        failed: Some(1),
    };
    let stats = TaskForceStats::capture_detection(&memory, Ok(0x1000));
    assert_eq!(
        (stats.detection_138, stats.carrier_detection_160),
        (Ok(i64::MIN), Ok(0))
    );
    assert_eq!(
        stats.visibility_148,
        Err("selected transport visibility stat query unreached")
    );
    assert_eq!(stats.visibility_150, stats.visibility_148);
    assert_eq!(memory.attempts.each_ref().map(Cell::get), [1, 0, 0, 1]);
}

#[derive(Default)]
struct AirBytes {
    bytes: std::collections::BTreeMap<usize, u8>,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
}
impl AirBytes {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.bytes.extend(
            bytes
                .iter()
                .enumerate()
                .map(|(index, byte)| (at + index, *byte)),
        );
    }
}
impl Memory for AirBytes {
    fn copy(&self, at: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((at, destination.len()));
        for (index, destination) in destination.iter_mut().enumerate() {
            let Some(byte) = self.bytes.get(&(at + index)) else {
                return false;
            };
            *destination = *byte;
        }
        true
    }
}
fn air_source(
    kind: u32,
) -> (
    AirBytes,
    crate::cair_regional_mission::WingIdentity,
    usize,
    usize,
) {
    let wing = 0x10000_usize;
    let base = 0x100000000_usize;
    let mission = wing + crate::cair_wing::MISSION;
    let mut memory = AirBytes::default();
    memory.put(
        mission,
        &((base + crate::air_mission::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    memory.put(
        mission + crate::air_mission::WING,
        &(wing as u64).to_le_bytes(),
    );
    memory.put(
        mission + crate::air_mission::CURRENT_TYPE,
        &kind.to_le_bytes(),
    );
    (
        memory,
        crate::cair_regional_mission::WingIdentity {
            mission_pointer: mission,
            wing: u64::MAX,
            mission_mask: 0x100,
            ..Default::default()
        },
        wing,
        base,
    )
}

#[test]
fn air_original_current_type_zero_skips_all_unreached_words_and_stats() {
    let (memory, original, _, base) = air_source(0);
    let value = AirMissionWords::capture_observed(&memory, base, &original, |_, _, _, _| {
        panic!("zero current type must not select statistics")
    })
    .unwrap();
    assert_eq!(value.current_type, Some(0));
    assert_eq!(
        (
            value.aircraft,
            value.disruption,
            value.detection_13,
            value.detection_14
        ),
        (None, None, None, None)
    );
    assert_eq!(value.faults, 0);
    assert_eq!(memory.reads.borrow().len(), 3);
}

#[test]
fn air_original_selector_is_loaned_once_and_stat_faults_remain_independent() {
    let (mut memory, original, wing, base) = air_source(2);
    let mission = original.mission_pointer;
    let selector = wing + 0x968 + 0x18 + 1;
    let stats = wing + 0x390;
    memory.put(selector, &[255]);
    memory.put(mission + 0xe0, &(-7_i32).to_le_bytes());
    memory.put(mission + 0xa8, &i64::MAX.to_le_bytes());
    memory.put(stats + 0x13 * 8, &i64::MIN.to_le_bytes());
    let mut loaned = None;
    let value = AirMissionWords::capture_observed(
        &memory,
        base,
        &original,
        |pointer, kind, original, _| loaned = Some((pointer, kind, original)),
    )
    .unwrap();
    assert_eq!(loaned, Some((wing, 2, Ok(stats))));
    assert_eq!(value.current_type, Some(2));
    assert_eq!(value.aircraft, Some(-7));
    assert_eq!(value.disruption, Some(i64::MAX));
    assert_eq!(value.detection_13, Some(i64::MIN));
    assert_eq!(value.detection_14, None);
    assert_eq!(value.faults, 1);
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == selector)
            .count(),
        1
    );
}

#[test]
fn air_original_failed_stat_selector_never_falls_back_or_heals_one_leaf() {
    let (mut memory, original, wing, base) = air_source(2);
    memory.put(original.mission_pointer + 0xe0, &100_i32.to_le_bytes());
    memory.put(original.mission_pointer + 0xa8, &0_i64.to_le_bytes());
    memory.put(wing + 0x390 + 0x13 * 8, &1_i64.to_le_bytes());
    memory.put(wing + 0x390 + 0x14 * 8, &2_i64.to_le_bytes());
    let mut loaned = None;
    let value = AirMissionWords::capture_observed(&memory, base, &original, |_, _, selector, _| {
        loaned = Some(selector)
    })
    .unwrap();
    assert_eq!(
        loaned,
        Some(Err("transport original air mission word unreadable"))
    );
    assert_eq!((value.detection_13, value.detection_14), (None, None));
    assert_eq!(value.faults, 2);
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| at == wing + 0x390 + 0x13 * 8 || at == wing + 0x390 + 0x14 * 8)
    );
}

#[test]
fn air_mission_outside_disruption_mask_keeps_detection_without_disruption_read() {
    let (mut memory, original, wing, base) = air_source(1);
    memory.put(wing + 0x968 + 0x18, &[255]);
    memory.put(original.mission_pointer + 0xe0, &100_i32.to_le_bytes());
    memory.put(wing + 0x390 + 0x13 * 8, &7_i64.to_le_bytes());
    memory.put(wing + 0x390 + 0x14 * 8, &11_i64.to_le_bytes());
    let value = AirMissionWords::capture(&memory, base, &original).unwrap();
    assert_eq!(value.disruption, None);
    assert_eq!(
        (value.detection_13, value.detection_14),
        (Some(7), Some(11))
    );
    assert_eq!(value.faults, 0);
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| at == original.mission_pointer + 0xa8)
    );
}

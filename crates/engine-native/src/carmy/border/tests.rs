use super::*;
use engine_native_parts::csub_unit_definition;
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;

struct Trace<'a> {
    bytes: &'a [u8],
    spans: RefCell<Vec<(usize, usize)>>,
}
impl Memory for Trace<'_> {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        self.spans.borrow_mut().push((at, output.len()));
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(at, output)
    }
}
fn packet() -> StatusPacket {
    let mut packet = StatusPacket::new(3).unwrap();
    packet.units.extend([
        StatusUnitInputs {
            division: 11,
            maxima: 0,
            ..Default::default()
        },
        StatusUnitInputs {
            division: 22,
            maxima: 0,
            ..Default::default()
        },
        StatusUnitInputs {
            division: 33,
            maxima: 1,
            ..Default::default()
        },
    ]);
    packet.maxima.extend([
        engine_native_parts::csub_unit_definition::DivisionMaxima {
            pointer: 0x1000,
            ..Default::default()
        },
        engine_native_parts::csub_unit_definition::DivisionMaxima {
            pointer: usize::MAX,
            ..Default::default()
        },
    ]);
    packet.border_index.extend([(11, 0), (22, 1), (33, 2)]);
    packet.outcome = Outcome::Rows { len: 3 };
    packet
}
#[test]
fn scoped_border_stats_read_only_selected_cache_and_memo_shared_pointer() {
    let mut bytes = vec![0; 0x2000];
    bytes[0x1000..0x1008]
        .copy_from_slice(&(csub_unit_definition::VTABLE_RVA.0 as u64).to_le_bytes());
    for (offset, value) in [(0xa8, i64::MIN), (0xc0, -100_001), (0xc8, i64::MAX)] {
        bytes[0x1000 + offset..0x1008 + offset].copy_from_slice(&value.to_le_bytes());
    }
    let memory = Trace {
        bytes: &bytes,
        spans: RefCell::new(Vec::with_capacity(3)),
    };
    let mut packet = packet();
    let allocated = packet.allocated_bytes();
    assert_eq!(
        packet.capture_border_stats(&memory, 0, [11, 22]),
        Outcome::Rows { len: 3 }
    );
    assert_eq!(
        packet.units[0].border_combat_stats,
        Some([i64::MIN, -100_001, i64::MAX])
    );
    assert_eq!(
        packet.units[1].border_combat_stats,
        packet.units[0].border_combat_stats
    );
    assert_eq!(packet.units[2].border_combat_stats, None);
    assert_eq!(packet.allocated_bytes(), allocated);
    let spans = memory.spans.borrow();
    assert_eq!(spans.iter().filter(|&&(at, _)| at == 0x10a8).count(), 1);
    assert!(spans.contains(&(0x10c0, 16)));
}
#[test]
fn demanded_cache_failure_and_missing_handle_never_become_outside_demand() {
    for handle in [33, 44] {
        let mut packet = packet();
        assert!(matches!(
            packet.capture_border_stats(
                &SliceMemory {
                    base: 0,
                    bytes: &[]
                },
                0,
                [handle]
            ),
            Outcome::Unavailable { .. }
        ));
    }
}

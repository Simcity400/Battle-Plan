use super::*;
use crate::memory::SliceMemory;

const STATUS: usize = 0x100;
const SUBJECT: usize = 0x600;

fn fixture(kind: u32, parties: [u32; 2]) -> Vec<u8> {
    let mut bytes = vec![0; 0x800];
    bytes[STATUS + SUBJECT_RELATION..STATUS + SUBJECT_RELATION + 8]
        .copy_from_slice(&(SUBJECT as u64).to_le_bytes());
    bytes[SUBJECT..SUBJECT + 8]
        .copy_from_slice(&(csubject_relation::VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[SUBJECT + crelation::TYPE_ID..SUBJECT + crelation::TYPE_ID + 4]
        .copy_from_slice(&kind.to_le_bytes());
    for (index, party) in parties.into_iter().enumerate() {
        let at = SUBJECT + crelation::FIRST_COUNTRY + index * 4;
        bytes[at..at + 4].copy_from_slice(&party.to_le_bytes());
    }
    bytes
}

struct Fault<'a> {
    bytes: &'a [u8],
    address: usize,
}

impl Memory for Fault<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address <= self.address && self.address < address.saturating_add(output.len()) {
            false
        } else {
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, output)
        }
    }
}

#[test]
fn null_and_wrong_type_do_not_read_unreached_parties() {
    let mut bytes = fixture(0x380a, [0, u32::MAX]);
    let memory = Fault {
        bytes: &bytes,
        address: SUBJECT + crelation::FIRST_COUNTRY,
    };
    assert_eq!(
        zero_subject_at(&memory, 0, STATUS, &mut ForwardZeroObservation::default()),
        Ok(ZeroSubjectInputs {
            relation_type: Some(0x380a),
            parties: None,
        })
    );
    bytes[STATUS + SUBJECT_RELATION..STATUS + SUBJECT_RELATION + 8].fill(0);
    let memory = Fault {
        bytes: &bytes,
        address: SUBJECT,
    };
    assert_eq!(
        zero_subject_at(&memory, 0, STATUS, &mut ForwardZeroObservation::default()),
        Ok(ZeroSubjectInputs::default())
    );
}

#[test]
fn subject_parties_preserve_zero_alias_and_extreme_raw_references() {
    for parties in [[0, u32::MAX], [7, 0], [7, 71]] {
        let bytes = fixture(csubject_relation::TYPE_VALUE, parties);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            zero_subject_at(&memory, 0, STATUS, &mut ForwardZeroObservation::default()),
            Ok(ZeroSubjectInputs {
                relation_type: Some(csubject_relation::TYPE_VALUE),
                parties: Some(if parties[0] == 0 {
                    [Some(0), None]
                } else {
                    parties.map(Some)
                }),
            })
        );
    }
}

#[test]
fn reached_faults_are_independent_of_zero_war_and_never_become_null() {
    let bytes = fixture(csubject_relation::TYPE_VALUE, [7, 71]);
    for (address, reason) in [
        (
            STATUS + SUBJECT_RELATION,
            "slot-0 subject pointer unreadable",
        ),
        (
            SUBJECT + crelation::TYPE_ID,
            "slot-0 subject type unreadable",
        ),
        (SUBJECT, "slot-0 subject vtable mismatch"),
        (
            SUBJECT + crelation::FIRST_COUNTRY,
            "slot-0 subject first party unreadable",
        ),
        (
            SUBJECT + crelation::SECOND_COUNTRY,
            "slot-0 subject second party unreadable",
        ),
    ] {
        let memory = Fault {
            bytes: &bytes,
            address,
        };
        assert_eq!(
            zero_subject_at(&memory, 0, STATUS, &mut ForwardZeroObservation::default()),
            Err(reason)
        );
        assert_eq!(zero_war_at(&memory, STATUS), Ok([0, 0]));
    }
}

#[test]
fn first_zero_skips_second_and_failed_first_can_preserve_a_known_second_zero() {
    struct Counting<'a> {
        memory: Fault<'a>,
        first: std::cell::Cell<usize>,
        second: std::cell::Cell<usize>,
    }
    impl Memory for Counting<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            for (at, count) in [
                (SUBJECT + crelation::FIRST_COUNTRY, &self.first),
                (SUBJECT + crelation::SECOND_COUNTRY, &self.second),
            ] {
                if address <= at && at < address.saturating_add(output.len()) {
                    count.set(count.get() + 1);
                }
            }
            self.memory.copy(address, output)
        }
    }
    for (parties, fault, expected, second_reads) in [
        ([0, 71], crelation::SECOND_COUNTRY, [Some(0), None], 0),
        ([7, 0], crelation::FIRST_COUNTRY, [None, Some(0)], 1),
    ] {
        let bytes = fixture(csubject_relation::TYPE_VALUE, parties);
        let memory = Counting {
            memory: Fault {
                bytes: &bytes,
                address: SUBJECT + fault,
            },
            first: std::cell::Cell::new(0),
            second: std::cell::Cell::new(0),
        };
        let value = ZeroSubjectInputs {
            relation_type: Some(csubject_relation::TYPE_VALUE),
            parties: Some(expected),
        };
        assert_eq!(
            zero_subject_at(&memory, 0, STATUS, &mut ForwardZeroObservation::default()),
            Ok(value)
        );
        assert_eq!(value.read_refusal().is_some(), expected[0].is_none());
        assert_eq!(memory.first.get(), 1);
        assert_eq!(memory.second.get(), second_reads);
    }
}

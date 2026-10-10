//! Authored source-only fixtures. UNRUN; these provide no native acceptance.
use super::*;
use std::cell::{Cell, RefCell};

const TEXT: usize = 0x40;
const PAYLOAD: usize = 0x100;
struct Traced {
    bytes: Vec<u8>,
    reads: RefCell<Vec<(usize, usize)>>,
    fail_once: Cell<Option<usize>>,
}
impl Traced {
    fn new() -> Self {
        Self {
            bytes: vec![0; 0x1000],
            reads: RefCell::new(Vec::new()),
            fail_once: Cell::new(None),
        }
    }
    fn put(&mut self, address: usize, bytes: &[u8]) {
        self.bytes[address..address + bytes.len()].copy_from_slice(bytes);
    }
    fn heap(&mut self, payload: &[u8]) {
        self.put(TEXT, &(PAYLOAD as u64).to_le_bytes());
        self.put(
            TEXT + msvc_string::LENGTH,
            &(payload.len() as u64).to_le_bytes(),
        );
        self.put(TEXT + msvc_string::CAPACITY, &31_u64.to_le_bytes());
        self.put(PAYLOAD, payload);
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.put(address, &(data as u64).to_le_bytes());
        self.put(address + 8, &count.to_le_bytes());
        self.put(address + 12, &count.to_le_bytes());
    }
}
impl Memory for Traced {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self.fail_once.get() == Some(address) {
            self.fail_once.set(None);
            return false;
        }
        let Some(end) = address.checked_add(output.len()) else {
            return false;
        };
        let Some(source) = self.bytes.get(address..end) else {
            return false;
        };
        output.copy_from_slice(source);
        true
    }
}
fn journal(occurrences: usize) -> Journal {
    let mut journal = Journal::try_new(Capacity {
        occurrences,
        links: 16,
        demands: 16,
        bytes: 128,
    })
    .unwrap();
    journal.begin(0, 0x200, 71);
    journal
}

#[test]
fn worker_journal_grows_every_arena_before_first_and_later_publication() {
    for count in [1_500, 5_000] {
        let mut memory = Traced::new();
        memory.heap(b"seventeen_bytekey");
        let mut journal = Journal::vacant();
        journal.enable_growth();
        journal.begin(0, 0x200, 71);
        journal.link_controls(Ok([Array { data: 1, count }, Array { data: 2, count }]));
        for raw in 1..count {
            let index = journal.begin_text(raw as i32, Source::Countries, TEXT);
            journal.finish_text(
                index,
                TextAttempt {
                    address: TEXT,
                    length: Attempt::Completed(Ok(17)),
                    capacity: Attempt::Completed(Ok(31)),
                    policy_limited: true,
                    ..Default::default()
                },
                None,
            );
            journal.link(raw, Ok(raw));
            let mut loan = journal.map_loan();
            loan.admit(raw).unwrap();
            loan.observe(
                raw,
                Ok(ccountry::Country {
                    pointer: raw as usize,
                    canonical_slot: raw,
                    id: raw,
                }),
            );
            journal.demand_tag(&memory, raw as i32);
            assert_eq!(tag_bytes(&journal, raw as i32), b"seventeen_bytekey");
        }
        journal.seal();
        assert!(journal.complete());
        assert_eq!(journal.entries().len(), count as usize - 1);
        assert_eq!(journal.required().links, count as usize);
        let before = memory.reads.borrow().len();
        journal.demand_tag(&memory, (count - 1) as i32);
        assert_eq!(
            memory.reads.borrow().len(),
            before,
            "original payload never retried"
        );
        // An unchanged roster can acquire a much larger mod-authored tag later.
        let payload = vec![b'x'; 5_001];
        memory.bytes.resize(PAYLOAD + payload.len() + 1, 0);
        memory.heap(&payload);
        memory.put(
            TEXT + msvc_string::CAPACITY,
            &(payload.len() as u64).to_le_bytes(),
        );
        journal.begin(0, 0x200, 72);
        let index = journal.begin_text(1, Source::Countries, TEXT);
        journal.finish_text(
            index,
            TextAttempt {
                address: TEXT,
                length: Attempt::Completed(Ok(payload.len() as u64)),
                capacity: Attempt::Completed(Ok(payload.len() as u64)),
                policy_limited: true,
                ..Default::default()
            },
            None,
        );
        journal.demand_tag(&memory, 1);
        journal.seal();
        assert!(journal.complete());
        assert_eq!(tag_bytes(&journal, 1), payload);
    }
    let mut fixed = Journal::vacant();
    assert!(fixed.begin_text(1, Source::Countries, TEXT).is_none());
    fixed.seal();
    assert!(!fixed.complete());
}
fn bounded(memory: &Traced, journal: &mut Journal) {
    assert!(
        msvc_string::read_text_observed::<8, false>(memory, TEXT, 5, Source::Countries, journal)
            .is_none()
    );
}
fn tag_bytes(journal: &Journal, raw: i32) -> &[u8] {
    let entry = journal
        .entries()
        .iter()
        .find(|entry| entry.raw == raw)
        .unwrap();
    let Tag::Bytes(span) = entry.tag else {
        panic!("tag refused: {:?}", entry.tag);
    };
    journal.bytes(span).unwrap()
}

#[test]
fn bounded_prefix_completes_only_unattempted_stages_once() {
    for prefetched in [false, true] {
        let mut memory = Traced::new();
        memory.heap(b"seventeen_bytekey");
        let mut journal = journal(16);
        let result = if prefetched {
            msvc_string::read_text_observed::<8, true>(
                &memory,
                TEXT,
                5,
                Source::Countries,
                &mut journal,
            )
        } else {
            msvc_string::read_text_observed::<8, false>(
                &memory,
                TEXT,
                5,
                Source::Countries,
                &mut journal,
            )
        };
        assert!(result.is_none());
        assert_eq!(
            *memory.reads.borrow(),
            [
                (TEXT + msvc_string::LENGTH, 8),
                (TEXT + msvc_string::CAPACITY, 8)
            ]
        );
        journal.demand_tag(&memory, 5);
        journal.demand_tag(&memory, 5);
        assert_eq!(tag_bytes(&journal, 5), b"seventeen_bytekey");
        assert_eq!(
            *memory.reads.borrow(),
            [
                (TEXT + msvc_string::LENGTH, 8),
                (TEXT + msvc_string::CAPACITY, 8),
                (TEXT, 8),
                (PAYLOAD + 17, 1),
                (PAYLOAD, 17)
            ]
        );
    }
}

#[test]
fn earliest_text_fault_survives_later_original_success() {
    let mut memory = Traced::new();
    memory.heap(b"tag");
    let mut journal = journal(16);
    memory.fail_once.set(Some(TEXT + msvc_string::LENGTH));
    bounded(&memory, &mut journal);
    assert!(
        msvc_string::read_text_observed::<8, false>(
            &memory,
            TEXT,
            5,
            Source::AppearanceOriginal,
            &mut journal
        )
        .is_some()
    );
    let before = memory.reads.borrow().len();
    journal.demand_tag(&memory, 5);
    assert_eq!(
        journal.entries()[0].tag,
        Tag::Refused("string length unreadable")
    );
    assert_eq!(memory.reads.borrow().len(), before);
}

#[test]
fn growing_journal_keeps_first_original_fault_and_never_retries_it() {
    let mut memory = Traced::new();
    memory.heap(b"tag");
    let mut journal = Journal::vacant();
    journal.enable_growth();
    memory.fail_once.set(Some(TEXT + msvc_string::LENGTH));
    bounded(&memory, &mut journal);
    assert!(
        msvc_string::read_text_observed::<8, false>(
            &memory,
            TEXT,
            5,
            Source::AppearanceOriginal,
            &mut journal
        )
        .is_some()
    );
    let before = memory.reads.borrow().len();
    journal.demand_tag(&memory, 5);
    assert_eq!(
        journal.entries()[0].tag,
        Tag::Refused("string length unreadable")
    );
    assert_eq!(memory.reads.borrow().len(), before);
}

#[test]
fn exact_original_header_fault_beats_generic_success_without_retry() {
    let memory = Traced::new();
    let mut journal = journal(16);
    journal.tag_control(Ok(Array {
        data: 0x600,
        count: 8,
    }));
    memory
        .fail_once
        .set(Some(0x200 + crate::cgame_state::COUNTRY_TAGS));
    let mut row = ccountry::AppearanceRow::default();
    row.original_tag_reference = 5;
    let mut names = [0; 64];
    let mut used = 0;
    let mut required = 0;
    ccountry::capture_appearance_original_tags_observed(
        &memory,
        0x200,
        std::slice::from_mut(&mut row),
        &mut names,
        &mut used,
        &mut required,
        &mut journal,
    );
    let before = memory.reads.borrow().len();
    journal.tag_control(Ok(Array {
        data: 0x600,
        count: 8,
    }));
    journal.demand_tag(&memory, 5);
    assert_eq!(
        journal.entries()[0].tag,
        Tag::Refused("country original tag strings unreadable or invalid")
    );
    assert_eq!(memory.reads.borrow().len(), before);
}

#[test]
fn bounded_prefix_reuses_later_descriptor_controls_and_owned_payload() {
    for copied in [false, true] {
        let mut memory = Traced::new();
        memory.heap(b"seventeen_bytekey");
        let mut journal = journal(16);
        bounded(&memory, &mut journal);
        let descriptor = msvc_string::Descriptor::read_observed(
            &memory,
            TEXT,
            5,
            Source::AppearanceOriginal,
            &mut journal,
        )
        .unwrap();
        if copied {
            descriptor
                .copy_observed(&memory, &mut [0; 17], &mut journal)
                .unwrap();
        }
        let before = memory.reads.borrow().len();
        journal.demand_tag(&memory, 5);
        assert_eq!(tag_bytes(&journal, 5), b"seventeen_bytekey");
        assert_eq!(
            &memory.reads.borrow()[before..],
            if copied {
                &[][..]
            } else {
                &[(PAYLOAD, 17)][..]
            }
        );
    }
}

#[test]
fn partial_originals_keep_later_payload_fault_and_changed_controls_final() {
    for descriptor_first in [false, true] {
        let mut memory = Traced::new();
        memory.heap(b"seventeen_bytekey");
        let mut journal = journal(16);
        if descriptor_first {
            msvc_string::Descriptor::read_observed(
                &memory,
                TEXT,
                5,
                Source::Countries,
                &mut journal,
            )
            .unwrap();
        } else {
            bounded(&memory, &mut journal);
        }
        let later = msvc_string::Descriptor::read_observed(
            &memory,
            TEXT,
            5,
            Source::AppearanceOriginal,
            &mut journal,
        )
        .unwrap();
        memory.fail_once.set(Some(PAYLOAD));
        assert!(
            later
                .copy_observed(&memory, &mut [0; 17], &mut journal)
                .is_err()
        );
        let before = memory.reads.borrow().len();
        journal.demand_tag(&memory, 5);
        assert!(matches!(journal.entries()[0].tag, Tag::Refused(_)));
        assert_eq!(memory.reads.borrow().len(), before);
    }
    let mut memory = Traced::new();
    memory.heap(b"seventeen_bytekey");
    let mut journal = journal(16);
    bounded(&memory, &mut journal);
    memory.put(TEXT + msvc_string::LENGTH, &16_u64.to_le_bytes());
    memory.put(PAYLOAD + 16, &[0]);
    let later = msvc_string::Descriptor::read_observed(
        &memory,
        TEXT,
        5,
        Source::AppearanceOriginal,
        &mut journal,
    )
    .unwrap();
    later
        .copy_observed(&memory, &mut [0; 16], &mut journal)
        .unwrap();
    let before = memory.reads.borrow().len();
    journal.demand_tag(&memory, 5);
    assert_eq!(
        journal.entries()[0].tag,
        Tag::Refused("raw country original tag controls changed")
    );
    assert_eq!(memory.reads.borrow().len(), before);
}

#[test]
fn lost_later_occurrence_blocks_new_native_completion() {
    let mut memory = Traced::new();
    memory.heap(b"seventeen_bytekey");
    let mut journal = journal(1);
    bounded(&memory, &mut journal);
    bounded(&memory, &mut journal);
    let before = memory.reads.borrow().len();
    journal.demand_tag(&memory, 5);
    assert_eq!(
        journal.entries()[0].tag,
        Tag::Refused("raw country original observation incomplete")
    );
    assert_eq!(memory.reads.borrow().len(), before);
    journal.seal();
    assert!(!journal.complete());
}

#[test]
fn observed_alias_zero_and_link_fault_are_loaned_without_native_retry() {
    let memory = Traced::new();
    let mut journal = journal(16);
    journal.link_controls(Ok([
        Array {
            data: 0x500,
            count: 2,
        },
        Array {
            data: 0x600,
            count: 16,
        },
    ]));
    journal.link(5, Ok(0));
    journal.link(6, Err("original link fault"));
    journal.link(6, Ok(1));
    let mut loan = journal.map_loan();
    let reader = ccountry::Reader::with_link_observations(&memory, 0, 0x200, loan.links).unwrap();
    assert_eq!(reader.link(5), Ok(None));
    assert_eq!(reader.link(6), Err("original link fault"));
    loan.admit(5).unwrap();
    loan.observe(
        5,
        Ok(ccountry::Country {
            pointer: 0x900,
            canonical_slot: 0,
            id: 0,
        }),
    );
    loan.observe(6, Err("original link fault"));
    loan.observe(
        6,
        Ok(ccountry::Country {
            pointer: 0x980,
            canonical_slot: 1,
            id: 1,
        }),
    );
    assert_eq!(
        journal.entries()[0].mapping,
        Mapping::Observed {
            canonical_slot: 0,
            country_id: 0
        }
    );
    assert_eq!(
        journal.entries()[1].mapping,
        Mapping::Refused("original link fault")
    );
    assert!(memory.reads.borrow().is_empty());
}

#[test]
fn early_country_header_fault_cannot_be_healed_by_later_original_walk() {
    let mut memory = Traced::new();
    let state = 0x200;
    memory.array(state + crate::cgame_state::COUNTRIES, 0x800, 2);
    memory.array(state + crate::cgame_state::COUNTRY_LINKS, 0x900, 1);
    memory
        .fail_once
        .set(Some(state + crate::cgame_state::COUNTRIES));
    let mut journal = journal(16);
    assert!(matches!(
        ccountry::walk_observed(&memory, 0, state, &mut [], &mut journal),
        Outcome::Unavailable { .. }
    ));
    assert_eq!(
        ccountry::walk_links_observed(&memory, 0, state, &mut [], &mut journal),
        Outcome::Rows { len: 0 }
    );
    let before = memory.reads.borrow().len();
    let loan = journal.map_loan();
    assert!(matches!(
        ccountry::Reader::with_link_observations(&memory, 0, state, loan.links),
        Err("country array unreadable or invalid")
    ));
    assert_eq!(memory.reads.borrow().len(), before);
}

#[test]
fn empty_bytes_refusal_and_unreached_side_survive_pointer_retirement() {
    let mut memory = Traced::new();
    memory.put(
        TEXT + msvc_string::CAPACITY,
        &msvc_string::INLINE_CAPACITY.to_le_bytes(),
    );
    let mut journal = journal(16);
    assert!(
        msvc_string::read_text_observed::<8, false>(
            &memory,
            TEXT,
            5,
            Source::Countries,
            &mut journal
        )
        .is_some()
    );
    journal.demand_tag(&memory, 5);
    journal.demand_tag(&memory, -1);
    journal.map_loan().observe(6, Err("mapping fault"));
    journal.sort();
    journal.seal();
    let required = journal.required();
    journal.finish_native();
    assert!(journal.complete());
    assert_eq!(journal.required(), required);
    assert!(tag_bytes(&journal, 5).is_empty());
    assert!(matches!(journal.entries()[0].tag, Tag::Refused(_)));
    assert_eq!(journal.entries()[2].tag, Tag::Unreached);
    assert!(journal.occurrences.is_empty() && journal.controls.is_none() && journal.tags.is_none());
    assert_eq!((journal.base, journal.state), (0, 0));
}

#[test]
fn observed_capacity_demand_survives_retirement_and_reusable_growth() {
    let memory = Traced::new();
    let mut journal = Journal::vacant();
    journal.begin(0, 0x200, 71);
    journal.link_controls(Ok([
        Array {
            data: 0x500,
            count: 2,
        },
        Array {
            data: 0x600,
            count: 4,
        },
    ]));
    journal.begin_text(5, Source::Countries, TEXT);
    journal.demand_tag(&memory, -1);
    let required = journal.required();
    journal.finish_native();
    assert_eq!(journal.required(), required);
    assert!(required.occurrences >= 2 && required.links == 4 && required.demands == 1);
    journal.reserve(required).unwrap();
    journal.begin(0, 0x200, 72);
    journal.demand_tag(&memory, -1);
    journal.seal();
    assert!(journal.complete());
    assert!(matches!(journal.entries()[0].tag, Tag::Refused(_)));
    assert!(memory.reads.borrow().is_empty());
}

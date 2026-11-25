use super::*;
use crate::corders_group as group;
use engine_native_memory::memory::SliceMemory;
use std::cell::Cell;

struct Fixture {
    bytes: Vec<u8>,
    roots: Vec<group::Root>,
}
impl Fixture {
    fn new() -> Self {
        let mut f = Self {
            bytes: vec![0; 0x9000],
            roots: Vec::new(),
        };
        for (i, pointer) in [0x1000, 0x3000].into_iter().enumerate() {
            f.put(pointer, VTABLE_RVA.0 as u64);
            f.put(
                pointer + TYPE,
                ((i as u64 + 1) << 32) | u64::from(REFERENCE_TYPE),
            );
            f.put(
                pointer + CHARACTER_TYPE,
                if i == 0 { (99 << 32) | 73 } else { 0 },
            );
            for (lane, offset) in PORTRAIT_STRINGS.into_iter().enumerate() {
                f.bytes[pointer + offset] = b'a' + lane as u8 + i as u8;
                f.put(pointer + offset + 16, 1);
                f.put(pointer + offset + 24, 15);
            }
        }
        for (i, (provider, leader)) in [(1, 0x1000), (0, u64::MAX), (1, 0), (1, 0x1000)]
            .into_iter()
            .enumerate()
        {
            let pointer = 0x6000 + i * 0x100;
            f.roots.push(group::Root {
                pointer,
                handle: ((i as u64 + 1) << 32) | u64::from(group::REFERENCE_TYPE),
                country: 1,
            });
            f.put(pointer + group::HQ_PROVIDER, provider);
            f.put(pointer + group::LEADER, leader);
        }
        f
    }
    fn put(&mut self, pointer: usize, value: u64) {
        self.bytes[pointer..pointer + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn memory(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
    }
}

#[test]
fn hq_scope_is_camera_independent_deduplicated_and_fresh() {
    let mut f = Fixture::new();
    let mut sources = vec![(0, 0); 4];
    let mut rows = vec![PortraitRow::default(); 4];
    let result = walk_hq_portraits(&f.memory(), 0, &f.roots, &mut sources, &mut rows);
    assert_eq!(result.outcome, Outcome::Rows { len: 1 });
    assert_eq!(result.sources, Outcome::Rows { len: 1 });
    assert_eq!(sources[0], ((1 << 32) | u64::from(REFERENCE_TYPE), 0x1000));
    assert_eq!(rows[0].character, (99 << 32) | 73);
    assert_eq!(rows[0].strings.iter().map(|s| s.as_bytes()[0]).collect::<Vec<_>>(), [b'a', b'b', b'c']);
    f.put(0x6300 + group::LEADER, 0x3000);
    let result = walk_hq_portraits(&f.memory(), 0, &f.roots, &mut sources, &mut rows);
    assert_eq!(result.outcome, Outcome::Rows { len: 2 });
    assert_eq!(rows[1].character, 0);
    assert_eq!(rows[1].strings.iter().map(|s| s.as_bytes()[0]).collect::<Vec<_>>(), [b'b', b'c', b'd']);
    for index in 0..4 {
        f.put(0x6000 + index * 0x100 + group::HQ_PROVIDER, 0);
    }
    let result = walk_hq_portraits(&f.memory(), 0, &f.roots, &mut sources, &mut rows);
    assert_eq!(result.outcome, Outcome::Rows { len: 0 });
    assert_eq!(result.sources, Outcome::Rows { len: 0 });
    assert_eq!(
        walk_portrait_paths_memo(
            &SliceMemory {
                base: 0,
                bytes: &[]
            },
            0,
            0,
            &[],
            &mut Vec::new(),
            &mut PortraitTagMemo::default()
        ),
        Outcome::Rows { len: 0 }
    );
}

#[test]
fn hq_conflicting_identity_is_explicit() {
    let mut f = Fixture::new();
    let mut sources = vec![(0, 0); 4];
    let mut rows = vec![PortraitRow::default(); 2];
    f.put(0x6300 + group::LEADER, 0x3000);
    f.put(0x3000 + TYPE, (1 << 32) | u64::from(REFERENCE_TYPE));
    let result = walk_hq_portraits(&f.memory(), 0, &f.roots, &mut sources, &mut rows);
    assert_eq!(
        result.outcome,
        Outcome::Unavailable {
            reason: "HQ leader identity has multiple pointers"
        }
    );
}

struct FaultMemory<'a> {
    memory: SliceMemory<'a>,
    calls: Cell<usize>,
    fail: usize,
}
impl Memory for FaultMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let call = self.calls.get() + 1;
        self.calls.set(call);
        call != self.fail && self.memory.copy(address, output)
    }
}

#[test]
fn hq_every_required_read_failure_withholds_the_table() {
    let f = Fixture::new();
    let mut sources = vec![(0, 0); 4];
    let mut rows = vec![PortraitRow::default(); 2];
    let memory = FaultMemory {
        memory: f.memory(),
        calls: Cell::new(0),
        fail: 0,
    };
    assert_eq!(
        walk_hq_portraits(&memory, 0, &f.roots, &mut sources, &mut rows).outcome,
        Outcome::Rows { len: 1 }
    );
    for fail in 1..=memory.calls.get() {
        let memory = FaultMemory {
            memory: f.memory(),
            calls: Cell::new(0),
            fail,
        };
        assert!(
            matches!(
                walk_hq_portraits(&memory, 0, &f.roots, &mut sources, &mut rows).outcome,
                Outcome::Unavailable { .. }
            ),
            "read {fail}"
        );
    }
}

#[derive(Default)]
struct Bytes(std::collections::BTreeMap<usize, u8>);
impl Bytes {
    fn put(&mut self, at: usize, value: &[u8]) {
        self.0
            .extend(value.iter().enumerate().map(|(i, &b)| (at + i, b)));
    }
}
impl Memory for Bytes {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        output.iter_mut().enumerate().all(|(i, b)| {
            self.0
                .get(&(address + i))
                .map(|value| *b = *value)
                .is_some()
        })
    }
}

#[test]
fn worker_portrait_paths_grow_source_rows_country_slot_memo_and_long_tag_bytes() {
    use crate::{ccountry, cgame_state as gs};
    const GS: usize = 0x4000;
    const POINTERS: usize = 0x100000;
    const LINKS: usize = 0x200000;
    const TAGS: usize = 0x300000;
    const COUNTRY: usize = 0x500000;
    const LEADERS: usize = 0x1000000;
    const PAYLOAD: usize = 0x8000000;
    let mut rows = Vec::new();
    let mut memo = PortraitTagMemo::default();
    for (countries, len) in [(5_000, 513), (12_000, 1025)] {
        let mut bytes = Bytes::default();
        let array = |f: &mut Bytes, at: usize, data: usize, count: u32| {
            f.put(at, &(data as u64).to_le_bytes());
            f.put(at + 8, &count.to_le_bytes());
            f.put(at + 12, &count.to_le_bytes());
        };
        array(&mut bytes, GS + gs::COUNTRIES, POINTERS, countries + 1);
        array(&mut bytes, GS + gs::COUNTRY_LINKS, LINKS, countries + 1);
        array(&mut bytes, GS + gs::COUNTRY_TAGS, TAGS, countries + 1);
        bytes.put(
            POINTERS + countries as usize * 8,
            &(COUNTRY as u64).to_le_bytes(),
        );
        bytes.put(LINKS + countries as usize * 4, &countries.to_le_bytes());
        bytes.put(COUNTRY, &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes.put(COUNTRY + ccountry::ID, &countries.to_le_bytes());
        bytes.put(COUNTRY + ccountry::ORIGINAL_TAG, &countries.to_le_bytes());
        let tag = TAGS + countries as usize * msvc_string::STRIDE;
        bytes.put(tag, &(PAYLOAD as u64).to_le_bytes());
        bytes.put(tag + 16, &(len as u64).to_le_bytes());
        bytes.put(tag + 24, &(len as u64).to_le_bytes());
        bytes.put(PAYLOAD, &vec![0xff; len]);
        bytes.put(PAYLOAD + len, &[0]);
        let sources: Vec<_> = (0..5_000)
            .map(|index| {
                let pointer = LEADERS + index * 0x1000;
                let leader = ((index as u64 + 1) << 32) | u64::from(REFERENCE_TYPE);
                bytes.put(pointer, &(VTABLE_RVA.0 as u64).to_le_bytes());
                bytes.put(pointer + TYPE, &leader.to_le_bytes());
                bytes.put(pointer + super::COUNTRY, &countries.to_le_bytes());
                (leader, pointer)
            })
            .collect();
        assert_eq!(
            walk_portrait_paths_memo(&bytes, 0, GS, &sources, &mut rows, &mut memo),
            Outcome::Rows { len: 5_000 }
        );
        assert!(memo.cells.len() > countries as usize);
        assert_eq!(rows[4_999].tag.as_bytes(), vec![0xff; len]);
    }
}

#[test]
fn portrait_paths_resolve_original_tag_strings_and_keep_unlinked_countries_empty() {
    use crate::{ccountry, cgame_state as gs};
    use engine_native_memory::read_census::ReadCensus;
    use std::cell::RefCell;
    const GS: usize = 0x4000;
    const COUNTRIES: usize = 0x4800;
    const LINKS: usize = 0x5000;
    const TAGS: usize = 0x5200;
    const COUNTRY: usize = 0x9000;
    let mut f = Bytes::default();
    let ids = [(7u64 << 32) | 4713, (8u64 << 32) | 4713];
    let sources = [(ids[0], 0x6000), (ids[1], 0x7000)];
    for (id, pointer, raw) in [(ids[0], 0x6000, 2u32), (ids[1], 0x7000, 3)] {
        f.put(pointer, &[0; 0x1000]);
        f.put(pointer, &(VTABLE_RVA.0 as u64).to_le_bytes());
        f.put(pointer + 8, &id.to_le_bytes());
        f.put(pointer + super::COUNTRY, &raw.to_le_bytes());
        for offset in PORTRAIT_STRINGS {
            f.put(pointer + offset + 24, &15u64.to_le_bytes());
        }
    }
    f.put(GS, &[0; 0x400]);
    let array = |f: &mut Bytes, at: usize, data: usize, count: u32| {
        f.put(at, &(data as u64).to_le_bytes());
        f.put(at + 8, &count.to_le_bytes());
        f.put(at + 12, &count.to_le_bytes());
    };
    array(&mut f, GS + gs::COUNTRIES, COUNTRIES, 3);
    array(&mut f, GS + gs::COUNTRY_LINKS, LINKS, 4);
    array(&mut f, GS + gs::COUNTRY_TAGS, TAGS, 4);
    f.put(COUNTRIES, &[0; 24]);
    f.put(COUNTRIES + 8, &(COUNTRY as u64).to_le_bytes());
    f.put(LINKS, &[0; 16]);
    f.put(LINKS + 4, &1u32.to_le_bytes());
    f.put(LINKS + 8, &1u32.to_le_bytes());
    f.put(COUNTRY, &[0; 0x1400]);
    f.put(COUNTRY, &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
    f.put(COUNTRY + ccountry::ID, &2u32.to_le_bytes());
    f.put(COUNTRY + ccountry::ORIGINAL_TAG, &1i32.to_le_bytes());
    f.put(TAGS, &[0; 0x80]);
    f.put(TAGS + 0x20, b"GER");
    f.put(TAGS + 0x20 + 16, &3u64.to_le_bytes());
    f.put(TAGS + 0x20 + 24, &15u64.to_le_bytes());
    let mut rows = vec![PortraitPathRow::default(); 2];
    let census = ReadCensus::new(&f);
    assert_eq!(
        walk_portrait_paths(&census, 0, GS, &sources, &mut rows),
        Outcome::Rows { len: 2 }
    );
    assert_eq!((census.calls(), census.bytes()), (19, 160));
    assert_eq!((rows[0].country, rows[0].tag_reference), (2, 1));
    assert_eq!(rows[0].tag.as_bytes(), b"GER");
    assert_eq!((rows[1].country, rows[1].tag_reference), (3, 0));
    assert!(rows[1].tag.as_bytes().is_empty());
    let expected = rows.clone();

    f.put(0x6008, &(ids[0] + (1 << 32)).to_le_bytes());
    assert_eq!(
        walk_portrait_paths(&f, 0, GS, &sources, &mut rows),
        Outcome::Unavailable {
            reason: "leader portrait path identity mismatch"
        }
    );
    f.put(0x6008, &ids[0].to_le_bytes());
    f.put(0x6000, &0u64.to_le_bytes());
    assert_eq!(
        walk_portrait_paths(&f, 0, GS, &sources, &mut rows),
        Outcome::Unavailable {
            reason: "unit leader vtable mismatch"
        }
    );
    f.put(0x6000, &(VTABLE_RVA.0 as u64).to_le_bytes());

    f.put(LINKS + 12, &1u32.to_le_bytes());
    let census = ReadCensus::new(&f);
    assert_eq!(
        walk_portrait_paths(&census, 0, GS, &sources, &mut rows),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(
        rows.iter().map(|row| (row.country, row.tag_reference)).collect::<Vec<_>>(),
        [(2, 1), (3, 1)]
    );
    assert!(rows.iter().all(|row| row.tag.as_bytes() == b"GER"));
    assert_eq!((census.calls(), census.bytes()), (25, 200));

    struct Observed<'a> {
        source: &'a Bytes,
        reads: RefCell<Vec<(usize, usize)>>,
        fault: usize,
    }
    impl Memory for Observed<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            if self.fault == address {
                let prefix = output.len() / 2;
                self.source.copy(address, &mut output[..prefix]);
                return false;
            }
            self.source.copy(address, output)
        }
    }
    for (fault, reason) in [
        (
            COUNTRY + ccountry::ORIGINAL_TAG,
            "leader portrait original tag unreadable",
        ),
        (
            TAGS + 0x20 + 16,
            "leader portrait tag string invalid",
        ),
        (
            TAGS + 0x20 + 24,
            "leader portrait tag string invalid",
        ),
        (
            TAGS + 0x20 + 3,
            "leader portrait tag string invalid",
        ),
        (
            TAGS + 0x20,
            "leader portrait tag string invalid",
        ),
        (0x7000, "unit leader identity unreadable"),
        (
            0x7000 + super::COUNTRY,
            "leader portrait country unreadable",
        ),
        (
            LINKS + 12,
            "country link unreadable or outside country table",
        ),
    ] {
        let observed = Observed {
            source: &f,
            reads: RefCell::new(Vec::new()),
            fault,
        };
        assert_eq!(
            walk_portrait_paths(&observed, 0, GS, &sources, &mut rows),
            Outcome::Unavailable { reason }
        );
        assert_eq!(observed.reads.borrow().last().unwrap().0, fault);
    }

    f.put(TAGS + 0x20, &[0xff, 0, b'X']);
    assert_eq!(
        walk_portrait_paths(&f, 0, GS, &sources, &mut rows),
        Outcome::Rows { len: 2 }
    );
    assert!(rows.iter().all(|row| row.tag.as_bytes() == [0xff, 0, b'X']));

    let mut heap_tag = [0xa5; 21];
    heap_tag[20] = 0;
    f.put(0xd000, &heap_tag);
    f.put(TAGS + 0x20, &0xd000u64.to_le_bytes());
    f.put(TAGS + 0x20 + 16, &20u64.to_le_bytes());
    f.put(TAGS + 0x20 + 24, &31u64.to_le_bytes());
    let census = ReadCensus::new(&f);
    assert_eq!(
        walk_portrait_paths(&census, 0, GS, &sources, &mut rows),
        Outcome::Rows { len: 2 }
    );
    assert!(rows.iter().all(|row| row.tag.as_bytes() == &heap_tag[..20]));
    assert_eq!((census.calls(), census.bytes()), (26, 225));
    f.put(TAGS + 0x20, &[0; 32]);
    f.put(TAGS + 0x20, b"GER");
    f.put(TAGS + 0x20 + 16, &3u64.to_le_bytes());
    f.put(TAGS + 0x20 + 24, &15u64.to_le_bytes());

    f.put(0x7000 + super::COUNTRY, &0u32.to_le_bytes());
    assert_eq!(
        walk_portrait_paths(&f, 0, GS, &sources, &mut rows),
        Outcome::Rows { len: 2 }
    );
    assert_eq!((rows[1].country, rows[1].tag_reference), (0, 0));
    assert!(rows[1].tag.as_bytes().is_empty());
    f.put(0x7000 + super::COUNTRY, &3u32.to_le_bytes());
    f.put(LINKS + 12, &2u32.to_le_bytes());
    assert_eq!(
        walk_portrait_paths(&f, 0, GS, &sources, &mut rows),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(rows, expected);

    const OTHER_COUNTRY: usize = 0xb000;
    f.put(COUNTRIES, &[0; 66 * 8]);
    f.put(COUNTRIES + 8, &(COUNTRY as u64).to_le_bytes());
    f.put(COUNTRIES + 65 * 8, &(OTHER_COUNTRY as u64).to_le_bytes());
    array(&mut f, GS + gs::COUNTRIES, COUNTRIES, 66);
    f.put(LINKS, &[0; 66 * 4]);
    for (raw, canonical) in [(1usize, 1u32), (2, 1), (3, 1), (65, 65)] {
        f.put(LINKS + raw * 4, &canonical.to_le_bytes());
    }
    array(&mut f, GS + gs::COUNTRY_LINKS, LINKS, 66);
    f.put(OTHER_COUNTRY, &[0; 0x1400]);
    f.put(
        OTHER_COUNTRY,
        &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    f.put(OTHER_COUNTRY + ccountry::ID, &65u32.to_le_bytes());
    f.put(OTHER_COUNTRY + ccountry::ORIGINAL_TAG, &2i32.to_le_bytes());
    f.put(TAGS + 0x40, b"FRA");
    f.put(TAGS + 0x40 + 16, &3u64.to_le_bytes());
    f.put(TAGS + 0x40 + 24, &15u64.to_le_bytes());
    f.put(0x7000 + super::COUNTRY, &65u32.to_le_bytes());
    let third = (9u64 << 32) | 4713;
    f.put(0x8000, &[0; 0x1000]);
    f.put(0x8000, &(VTABLE_RVA.0 as u64).to_le_bytes());
    f.put(0x8008, &third.to_le_bytes());
    f.put(0x8000 + super::COUNTRY, &3u32.to_le_bytes());
    let sources = [sources[0], sources[1], (third, 0x8000)];
    let mut rows = vec![PortraitPathRow::default(); 3];
    let census = ReadCensus::new(&f);
    assert_eq!(
        walk_portrait_paths(&census, 0, GS, &sources, &mut rows),
        Outcome::Rows { len: 3 }
    );
    assert_eq!(
        rows.iter().map(|row| (row.country, row.tag_reference)).collect::<Vec<_>>(),
        [(2, 1), (65, 2), (3, 1)]
    );
    assert_eq!(
        rows.iter().map(|row| row.tag.as_bytes()).collect::<Vec<_>>(),
        [b"GER".as_slice(), b"FRA", b"GER"]
    );
    assert_eq!((census.calls(), census.bytes()), (39, 288));

    f.put(COUNTRY + ccountry::ORIGINAL_TAG, &0i32.to_le_bytes());
    assert_eq!(
        walk_portrait_paths(&f, 0, GS, &sources, &mut rows),
        Outcome::Rows { len: 3 }
    );
    assert!(
        rows.iter()
            .all(|row| row.tag_reference == 2 && row.tag.as_bytes() == b"FRA")
    );
}

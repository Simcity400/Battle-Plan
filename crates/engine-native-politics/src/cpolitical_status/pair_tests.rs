use super::*;
use engine_native_memory::read_slice;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};

const BASE: usize = 0x140000000;
const COUNTRY_POINTER: usize = 0x1000;
const STATUS: usize = 0x3000;
const PARTY: usize = 0x4000;
const IDEOLOGY: usize = 0x5000;

#[derive(Clone, Default)]
struct TraceMemory {
    bytes: BTreeMap<usize, u8>,
    reads: RefCell<Vec<(usize, usize)>>,
    scatters: Cell<usize>,
    fault: Option<(usize, usize)>,
}

impl TraceMemory {
    fn put(&mut self, address: usize, bytes: &[u8]) {
        for (offset, &byte) in bytes.iter().enumerate() {
            self.bytes
                .insert(address.checked_add(offset).unwrap(), byte);
        }
    }

    fn fixture(status: usize, country: usize, ruling: u64) -> Self {
        let mut memory = Self::default();
        for (offset, bytes) in [
            (LAST_ELECTION, {
                let mut dates = vec![0; 4];
                dates[..4].copy_from_slice(&(-400_i32).to_le_bytes());
                dates
            }),
            (RULING_PARTY, vec![0; RULING_SPAN]),
            (ELECTION_FREQUENCY, vec![48, 0, 0, 0, 1]),
            (RULING_PARTY, ruling.to_le_bytes().to_vec()),
        ] {
            if let Some(address) = status.checked_add(offset)
                && address.checked_add(bytes.len()).is_some()
            {
                memory.put(address, &bytes);
            }
        }
        if let Some(address) = country.checked_add(ccountry::CAPITAL)
            && address.checked_add(8).is_some()
        {
            memory.put(address, &(-17_i32).to_le_bytes());
            memory.put(address + 4, &0_i32.to_le_bytes());
        }
        memory.put(
            PARTY,
            &((BASE + cpolitical_party::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        memory.put(PARTY + cpolitical_party::ID, &33_u32.to_le_bytes());
        memory.put(
            PARTY + cpolitical_party::IDEOLOGY_GROUP,
            &(IDEOLOGY as u64).to_le_bytes(),
        );
        memory.put(
            IDEOLOGY,
            &((BASE + cideology_group::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        memory.put(IDEOLOGY + cideology_group::ID, &33_u32.to_le_bytes());
        memory
    }
}

impl Memory for TraceMemory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let ordinal = self.reads.borrow().len();
        self.reads.borrow_mut().push((address, destination.len()));
        for (offset, byte) in destination.iter_mut().enumerate() {
            if self.fault == Some((ordinal, offset)) {
                return false;
            }
            let Some(source) = address
                .checked_add(offset)
                .and_then(|at| self.bytes.get(&at))
            else {
                return false;
            };
            *byte = *source;
        }
        true
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.scatters.set(self.scatters.get() + 1);
        for (index, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if !read_slice(self, address, destination) {
                return index;
            }
        }
        N
    }
}

#[test]
fn literal_reuses_inline_bytes_and_keeps_heap_faults_and_terminators() {
    const STRING: usize = 0x6000;
    const HEAP: usize = 0x7000;
    for (len, capacity) in [(0, 15_u64), (3, 15), (15, 15), (0, 31), (17, 31)] {
        let mut source = TraceMemory::default();
        let mut header = [0; msvc_string::STRIDE];
        let payload: Vec<u8> = (0..len).map(|index| [0xff, 0, b'a'][index % 3]).collect();
        let inline = capacity == msvc_string::INLINE_CAPACITY;
        let address = if inline { STRING } else { HEAP };
        if inline {
            header[..len].copy_from_slice(&payload);
        } else {
            header[..8].copy_from_slice(&(HEAP as u64).to_le_bytes());
            source.put(HEAP, &payload);
            source.put(HEAP + len, &[0]);
        }
        header[msvc_string::LENGTH..msvc_string::CAPACITY]
            .copy_from_slice(&(len as u64).to_le_bytes());
        header[msvc_string::CAPACITY..].copy_from_slice(&capacity.to_le_bytes());
        source.put(STRING, &header);
        let descriptor = literal(&source, STRING).unwrap();
        let mut output = [0; 31];
        descriptor.copy(&source, &mut output).unwrap();
        assert_eq!(&output[..len], payload);
        let mut expected = vec![(STRING, msvc_string::STRIDE)];
        if !inline {
            expected.push((HEAP + len, 1));
            if len != 0 {
                expected.push((HEAP, len));
            }
        }
        assert_eq!(*source.reads.borrow(), expected);
        for (ordinal, &(_, width)) in expected.iter().enumerate() {
            for byte in 0..width {
                let mut memory = source.clone();
                memory.reads.borrow_mut().clear();
                memory.fault = Some((ordinal, byte));
                let result = literal(&memory, STRING)
                    .and_then(|descriptor| descriptor.copy(&memory, &mut output));
                assert_eq!(
                    result,
                    Err(match ordinal {
                        0 => "politics string header unreadable",
                        1 => "politics string terminator unreadable or invalid",
                        _ => "string bytes unreadable",
                    })
                );
                assert_eq!(memory.reads.borrow().len(), ordinal + 1);
            }
        }
        source.reads.borrow_mut().clear();
        source.put(address + len, &[1]);
        assert!(matches!(
            literal(&source, STRING),
            Err("politics string terminator unreadable or invalid")
        ));
    }
}

#[test]
fn literal_invalid_headers_stop_before_payload_acquisition() {
    const STRING: usize = 0x6000;
    for (len, capacity, pointer, reason) in [
        (16_u64, 15_u64, 0, "politics string header invalid"),
        (0, 14, 0, "politics string header invalid"),
        (0, 31, 0, "politics string heap pointer null or overflow"),
        (
            1,
            31,
            u64::MAX,
            "politics string terminator unreadable or invalid",
        ),
    ] {
        let mut memory = TraceMemory::default();
        let mut header = [0; msvc_string::STRIDE];
        header[..8].copy_from_slice(&pointer.to_le_bytes());
        header[msvc_string::LENGTH..msvc_string::CAPACITY].copy_from_slice(&len.to_le_bytes());
        header[msvc_string::CAPACITY..].copy_from_slice(&capacity.to_le_bytes());
        memory.put(STRING, &header);
        assert!(matches!(literal(&memory, STRING), Err(actual) if actual == reason));
        assert_eq!(*memory.reads.borrow(), [(STRING, msvc_string::STRIDE)]);
    }
}

#[test]
fn the_idea_count_probe_still_validates_tokens_without_reading_heap_payloads() {
    const POINTERS: usize = 0x6000;
    const IDEA: usize = 0x7000;
    const HEAP: usize = 0x8000;
    for inline in [true, false] {
        for terminated in [true, false] {
            for count in [true, false] {
                let mut memory = TraceMemory::default();
                let mut array = [0; 16];
                array[..8].copy_from_slice(&(POINTERS as u64).to_le_bytes());
                array[8..12].copy_from_slice(&1_u32.to_le_bytes());
                array[12..].copy_from_slice(&1_u32.to_le_bytes());
                memory.put(STATUS + IDEAS, &array);
                memory.put(POINTERS, &(IDEA as u64).to_le_bytes());
                memory.put(IDEA, &((BASE + cidea::VTABLE_RVA.0) as u64).to_le_bytes());
                let mut header = [0; msvc_string::STRIDE];
                if inline {
                    header[..3].copy_from_slice(&[0xff, 0, b'a']);
                    header[3] = u8::from(!terminated);
                } else {
                    header[..8].copy_from_slice(&(HEAP as u64).to_le_bytes());
                    memory.put(HEAP + 3, &[u8::from(!terminated)]);
                }
                header[msvc_string::LENGTH..msvc_string::CAPACITY]
                    .copy_from_slice(&3_u64.to_le_bytes());
                header[msvc_string::CAPACITY..]
                    .copy_from_slice(&(if inline { 15_u64 } else { 31 }).to_le_bytes());
                memory.put(IDEA + cidea::TOKEN, &header);
                let reader = Reader {
                    memory: &memory,
                    base: BASE,
                    country: COUNTRY_POINTER,
                    status: STATUS,
                };
                let mut output = Vec::new();
                let mut arena = Vec::new();
                let walked = if count {
                    reader.ideas_buffer(engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count)
                } else {
                    reader.ideas(&mut output, &mut arena)
                };
                let expected = if !terminated {
                    Outcome::Unavailable {
                        reason: "politics string terminator unreadable or invalid",
                    }
                } else if count {
                    assert_eq!(walked.required_bytes, 3);
                    Outcome::Truncated {
                        len: 0,
                        unread_rows: 1,
                    }
                } else if inline {
                    assert_eq!(arena[..3], [0xff, 0, b'a']);
                    Outcome::Rows { len: 1 }
                } else {
                    Outcome::Unavailable {
                        reason: "active idea token unreadable",
                    }
                };
                assert_eq!(walked.outcome, expected);
                assert_eq!(
                    memory.reads.borrow().contains(&(HEAP, 3)),
                    !inline && terminated && !count
                );
            }
        }
    }
}

fn scalar_row(reader: &Reader<'_, impl Memory>) -> Result<Row, &'static str> {
    let dates = read::<[u8; 4]>(reader.memory, reader.at(LAST_ELECTION)?)
        .ok_or("last election unreadable")?;
    let last_election = i32::from_le_bytes(dates[..4].try_into().unwrap());
    let capitals = reader
        .country
        .checked_add(ccountry::CAPITAL)
        .and_then(|p| read::<[u8; 8]>(reader.memory, p))
        .map(|bytes| {
            [
                i32::from_le_bytes(bytes[..4].try_into().unwrap()),
                i32::from_le_bytes(bytes[4..].try_into().unwrap()),
            ]
        })
        .ok_or("current capital unreadable")?;
    let ruling = read::<[u8; RULING_SPAN]>(reader.memory, reader.at(RULING_PARTY)?)
        .ok_or("ruling party pointer unreadable")?;
    let frequency = i32::from_le_bytes(ruling[0x18..0x1c].try_into().unwrap());
    let has_elections = ruling[0x1c] != 0 && frequency > 0;
    let pointer = u64::from_le_bytes(ruling[..8].try_into().unwrap()) as usize;
    let ruling_party = if pointer == 0 {
        None
    } else {
        Some(cpolitical_party::identity(reader.memory, reader.base, pointer)?.0)
    };
    Ok(Row {
        last_election,
        next_election_cache: None,
        election_interval: frequency,
        capital: capitals[0],
        original_capital: capitals[1],
        ruling_party,
        has_elections,
    })
}

fn compare(
    source: &TraceMemory,
    base: usize,
    status: usize,
    country: usize,
    expected: Result<Row, &'static str>,
    expected_reads: &[(usize, usize)],
) {
    let actual_memory = source.clone();
    let reference_memory = source.clone();
    let actual = Reader {
        memory: &actual_memory,
        base,
        country,
        status,
    }
    .row();
    let reference = scalar_row(&Reader {
        memory: &reference_memory,
        base,
        country,
        status,
    });
    assert_eq!(actual, reference);
    assert_eq!(actual, expected);
    assert_eq!(
        *actual_memory.reads.borrow(),
        *reference_memory.reads.borrow()
    );
    assert_eq!(&*actual_memory.reads.borrow(), expected_reads);
}

fn expected(ruling_party: Option<u32>) -> Result<Row, &'static str> {
    Ok(Row {
        last_election: -400,
        next_election_cache: None,
        election_interval: 48,
        capital: -17,
        original_capital: 0,
        ruling_party,
        has_elections: true,
    })
}

fn ordinary_reads() -> [(usize, usize); 6] {
    [
        (STATUS + LAST_ELECTION, 4),
        (COUNTRY_POINTER + ccountry::CAPITAL, 8),
        (STATUS + RULING_PARTY, RULING_SPAN),
        (PARTY, 12),
        (PARTY + cpolitical_party::IDEOLOGY_GROUP, 8),
        (IDEOLOGY, 12),
    ]
}

#[test]
fn politics_pairs_preserve_every_fault_ordinal_and_partial_field() {
    let mut memory = TraceMemory::fixture(STATUS, COUNTRY_POINTER, PARTY as u64);
    let reads = ordinary_reads();
    compare(
        &memory,
        BASE,
        STATUS,
        COUNTRY_POINTER,
        expected(Some(33)),
        &reads,
    );
    let reader = Reader {
        memory: &memory,
        base: BASE,
        country: COUNTRY_POINTER,
        status: STATUS,
    };
    assert_eq!(reader.row(), expected(Some(33)));
    assert_eq!(memory.scatters.get(), 1);
    memory.reads.borrow_mut().clear();
    memory.scatters.set(0);
    for (ordinal, reason) in [
        "last election unreadable",
        "current capital unreadable",
        "ruling party pointer unreadable",
        "political party identity unreadable",
        "political party ideology pointer unreadable or null",
        "ideology identity unreadable",
    ]
    .into_iter()
    .enumerate()
    {
        for byte in [0, reads[ordinal].1 - 1] {
            memory.fault = Some((ordinal, byte));
            compare(
                &memory,
                BASE,
                STATUS,
                COUNTRY_POINTER,
                Err(reason),
                &reads[..=ordinal],
            );
        }
    }
}

#[test]
fn politics_pairs_test_elections_as_has_elections_does() {
    let reads = ordinary_reads();
    for (frequency, byte, has_elections) in [
        (48, 1, true),
        (48, 2, true),
        (1, 0xff, true),
        (48, 0, false),
        (0, 1, false),
        (-1, 1, false),
        (i32::MIN, 1, false),
    ] {
        let mut memory = TraceMemory::fixture(STATUS, COUNTRY_POINTER, PARTY as u64);
        memory.put(STATUS + ELECTION_FREQUENCY, &frequency.to_le_bytes());
        memory.put(STATUS + ELECTIONS_ALLOWED, &[byte]);
        let mut row = expected(Some(33)).unwrap();
        row.has_elections = has_elections;
        row.election_interval = frequency;
        compare(&memory, BASE, STATUS, COUNTRY_POINTER, Ok(row), &reads);
    }
}

#[test]
fn politics_pairs_keep_null_and_identity_validation_stops() {
    let reads = ordinary_reads();
    let memory = TraceMemory::fixture(STATUS, COUNTRY_POINTER, 0);
    compare(
        &memory,
        BASE,
        STATUS,
        COUNTRY_POINTER,
        expected(None),
        &reads[..3],
    );
    for (address, bytes, reason, count) in [
        (
            PARTY,
            0_u64.to_le_bytes().to_vec(),
            "political party vtable mismatch",
            4,
        ),
        (
            PARTY + cpolitical_party::IDEOLOGY_GROUP,
            0_u64.to_le_bytes().to_vec(),
            "political party ideology pointer unreadable or null",
            5,
        ),
        (
            IDEOLOGY,
            0_u64.to_le_bytes().to_vec(),
            "political party ideology vtable mismatch",
            6,
        ),
        (
            IDEOLOGY + cideology_group::ID,
            34_u32.to_le_bytes().to_vec(),
            "political party and ideology ids differ",
            6,
        ),
    ] {
        let mut memory = TraceMemory::fixture(STATUS, COUNTRY_POINTER, PARTY as u64);
        memory.put(address, &bytes);
        compare(
            &memory,
            BASE,
            STATUS,
            COUNTRY_POINTER,
            Err(reason),
            &reads[..count],
        );
    }
    let memory = TraceMemory::fixture(STATUS, COUNTRY_POINTER, PARTY as u64);
    compare(
        &memory,
        usize::MAX,
        STATUS,
        COUNTRY_POINTER,
        Err("module address overflow"),
        &reads[..3],
    );
}

#[test]
fn politics_pairs_keep_address_overflow_and_earlier_fault_priority() {
    for (status, reason, observed) in [
        (
            usize::MAX - LAST_ELECTION + 1,
            "political status address overflow",
            0,
        ),
        (usize::MAX - LAST_ELECTION, "last election unreadable", 0),
        (
            usize::MAX - RULING_PARTY + 1,
            "political status address overflow",
            2,
        ),
        (
            usize::MAX - RULING_PARTY,
            "ruling party pointer unreadable",
            2,
        ),
        (
            usize::MAX - ELECTIONS_ALLOWED,
            "ruling party pointer unreadable",
            2,
        ),
    ] {
        let mut memory = TraceMemory::fixture(status, COUNTRY_POINTER, 0);
        let prefix: Vec<_> = status
            .checked_add(LAST_ELECTION)
            .map(|address| (address, 4))
            .into_iter()
            .chain([(COUNTRY_POINTER + ccountry::CAPITAL, 8)])
            .take(observed)
            .collect();
        compare(&memory, BASE, status, COUNTRY_POINTER, Err(reason), &prefix);
        if observed != 0 {
            for (ordinal, reason) in ["last election unreadable", "current capital unreadable"]
                .into_iter()
                .enumerate()
            {
                memory.fault = Some((ordinal, 0));
                compare(
                    &memory,
                    BASE,
                    status,
                    COUNTRY_POINTER,
                    Err(reason),
                    &prefix[..=ordinal],
                );
            }
        }
    }
    let status = usize::MAX - ELECTIONS_ALLOWED - 1;
    let memory = TraceMemory::fixture(status, COUNTRY_POINTER, 0);
    compare(
        &memory,
        BASE,
        status,
        COUNTRY_POINTER,
        expected(None),
        &[
            (status + LAST_ELECTION, 4),
            (COUNTRY_POINTER + ccountry::CAPITAL, 8),
            (status + RULING_PARTY, RULING_SPAN),
        ],
    );
    for country in [
        usize::MAX - ccountry::CAPITAL + 1,
        usize::MAX - ccountry::CAPITAL,
        usize::MAX - ccountry::CAPITAL - 7,
    ] {
        let memory = TraceMemory::fixture(STATUS, country, 0);
        compare(
            &memory,
            BASE,
            STATUS,
            country,
            Err("current capital unreadable"),
            &ordinary_reads()[..1],
        );
    }
    let country = usize::MAX - ccountry::CAPITAL - 8;
    let memory = TraceMemory::fixture(STATUS, country, 0);
    compare(
        &memory,
        BASE,
        STATUS,
        country,
        expected(None),
        &[
            (STATUS + LAST_ELECTION, 4),
            (country + ccountry::CAPITAL, 8),
            (STATUS + RULING_PARTY, RULING_SPAN),
        ],
    );
}

#[test]
fn manpower_election_cache_reads_only_eligible_initializer_dates_and_refuses_independently() {
    for last in [engine_native_memory::cgame_date::NULL_RAW_HOUR, RESET_LAST_ELECTION] {
        let mut memory = TraceMemory::fixture(STATUS, COUNTRY_POINTER, 0);
        memory.put(STATUS + LAST_ELECTION, &last.to_le_bytes());
        memory.put(STATUS + NEXT_ELECTION_CACHE, &12345_i32.to_le_bytes());
        let row = Reader {
            memory: &memory,
            base: BASE,
            country: COUNTRY_POINTER,
            status: STATUS,
        }
        .row()
        .unwrap();
        assert_eq!(row.next_election_cache, Some(12345));
        assert_eq!(
            memory.reads.borrow().last(),
            Some(&(STATUS + NEXT_ELECTION_CACHE, 4))
        );
        memory.reads.borrow_mut().clear();
        memory.fault = Some((3, 0));
        let refused = Reader {
            memory: &memory,
            base: BASE,
            country: COUNTRY_POINTER,
            status: STATUS,
        }
        .row()
        .unwrap();
        assert_eq!(refused.last_election, last);
        assert_eq!(refused.next_election_cache, None);
        memory.fault = None;
        memory.reads.borrow_mut().clear();
        let uncertified = Reader {
            memory: &memory,
            base: BASE,
            country: COUNTRY_POINTER,
            status: STATUS,
        }
        .row_with_election_cache(false)
        .unwrap();
        assert_eq!(uncertified.next_election_cache, None);
        assert!(
            !memory
                .reads
                .borrow()
                .iter()
                .any(|&(at, _)| at == STATUS + NEXT_ELECTION_CACHE)
        );
        memory.reads.borrow_mut().clear();
        memory.put(STATUS + ELECTIONS_ALLOWED, &[0]);
        let disabled = Reader {
            memory: &memory,
            base: BASE,
            country: COUNTRY_POINTER,
            status: STATUS,
        }
        .row()
        .unwrap();
        assert_eq!(disabled.next_election_cache, None);
        assert!(
            !memory
                .reads
                .borrow()
                .iter()
                .any(|&(at, _)| at == STATUS + NEXT_ELECTION_CACHE)
        );
    }
}

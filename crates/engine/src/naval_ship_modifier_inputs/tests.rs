use super::*;
use std::cell::{Cell, RefCell};
const IMAGE: usize = 0x1000_0000;
struct Fixture {
    bytes: Vec<u8>,
    globals: Vec<(usize, Vec<u8>)>,
    calls: Cell<usize>,
    copied: Cell<usize>,
    reads: RefCell<Vec<(usize, usize)>>,
    fail: Option<usize>,
}
impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.copied.set(self.copied.get() + output.len());
        self.reads.borrow_mut().push((address, output.len()));
        if self.fail == Some(address) {
            return false;
        }
        for (base, bytes) in &self.globals {
            if (crate::memory::SliceMemory { base: *base, bytes }).copy(address, output) {
                return true;
            }
        }
        if address == IMAGE + crate::modifier_definition::TABLE_RVA && output.len() == 16 {
            output[..8].copy_from_slice(&0x4000u64.to_le_bytes());
            output[8..12].copy_from_slice(&64u32.to_le_bytes());
            output[12..].copy_from_slice(&64u32.to_le_bytes());
            return true;
        }
        crate::memory::SliceMemory {
            base: 0x1000,
            bytes: &self.bytes,
        }
        .copy(address, output)
    }
}
fn fixture() -> Fixture {
    let mut f = Fixture {
        bytes: vec![0; 0x7000],
        globals: Vec::new(),
        calls: Cell::new(0),
        copied: Cell::new(0),
        reads: RefCell::default(),
        fail: None,
    };
    let mut put = |p: usize, b: &[u8]| f.bytes[p - 0x1000..p - 0x1000 + b.len()].copy_from_slice(b);
    put(0x1800, &0x2000u64.to_le_bytes());
    put(0x1808, &3u32.to_le_bytes());
    put(0x180c, &2u32.to_le_bytes());
    for (i, key, value) in [(0usize, 0x2au32, i64::MIN), (1, 0x2b, i64::MAX)] {
        put(0x2000 + i * 16, &key.to_le_bytes());
        put(0x2008 + i * 16, &value.to_le_bytes());
        put(
            0x4000
                + key as usize * crate::modifier_definition::STRIDE
                + crate::modifier_definition::FLAGS,
            &0x81234567u32.to_le_bytes(),
        );
    }
    f
}
impl Fixture {
    fn put(&mut self, address: usize, bytes: &[u8]) {
        let start = address - 0x1000;
        self.bytes[start..start + bytes.len()].copy_from_slice(bytes);
    }
    fn q(&mut self, address: usize, value: u64) {
        self.put(address, &value.to_le_bytes());
    }
}

const TASK: usize = 0x1000;
const HEAD: usize = 0x1200;
const DICTIONARY: usize = 0x1300;
const GENERATOR: usize = 0x1400;
const NODES: usize = 0x10000;
const TOKENS: usize = 0x50000;

fn doctrine_fixture(count: usize) -> Fixture {
    use crate::cpdx_lexer as lexer;
    let mut f = fixture();
    f.bytes.resize(TOKENS + (count + 1) * 32 - 0x1000, 0);
    let mut witness = vec![0; 64];
    witness[0] = 1;
    for offset in [4, 12] {
        witness[offset..offset + 4].copy_from_slice(&(count as u32).to_le_bytes());
    }
    witness[48..56].copy_from_slice(&(TOKENS as u64).to_le_bytes());
    for offset in [56, 60] {
        witness[offset..offset + 4].copy_from_slice(&(count as u32 + 1).to_le_bytes());
    }
    f.globals.push((IMAGE + lexer::INITIALIZED_RVA, witness));
    for (rva, value) in [
        (0x0337_2490, GENERATOR),
        (0x0337_2488, GENERATOR),
        (0x0306_ebd8, DICTIONARY),
    ] {
        f.globals
            .push((IMAGE + rva, (value as u64).to_le_bytes().to_vec()));
    }
    for index in 0..=count {
        f.q(TOKENS + index * 32 + 24, 15);
    }
    for offset in [0x28, 0x48] {
        f.q(GENERATOR + offset, 15);
    }
    f.q(DICTIONARY, DICTIONARY as u64);
    f.q(DICTIONARY + 8, DICTIONARY as u64);
    f.put(0x1800, &[0; 16]);
    for (at, value) in [
        (TASK + 0x640, HEAD),
        (TASK + 0x648, count),
        (HEAD, NODES),
        (HEAD + 8, NODES),
        (HEAD + 16, NODES + (count - 1) * 0x38),
    ] {
        f.q(at, value as u64);
    }
    f.put(HEAD + 0x19, &[1]);
    for index in 0..count {
        let node = NODES + index * 0x38;
        for (offset, value) in [
            (0, HEAD),
            (8, if index == 0 { HEAD } else { node - 0x38 }),
            (
                16,
                if index + 1 == count {
                    HEAD
                } else {
                    node + 0x38
                },
            ),
            (0x30, 0x17f0),
        ] {
            f.q(node + offset, value as u64);
        }
        f.put(node + 0x20, &(index as u32 + 1).to_le_bytes());
    }
    f
}

fn doctrine_sources(memory: &Fixture) -> Result<Vec<SourceRow>, &'static str> {
    let mut sources = Vec::new();
    let (mut terms, mut lengths, mut required) = (Vec::new(), [0; 2], [0; 2]);
    let mut capture = Capture {
        memory,
        base: IMAGE,
        buffers: InfluenceBuffers {
            sources: &mut sources,
            terms: &mut terms,
            lengths: &mut lengths,
            required: &mut required,
        },
    };
    visit_doctrine(memory, IMAGE, TASK, true, &mut |row, accumulator| {
        capture.source(row, accumulator)
    })?;
    Ok(sources)
}

#[test]
fn doctrine_exceeds_4096_nodes() {
    let sources = doctrine_sources(&doctrine_fixture(4097)).unwrap();
    assert_eq!(sources.len(), 4097);
    assert_eq!(sources[4096].index, 4096);
}

#[test]
fn doctrine_source_visitor_sizes_without_key_name_or_weight_operands() {
    for count in [1500, 5000] {
        let mut memory = doctrine_fixture(count);
        memory.fail = Some(IMAGE + crate::cpdx_lexer::INITIALIZED_RVA);
        let mut sources = Vec::new();
        visit_doctrine(&memory, IMAGE, TASK, false, &mut |row, accumulator| {
            sources.push((row.scope, row.index, accumulator));
            Ok(())
        })
        .unwrap();
        assert_eq!(sources.len(), count);
        assert_eq!(sources[count - 1], (4, count as u32 - 1, Some(0x1800)));
        assert!(
            memory
                .reads
                .borrow()
                .iter()
                .all(|&(address, _)| address != IMAGE + crate::cpdx_lexer::INITIALIZED_RVA)
        );
    }
}

#[test]
fn doctrine_count_mismatch_fails_by_name() {
    let mut f = doctrine_fixture(2);
    f.q(TASK + 0x648, 1);
    assert_eq!(
        doctrine_sources(&f),
        Err("naval doctrine tree links or count invalid")
    );
}

#[test]
fn dictionary_exceeds_65536_nodes() {
    const COUNT: usize = 65_537;
    const ENTRIES: usize = 0x80000;
    let mut f = doctrine_fixture(1);
    f.bytes.resize(ENTRIES + COUNT * 24 - 0x1000, 0);
    f.put(GENERATOR + 0x10, b"prefix");
    f.q(GENERATOR + 0x20, 6);
    f.q(DICTIONARY, (ENTRIES + (COUNT - 1) * 24) as u64);
    f.q(DICTIONARY + 8, ENTRIES as u64);
    for index in 0..COUNT {
        let node = ENTRIES + index * 24;
        f.q(node, if index == 0 { DICTIONARY } else { node - 24 } as u64);
        f.q(
            node + 8,
            if index + 1 == COUNT {
                DICTIONARY
            } else {
                node + 24
            } as u64,
        );
        f.put(node + 0x10, &1u32.to_le_bytes());
    }
    assert_eq!(doctrine_keys(&f, IMAGE, 1), Ok([0; 2]));
}

#[test]
fn dictionary_cycle_fails_its_predecessor_check() {
    let mut f = doctrine_fixture(1);
    const NODE: usize = 0x1500;
    f.q(DICTIONARY + 8, NODE as u64);
    f.q(NODE, DICTIONARY as u64);
    f.q(NODE + 8, NODE as u64);
    f.put(NODE + 0x10, &1u32.to_le_bytes());
    assert_eq!(
        doctrine_keys(&f, IMAGE, 1),
        Err("naval doctrine modifier dictionary link invalid")
    );
}

#[test]
fn modifier_terms_and_keys_exceed_65536() {
    use crate::modifier_definition as md;
    const COUNT: usize = 65_537;
    const TERMS: usize = 0x10000;
    const DEFINITIONS: usize = 0x200000;
    let mut f = fixture();
    f.bytes
        .resize(DEFINITIONS + (COUNT + 1) * md::STRIDE - 0x1000, 0);
    f.q(0x1800, TERMS as u64);
    for at in [0x1808, 0x180c] {
        f.put(at, &(COUNT as u32).to_le_bytes());
    }
    let mut header = vec![0; 16];
    header[..8].copy_from_slice(&(DEFINITIONS as u64).to_le_bytes());
    for offset in [8, 12] {
        header[offset..offset + 4].copy_from_slice(&(COUNT as u32 + 1).to_le_bytes());
    }
    f.globals.push((IMAGE + md::TABLE_RVA, header));
    for index in 0..COUNT {
        f.put(TERMS + index * 16, &(index as u32 + 1).to_le_bytes());
        f.q(TERMS + index * 16 + 8, i64::MIN as u64);
    }
    let mut terms = Vec::new();
    Capture {
        memory: &f,
        base: IMAGE,
        buffers: InfluenceBuffers {
            sources: &mut Vec::new(),
            terms: &mut terms,
            lengths: &mut [0; 2],
            required: &mut [0; 2],
        },
    }
    .source(SourceRow::default(), Some(0x1800))
    .unwrap();
    assert_eq!(terms.len(), COUNT);
    assert_eq!(
        (terms[COUNT - 1].key, terms[COUNT - 1].raw),
        (COUNT as u32, i64::MIN)
    );
}

#[test]
fn terrain_buckets_exceed_65536() {
    const BUCKETS: u32 = 131_072;
    const DATA: usize = 0x10000;
    let mut f = fixture();
    f.bytes.resize(DATA + BUCKETS as usize * 12 - 0x1000, 0);
    f.q(TASK + 0x1b8, DATA as u64);
    f.put(TASK + 0x1c4, &(BUCKETS - 1).to_le_bytes());
    assert_eq!(terrain_class(&f, TASK, 0), Ok((TASK + 0xa8, TASK + 0x168)));
}

#[test]
fn native_modifier_sources_share_owned_terms_per_accumulator() {
    use crate::modifier_definition as md;
    let source_reads = [
        (0x1800, 16),
        (IMAGE + md::TABLE_RVA, 16),
        (0x2000, 32),
        (0x4000 + 0x2a * md::STRIDE + md::FLAGS, 12),
        (0x4000 + 0x2b * md::STRIDE + md::FLAGS, 12),
    ];
    for (source_size, term_size) in [(0, 0), (1, 1), (2, 4), (4, 8)] {
        let memory = fixture();
        let mut sources = vec![SourceRow::default(); source_size];
        let mut terms = vec![ModifierTerm::default(); term_size];
        let mut lengths = [0; 2];
        let mut required = [0; 2];
        let mut cap = Capture {
            memory: &memory,
            base: IMAGE,
            buffers: InfluenceBuffers {
                sources: &mut sources,
                terms: &mut terms,
                lengths: &mut lengths,
                required: &mut required,
            },
        };
        for index in 0..2 {
            cap.source(
                SourceRow {
                    scope: 2,
                    index,
                    ..Default::default()
                },
                Some(0x1800),
            )
            .unwrap();
        }
        cap.source(SourceRow::default(), None).unwrap();
        assert_eq!(lengths, [3, 2]);
        assert_eq!(required, [3, 2]);
        for (index, source) in sources[..2].iter().enumerate() {
            assert_eq!(source.scope, 2);
            assert_eq!(source.index, index as u32);
            assert_eq!(source.terms, [0, 2]);
        }
        assert_eq!(sources[2].terms, [2, 0]);
        assert_eq!([terms[0].raw, terms[1].raw], [i64::MIN, i64::MAX]);
        assert_eq!([terms[0].key, terms[1].key], [0x2a, 0x2b]);
        assert_eq!(terms[0].metadata_flags, 0x81234567);
        assert_eq!((memory.calls.get(), memory.copied.get()), (5, 88));
        assert_eq!(*memory.reads.borrow(), source_reads);
    }
}

#[test]
fn modifier_source_faults_stop_at_primary_prefix_and_leave_lengths_unpublished() {
    use crate::modifier_definition as md;
    for (fail, calls, copied, required_terms, reason) in [
        (0x1800, 1, 16, 0, "naval field unreadable"),
        (
            IMAGE + md::TABLE_RVA,
            2,
            32,
            2,
            "modifier definition table unreadable",
        ),
        (0x2000, 3, 64, 2, "naval weapon modifier bytes unreadable"),
        (
            0x4000 + 0x2a * md::STRIDE + md::FLAGS,
            4,
            76,
            2,
            "naval field unreadable",
        ),
        (
            0x4000 + 0x2b * md::STRIDE + md::FLAGS,
            5,
            88,
            2,
            "naval field unreadable",
        ),
    ] {
        let mut expected_reads = None;
        for (source_size, term_size) in [(1, 2), (0, 0), (1, 1)] {
            let mut memory = fixture();
            memory.fail = Some(fail);
            let mut sources = vec![SourceRow::default(); source_size];
            let mut terms = vec![ModifierTerm::default(); term_size];
            let mut lengths = [0; 2];
            let mut required = [0; 2];
            let mut cap = Capture {
                memory: &memory,
                base: IMAGE,
                buffers: InfluenceBuffers {
                    sources: &mut sources,
                    terms: &mut terms,
                    lengths: &mut lengths,
                    required: &mut required,
                },
            };
            assert_eq!(cap.source(SourceRow::default(), Some(0x1800)), Err(reason));
            assert_eq!((memory.calls.get(), memory.copied.get()), (calls, copied));
            assert_eq!(lengths, [0, 0]);
            assert_eq!(required, [1, required_terms]);
            assert!(sources.iter().all(|source| *source == SourceRow::default()));
            let reads = memory.reads.into_inner();
            assert_eq!(*expected_reads.get_or_insert_with(|| reads.clone()), reads);
        }
    }
}

#[test]
fn modifier_source_growth_keeps_checked_demand_and_native_count_bounds() {
    for (mut lengths, mut required, calls, reason) in [
        (
            [0, 0],
            [usize::MAX, 0],
            0,
            "naval weapon source demand overflow",
        ),
        (
            [0, 0],
            [0, usize::MAX],
            1,
            "naval weapon term demand overflow",
        ),
        (
            [0, usize::MAX],
            [0, 0],
            1,
            "naval weapon term length overflow",
        ),
        (
            [usize::MAX, 0],
            [0, 0],
            1,
            "naval weapon source length overflow",
        ),
    ] {
        let memory = fixture();
        let mut sources = Vec::new();
        let mut terms = Vec::new();
        let original_lengths = lengths;
        let mut cap = Capture {
            memory: &memory,
            base: IMAGE,
            buffers: InfluenceBuffers {
                sources: &mut sources,
                terms: &mut terms,
                lengths: &mut lengths,
                required: &mut required,
            },
        };
        assert_eq!(cap.source(SourceRow::default(), Some(0x1800)), Err(reason));
        assert_eq!(memory.calls.get(), calls);
        assert_eq!(lengths, original_lengths);
        assert!(sources.is_empty());
        assert!(terms.is_empty());
    }
    let mut memory = fixture();
    memory.bytes[0x180c - 0x1000..0x1810 - 0x1000].copy_from_slice(&4u32.to_le_bytes());
    let mut sources = Vec::new();
    let mut terms = Vec::new();
    let mut lengths = [0; 2];
    let mut required = [0; 2];
    let mut cap = Capture {
        memory: &memory,
        base: IMAGE,
        buffers: InfluenceBuffers {
            sources: &mut sources,
            terms: &mut terms,
            lengths: &mut lengths,
            required: &mut required,
        },
    };
    assert_eq!(
        cap.source(SourceRow::default(), Some(0x1800)),
        Err("naval weapon array invalid")
    );
    assert_eq!(*memory.reads.borrow(), [(0x1800, 16)]);
    assert_eq!(lengths, [0; 2]);
    assert_eq!(required, [1, 0]);
    assert!(sources.is_empty());
    assert!(terms.is_empty());
}

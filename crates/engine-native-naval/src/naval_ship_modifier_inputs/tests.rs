use engine_native::cship::modifier_sources::{doctrine_keys, terrain_from_template_kept};
use super::*;
use engine_native::cship::modifier_sources::terrain_from_template_with_policy;
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
            if (engine_native_memory::memory::SliceMemory { base: *base, bytes }).copy(address, output) {
                return true;
            }
        }
        if address == IMAGE + engine_native_parts::modifier_definition::TABLE_RVA.0 && output.len() == 16 {
            output[..8].copy_from_slice(&0x4000u64.to_le_bytes());
            output[8..12].copy_from_slice(&64u32.to_le_bytes());
            output[12..].copy_from_slice(&64u32.to_le_bytes());
            return true;
        }
        engine_native_memory::memory::SliceMemory {
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
                + key as usize * engine_native_parts::modifier_definition::STRIDE
                + engine_native_parts::modifier_definition::FLAGS,
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
    fn heap_text(&mut self, address: usize, payload: usize, value: &[u8]) {
        self.bytes
            .resize(self.bytes.len().max(payload + value.len() + 1 - 0x1000), 0);
        self.q(address, payload as u64);
        self.q(address + 16, value.len() as u64);
        self.q(address + 24, value.len() as u64);
        self.put(payload, value);
        self.put(payload + value.len(), &[0]);
    }
}

const TASK: usize = 0x1000;
const HEAD: usize = 0x1200;
const DICTIONARY: usize = 0x1300;
const GENERATOR: usize = 0x1400;
const NODES: usize = 0x10000;
const TOKENS: usize = 0x50000;

fn doctrine_fixture(count: usize) -> Fixture {
    use engine_native_memory::cpdx_lexer as lexer;
    let mut f = fixture();
    let tokens = TOKENS.max(NODES + count * 0x38);
    f.bytes.resize(tokens + (count + 1) * 32 - 0x1000, 0);
    let mut witness = vec![0; 64];
    witness[0] = 1;
    for offset in [4, 12] {
        witness[offset..offset + 4].copy_from_slice(&(count as u32).to_le_bytes());
    }
    witness[48..56].copy_from_slice(&(tokens as u64).to_le_bytes());
    for offset in [56, 60] {
        witness[offset..offset + 4].copy_from_slice(&(count as u32 + 1).to_le_bytes());
    }
    f.globals.push((IMAGE + lexer::INITIALIZED_RVA.0, witness));
    for (rva, value) in [
        (0x0343_9158, GENERATOR),
        (0x0343_9150, GENERATOR),
        (0x030f_eba8, DICTIONARY),
    ] {
        f.globals
            .push((IMAGE + rva, (value as u64).to_le_bytes().to_vec()));
    }
    for index in 0..=count {
        f.q(tokens + index * 32 + 24, 15);
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
        metadata: None,
    };
    visit_doctrine::<true>(
        memory,
        IMAGE,
        TASK,
        &mut DoctrineKeys::default(),
        &mut |row, accumulator| capture.source(row, accumulator),
    )?;
    Ok(sources)
}

#[test]
fn doctrine_exceeds_4096_nodes() {
    let sources = doctrine_sources(&doctrine_fixture(4097)).unwrap();
    assert_eq!(sources.len(), 4097);
    assert_eq!(sources[4096].index, 4096);
}

#[test]
fn doctrine_source_visitor_sizes_populations_in_operand_order() {
    for count in [15, 50] {
        let mut memory = doctrine_fixture(count);
        for index in 0..count {
            let node = NODES + index * 0x38;
            memory.put(node + 0x28, &(index as i32).to_le_bytes());
            memory.put(node + 0x2c, &(index as i32 + 7).to_le_bytes());
        }
        let mut sources = Vec::new();
        visit_doctrine::<true>(
            &memory,
            IMAGE,
            TASK,
            &mut DoctrineKeys::default(),
            &mut |row, accumulator| {
                sources.push((row, accumulator));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(sources.len(), count);
        for (index, (source, accumulator)) in sources.iter().enumerate() {
            assert_eq!(source.scope, 4);
            assert_eq!(source.index, index as u32);
            assert_eq!(source.selector, index as u32 + 1);
            assert_eq!(source.weights, [index as i64 + 7, index as i64]);
            assert_eq!(*accumulator, Some(0x1800));
        }
        let failed = IMAGE + engine_native_memory::cpdx_lexer::INITIALIZED_RVA.0;
        memory.fail = Some(failed);
        memory.reads.borrow_mut().clear();
        sources.clear();
        assert_eq!(
            visit_doctrine::<true>(
                &memory,
                IMAGE,
                TASK,
                &mut DoctrineKeys::default(),
                &mut |row, accumulator| {
                    sources.push((row, accumulator));
                    Ok(())
                }
            ),
            Err("naval field unreadable")
        );
        assert!(sources.is_empty());
        let reads = memory.reads.borrow();
        assert_eq!(reads.last(), Some(&(failed, 64)));
        assert_eq!(
            reads
                .iter()
                .filter(|&&(address, _)| address == failed)
                .count(),
            1
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
    assert_eq!(doctrine_keys::<true>(&f, IMAGE, 1), Ok([0; 2]));
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
        doctrine_keys::<true>(&f, IMAGE, 1),
        Err("naval doctrine modifier dictionary link invalid")
    );
}

#[test]
fn cold_doctrine_keys_keep_mod_heap_segments_and_original_text_faults() {
    const NODE: usize = 0x1500;
    const DATA: usize = TOKENS + 0x1000;
    for length in [257, 4097, 5000] {
        let mut memory = doctrine_fixture(2);
        let prefix = vec![b'p'; length];
        let selected = vec![b's'; length];
        let suffix = vec![b't'; length];
        let combined: Vec<_> = prefix
            .iter()
            .chain(&selected)
            .chain(&suffix)
            .copied()
            .collect();
        let selected_data = DATA;
        let prefix_data = selected_data + length + 1;
        let suffix_data = prefix_data + length + 1;
        let name_data = suffix_data + length + 1;
        memory.heap_text(TOKENS + 32, selected_data, &selected);
        memory.heap_text(GENERATOR + 0x10, prefix_data, &prefix);
        memory.heap_text(GENERATOR + 0x30, suffix_data, &suffix);
        memory.heap_text(TOKENS + 64, name_data, &combined);
        memory.q(DICTIONARY, NODE as u64);
        memory.q(DICTIONARY + 8, NODE as u64);
        memory.q(NODE, DICTIONARY as u64);
        memory.q(NODE + 8, DICTIONARY as u64);
        memory.put(NODE + 0x10, &2u32.to_le_bytes());
        memory.put(NODE + 0x14, &77u32.to_le_bytes());
        for address in [TOKENS + 32, GENERATOR + 0x10, GENERATOR + 0x30, TOKENS + 64] {
            assert!(engine_native_memory::msvc_string::read_text::<256>(&memory, address).is_none());
        }
        memory.reads.borrow_mut().clear();
        assert_eq!(doctrine_keys::<true>(&memory, IMAGE, 1), Ok([77; 2]));
        let capture_reads = memory.reads.borrow().clone();
        memory.reads.borrow_mut().clear();
        assert_eq!(doctrine_keys::<false>(&memory, IMAGE, 1), Ok([77; 2]));
        assert_eq!(*memory.reads.borrow(), capture_reads);
        for (failed, reason) in [
            (TOKENS + 32 + 16, "naval doctrine token string invalid"),
            (TOKENS + 32 + 24, "naval doctrine token string invalid"),
            (TOKENS + 32, "naval doctrine token string invalid"),
            (
                selected_data + length,
                "naval doctrine token string invalid",
            ),
            (selected_data + 256, "naval doctrine token string invalid"),
            (prefix_data + 256, "naval doctrine prefix invalid"),
            (suffix_data + 256, "naval doctrine suffix invalid"),
            (name_data + 256, "naval doctrine token string invalid"),
        ] {
            memory.fail = Some(failed);
            memory.reads.borrow_mut().clear();
            assert_eq!(doctrine_keys::<true>(&memory, IMAGE, 1), Err(reason));
            let reads = memory.reads.borrow();
            assert_eq!(reads.last().map(|&(address, _)| address), Some(failed));
            assert_eq!(
                reads
                    .iter()
                    .filter(|&&(address, _)| address == failed)
                    .count(),
                1
            );
        }
    }
}

#[test]
fn cold_terrain_fallback_keeps_mod_heap_names_and_named_first_faults() {
    const DATABASE: usize = 0x2100;
    const TYPES: usize = 0x2800;
    const LONG: usize = 0x3000;
    const MATCHED: usize = 0x3400;
    const DATA: usize = 0x10000;
    let key = *b"abcde";
    for length in [257, 4097, 5000] {
        let mut memory = fixture();
        memory.globals.push((IMAGE + 0x029f_5b58, key.to_vec()));
        memory.globals.push((
            IMAGE + engine_native_parts::cterrain_database::SINGLETON_RVA.0,
            (DATABASE as u64).to_le_bytes().to_vec(),
        ));
        memory.q(DATABASE + 0x40, TYPES as u64);
        memory.q(DATABASE + 0x48, (2u64 << 32) | 2);
        memory.q(TYPES, LONG as u64);
        memory.q(TYPES + 8, MATCHED as u64);
        for terrain in [LONG, MATCHED] {
            memory.q(terrain, (IMAGE + engine_native_parts::cterrain_type::VTABLE_RVA.0) as u64);
            memory.put(terrain + 0x10, &[1]);
        }
        memory.heap_text(LONG + 0x18, DATA, &vec![b'z'; length]);
        memory.put(MATCHED + 0x18, &key);
        memory.q(MATCHED + 0x28, key.len() as u64);
        memory.q(MATCHED + 0x30, 15);
        assert_eq!(ContextRow::default().terrain_name.allocated_bytes(), 0);
        assert!(engine_native_memory::msvc_string::read_text::<256>(&memory, LONG + 0x18).is_none());
        memory.reads.borrow_mut().clear();
        assert_eq!(
            terrain_from_template_with_policy::<true>(&memory, IMAGE, TASK),
            Ok(MATCHED)
        );
        let capture_reads = memory.reads.borrow().clone();
        memory.reads.borrow_mut().clear();
        assert_eq!(
            terrain_from_template_with_policy::<false>(&memory, IMAGE, TASK),
            Ok(MATCHED)
        );
        assert_eq!(*memory.reads.borrow(), capture_reads);
        let captured =
            text::<true>(&memory, LONG + 0x18, "naval selected terrain name invalid").unwrap();
        assert_eq!(captured.as_bytes(), vec![b'z'; length]);
        assert!(captured.allocated_bytes() >= length);
        let failed = DATA + 256;
        memory.fail = Some(failed);
        memory.reads.borrow_mut().clear();
        assert_eq!(
            terrain_from_template_with_policy::<true>(&memory, IMAGE, TASK),
            Err("naval terrain name invalid")
        );
        let reads = memory.reads.borrow();
        assert_eq!(reads.last().map(|&(address, _)| address), Some(failed));
        assert_eq!(
            reads
                .iter()
                .filter(|&&(address, _)| address == failed)
                .count(),
            1
        );
    }
}

#[test]
fn modifier_terms_and_keys_exceed_65536() {
    use engine_native_parts::modifier_definition as md;
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
    f.globals.push((IMAGE + md::TABLE_RVA.0, header));
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
        metadata: None,
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
    use engine_native_parts::modifier_definition as md;
    let source_reads = [
        (0x1800, 16),
        (IMAGE + md::TABLE_RVA.0, 16),
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
            metadata: None,
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
    use engine_native_parts::modifier_definition as md;
    for (fail, calls, copied, required_terms, reason) in [
        (0x1800, 1, 16, 0, "naval field unreadable"),
        (
            IMAGE + md::TABLE_RVA.0,
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
                metadata: None,
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
            metadata: None,
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
        metadata: None,
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

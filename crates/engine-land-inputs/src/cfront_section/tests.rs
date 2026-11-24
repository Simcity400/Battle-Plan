use super::*;
use engine_native_memory::memory::CountingMemory;

const PAIRS_AT: usize = 0x1000;
const FIRST: usize = 0x100;
const SECOND: usize = 0x400;

fn image(pairs: usize) -> Vec<u8> {
    let mut bytes = vec![0_u8; PAIRS_AT + pairs * 16];
    for (at, id) in [(FIRST, 4_u32), (SECOND, 8)] {
        bytes[at..at + 8].copy_from_slice(&(cprovince::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[at + cprovince::ID..at + cprovince::ID + 4].copy_from_slice(&id.to_le_bytes());
    }
    for pair in 0..pairs {
        let (left, right) = if pair % 2 == 0 {
            (FIRST, SECOND)
        } else {
            (SECOND, FIRST)
        };
        set_pair(&mut bytes, pair, left as u64, right as u64);
    }
    bytes
}

fn set_pair(bytes: &mut [u8], pair: usize, left: u64, right: u64) {
    let at = PAIRS_AT + pair * 16;
    bytes[at..at + 8].copy_from_slice(&left.to_le_bytes());
    bytes[at + 8..at + 16].copy_from_slice(&right.to_le_bytes());
}

fn per_pair(
    memory: &impl Memory,
    base: usize,
    pairs: Array,
    output: &mut Vec<u32>,
    start: usize,
) -> Result<(), &'static str> {
    for i in 0..pairs.count {
        let mut pair = [0; 2];
        let pointers = read::<[u8; 16]>(memory, pairs.element(i, 16).ok_or("front pair overflow")?)
            .ok_or("front pair unreadable")?;
        for (word, bytes) in pair.iter_mut().zip(pointers.chunks_exact(8)) {
            let pointer = u64::from_le_bytes(bytes.try_into().unwrap()) as usize;
            if pointer == 0
                || read::<u64>(memory, pointer)
                    != base.checked_add(cprovince::VTABLE_RVA.0).map(|v| v as u64)
            {
                return Err("front province identity mismatch");
            }
            *word = engine_native::corder_instance::plan_field(memory, pointer, cprovince::ID)?;
            if *word == 0 {
                return Err("front province sentinel");
            }
        }
        let end = start + (i as usize + 1) * 2;
        if end > output.len() {
            output.resize(end, 0);
        }
        output[end - 2..end].copy_from_slice(&pair);
    }
    Ok(())
}

#[test]
fn every_fault_keeps_the_per_pair_ids_prefix_and_first_failure() {
    let count = PAIR_BATCH + 5;
    let mut cases = vec![image(count)];
    for pair in [3, PAIR_BATCH + 2] {
        for (left, right) in [
            (FIRST as u64, 0),
            (0x20, FIRST as u64),
            (SECOND as u64, 0x9000_0000),
        ] {
            let mut bytes = image(count);
            set_pair(&mut bytes, pair, left, right);
            cases.push(bytes);
        }
        let mut bytes = image(count);
        bytes[SECOND + cprovince::ID..SECOND + cprovince::ID + 4].fill(0);
        cases.push(bytes);
        let mut bytes = image(count);
        bytes.truncate(PAIRS_AT + pair * 16 + 8);
        cases.push(bytes);
    }
    let mut bytes = image(count);
    let province = bytes.len();
    bytes.extend_from_slice(&(cprovince::VTABLE_RVA.0 as u64).to_le_bytes());
    set_pair(&mut bytes, 7, FIRST as u64, province as u64);
    cases.push(bytes);
    for bytes in &cases {
        let memory = engine_memory(bytes);
        let pairs = Array {
            data: PAIRS_AT,
            count: count as u32,
        };
        for start in [0, 6] {
            let (mut batched, mut expected) = (vec![9; start], vec![9; start]);
            let result = copy_pairs(&memory, 0, pairs, &mut batched, start);
            let reference = per_pair(&memory, 0, pairs, &mut expected, start);
            assert_eq!(result, reference);
            assert_eq!(batched, expected);
        }
    }
}

fn engine_memory(bytes: &[u8]) -> engine_native_memory::memory::SliceMemory<'_> {
    engine_native_memory::memory::SliceMemory { base: 0, bytes }
}

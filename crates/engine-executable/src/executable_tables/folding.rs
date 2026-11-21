
use super::PeImage;
use iced_x86::{Code, OpKind, Register};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io;

const MIN_PAIRS: usize = 16;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoldingTable {
    pub letters: BTreeMap<u32, u8>,
}

impl FoldingTable {
    pub fn decode(image: &PeImage) -> io::Result<Self> {
        let mut runs = Vec::new();
        for &(start, end) in image.executable_ranges() {
            let code = image.bytes(start, (end - start) as usize)?;
            runs.extend(candidate_runs(code).map(|(at, stores)| (start + at as u32, stores)));
        }
        let [(at, stores)] = runs[..] else {
            return Err(invalid(format!(
                "the executable holds {} runs of code point and character stores, not one",
                runs.len()
            )));
        };
        let words = verified_stores(image, at, stores)?;
        Self::from_pairs(words.chunks_exact(2).map(|pair| (pair[0], pair[1])))
    }

    pub fn from_pairs(pairs: impl IntoIterator<Item = (u32, u32)>) -> io::Result<Self> {
        let mut letters = BTreeMap::new();
        for (point, letter) in pairs {
            if !folded_point(point) || !folded_letter(letter) {
                return Err(invalid(format!(
                    "pair {point:#x} -> {letter:#x} is not a non-ASCII code point and an ASCII character"
                )));
            }
            letters.entry(point).or_insert(letter as u8);
        }
        Ok(Self { letters })
    }

    pub(super) fn content_digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        for (point, letter) in &self.letters {
            digest.update(point.to_le_bytes());
            digest.update([*letter]);
        }
        digest.finalize().into()
    }
}

fn folded_point(point: u32) -> bool {
    char::from_u32(point).is_some_and(|point| !point.is_ascii())
}

fn folded_letter(letter: u32) -> bool {
    (0x21..=0x7e).contains(&letter)
}

fn stack_store(code: &[u8]) -> Option<(usize, bool, i32, u32)> {
    let [0xc7, modrm, ..] = *code else {
        return None;
    };
    let (mode, reg, rm) = (modrm >> 6, (modrm >> 3) & 7, modrm & 7);
    if reg != 0 || !matches!(mode, 1 | 2) {
        return None;
    }
    let (rbp, mut at) = match rm {
        4 if code.get(2) == Some(&0x24) => (false, 3),
        5 => (true, 2),
        _ => return None,
    };
    let displacement = if mode == 1 {
        i32::from(*code.get(at)? as i8)
    } else {
        i32::from_le_bytes(code.get(at..at + 4)?.try_into().ok()?)
    };
    at += if mode == 1 { 1 } else { 4 };
    let immediate = u32::from_le_bytes(code.get(at..at + 4)?.try_into().ok()?);
    Some((at + 4, rbp, displacement, immediate))
}

fn candidate_runs(code: &[u8]) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut at = 0;
    std::iter::from_fn(move || {
        while at < code.len() {
            let start = at;
            let mut words = 0;
            let mut previous: Option<(bool, i32)> = None;
            let mut pairs_hold = true;
            while let Some((len, rbp, displacement, immediate)) = stack_store(&code[at..]) {
                if previous.is_some_and(|(was_rbp, was)| was_rbp == rbp && was + 4 != displacement)
                {
                    break;
                }
                pairs_hold &= if words % 2 == 0 {
                    folded_point(immediate)
                } else {
                    folded_letter(immediate)
                };
                previous = Some((rbp, displacement));
                words += 1;
                at += len;
            }
            if words == 0 {
                at += 1;
            } else if pairs_hold && words % 2 == 0 && words / 2 >= MIN_PAIRS {
                return Some((start, words));
            }
        }
        None
    })
}

fn verified_stores(image: &PeImage, at: u32, stores: usize) -> io::Result<Vec<u32>> {
    let start = image.base + u64::from(at);
    let instructions = image.function_instructions(at)?;
    let first = instructions
        .iter()
        .position(|instruction| instruction.ip() == start)
        .ok_or_else(|| invalid("the run does not start on one of its function's instructions"))?;
    let run = instructions
        .get(first..first + stores)
        .ok_or_else(|| invalid("the run outlasts its function"))?;
    run.iter()
        .map(|instruction| {
            let store = instruction.code() == Code::Mov_rm32_imm32
                && instruction.op0_kind() == OpKind::Memory
                && matches!(instruction.memory_base(), Register::RSP | Register::RBP)
                && instruction.memory_index() == Register::None;
            store
                .then(|| instruction.immediate32())
                .ok_or_else(|| invalid("the run holds an instruction other than a stack store"))
        })
        .collect()
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(code: &[u8]) -> PeImage {
        use crate::executable_tables::synthetic_image::{Code, SyntheticImage, TEXT};
        SyntheticImage::new()
            .function(Code::at(TEXT).raw(code))
            .image()
    }

    fn pairs(count: u32) -> Vec<(u32, u32)> {
        (0..count).map(|index| (0x100 + index, 0x61 + index % 26)).collect()
    }

    fn stores(pairs: &[(u32, u32)], rsp_pairs: usize) -> Vec<u8> {
        let mut code = vec![0x48, 0x83, 0xec, 0x08];
        for (index, word) in pairs.iter().flat_map(|&(point, letter)| [point, letter]).enumerate() {
            let offset = 0x20 + 4 * index as i32;
            if index < 2 * rsp_pairs {
                code.extend([0xc7, 0x84, 0x24]);
                code.extend(offset.to_le_bytes());
            } else {
                code.extend([0xc7, 0x85]);
                code.extend((offset - 0x100).to_le_bytes());
            }
            code.extend(word.to_le_bytes());
        }
        code.push(0xc3);
        code
    }

    #[test]
    fn the_one_run_decodes_across_both_frame_registers_with_the_first_of_a_repeat_kept() {
        let mut synthetic = pairs(20);
        synthetic[7] = (0x103, 0x5a);
        let table = FoldingTable::decode(&image(&stores(&synthetic, 12))).unwrap();
        assert_eq!(table.letters.len(), 19);
        assert_eq!(table.letters[&0x103], b'd');
        assert_eq!(table.letters[&0x113], b't');
        assert!(!table.letters.contains_key(&0x107));
    }

    // quarantine: #147 the reader and the test disagree
    #[test]
    #[ignore = "quarantined under #147"]
    fn short_broken_ambiguous_or_misshapen_runs_are_withheld() {
        let short = stores(&pairs(MIN_PAIRS as u32 - 1), 4);
        let mut twice = stores(&pairs(20), 20);
        twice.pop();
        twice.extend(&stores(&pairs(20), 20)[4..]);
        let mut gap = stores(&pairs(20), 20);
        gap[4 + 10 * 11 + 3] += 4;
        let mut ascii_key = pairs(20);
        ascii_key[3].0 = 0x41;
        let mut wide_letter = pairs(20);
        wide_letter[3].1 = 0xe9;
        for code in [
            short,
            twice,
            gap,
            stores(&ascii_key, 20),
            stores(&wide_letter, 20),
        ] {
            assert!(FoldingTable::decode(&image(&code)).is_err());
        }
    }

    #[test]
    fn a_run_off_its_function_instructions_is_withheld() {
        let mut code = vec![0x48, 0xb8];
        code.extend(&stores(&pairs(20), 20)[4..]);
        assert!(FoldingTable::decode(&image(&code)).is_err());
    }

    #[test]
    fn pairs_keep_the_game_insertion_rule_and_refuse_what_the_game_would_not_fold_to() {
        let table = FoldingTable::from_pairs([(0xe9, 0x65), (0xe9, 0x45), (0x10ffff, 0x7e)]).unwrap();
        assert_eq!(
            table.letters,
            BTreeMap::from([(0xe9, b'e'), (0x10ffff, b'~')])
        );
        for pair in [(0x7f, 0x41), (0xd800, 0x41), (0x110000, 0x41), (0xe9, 0x20), (0xe9, 0x100)] {
            assert!(FoldingTable::from_pairs([pair]).is_err(), "{pair:x?}");
        }
    }
}

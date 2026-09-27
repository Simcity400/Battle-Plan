//! The fixed-index contract of one attachment. Built-in modifier registrations
//! and static lexer tokens are process-static: `140552CB0` registers built-ins
//! at startup, `1405FF750` appends dynamic keys above the gap and `1424A87E0`
//! computes the static token maximum once. A consistent check is therefore
//! final for the attachment; an interrupted one is repeated on a later hour.

use crate::{
    Memory, cpdx_lexer, cresearch_slot, modifier_definition as definition, modifier_keys,
    msvc_string, read,
};

/// Default row between built-ins and dynamic definitions (`140548F00`;
/// `1405FF750` registers from 0x29a + 1). A layout sentinel, not a content count.
pub const GAP: u32 = 0x29a;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub id: u32,
    pub name: &'static str,
}

/// Static lexer tokens the executable hard-codes and readers compare by id.
/// Dynamic ids follow load order, so each must be static and spell its name.
pub const TOKENS: &[Token] = &[
    Token {
        id: cresearch_slot::IDLE_HANDLE,
        name: "empty",
    },
    Token {
        id: cpdx_lexer::FIXED15_TOKEN,
        name: "long_float",
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Failure {
    pub name: &'static str,
    pub index: u32,
    pub reason: &'static str,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{} ({:#x}): {}", self.name, self.index, self.reason)
    }
}

/// Final, allocation-free result of a consistent check. The game thread
/// publishes it; named failures are formatted during conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    pub static_token_maximum: u32,
    pub gap: Result<(), &'static str>,
    pub modifiers: [Result<(), &'static str>; modifier_keys::ALL.len()],
    pub tokens: [Result<(), &'static str>; TOKENS.len()],
}

impl Report {
    pub fn is_verified(&self) -> bool {
        self.gap.is_ok() && self.modifiers.iter().chain(&self.tokens).all(Result::is_ok)
    }

    /// The first failure among modifier `keys` and static `tokens`; a changed
    /// gap fails every modifier key.
    pub fn failure(&self, keys: &[u32], tokens: &[u32]) -> Option<Failure> {
        if let (false, Err(reason)) = (keys.is_empty(), self.gap) {
            return Some(Failure {
                name: "modifier definition gap",
                index: GAP,
                reason,
            });
        }
        let modifiers = modifier_keys::ALL
            .iter()
            .map(|key| (key.index, key.name))
            .zip(&self.modifiers)
            .filter(|((index, _), _)| keys.contains(index));
        let static_tokens = TOKENS
            .iter()
            .map(|token| (token.id, token.name))
            .zip(&self.tokens)
            .filter(|((id, _), _)| tokens.contains(id));
        modifiers
            .chain(static_tokens)
            .find_map(|((index, name), result)| {
                result.err().map(|reason| Failure {
                    name,
                    index,
                    reason,
                })
            })
    }
}

const _: () = assert!(definition::TOKEN == definition::INDEX + 4);

/// `Err` means the check could not complete consistently: the table or
/// dictionary was unreadable, or a writer changed either meanwhile. All reads
/// are guarded.
pub fn check(memory: &impl Memory, base: usize) -> Result<Report, &'static str> {
    let (dictionary, witness) = cpdx_lexer::TokenNames::witnessed(memory, base)?;
    let table = definition::table(memory, base)?;
    let mut buffer = [0; longest_name()];
    let mut spells = |token: u32, name: &str| -> Result<bool, &'static str> {
        let text = dictionary.text(memory, token)?;
        if text.len() != name.len() {
            return Ok(false);
        }
        text.copy(memory, &mut buffer)?;
        Ok(&buffer[..name.len()] == name.as_bytes())
    };
    let gap = match table.element(GAP, definition::STRIDE) {
        None => Err("gap row missing"),
        Some(address) => {
            let row =
                read::<[u8; definition::STRIDE]>(memory, address).ok_or("gap row unreadable")?;
            if row == gap_bytes() {
                Ok(())
            } else {
                Err("row is not the default-constructed gap")
            }
        }
    };
    let mut modifiers = [Ok(()); modifier_keys::ALL.len()];
    for (result, key) in modifiers.iter_mut().zip(modifier_keys::ALL) {
        let Some(address) = table.element(key.index, definition::STRIDE) else {
            *result = Err("definition row missing");
            continue;
        };
        let words = read::<[u8; 8]>(memory, address + definition::INDEX)
            .ok_or("definition index/token unreadable")?;
        let index = u32::from_le_bytes(words[..4].try_into().unwrap());
        let token = u32::from_le_bytes(words[4..].try_into().unwrap());
        if index != key.index || token == 0 {
            *result = Err("definition index/token inconsistent");
        } else if !spells(token, key.name)? {
            *result = Err("definition token spells a different script name");
        }
    }
    let static_token_maximum = witness.static_maximum();
    let mut tokens = [Ok(()); TOKENS.len()];
    for (result, token) in tokens.iter_mut().zip(TOKENS) {
        if token.id > static_token_maximum {
            *result = Err("exceeds the static token maximum");
        } else if !spells(token.id, token.name)? {
            *result = Err("token spells a different name");
        }
    }
    cpdx_lexer::unchanged(memory, base, witness)?;
    if definition::table(memory, base)? != table {
        return Err("modifier definition table changed during layout check");
    }
    Ok(Report {
        static_token_maximum,
        gap,
        modifiers,
        tokens,
    })
}

const fn longest_name() -> usize {
    let mut result = 0;
    let mut index = 0;
    while index < modifier_keys::ALL.len() {
        if modifier_keys::ALL[index].name.len() > result {
            result = modifier_keys::ALL[index].name.len();
        }
        index += 1;
    }
    index = 0;
    while index < TOKENS.len() {
        if TOKENS[index].name.len() > result {
            result = TOKENS[index].name.len();
        }
        index += 1;
    }
    result
}

fn gap_bytes() -> [u8; definition::STRIDE] {
    let mut bytes = [0; definition::STRIDE];
    for string in [
        definition::NAME_KEY,
        definition::POSTFIX_KEY,
        definition::ARGUMENT,
    ] {
        let at = string + msvc_string::CAPACITY;
        bytes[at..at + 8].copy_from_slice(&msvc_string::INLINE_CAPACITY.to_le_bytes());
    }
    bytes[definition::CATEGORY_MASK..definition::CATEGORY_MASK + 4]
        .copy_from_slice(&u32::MAX.to_le_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, collections::BTreeMap};

    const TABLE: usize = 0x10000;
    const NAMES: usize = 0x400000;
    const TEXT: usize = 0x600000;
    /// Fixture modifier tokens sit between the two fixed token ids.
    const KEY_TOKENS: u32 = 0x1000;

    struct Fixture(BTreeMap<usize, u8>);
    impl Fixture {
        fn put(&mut self, address: usize, bytes: &[u8]) {
            self.0.extend(
                bytes
                    .iter()
                    .enumerate()
                    .map(|(index, &byte)| (address + index, byte)),
            );
        }
        fn new() -> Self {
            let mut result = Self(BTreeMap::new());
            let mut header = [0; 16];
            header[..8].copy_from_slice(&(TABLE as u64).to_le_bytes());
            header[8..12].copy_from_slice(&(GAP + 1).to_le_bytes());
            header[12..].copy_from_slice(&(GAP + 1).to_le_bytes());
            result.put(definition::TABLE_RVA, &header);
            result.put(TABLE + GAP as usize * definition::STRIDE, &gap_bytes());
            let mut lexer = [0; 64];
            lexer[0] = 1;
            lexer[48..56].copy_from_slice(&(NAMES as u64).to_le_bytes());
            lexer[56..60].copy_from_slice(&0x6001_u32.to_le_bytes());
            lexer[60..64].copy_from_slice(&0x6001_u32.to_le_bytes());
            result.put(cpdx_lexer::INITIALIZED_RVA, &lexer);
            result.maximum(0x5000);
            for key in modifier_keys::ALL {
                let mut row = [0; 8];
                row[..4].copy_from_slice(&key.index.to_le_bytes());
                row[4..].copy_from_slice(&(KEY_TOKENS + key.index).to_le_bytes());
                result.put(
                    TABLE + key.index as usize * definition::STRIDE + definition::INDEX,
                    &row,
                );
                result.name(KEY_TOKENS + key.index, key.name.as_bytes());
            }
            for token in TOKENS {
                result.name(token.id, token.name.as_bytes());
            }
            result
        }
        fn maximum(&mut self, maximum: u32) {
            self.put(cpdx_lexer::MAX_TOKEN_ID_RVA, &maximum.to_le_bytes());
            self.put(cpdx_lexer::STATIC_MAX_TOKEN_ID_RVA, &maximum.to_le_bytes());
        }
        fn name(&mut self, token: u32, name: &[u8]) {
            let mut header = [0; 32];
            let pointer = TEXT + token as usize * 128;
            header[..8].copy_from_slice(&(pointer as u64).to_le_bytes());
            header[16..24].copy_from_slice(&(name.len() as u64).to_le_bytes());
            header[24..32].copy_from_slice(&(name.len().max(16) as u64).to_le_bytes());
            self.put(NAMES + token as usize * 32, &header);
            self.put(pointer, name);
            self.put(pointer + name.len(), &[0]);
        }
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
            for (index, byte) in bytes.iter_mut().enumerate() {
                let Some(value) = self.0.get(&(address + index)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }

    #[test]
    fn every_fixed_key_is_verified_and_mismatch_names_the_exact_key() {
        let mut fixture = Fixture::new();
        let valid = check(&fixture, 0).unwrap();
        assert!(valid.is_verified());
        assert_eq!(valid.static_token_maximum, 0x5000);
        for (position, &key) in modifier_keys::ALL.iter().enumerate() {
            let mut wrong = key.name.as_bytes().to_vec();
            wrong[0] = b'!';
            fixture.name(KEY_TOKENS + key.index, &wrong);
            let report = check(&fixture, 0).unwrap();
            let failure = report.failure(&[key.index], &[]).unwrap();
            assert_eq!((failure.name, failure.index), (key.name, key.index));
            assert!(failure.to_string().contains("different script name"));
            assert_eq!(report.modifiers.iter().filter(|r| r.is_err()).count(), 1);
            assert!(report.modifiers[position].is_err());
            fixture.name(KEY_TOKENS + key.index, key.name.as_bytes());
        }
    }

    #[test]
    fn gap_is_required_and_must_be_the_whole_default_row() {
        for offset in [
            0,
            16,
            24,
            definition::FLAGS,
            definition::CATEGORY_MASK,
            definition::FEATURE_SELECTOR,
            definition::INDEX,
            definition::TOKEN,
            definition::PRECISION,
        ] {
            let mut fixture = Fixture::new();
            let at = TABLE + GAP as usize * definition::STRIDE + offset;
            *fixture.0.get_mut(&at).unwrap() ^= 1;
            let report = check(&fixture, 0).unwrap();
            let failure = report
                .failure(&[modifier_keys::ARMY_SPEED_FACTOR], &[])
                .unwrap();
            assert_eq!(failure.name, "modifier definition gap");
            assert_eq!(report.failure(&[], &[cresearch_slot::IDLE_HANDLE]), None);
        }
        let mut fixture = Fixture::new();
        fixture.put(definition::TABLE_RVA + 12, &GAP.to_le_bytes());
        assert_eq!(check(&fixture, 0).unwrap().gap, Err("gap row missing"));
    }

    #[test]
    fn inconsistent_rows_fail_and_short_reads_repeat_the_check() {
        let key = modifier_keys::ALL[0];
        let address = TABLE + key.index as usize * definition::STRIDE + definition::INDEX;
        for offset in 0..8 {
            let mut fixture = Fixture::new();
            fixture.0.remove(&(address + offset));
            assert_eq!(check(&fixture, 0), Err("definition index/token unreadable"));
        }
        let mut fixture = Fixture::new();
        fixture.put(address, &0_u32.to_le_bytes());
        assert_eq!(
            check(&fixture, 0).unwrap().modifiers[0],
            Err("definition index/token inconsistent")
        );
        fixture.put(address, &key.index.to_le_bytes());
        fixture.put(address + 4, &0_u32.to_le_bytes());
        assert_eq!(
            check(&fixture, 0).unwrap().modifiers[0],
            Err("definition index/token inconsistent")
        );
        fixture.put(definition::TABLE_RVA + 12, &key.index.to_le_bytes());
        assert_eq!(
            check(&fixture, 0).unwrap().modifiers[0],
            Err("definition row missing")
        );
    }

    #[test]
    fn static_tokens_must_spell_their_name_within_the_inclusive_maximum() {
        let mut fixture = Fixture::new();
        fixture.maximum(cresearch_slot::IDLE_HANDLE);
        assert!(check(&fixture, 0).unwrap().is_verified());
        fixture.maximum(cresearch_slot::IDLE_HANDLE - 1);
        let report = check(&fixture, 0).unwrap();
        assert_eq!(report.static_token_maximum, cresearch_slot::IDLE_HANDLE - 1);
        let failure = report
            .failure(
                &[],
                &[cresearch_slot::IDLE_HANDLE, cpdx_lexer::FIXED15_TOKEN],
            )
            .unwrap();
        assert_eq!(
            failure.to_string(),
            "empty (0x2a4e): exceeds the static token maximum"
        );
        assert_eq!(report.failure(&[modifier_keys::NAVY_ORG], &[]), None);
        for (position, token) in TOKENS.iter().enumerate() {
            let mut fixture = Fixture::new();
            fixture.name(token.id, b"renumbered");
            let report = check(&fixture, 0).unwrap();
            assert_eq!(
                report.tokens[position],
                Err("token spells a different name")
            );
            assert_eq!(report.failure(&[], &[token.id]).unwrap().name, token.name);
        }
    }

    #[test]
    fn changing_dictionary_or_table_repeats_the_check() {
        struct Changing {
            fixture: Fixture,
            address: usize,
            observations: Cell<usize>,
        }
        impl Memory for Changing {
            fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
                if !self.fixture.copy(address, bytes) {
                    return false;
                }
                if address == self.address {
                    let calls = self.observations.get();
                    self.observations.set(calls + 1);
                    if calls != 0 {
                        bytes[0] ^= 1;
                    }
                }
                true
            }
        }
        for address in [cpdx_lexer::INITIALIZED_RVA, definition::TABLE_RVA] {
            let fixture = Changing {
                fixture: Fixture::new(),
                address,
                observations: Cell::new(0),
            };
            assert!(check(&fixture, 0).is_err());
            assert_eq!(fixture.observations.get(), 2);
        }
    }

    #[test]
    fn registry_has_unique_indices_and_names_and_excludes_gap() {
        for (i, key) in modifier_keys::ALL.iter().enumerate() {
            assert!(key.index < GAP);
            assert!(!key.name.is_empty());
            assert!(
                modifier_keys::ALL[..i]
                    .iter()
                    .all(|other| other.index != key.index && other.name != key.name)
            );
        }
    }
}

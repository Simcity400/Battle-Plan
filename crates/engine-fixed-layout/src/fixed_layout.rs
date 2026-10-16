//! Fixed-index contract for one attachment. Built-in modifier registrations
//! and static tokens are process-static. Dynamic definitions append beyond the
//! gap. A consistent check is final for the attachment; an interrupted check
//! is repeated on a later hour. Checking runs hourly because an attachment
//! made during startup can precede completed registration.

use crate::{
    Memory, cbuilding_database, cpdx_lexer, modifier_definition as definition, modifier_keys,
    msvc_string, read,
};

/// Default row between built-ins and dynamic definitions; a layout sentinel
/// rather than a content count.
pub use crate::build::FIXED_MODIFIER_GAP as GAP;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub id: u32,
    pub name: &'static str,
}

/// Narrow static lexer-token identities compared by fixed-index readers.
/// Dynamic IDs follow load order, so these dependencies must remain static.
pub const TOKEN_IDS: &[u32] = &[
    cbuilding_database::INFRASTRUCTURE_TOKEN,
    crate::raid_tooltip_operands::NUCLEAR_BOMB_TOKEN,
    crate::raid_tooltip_operands::THERMONUCLEAR_BOMB_TOKEN,
    cpdx_lexer::IDLE_RESEARCH_TOKEN,
    cpdx_lexer::FIXED15_TOKEN,
    cpdx_lexer::MAXIMUM_SPEED_TOKEN,
];

pub fn tokens() -> impl Iterator<Item = Token> + Clone {
    TOKEN_IDS.iter().map(|&id| Token {
        id,
        name: crate::executable_tables::get()
            .keywords
            .by_id(id)
            .expect("fixed token identity is present in verified executable metadata"),
    })
}

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
    pub modifiers: [Result<(), &'static str>; GAP as usize],
    pub tokens: [Result<(), &'static str>; TOKEN_IDS.len()],
}

impl Report {
    pub fn is_verified(&self) -> bool {
        self.gap.is_ok() && self.modifiers.iter().chain(&self.tokens).all(Result::is_ok)
    }

    /// Every mismatch: the gap and each built-in row, then the static tokens.
    pub fn failures(&self) -> impl Iterator<Item = Failure> + Clone + '_ {
        self.layout_failures().chain(self.token_failures())
    }

    fn layout_failures(&self) -> impl Iterator<Item = Failure> + Clone + '_ {
        let gap = self.gap.err().map(|reason| Failure {
            name: "modifier definition gap",
            index: GAP,
            reason,
        });
        let rows = modifier_keys::names()
            .iter()
            .zip(&self.modifiers)
            .enumerate()
            .filter_map(|(index, (&name, result))| {
                result.err().map(|reason| Failure {
                    name,
                    index: index as u32,
                    reason,
                })
            });
        gap.into_iter().chain(rows)
    }

    fn token_failures(&self) -> impl Iterator<Item = Failure> + Clone + '_ {
        tokens().zip(&self.tokens).filter_map(|(token, result)| {
            result.err().map(|reason| Failure {
                name: token.name,
                index: token.id,
                reason,
            })
        })
    }

    /// The failure that refuses a consumer of modifier `keys` and static
    /// `tokens`. The built-in layout is one contract: a changed gap or any
    /// mismatched built-in row, used or not, means the executable's index
    /// assignment moved, so it fails every modifier key. Tokens fail alone.
    pub fn failure(&self, keys: &[u32], tokens: &[u32]) -> Option<Failure> {
        (!keys.is_empty())
            .then(|| self.layout_failures().next())
            .flatten()
            .or_else(|| {
                self.token_failures()
                    .find(|failure| tokens.contains(&failure.index))
            })
    }
}

const _: () = assert!(definition::TOKEN == definition::INDEX + 4);

/// `Err` means the check could not complete consistently: the table or
/// dictionary was unreadable, or a writer changed either meanwhile. All reads
/// are guarded.
pub fn check(memory: &impl Memory, base: usize, buffer: &mut [u8]) -> Result<Report, &'static str> {
    let (dictionary, witness) = cpdx_lexer::TokenNames::witnessed(memory, base)?;
    let table = definition::table(memory, base)?;
    let mut spells = |token: u32, name: &str| -> Result<bool, &'static str> {
        let text = dictionary.text(memory, token)?;
        if text.len() != name.len() {
            return Ok(false);
        }
        text.copy(memory, buffer)?;
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
    let mut modifiers = [Ok(()); GAP as usize];
    for (key, (result, &name)) in modifiers.iter_mut().zip(modifier_keys::names()).enumerate() {
        let key = key as u32;
        let Some(address) = table.element(key, definition::STRIDE) else {
            *result = Err("definition row missing");
            continue;
        };
        let words = read::<[u8; 8]>(memory, address + definition::INDEX)
            .ok_or("definition index/token unreadable")?;
        let index = u32::from_le_bytes(words[..4].try_into().unwrap());
        let token = u32::from_le_bytes(words[4..].try_into().unwrap());
        if index != key || token == 0 {
            *result = Err("definition index/token inconsistent");
        } else if !spells(token, name)? {
            *result = Err("definition token spells a different script name");
        }
    }
    let static_token_maximum = witness.static_maximum();
    let mut tokens = [Ok(()); TOKEN_IDS.len()];
    for (result, token) in tokens.iter_mut().zip(self::tokens()) {
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

/// Text storage required for a complete fixed-layout check. Prepare it before
/// publishing capture hooks; the check never grows its caller-owned buffer.
pub fn longest_name() -> usize {
    modifier_keys::names()
        .iter()
        .map(|name| name.len())
        .chain(tokens().map(|token| token.name.len()))
        .max()
        .unwrap_or(0)
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

    fn initialize_tables() {
        crate::executable_tables::initialize_for_tests()
            .expect("fixed-layout tests require BATTLE_PLAN_GAME_DIR with the supported installation");
    }

    fn check(memory: &impl Memory, base: usize) -> Result<Report, &'static str> {
        initialize_tables();
        let mut buffer = vec![0; super::longest_name()];
        super::check(memory, base, &mut buffer)
    }

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
            initialize_tables();
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
            for (index, &name) in modifier_keys::names().iter().enumerate() {
                let index = index as u32;
                let mut row = [0; 8];
                row[..4].copy_from_slice(&index.to_le_bytes());
                row[4..].copy_from_slice(&(KEY_TOKENS + index).to_le_bytes());
                result.put(
                    TABLE + index as usize * definition::STRIDE + definition::INDEX,
                    &row,
                );
                result.name(KEY_TOKENS + index, name.as_bytes());
            }
            for token in tokens() {
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

    /// Any built-in row, including one no reader uses, fails every fixed key
    /// by its own name, and only the static tokens' consumers survive it.
    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn every_builtin_is_verified_and_a_mismatch_fails_every_fixed_key_by_name() {
        let mut fixture = Fixture::new();
        let valid = check(&fixture, 0).unwrap();
        assert!(valid.is_verified());
        assert_eq!(valid.static_token_maximum, 0x5000);
        for (index, &name) in modifier_keys::names().iter().enumerate() {
            let index = index as u32;
            let mut wrong = name.as_bytes().to_vec();
            wrong[0] = b'!';
            fixture.name(KEY_TOKENS + index, &wrong);
            let report = check(&fixture, 0).unwrap();
            assert!(!report.is_verified());
            assert_eq!(report.modifiers.iter().filter(|r| r.is_err()).count(), 1);
            for key in [modifier_keys::ALL_INDICES[0], modifier_keys::NAVY_ORG] {
                let failure = report.failure(&[key], &[]).unwrap();
                assert_eq!((failure.name, failure.index), (name, index));
                assert!(failure.to_string().contains("different script name"));
            }
            assert_eq!(
                report.failure(&[], &[cpdx_lexer::IDLE_RESEARCH_TOKEN]),
                None
            );
            assert_eq!(report.failures().collect::<Vec<_>>().len(), 1);
            fixture.name(KEY_TOKENS + index, name.as_bytes());
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
            assert_eq!(
                report.failure(&[], &[cpdx_lexer::IDLE_RESEARCH_TOKEN]),
                None
            );
        }
        let mut fixture = Fixture::new();
        fixture.put(definition::TABLE_RVA + 12, &GAP.to_le_bytes());
        assert_eq!(check(&fixture, 0).unwrap().gap, Err("gap row missing"));
    }

    #[test]
    fn inconsistent_rows_fail_and_short_reads_repeat_the_check() {
        initialize_tables();
        let key = modifier_keys::all().next().unwrap();
        let address = TABLE + key.index as usize * definition::STRIDE + definition::INDEX;
        for offset in 0..8 {
            let mut fixture = Fixture::new();
            fixture.0.remove(&(address + offset));
            assert_eq!(check(&fixture, 0), Err("definition index/token unreadable"));
        }
        let mut fixture = Fixture::new();
        fixture.put(address, &0_u32.to_le_bytes());
        assert_eq!(
            check(&fixture, 0).unwrap().modifiers[key.index as usize],
            Err("definition index/token inconsistent")
        );
        fixture.put(address, &key.index.to_le_bytes());
        fixture.put(address + 4, &0_u32.to_le_bytes());
        assert_eq!(
            check(&fixture, 0).unwrap().modifiers[key.index as usize],
            Err("definition index/token inconsistent")
        );
        fixture.put(definition::TABLE_RVA + 12, &key.index.to_le_bytes());
        assert_eq!(
            check(&fixture, 0).unwrap().modifiers[key.index as usize],
            Err("definition row missing")
        );
    }

    #[test]
    fn static_tokens_must_spell_their_name_within_the_inclusive_maximum() {
        initialize_tables();
        for (position, token) in tokens().enumerate() {
            let mut fixture = Fixture::new();
            // A token exactly at the inclusive static maximum is accepted,
            // even with later dynamic tokens in the reverse dictionary.
            fixture.put(cpdx_lexer::STATIC_MAX_TOKEN_ID_RVA, &token.id.to_le_bytes());
            fixture.put(cpdx_lexer::MAX_TOKEN_ID_RVA, &0x5000_u32.to_le_bytes());
            fixture.put(
                cpdx_lexer::DYNAMIC_COUNT_RVA,
                &(0x5000 - token.id).to_le_bytes(),
            );
            fixture.put(cpdx_lexer::DYNAMIC_CAPACITY_RVA, &0x5000_u32.to_le_bytes());
            fixture.put(cpdx_lexer::DYNAMIC_TOKENS_RVA, &0x800000_u64.to_le_bytes());
            assert_eq!(check(&fixture, 0).unwrap().tokens[position], Ok(()));
            fixture.put(
                cpdx_lexer::STATIC_MAX_TOKEN_ID_RVA,
                &(token.id - 1).to_le_bytes(),
            );
            fixture.put(
                cpdx_lexer::DYNAMIC_COUNT_RVA,
                &(0x5001 - token.id).to_le_bytes(),
            );
            let report = check(&fixture, 0).unwrap();
            assert_eq!(report.static_token_maximum, token.id - 1);
            let failure = report.failure(&[], &[token.id]).unwrap();
            assert_eq!(failure.name, token.name);
            assert_eq!(failure.reason, "exceeds the static token maximum");
            assert!(report.failures().any(|item| item == failure));

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
        initialize_tables();
        assert_eq!(modifier_keys::names().len(), GAP as usize);
        for (index, name) in modifier_keys::names().iter().enumerate() {
            assert!(!name.is_empty());
            assert!(!modifier_keys::names()[..index].contains(name));
        }
        let keys: Vec<_> = modifier_keys::all().collect();
        for (i, key) in keys.iter().enumerate() {
            assert!(
                keys[..i]
                    .iter()
                    .all(|other| other.index != key.index && other.name != key.name)
            );
        }
    }

    #[test]
    fn appended_dynamic_rows_do_not_change_the_builtin_layout() {
        let mut fixture = Fixture::new();
        fixture.put(definition::TABLE_RVA + 8, &(GAP + 2).to_le_bytes());
        fixture.put(definition::TABLE_RVA + 12, &(GAP + 2).to_le_bytes());
        // An unreadable dynamic row is intentionally outside this check.
        assert!(check(&fixture, 0).unwrap().is_verified());
    }
}

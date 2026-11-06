//! The walk's lexer tokens by their literal under the lexer's fold ([`Lexicon`]): one index of
//! the walker's LexicalTokens rows, built at the walk's first hour or when the table becomes
//! complete, and kept from the rows each hour changes. It holds each row's slot by its
//! literal's hash and nothing of the literal itself, which a lookup reads from the walker.

use super::{Facts, countries::edited};
use replay::{columns, segments::State};
use replay_data::rows::TableId;
use replay_frame::frame::Columns;
use std::{collections::BTreeMap, io};

/// Where [`Lexicon::index`] holds no row; no hash and slot pack to it ([`hash`]).
const EMPTY: u64 = u64::MAX;
/// Where [`Lexicon::hashes`] holds no indexed row.
const NONE: u32 = u32::MAX;

/// `literal`'s hash under the fold, never [`NONE`].
fn hash(literal: &[u8]) -> u32 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in literal {
        hash = (hash ^ u64::from(byte.to_ascii_lowercase())).wrapping_mul(0x0100_0000_01b3);
    }
    let hash = (hash >> 32) as u32 ^ hash as u32;
    hash.min(NONE - 1)
}

/// Whether `rows` read the live row keyed `key`.
fn reads(rows: Columns<'_>, key: u128) -> bool {
    rows.is_whole() || rows.position(key).is_ok()
}

/// What a literal folds to among the rows.
#[derive(Debug, PartialEq, Eq)]
pub enum Found<'a> {
    None,
    /// One row's token.
    One(u32),
    /// Several rows', and the literal of the last by key.
    Several(&'a [u8]),
}

/// The walker's LexicalTokens rows by their literal under the lexer's fold, while the table is
/// complete.
pub struct Lexicon {
    literal: usize,
    format: u16,
    complete: bool,
    /// The hour it was last kept at, and the walker's table it indexes then.
    at: (u64, i32, usize),
    /// Open addressing by hash, linear probing, a power of two long and never past half full:
    /// each row's hash in the high half, its slot in the low.
    index: Vec<u64>,
    shift: u32,
    len: usize,
    /// Each slot's hash as indexed; [`NONE`] where it holds none.
    hashes: Vec<u32>,
    /// The rows whose literal does not read, by key, with why.
    faults: BTreeMap<u128, (io::ErrorKind, String)>,
}

impl Lexicon {
    pub(crate) fn new(format: u16) -> io::Result<Self> {
        Ok(Self {
            literal: columns::field(TableId::LexicalTokens, "literal")?,
            format,
            complete: false,
            at: (0, 0, 0),
            index: Vec::new(),
            shift: 64,
            len: 0,
            hashes: Vec::new(),
            faults: BTreeMap::new(),
        })
    }

    /// Whether it indexes the table `rows` read: the walker's LexicalTokens of `(branch, hour)`.
    pub fn indexes(&self, (branch, hour): (u64, i32), rows: Columns<'_>) -> bool {
        self.complete && self.at == (branch, hour, std::ptr::from_ref(rows.table()) as usize)
    }

    /// Why a literal of `rows` does not read, the first by key: reading every literal of
    /// them refuses with it.
    pub fn fault(&self, rows: Columns<'_>) -> Option<io::Error> {
        self.faults
            .iter()
            .find(|&(&key, _)| reads(rows, key))
            .map(|(_, (kind, why))| io::Error::new(*kind, why.as_str()))
    }

    /// What `name` folds to among `rows`, whose table it indexes ([`Self::indexes`]).
    pub fn find<'a>(&self, rows: Columns<'a>, name: &[u8]) -> io::Result<Found<'a>> {
        if self.index.is_empty() {
            return Ok(Found::None);
        }
        let hash = hash(name);
        let mask = self.index.len() - 1;
        let mut at = self.home(hash);
        let mut found: Option<(u128, &'a [u8])> = None;
        let mut several = false;
        loop {
            let entry = self.index[at];
            if entry == EMPTY {
                break;
            }
            if (entry >> 32) as u32 == hash {
                let slot = entry as u32;
                let literal = rows.text(slot, self.literal)?;
                let key = rows.key(slot);
                if literal.eq_ignore_ascii_case(name) && reads(rows, key) {
                    several |= found.is_some();
                    if found.is_none_or(|(last, _)| key > last) {
                        found = Some((key, literal));
                    }
                }
            }
            at = (at + 1) & mask;
        }
        Ok(match found {
            None => Found::None,
            Some((_, literal)) if several => Found::Several(literal),
            Some((key, _)) => Found::One(key as u32),
        })
    }

    pub(crate) fn hour(
        &mut self,
        state: &State,
        facts: &Facts<'_>,
        (branch, hour): (u64, i32),
    ) -> io::Result<()> {
        let complete = !state.skips(TableId::LexicalTokens.index())
            && columns::complete(state, TableId::LexicalTokens);
        let rebuilt = facts.first() || complete != self.complete;
        self.complete = complete;
        if !complete {
            self.at = (0, 0, 0);
            self.clear();
            return Ok(());
        }
        let held = columns::table(state, TableId::LexicalTokens)?;
        self.at = (branch, hour, std::ptr::from_ref(held) as usize);
        let rows = Columns::new(
            TableId::LexicalTokens,
            held,
            state.dictionary(),
            (&[][..], true),
            self.format,
        );
        if rebuilt {
            self.clear();
            self.reserve(held.live().count())?;
            engine_allocation::reserve(&mut self.hashes, held.slots())?;
            self.hashes.resize(held.slots(), NONE);
            for slot in held.live().ones() {
                self.insert(rows, slot)?;
            }
            return Ok(());
        }
        let literal = 1_u64 << self.literal;
        if !facts.touched(TableId::LexicalTokens, literal) {
            return Ok(());
        }
        let changes = columns::changes(state, TableId::LexicalTokens)?;
        for (&slot, &key) in changes.deleted.iter().zip(&changes.deleted_keys) {
            self.remove(slot, key);
        }
        let edited = edited(state, TableId::LexicalTokens, literal)?;
        self.reserve(edited.len())?;
        for slot in edited {
            if let Some(key) = held.key(slot) {
                self.remove(slot, key);
                self.insert(rows, slot)?;
            }
        }
        Ok(())
    }

    fn clear(&mut self) {
        self.index = Vec::new();
        self.shift = 64;
        self.len = 0;
        self.hashes = Vec::new();
        self.faults.clear();
    }

    fn home(&self, hash: u32) -> usize {
        (u64::from(hash).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> self.shift) as usize
    }

    /// Room for `additional` rows more, the index never past half full.
    fn reserve(&mut self, additional: usize) -> io::Result<()> {
        let wanted = (self.len + additional) * 2;
        if wanted <= self.index.len() {
            return Ok(());
        }
        let slots = wanted.max(2).next_power_of_two();
        let old = std::mem::replace(&mut self.index, engine_allocation::filled(slots, EMPTY)?);
        self.shift = 64 - slots.trailing_zeros();
        for entry in old.into_iter().filter(|&entry| entry != EMPTY) {
            self.place(entry);
        }
        Ok(())
    }

    fn place(&mut self, entry: u64) {
        let mask = self.index.len() - 1;
        let mut at = self.home((entry >> 32) as u32);
        while self.index[at] != EMPTY {
            at = (at + 1) & mask;
        }
        self.index[at] = entry;
    }

    /// Indexes live `slot` by its literal, or keeps why it does not read.
    fn insert(&mut self, rows: Columns<'_>, slot: u32) -> io::Result<()> {
        match rows.text(slot, self.literal) {
            Ok(literal) => {
                let hash = hash(literal);
                self.reserve(1)?;
                self.place(u64::from(hash) << 32 | u64::from(slot));
                self.len += 1;
                let slots = slot as usize + 1;
                if let Some(more) = slots.checked_sub(self.hashes.len()) {
                    engine_allocation::reserve(&mut self.hashes, more)?;
                    self.hashes.resize(slots, NONE);
                }
                self.hashes[slot as usize] = hash;
            }
            Err(why) => {
                self.faults
                    .insert(rows.key(slot), (why.kind(), why.to_string()));
            }
        }
        Ok(())
    }

    /// Forgets `slot`, which held `key`.
    fn remove(&mut self, slot: u32, key: u128) {
        self.faults.remove(&key);
        let Some(hash) = self.hashes.get_mut(slot as usize) else {
            return;
        };
        let hash = std::mem::replace(hash, NONE);
        if hash == NONE {
            return;
        }
        let entry = u64::from(hash) << 32 | u64::from(slot);
        let mask = self.index.len() - 1;
        let mut at = self.home(hash);
        while self.index[at] != entry {
            if self.index[at] == EMPTY {
                return;
            }
            at = (at + 1) & mask;
        }
        self.len -= 1;
        // Backward shift: each entry past the hole moves into it unless its home lies between.
        let mut hole = at;
        let mut next = (hole + 1) & mask;
        while self.index[next] != EMPTY {
            let home = self.home((self.index[next] >> 32) as u32);
            if (next.wrapping_sub(home) & mask) >= (next.wrapping_sub(hole) & mask) {
                self.index[hole] = self.index[next];
                hole = next;
            }
            next = (next + 1) & mask;
        }
        self.index[hole] = EMPTY;
    }
}

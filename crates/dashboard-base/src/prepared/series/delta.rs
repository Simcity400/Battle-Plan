//! An accumulator saved as a base, then only the keys whose saved form's hash changed.
use super::{Accumulator, Batch, KeyState, invalid};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::{
    collections::{BTreeSet, HashMap},
    io,
};

/// Bytes of key states a base's piece holds at most before the next begins.
const PIECE: usize = 4 << 20;

/// Each key's saved form, by its hash, as the last save wrote or the last load read it.
#[derive(Default)]
pub(super) struct Hashes(HashMap<u64, u64>);

fn hash(bytes: &[u8]) -> u64 {
    let digest = blake3::hash(bytes);
    u64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("eight bytes"))
}

#[derive(Serialize)]
struct Scalars<'a> {
    frontier: Option<i32>,
    previous: Option<i32>,
    active: &'a BTreeSet<u64>,
    #[serde(skip_serializing_if = "Batch::is_empty")]
    batch: &'a Batch,
}

#[derive(Deserialize)]
struct ScalarsOwned {
    frontier: Option<i32>,
    previous: Option<i32>,
    active: BTreeSet<u64>,
    #[serde(default)]
    batch: Batch,
}

#[derive(Serialize)]
struct Piece<'a> {
    /// It begins a base: every state before it is dropped.
    #[serde(rename = "w")]
    whole: bool,
    #[serde(rename = "s")]
    states: &'a [(u64, &'a RawValue)],
    /// The last piece of a save holds them.
    #[serde(rename = "c", skip_serializing_if = "Option::is_none")]
    scalars: Option<Scalars<'a>>,
}

#[derive(Deserialize)]
struct PieceRead<'a> {
    #[serde(rename = "w")]
    whole: bool,
    #[serde(rename = "s", borrow)]
    states: Vec<(u64, &'a RawValue)>,
    #[serde(rename = "c")]
    scalars: Option<ScalarsOwned>,
}

impl Accumulator {
    /// What changed since the last save, in pieces; the last holds the scalars.
    pub fn save_changes(&mut self, whole: bool) -> io::Result<Vec<Vec<u8>>> {
        let _cpu = ::replay_frame::cpu::scope_phase("series:save_changes");
        if whole {
            self.hashes.0.clear();
        }
        let mut pieces = Vec::new();
        let mut held: Vec<Box<RawValue>> = Vec::new();
        let mut keys: Vec<u64> = Vec::new();
        let mut bytes = 0;
        let mut first = whole;
        for (&key, state) in &self.states {
            let saved = serde_json::value::to_raw_value(state)?;
            let at = hash(saved.get().as_bytes());
            if self.hashes.0.insert(key, at) == Some(at) {
                continue;
            }
            bytes += saved.get().len();
            keys.push(key);
            held.push(saved);
            if bytes >= PIECE {
                pieces.push(piece(first, &keys, &held, None)?);
                (first, bytes) = (false, 0);
                keys.clear();
                held.clear();
            }
        }
        // An accumulator never drops a key: one gone is a defect, not a change to save.
        if self.hashes.0.len() > self.states.len() {
            return Err(invalid("A series key's state was dropped since the last save"));
        }
        let scalars = Scalars {
            frontier: self.frontier,
            previous: self.previous,
            active: &self.active,
            batch: &self.batch,
        };
        pieces.push(piece(first, &keys, &held, Some(scalars))?);
        Ok(pieces)
    }

    /// Takes up a save's pieces, in order, over what the saves before left.
    pub fn apply_changes(&mut self, bytes: &[u8]) -> io::Result<()> {
        let read: PieceRead<'_> = serde_json::from_slice(bytes)?;
        if read.whole {
            self.states.clear();
            self.hashes.0.clear();
        }
        for (key, saved) in read.states {
            let state: KeyState = serde_json::from_str(saved.get())?;
            self.hashes.0.insert(key, hash(saved.get().as_bytes()));
            self.states.insert(key, state);
        }
        if let Some(scalars) = read.scalars {
            self.frontier = scalars.frontier;
            self.previous = scalars.previous;
            self.active = scalars.active;
            self.batch = scalars.batch;
            if self.active.iter().any(|key| !self.states.contains_key(key)) {
                return Err(invalid("A saved series key is active with no state"));
            }
        }
        Ok(())
    }
}

fn piece(
    whole: bool,
    keys: &[u64],
    held: &[Box<RawValue>],
    scalars: Option<Scalars<'_>>,
) -> io::Result<Vec<u8>> {
    let states: Vec<(u64, &RawValue)> = keys.iter().copied().zip(held.iter().map(|raw| &**raw)).collect();
    Ok(serde_json::to_vec(&Piece {
        whole,
        states: &states,
        scalars,
    })?)
}

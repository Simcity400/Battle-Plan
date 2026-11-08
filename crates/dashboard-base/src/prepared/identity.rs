//! Identities of recorded facts, digested from their bytes, and an interner numbering values.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::HashMap, hash::Hash};

/// A digest of facts, each written with its width or length so no two sequences collide.
pub struct Digest(blake3::Hasher);

impl Default for Digest {
    fn default() -> Self {
        Self(blake3::Hasher::new())
    }
}

impl Digest {
    pub fn u8(&mut self, value: u8) -> &mut Self {
        self.0.update(&[value]);
        self
    }

    pub fn u32(&mut self, value: u32) -> &mut Self {
        self.0.update(&value.to_le_bytes());
        self
    }

    pub fn i32(&mut self, value: i32) -> &mut Self {
        self.0.update(&value.to_le_bytes());
        self
    }

    pub fn u64(&mut self, value: u64) -> &mut Self {
        self.0.update(&value.to_le_bytes());
        self
    }

    pub fn bytes(&mut self, value: &[u8]) -> &mut Self {
        self.u64(value.len() as u64);
        self.0.update(value);
        self
    }

    /// `value`, or its absence, told apart from every value.
    pub fn some<T>(&mut self, value: Option<T>, write: impl FnOnce(&mut Self, T)) -> &mut Self {
        match value {
            Some(value) => {
                self.u8(1);
                write(self, value);
            }
            None => {
                self.u8(0);
            }
        }
        self
    }

    pub fn finish(&self) -> Identity {
        let bytes = self.0.finalize();
        let bytes = bytes.as_bytes();
        Identity(
            u64::from_le_bytes(bytes[..8].try_into().expect("eight bytes")),
            u64::from_le_bytes(bytes[8..16].try_into().expect("eight bytes")),
        )
    }
}

/// A [`Digest`]'s result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub struct Identity(pub u64, pub u64);

/// Values numbered as they are first seen: the number a value is given never changes.
#[derive(Clone, Debug)]
pub struct Interner<T: Eq + Hash + Clone> {
    values: Vec<T>,
    numbers: HashMap<T, u32>,
}

impl<T: Eq + Hash + Clone> Default for Interner<T> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            numbers: HashMap::new(),
        }
    }
}

impl<T: Eq + Hash + Clone> Interner<T> {
    /// `value`'s number, given it now if it has none.
    pub fn intern(&mut self, value: &T) -> u32 {
        if let Some(&number) = self.numbers.get(value) {
            return number;
        }
        self.add(value.clone())
    }

    /// [`Self::intern`] of a value it owns, kept without a copy when new.
    pub fn intern_owned(&mut self, value: T) -> u32 {
        if let Some(&number) = self.numbers.get(&value) {
            return number;
        }
        self.add(value)
    }

    fn add(&mut self, value: T) -> u32 {
        let number = u32::try_from(self.values.len()).expect("fewer than 2^32 interned values");
        self.numbers.insert(value.clone(), number);
        self.values.push(value);
        number
    }

    pub fn number(&self, value: &T) -> Option<u32> {
        self.numbers.get(value).copied()
    }

    pub fn get(&self, number: u32) -> &T {
        &self.values[number as usize]
    }

    pub fn values(&self) -> &[T] {
        &self.values
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

// Saved as its values alone, in their numbers' order.
impl<T: Eq + Hash + Clone + Serialize> Serialize for Interner<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.values.serialize(serializer)
    }
}

impl<'de, T: Eq + Hash + Clone + Deserialize<'de>> Deserialize<'de> for Interner<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let values = Vec::<T>::deserialize(deserializer)?;
        let mut interner = Self::default();
        for value in values {
            if interner.numbers.contains_key(&value) {
                return Err(serde::de::Error::custom("an interned value repeats"));
            }
            interner.add(value);
        }
        Ok(interner)
    }
}

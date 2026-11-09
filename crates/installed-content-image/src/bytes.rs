use sha2::{Digest as _, Sha256};
use std::{
    any::Any,
    ops::{Deref, Range},
    sync::{Arc, OnceLock, Weak},
};

#[derive(Clone, Debug)]
struct Origin {
    key: [u8; 32],
    owner: Weak<dyn Any + Send + Sync>,
}

#[derive(Clone)]
pub struct Bytes {
    storage: Arc<Storage>,
    range: Range<usize>,
    origin: Option<Origin>,
}

// Opaque provenance never mutates the immutable payload across an unwind.
impl std::panic::UnwindSafe for Bytes {}
impl std::panic::RefUnwindSafe for Bytes {}

impl std::fmt::Debug for Bytes {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_ref().fmt(formatter)
    }
}

#[derive(Clone, Debug)]
struct Storage {
    bytes: Vec<u8>,
    digest: OnceLock<[u8; 32]>,
}

#[derive(Clone, Debug)]
pub struct WeakBytes {
    storage: Weak<Storage>,
    range: Range<usize>,
    origin: Option<Origin>,
}

impl WeakBytes {
    pub fn upgrade(&self) -> Option<Bytes> {
        self.storage.upgrade().map(|storage| Bytes {
            storage,
            range: self.range.clone(),
            origin: self.origin.clone(),
        })
    }
}

impl Bytes {
    pub fn as_slice(&self) -> &[u8] {
        self
    }

    pub fn make_mut(&mut self) -> &mut [u8] {
        if self.range != (0..self.storage.bytes.len()) {
            *self = self.as_ref().to_vec().into();
        }
        self.origin = None;
        let storage = Arc::make_mut(&mut self.storage);
        storage.digest.take();
        &mut storage.bytes
    }

    pub fn verified(bytes: Vec<u8>, digest: [u8; 32]) -> Self {
        let range = 0..bytes.len();
        Self {
            storage: Arc::new(Storage {
                bytes,
                digest: OnceLock::from(digest),
            }),
            range,
            origin: None,
        }
    }

    pub fn digest(&self) -> [u8; 32] {
        if self.range == (0..self.storage.bytes.len()) {
            *self
                .storage
                .digest
                .get_or_init(|| Sha256::digest(&self.storage.bytes).into())
        } else {
            Sha256::digest(self.as_ref()).into()
        }
    }

    pub fn prefix(&self, length: usize) -> Self {
        self.slice(0..length.min(self.len()))
    }

    pub fn slice(&self, range: Range<usize>) -> Self {
        assert!(range.start <= range.end && range.end <= self.len());
        Self {
            storage: Arc::clone(&self.storage),
            range: self.range.start + range.start..self.range.start + range.end,
            origin: self.origin.clone(),
        }
    }

    pub fn downgrade(&self) -> WeakBytes {
        WeakBytes {
            storage: Arc::downgrade(&self.storage),
            range: self.range.clone(),
            origin: self.origin.clone(),
        }
    }

    pub fn origin(&self) -> Option<[u8; 32]> {
        self.origin.as_ref().map(|origin| origin.key)
    }

    pub fn with_origin(mut self, key: [u8; 32], owner: Arc<dyn Any + Send + Sync>) -> Self {
        self.origin = Some(Origin {
            key,
            owner: Arc::downgrade(&owner),
        });
        self
    }

    pub fn owner<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.origin.as_ref()?.owner.upgrade()?.downcast().ok()
    }

    pub fn into_vec(self) -> Vec<u8> {
        if self.range == (0..self.storage.bytes.len()) {
            match Arc::try_unwrap(self.storage) {
                Ok(storage) => storage.bytes,
                Err(storage) => storage.bytes.clone(),
            }
        } else {
            self.as_ref().to_vec()
        }
    }
}

impl Default for Bytes {
    fn default() -> Self {
        Vec::new().into()
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(bytes: Vec<u8>) -> Self {
        let range = 0..bytes.len();
        Self {
            storage: Arc::new(Storage {
                bytes,
                digest: OnceLock::new(),
            }),
            range,
            origin: None,
        }
    }
}

impl From<&[u8]> for Bytes {
    fn from(bytes: &[u8]) -> Self {
        bytes.to_vec().into()
    }
}

impl From<&Bytes> for Bytes {
    fn from(bytes: &Bytes) -> Self {
        bytes.clone()
    }
}

impl From<&Vec<u8>> for Bytes {
    fn from(bytes: &Vec<u8>) -> Self {
        bytes.as_slice().into()
    }
}

impl<const N: usize> From<&[u8; N]> for Bytes {
    fn from(bytes: &[u8; N]) -> Self {
        bytes.as_slice().into()
    }
}

impl Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.storage.bytes[self.range.clone()]
    }
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl<T: AsRef<[u8]> + ?Sized> PartialEq<T> for Bytes {
    fn eq(&self, other: &T) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for Bytes {}

pub struct IntoIter {
    bytes: Bytes,
    range: Range<usize>,
}

impl Iterator for IntoIter {
    type Item = u8;
    fn next(&mut self) -> Option<u8> {
        self.range.next().map(|index| self.bytes[index])
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl DoubleEndedIterator for IntoIter {
    fn next_back(&mut self) -> Option<u8> {
        self.range.next_back().map(|index| self.bytes[index])
    }
}

impl ExactSizeIterator for IntoIter {}

impl IntoIterator for Bytes {
    type Item = u8;
    type IntoIter = IntoIter;
    fn into_iter(self) -> IntoIter {
        let range = 0..self.len();
        IntoIter { bytes: self, range }
    }
}

impl<'a> IntoIterator for &'a Bytes {
    type Item = &'a u8;
    type IntoIter = std::slice::Iter<'a, u8>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

use product_codec::Product;
use std::{io, sync::Arc};

pub struct Retained<T>(Arc<T>);
impl<T> Retained<T> {
    pub fn new(value: T) -> Self {
        Self(Arc::new(value))
    }
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn get_mut(&mut self) -> Option<&mut T> {
        Arc::get_mut(&mut self.0)
    }
}
impl<T> Clone for Retained<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> Default for Retained<Vec<T>> {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}
impl<T> From<Vec<T>> for Retained<Vec<T>> {
    fn from(values: Vec<T>) -> Self {
        Self::new(values)
    }
}
impl<T> FromIterator<T> for Retained<Vec<T>> {
    fn from_iter<I: IntoIterator<Item = T>>(items: I) -> Self {
        items.into_iter().collect::<Vec<_>>().into()
    }
}
impl<T> From<Arc<[T]>> for Retained<Vec<T>>
where
    T: Clone,
{
    fn from(values: Arc<[T]>) -> Self {
        Self::new(values.to_vec())
    }
}
impl<T> std::ops::Deref for Retained<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &*self.0
    }
}
impl<T: PartialEq> PartialEq for Retained<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || *self.0 == *other.0
    }
}
impl<T: Eq> Eq for Retained<T> {}
impl<T: std::fmt::Debug> std::fmt::Debug for Retained<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl Product for Retained<crate::identities::CountryKey> {
    const MIN_BYTES: usize = <crate::identities::CountryKey as Product>::MIN_BYTES;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.0.encode(out)
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> io::Result<Self> {
        let branch = u64::decode(input)?;
        let native = u32::decode(input)?;
        let canonical_slot = u32::decode(input)?;
        let observed_hour = i32::decode(input)?;
        let length = input.count64(1)?;
        let bytes = input.take(length)?;
        let text = std::str::from_utf8(bytes).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "country continuity is not UTF-8",
            )
        })?;
        Ok(Self::new(crate::identities::CountryKey {
            branch,
            native,
            canonical_slot,
            observed_hour,
            continuity: text.to_owned(),
        }))
    }
    fn schema(schema: &mut product_codec::Schema) {
        crate::identities::CountryKey::schema(schema);
    }
}

impl<T: Ord> PartialOrd for Retained<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T: Ord> Ord for Retained<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl<T: std::fmt::Display> std::fmt::Display for Retained<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<'a, T> IntoIterator for &'a Retained<Vec<T>> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'a, T> IntoIterator for &'a Retained<Arc<[T]>> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

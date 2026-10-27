use crate::credit::{Credit, MemoryBudget, display_products};
use product_codec::Product;
use std::{io, sync::Arc};

pub struct Retained<T>(Arc<Owned<T>>);
struct Owned<T> {
    value: T,
    _credit: Credit,
}
impl<T> Retained<T> {
    pub fn build(payload_bytes: usize, build: impl FnOnce() -> T) -> io::Result<Self> {
        Self::build_in(display_products(), payload_bytes, build)
    }
    pub fn build_in(
        owner: &MemoryBudget,
        payload_bytes: usize,
        build: impl FnOnce() -> T,
    ) -> io::Result<Self> {
        Self::try_build_in(owner, payload_bytes, || Ok(build()))
    }
    pub fn try_build_in(
        owner: &MemoryBudget,
        payload_bytes: usize,
        build: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<Self> {
        let bytes = payload_bytes
            .checked_add(Self::overhead())
            .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
        let credit = owner.admit_item(bytes)?;
        Ok(Self(Arc::new(Owned {
            value: build()?,
            _credit: credit,
        })))
    }
    pub fn with_credit(credit: Credit, build: impl FnOnce() -> T) -> Self {
        Self(Arc::new(Owned {
            value: build(),
            _credit: credit,
        }))
    }
    pub fn overhead() -> usize {
        size_of::<Owned<T>>() + 4 * size_of::<usize>() + align_of::<Owned<T>>() + align_of::<T>()
    }
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn get_mut(&mut self) -> Option<&mut T> {
        Arc::get_mut(&mut self.0).map(|owned| &mut owned.value)
    }
}
impl<T> Clone for Retained<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> Default for Retained<Vec<T>> {
    fn default() -> Self {
        Self::build(0, Vec::new).expect("retained empty fixture admission")
    }
}
impl<T> From<Vec<T>> for Retained<Vec<T>> {
    fn from(values: Vec<T>) -> Self {
        Self::build(values.capacity() * size_of::<T>(), || values)
            .expect("retained fixture admission")
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
        Self::build(values.len() * size_of::<T>(), || values.to_vec())
            .expect("retained fixture admission")
    }
}
impl<T> std::ops::Deref for Retained<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0.value
    }
}
impl<T: PartialEq> PartialEq for Retained<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || self.0.value == other.0.value
    }
}
impl<T: Eq> Eq for Retained<T> {}
impl<T: std::fmt::Debug> std::fmt::Debug for Retained<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.value.fmt(f)
    }
}
impl Product for Retained<crate::identities::CountryKey> {
    const MIN_BYTES: usize = <crate::identities::CountryKey as Product>::MIN_BYTES;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.0.value.encode(out)
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
        Self::build(length, || crate::identities::CountryKey {
            branch,
            native,
            canonical_slot,
            observed_hour,
            continuity: text.to_owned(),
        })
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
        self.0.value.cmp(&other.0.value)
    }
}

impl<T: std::fmt::Display> std::fmt::Display for Retained<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.value.fmt(f)
    }
}

impl<'a, T> IntoIterator for &'a Retained<Vec<T>> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.value.iter()
    }
}

impl<'a, T> IntoIterator for &'a Retained<Arc<[T]>> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.value.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_unadmittable_payload_never_runs_its_allocation_closure() {
        let built = std::cell::Cell::new(false);
        let result = Retained::build(usize::MAX, || {
            built.set(true);
            Vec::<u8>::new()
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::OutOfMemory);
        assert!(!built.get());
    }
}

#[cfg(test)]
mod tiny_owner_tests {
    use super::*;
    #[test]
    fn retained_display_payload_evicts_before_retry_and_pinned_pressure_is_terminal() {
        let bytes = 64;
        let owner = MemoryBudget::new(Retained::<Vec<u8>>::overhead() + bytes);
        let mut cached = Some(Retained::build_in(&owner, bytes, || vec![1; bytes]).unwrap());
        let allocated = std::cell::Cell::new(false);
        let error = Retained::build_in(&owner, bytes, || {
            allocated.set(true);
            vec![2; bytes]
        })
        .unwrap_err();
        assert!(!allocated.get());
        let error = owner.reclaim_failure(error, || {
            cached.take();
        });
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        let pinned = Retained::build_in(&owner, bytes, || vec![2; bytes]).unwrap();
        let error = Retained::build_in(&owner, bytes, || {
            allocated.set(true);
            vec![3; bytes]
        })
        .unwrap_err();
        let error = owner.reclaim_failure(error, || {});
        assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
        assert!(!allocated.get());
        assert!(owner.used() <= owner.limit());
        drop(pinned);
        assert_eq!(owner.used(), 0);
        assert_eq!(
            owner
                .admit_wait(owner.limit() + 1, || false)
                .unwrap_err()
                .kind(),
            io::ErrorKind::OutOfMemory
        );
    }
}

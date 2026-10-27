use crate::{
    credit::{Credit, MemoryBudget, display_products},
    retained::Retained,
};
use product_codec::{Decoder, Product, Schema};
use std::{
    io,
    ops::{Deref, Index, IndexMut},
    sync::{Arc, Mutex, OnceLock},
};

pub const PAGE_ITEMS: usize = 4096;

#[derive(Debug)]
struct Root<T> {
    len: usize,
    pages: Vec<Retained<Vec<T>>>,
    flat: OnceLock<Retained<Vec<T>>>,
    flattening: Mutex<()>,
    owner: MemoryBudget,
    _credit: Credit,
}
#[derive(Debug)]
pub struct Paged<T>(Option<Arc<Root<T>>>);
impl<T> Clone for Paged<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> Default for Paged<T> {
    fn default() -> Self {
        Self(None)
    }
}
fn overflow() -> io::Error {
    io::Error::new(
        io::ErrorKind::OutOfMemory,
        "paged display allocation size overflow",
    )
}
impl<T> Paged<T> {
    fn root(&self) -> &Root<T> {
        self.0.as_deref().expect("nonempty paged root")
    }
    fn directory(owner: &MemoryBudget, len: usize) -> io::Result<Credit> {
        let bytes = Self::decode_storage_bytes(len)?;
        if bytes > owner.limit() {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "paged display item exceeds its owner; common paging is required",
            ));
        }
        owner.admit_item(
            size_of::<Root<T>>()
                + 4 * size_of::<usize>()
                + align_of::<Root<T>>()
                + align_of::<Retained<Vec<T>>>()
                + len
                    .div_ceil(PAGE_ITEMS)
                    .checked_mul(size_of::<Retained<Vec<T>>>())
                    .ok_or_else(overflow)?,
        )
    }
    fn install(
        owner: &MemoryBudget,
        len: usize,
        pages: Vec<Retained<Vec<T>>>,
        credit: Credit,
    ) -> Self {
        Self(Some(Arc::new(Root {
            len,
            pages,
            flat: OnceLock::new(),
            flattening: Mutex::new(()),
            owner: owner.clone(),
            _credit: credit,
        })))
    }
    pub fn try_from_iter_in(
        owner: &MemoryBudget,
        len: usize,
        values: impl IntoIterator<Item = T>,
    ) -> io::Result<Self> {
        let mut values = values.into_iter();
        if len == 0 {
            if values.next().is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "paged display iterator length mismatch",
                ));
            }
            return Ok(Self::default());
        }
        let credit = Self::directory(owner, len)?;
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(len.div_ceil(PAGE_ITEMS))
            .map_err(|_| overflow())?;
        let mut left = len;
        while left != 0 {
            let count = left.min(PAGE_ITEMS);
            pages.push(Retained::try_build_in(
                owner,
                count.checked_mul(size_of::<T>()).ok_or_else(overflow)?,
                || {
                    let mut page = Vec::new();
                    page.try_reserve_exact(count).map_err(|_| overflow())?;
                    page.extend(values.by_ref().take(count));
                    if page.len() != count {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "paged display iterator length mismatch",
                        ));
                    }
                    Ok(page)
                },
            )?);
            left -= count;
        }
        if values.next().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "paged display iterator length mismatch",
            ));
        }
        Ok(Self::install(owner, len, pages, credit))
    }
    pub fn try_from_vec_in(owner: &MemoryBudget, values: Vec<T>) -> io::Result<Self> {
        Self::try_from_iter_in(owner, values.len(), values)
    }
    pub fn decode_storage_bytes(len: usize) -> io::Result<usize> {
        len.checked_mul(size_of::<T>())
            .and_then(|n| {
                n.checked_add(
                    len.div_ceil(PAGE_ITEMS).checked_mul(
                        size_of::<Retained<Vec<T>>>() + Retained::<Vec<T>>::overhead(),
                    )?,
                )
            })
            .and_then(|n| {
                n.checked_add(
                    size_of::<Root<T>>()
                        + 4 * size_of::<usize>()
                        + align_of::<Root<T>>()
                        + align_of::<Retained<Vec<T>>>(),
                )
            })
            .ok_or_else(overflow)
    }
    pub fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |root| root.len)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn get(&self, index: usize) -> Option<&T> {
        self.0
            .as_ref()?
            .pages
            .get(index / PAGE_ITEMS)?
            .get(index % PAGE_ITEMS)
    }
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        self.0
            .iter()
            .flat_map(|root| root.pages.iter())
            .flat_map(|page| page.iter())
    }
    pub fn pages(&self) -> impl ExactSizeIterator<Item = &[T]> {
        let pages: &[Retained<Vec<T>>] = self.0.as_ref().map_or(&[], |root| &root.pages);
        pages.iter().map(|page| page.as_slice())
    }
    pub fn page(&self, index: usize) -> &[T] {
        &self.root().pages[index]
    }
    pub fn ptr_eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
    pub fn eq_by(&self, other: &Self, mut same: impl FnMut(&T, &T) -> bool) -> bool {
        self.ptr_eq(other)
            || (self.len() == other.len()
                && self
                    .0
                    .iter()
                    .flat_map(|root| &root.pages)
                    .zip(other.0.iter().flat_map(|root| &root.pages))
                    .all(|(a, b)| a.ptr_eq(b) || a.iter().zip(b.iter()).all(|(a, b)| same(a, b))))
    }
    pub fn owned_bytes(&self) -> usize {
        self.0.as_ref().map_or(0, |root| {
            size_of::<Root<T>>()
                + 2 * size_of::<usize>()
                + root.pages.capacity() * size_of::<Retained<Vec<T>>>()
                + root
                    .pages
                    .iter()
                    .map(|p| Retained::<Vec<T>>::overhead() + p.capacity() * size_of::<T>())
                    .sum::<usize>()
                + root.flat.get().map_or(0, |flat| {
                    Retained::<Vec<T>>::overhead() + flat.capacity() * size_of::<T>()
                })
        })
    }
}
impl<T: Clone> Paged<T> {
    pub fn try_as_slice(&self) -> io::Result<&[T]> {
        let Some(root) = &self.0 else {
            return Ok(&[]);
        };
        if root.flat.get().is_none() {
            let _lock = root.flattening.lock().unwrap_or_else(|e| e.into_inner());
            if root.flat.get().is_none() {
                let flat = Retained::try_build_in(
                    &root.owner,
                    root.len.checked_mul(size_of::<T>()).ok_or_else(overflow)?,
                    || {
                        let mut values = Vec::new();
                        values.try_reserve_exact(root.len).map_err(|_| overflow())?;
                        values.extend(self.iter().cloned());
                        Ok(values)
                    },
                )?;
                let _ = root.flat.set(flat);
            }
        }
        Ok(root.flat.get().unwrap())
    }
    pub fn as_slice(&self) -> &[T] {
        self.try_as_slice()
            .expect("paged diagnostic flatten admission")
    }
    pub fn to_vec(&self) -> Vec<T> {
        self.iter().cloned().collect()
    }
    pub fn extend_from_within(&mut self, range: std::ops::RangeFull) {
        let mut values = self.to_vec();
        values.extend_from_within(range);
        *self = values.into();
    }
}
impl<T: Clone + PartialEq> Paged<T> {
    pub fn try_edited(&self, edits: impl IntoIterator<Item = (usize, T)>) -> io::Result<Self> {
        self.try_edited_results_by(edits.into_iter().map(Ok), PartialEq::eq)
    }
    pub fn try_edited_results_by(
        &self,
        edits: impl IntoIterator<Item = io::Result<(usize, T)>>,
        mut same: impl FnMut(&T, &T) -> bool,
    ) -> io::Result<Self> {
        let mut directory = None;
        for edit in edits {
            let (index, value) = edit?;
            assert!(index < self.len());
            let current = directory.as_ref().map_or_else(
                || self.get(index).unwrap(),
                |(pages, _): &(Vec<Retained<Vec<T>>>, Credit)| {
                    &pages[index / PAGE_ITEMS][index % PAGE_ITEMS]
                },
            );
            if same(current, &value) {
                continue;
            }
            if directory.is_none() {
                let credit = Self::directory(&self.root().owner, self.len())?;
                let mut pages = Vec::new();
                pages
                    .try_reserve_exact(self.root().pages.len())
                    .map_err(|_| overflow())?;
                pages.extend(self.root().pages.iter().cloned());
                directory = Some((pages, credit));
            }
            let (pages, _) = directory.as_mut().unwrap();
            let page = &mut pages[index / PAGE_ITEMS];
            if page.get_mut().is_none() {
                *page = Retained::build_in(
                    &self.root().owner,
                    page.len()
                        .checked_mul(size_of::<T>())
                        .ok_or_else(overflow)?,
                    || page.to_vec(),
                )?;
            }
            page.get_mut().unwrap()[index % PAGE_ITEMS] = value;
        }
        Ok(match directory {
            None => self.clone(),
            Some((pages, credit)) => Self::install(&self.root().owner, self.len(), pages, credit),
        })
    }
    pub fn edited(&self, edits: impl IntoIterator<Item = (usize, T)>) -> Self {
        self.try_edited(edits)
            .expect("paged diagnostic edit admission")
    }
    pub fn changed_pages(&self, previous: &Self) -> Vec<usize> {
        self.changed_page_indices(previous).collect()
    }
    pub fn changed_page_indices<'a>(
        &'a self,
        previous: &'a Self,
    ) -> impl Iterator<Item = usize> + 'a {
        self.0
            .iter()
            .filter(move |_| !self.ptr_eq(previous))
            .flat_map(move |root| {
                root.pages
                    .iter()
                    .enumerate()
                    .filter_map(move |(index, page)| {
                        (!previous
                            .0
                            .as_ref()
                            .and_then(|old| old.pages.get(index))
                            .is_some_and(|old| page.ptr_eq(old) || **page == **old))
                        .then_some(index)
                    })
            })
    }
}
impl<T> From<Vec<T>> for Paged<T> {
    fn from(values: Vec<T>) -> Self {
        Self::try_from_vec_in(display_products(), values)
            .expect("paged fixture allocation admission")
    }
}
impl<T: Clone> FromIterator<T> for Paged<T> {
    fn from_iter<I: IntoIterator<Item = T>>(values: I) -> Self {
        values.into_iter().collect::<Vec<_>>().into()
    }
}
impl<T> Index<usize> for Paged<T> {
    type Output = T;
    fn index(&self, index: usize) -> &T {
        self.get(index).expect("paged value index")
    }
}
impl<T: Clone> IndexMut<usize> for Paged<T> {
    fn index_mut(&mut self, index: usize) -> &mut T {
        if Arc::get_mut(self.0.as_mut().expect("paged index")).is_none() {
            let root = self.root();
            let credit = Self::directory(&root.owner, root.len)
                .expect("paged diagnostic directory admission");
            *self = Self::install(&root.owner, root.len, root.pages.clone(), credit);
        }
        let root = Arc::get_mut(self.0.as_mut().unwrap()).unwrap();
        root.flat.take();
        let page = &mut root.pages[index / PAGE_ITEMS];
        if page.get_mut().is_none() {
            *page = Retained::build_in(&root.owner, page.len() * size_of::<T>(), || page.to_vec())
                .expect("paged diagnostic page admission");
        }
        &mut page.get_mut().unwrap()[index % PAGE_ITEMS]
    }
}
impl<T: Clone> Deref for Paged<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}
impl<T: PartialEq> PartialEq for Paged<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other)
            || (self.len() == other.len()
                && self
                    .0
                    .iter()
                    .flat_map(|root| &root.pages)
                    .zip(other.0.iter().flat_map(|root| &root.pages))
                    .all(|(a, b)| a.ptr_eq(b) || **a == **b))
    }
}
impl<T: Eq> Eq for Paged<T> {}
impl<T: PartialEq> PartialEq<Vec<T>> for Paged<T> {
    fn eq(&self, other: &Vec<T>) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}
impl<T: PartialEq> PartialEq<Paged<T>> for Vec<T> {
    fn eq(&self, other: &Paged<T>) -> bool {
        other == self
    }
}
impl<T: Product + Clone> Product for Paged<T> {
    const MIN_BYTES: usize = 8;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        product_codec::put_count64(out, self.len())?;
        for page in self.pages() {
            T::encode_slice(page, out)?;
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let len = input.count64(T::MIN_BYTES)?;
        if len == 0 {
            return Ok(Self::default());
        }
        let owner = display_products();
        let credit = Self::directory(owner, len)?;
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(len.div_ceil(PAGE_ITEMS))
            .map_err(|_| overflow())?;
        let mut left = len;
        while left != 0 {
            let count = left.min(PAGE_ITEMS);
            pages.push(Retained::try_build_in(
                owner,
                count.checked_mul(size_of::<T>()).ok_or_else(overflow)?,
                || T::decode_many(input, count),
            )?);
            left -= count;
        }
        Ok(Self::install(owner, len, pages, credit))
    }
    fn schema(schema: &mut Schema) {
        Vec::<T>::schema(schema);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparse_equality_compares_only_distinct_pages() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        #[derive(Clone, Debug)]
        struct Counted(u32, Arc<AtomicUsize>);
        impl PartialEq for Counted {
            fn eq(&self, other: &Self) -> bool {
                self.1.fetch_add(1, Ordering::Relaxed);
                self.0 == other.0
            }
        }
        let comparisons = Arc::new(AtomicUsize::new(0));
        let owner = MemoryBudget::new(1 << 20);
        let base = Paged::try_from_iter_in(
            &owner,
            PAGE_ITEMS * 3,
            (0..PAGE_ITEMS * 3).map(|value| Counted(value as u32, comparisons.clone())),
        )
        .unwrap();
        let changed = base
            .try_edited([(PAGE_ITEMS * 2, Counted(u32::MAX, comparisons.clone()))])
            .unwrap();
        comparisons.store(0, Ordering::Relaxed);
        assert_ne!(base, changed);
        assert_eq!(comparisons.load(Ordering::Relaxed), 1);
        comparisons.store(0, Ordering::Relaxed);
        assert_eq!(base, base.clone());
        assert_eq!(comparisons.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn page_and_root_credit_survives_every_shared_output_owner() {
        let owner = MemoryBudget::new(1 << 20);
        let first =
            Paged::try_from_iter_in(&owner, PAGE_ITEMS * 2, (0..PAGE_ITEMS * 2).map(|n| n as u8))
                .unwrap();
        let first_bytes = owner.used();
        let copy = first.clone();
        assert_eq!(owner.used(), first_bytes);
        let changed = first.try_edited([(PAGE_ITEMS + 7, 42)]).unwrap();
        assert!(owner.used() > first_bytes);
        let page = changed.root().pages[0].clone();
        changed.try_as_slice().unwrap();
        drop(first);
        drop(copy);
        drop(changed);
        assert!(owner.used() > 0, "a page outlives its directory");
        drop(page);
        assert_eq!(owner.used(), 0);
        let small = MemoryBudget::new(8);
        let reads = std::cell::Cell::new(0);
        let error =
            Paged::try_from_iter_in(&small, 16, (0..16).inspect(|_| reads.set(reads.get() + 1)))
                .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
        assert_eq!(reads.get(), 0);
    }
    #[test]
    fn edits_share_unchanged_pages_and_keep_the_vector_wire() {
        let values: Vec<u8> = (0..PAGE_ITEMS * 3 + 7).map(|index| index as u8).collect();
        let base: Paged<_> = values.clone().into();
        let changed = base.edited([(PAGE_ITEMS + 3, 99), (PAGE_ITEMS + 4, 100)]);
        assert!(base.root().pages[0].ptr_eq(&changed.root().pages[0]));
        assert!(base.root().pages[2].ptr_eq(&changed.root().pages[2]));
        assert_eq!(changed.changed_pages(&base), [1]);
        assert!(changed.root().flat.get().is_none());
        let mut expected = values;
        expected[PAGE_ITEMS + 3] = 99;
        expected[PAGE_ITEMS + 4] = 100;
        assert_eq!(
            product_codec::encode(&changed).unwrap(),
            product_codec::encode(&expected).unwrap()
        );
        assert_eq!(
            product_codec::schema_digest::<Paged<u8>>(),
            product_codec::schema_digest::<Vec<u8>>()
        );
        let restored: Paged<u8> =
            product_codec::decode(&product_codec::encode(&expected).unwrap()).unwrap();
        assert_eq!(restored, expected);
        assert_eq!(base[PAGE_ITEMS + 3], (PAGE_ITEMS + 3) as u8);
    }
}

macro_rules! slice_index {
    ($($ty:ty),*) => { $(impl<T: Clone> Index<$ty> for Paged<T> {
        type Output = [T];
        fn index(&self, range: $ty) -> &[T] { &self.as_slice()[range] }
    })* };
}
slice_index!(
    std::ops::Range<usize>,
    std::ops::RangeFrom<usize>,
    std::ops::RangeTo<usize>,
    std::ops::RangeFull,
    std::ops::RangeInclusive<usize>,
    std::ops::RangeToInclusive<usize>
);

impl<T: PartialEq, const N: usize> PartialEq<[T; N]> for Paged<T> {
    fn eq(&self, other: &[T; N]) -> bool {
        self.len() == N && self.iter().eq(other.iter())
    }
}
impl<T: PartialEq, const N: usize> PartialEq<Paged<T>> for [T; N] {
    fn eq(&self, other: &Paged<T>) -> bool {
        other == self
    }
}

impl<T: PartialEq> PartialEq<Paged<T>> for &[T] {
    fn eq(&self, other: &Paged<T>) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}
impl<T: PartialEq> PartialEq<&[T]> for Paged<T> {
    fn eq(&self, other: &&[T]) -> bool {
        other == self
    }
}

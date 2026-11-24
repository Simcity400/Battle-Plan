use crate::retained::Retained;
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
    fn install(len: usize, pages: Vec<Retained<Vec<T>>>) -> Self {
        Self(Some(Arc::new(Root {
            len,
            pages,
            flat: OnceLock::new(),
            flattening: Mutex::new(()),
        })))
    }
    pub fn try_from_iter(len: usize, values: impl IntoIterator<Item = T>) -> io::Result<Self> {
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
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(len.div_ceil(PAGE_ITEMS))
            .map_err(|_| overflow())?;
        let mut left = len;
        while left != 0 {
            let count = left.min(PAGE_ITEMS);
            let mut page = Vec::new();
            page.try_reserve_exact(count).map_err(|_| overflow())?;
            page.extend(values.by_ref().take(count));
            if page.len() != count {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "paged display iterator length mismatch",
                ));
            }
            pages.push(Retained::new(page));
            left -= count;
        }
        if values.next().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "paged display iterator length mismatch",
            ));
        }
        Ok(Self::install(len, pages))
    }
    pub fn try_from_vec(values: Vec<T>) -> io::Result<Self> {
        Self::try_from_iter(values.len(), values)
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
    pub fn changed_pages_by(
        &self,
        previous: &Self,
        same: impl FnMut(&T, &T) -> bool,
    ) -> Vec<usize> {
        self.changed_page_indices_by(previous, same).collect()
    }
    pub fn changed_page_indices_by<'a>(
        &'a self,
        previous: &'a Self,
        mut same: impl FnMut(&T, &T) -> bool + 'a,
    ) -> impl Iterator<Item = usize> + 'a {
        let pages: &[Retained<Vec<T>>] = if self.ptr_eq(previous) {
            &[]
        } else {
            self.0.as_deref().map_or(&[], |root| root.pages.as_slice())
        };
        pages.iter().enumerate().filter_map(move |(index, page)| {
            (!previous
                .0
                .as_ref()
                .and_then(|old| old.pages.get(index))
                .is_some_and(|old| {
                    page.ptr_eq(old)
                        || (page.len() == old.len()
                            && page.iter().zip(old.iter()).all(|(a, b)| same(a, b)))
                }))
            .then_some(index)
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
                let mut values = Vec::new();
                values.try_reserve_exact(root.len).map_err(|_| overflow())?;
                values.extend(self.iter().cloned());
                let _ = root.flat.set(Retained::new(values));
            }
        }
        Ok(root.flat.get().unwrap())
    }
    pub fn as_slice(&self) -> &[T] {
        self.try_as_slice().expect("paged flatten allocation")
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
                |pages: &Vec<Retained<Vec<T>>>| &pages[index / PAGE_ITEMS][index % PAGE_ITEMS],
            );
            if same(current, &value) {
                continue;
            }
            if directory.is_none() {
                let mut pages = Vec::new();
                pages
                    .try_reserve_exact(self.root().pages.len())
                    .map_err(|_| overflow())?;
                pages.extend(self.root().pages.iter().cloned());
                directory = Some(pages);
            }
            let pages = directory.as_mut().unwrap();
            let page = &mut pages[index / PAGE_ITEMS];
            if page.get_mut().is_none() {
                *page = Retained::new(page.to_vec());
            }
            page.get_mut().unwrap()[index % PAGE_ITEMS] = value;
        }
        Ok(match directory {
            None => self.clone(),
            Some(pages) => Self::install(self.len(), pages),
        })
    }
    pub fn edited(&self, edits: impl IntoIterator<Item = (usize, T)>) -> Self {
        self.try_edited(edits).expect("paged edit allocation")
    }
    pub fn changed_pages(&self, previous: &Self) -> Vec<usize> {
        self.changed_page_indices(previous).collect()
    }
    pub fn changed_page_indices<'a>(
        &'a self,
        previous: &'a Self,
    ) -> impl Iterator<Item = usize> + 'a {
        self.changed_page_indices_by(previous, PartialEq::eq)
    }
}
impl<T> From<Vec<T>> for Paged<T> {
    fn from(values: Vec<T>) -> Self {
        Self::try_from_vec(values).expect("paged allocation")
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
            *self = Self::install(root.len, root.pages.clone());
        }
        let root = Arc::get_mut(self.0.as_mut().unwrap()).unwrap();
        root.flat.take();
        let page = &mut root.pages[index / PAGE_ITEMS];
        if page.get_mut().is_none() {
            *page = Retained::new(page.to_vec());
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
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(len.div_ceil(PAGE_ITEMS))
            .map_err(|_| overflow())?;
        let mut left = len;
        while left != 0 {
            let count = left.min(PAGE_ITEMS);
            pages.push(Retained::new(T::decode_many(input, count)?));
            left -= count;
        }
        Ok(Self::install(len, pages))
    }
    fn schema(schema: &mut Schema) {
        Vec::<T>::schema(schema);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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

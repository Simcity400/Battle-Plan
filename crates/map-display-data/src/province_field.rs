use crate::{
    copy_index::CopyIndex,
    credit::{Credit, display_products},
};
use product_codec::Product;
use std::{
    io,
    sync::{Arc, OnceLock},
};

const PAGE: usize = 64;
const ZERO: [u32; PAGE] = [0; PAGE];
#[derive(Clone)]
pub struct ProvinceField(pub Arc<Storage>);
#[derive(Clone)]
pub enum Storage {
    Dense(Arc<Vec<u32>>),
    Paged(Arc<Pages>),
}
pub struct Pages {
    len: usize,
    pages: CopyIndex<usize, [u32; PAGE]>,
    dense: OnceLock<Vec<u32>>,
    _credit: Credit,
}
impl ProvinceField {
    pub fn filled(len: usize) -> io::Result<Self> {
        Self::paged(len, CopyIndex::default())
    }
    fn paged(len: usize, pages: CopyIndex<usize, [u32; PAGE]>) -> io::Result<Self> {
        let bytes = len
            .checked_mul(size_of::<u32>())
            .and_then(|n| {
                n.checked_add(
                    size_of::<Pages>()
                        + size_of::<Storage>()
                        + 4 * size_of::<usize>()
                        + align_of::<Pages>(),
                )
            })
            .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
        let credit = display_products().admit_item(bytes)?;
        Ok(Self(Arc::new(Storage::Paged(Arc::new(Pages {
            len,
            pages,
            dense: OnceLock::new(),
            _credit: credit,
        })))))
    }
    pub fn patched(&self, changes: impl IntoIterator<Item = (usize, u32)>) -> io::Result<Self> {
        let mut pages = match &*self.0 {
            Storage::Paged(root) => root.pages.clone(),
            Storage::Dense(values) => {
                let mut pages = CopyIndex::default();
                for (at, values) in values.chunks(PAGE).enumerate() {
                    let mut page = ZERO;
                    page[..values.len()].copy_from_slice(values);
                    if page != ZERO {
                        pages = pages.inserted(at, page)?;
                    }
                }
                pages
            }
        };
        for (id, value) in changes {
            if id >= self.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "province plane edit is out of bounds",
                ));
            }
            let mut page = pages.get(&(id / PAGE)).copied().unwrap_or(ZERO);
            if page[id % PAGE] == value {
                continue;
            }
            page[id % PAGE] = value;
            pages = if page == ZERO {
                pages.removed(&(id / PAGE))?
            } else {
                pages.inserted(id / PAGE, page)?
            };
        }
        if let Storage::Paged(old) = &*self.0 {
            if pages.ptr_eq(&old.pages) {
                return Ok(self.clone());
            }
        }
        Self::paged(self.len(), pages)
    }
    pub fn patched_sorted(
        &self,
        changes: impl IntoIterator<Item = (usize, u32)>,
    ) -> io::Result<Self> {
        let Storage::Paged(root) = &*self.0 else {
            return self.patched(std::iter::empty())?.patched_sorted(changes);
        };
        let mut pages = root.pages.clone();
        let mut pending: Option<(usize, [u32; PAGE])> = None;
        let mut previous = None;
        for (id, value) in changes {
            if id >= root.len || previous.is_some_and(|previous| id < previous) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "province edits are unordered or out of bounds",
                ));
            }
            previous = Some(id);
            let at = id / PAGE;
            if pending.as_ref().is_some_and(|(held, _)| *held != at) {
                let (held, page) = pending.take().expect("pending province page");
                pages = if page == ZERO {
                    pages.removed(&held)?
                } else {
                    pages.inserted(held, page)?
                };
            }
            let (_, page) =
                pending.get_or_insert_with(|| (at, root.pages.get(&at).copied().unwrap_or(ZERO)));
            page[id % PAGE] = value;
        }
        if let Some((at, page)) = pending {
            pages = if page == ZERO {
                pages.removed(&at)?
            } else {
                pages.inserted(at, page)?
            };
        }
        if pages.ptr_eq(&root.pages) {
            return Ok(self.clone());
        }
        Self::paged(root.len, pages)
    }
    pub fn resized(&self, len: usize) -> io::Result<Self> {
        if len == self.len() {
            return Ok(self.clone());
        }
        let mut pages = match &*self.0 {
            Storage::Paged(root) => root.pages.clone(),
            Storage::Dense(_) => return self.patched(std::iter::empty())?.resized(len),
        };
        if len < self.len() {
            let removed = pages.clone();
            for (&at, _) in removed.iter_from(&len.div_ceil(PAGE)) {
                pages = pages.removed(&at)?;
            }
            if len % PAGE != 0
                && let Some(mut page) = pages.get(&(len / PAGE)).copied()
            {
                page[len % PAGE..].fill(0);
                pages = if page == ZERO {
                    pages.removed(&(len / PAGE))?
                } else {
                    pages.inserted(len / PAGE, page)?
                };
            }
        }
        Self::paged(len, pages)
    }
    pub fn len(&self) -> usize {
        match &*self.0 {
            Storage::Dense(v) => v.len(),
            Storage::Paged(p) => p.len,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn get(&self, id: usize) -> Option<&u32> {
        if id >= self.len() {
            return None;
        }
        Some(match &*self.0 {
            Storage::Dense(v) => &v[id],
            Storage::Paged(p) => &p.pages.get(&(id / PAGE)).unwrap_or(&ZERO)[id % PAGE],
        })
    }
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &u32> + DoubleEndedIterator {
        (0..self.len()).map(|id| self.get(id).unwrap())
    }
    pub fn as_slice(&self) -> &[u32] {
        self.dense().as_slice()
    }
    fn dense(&self) -> &Vec<u32> {
        match &*self.0 {
            Storage::Dense(v) => v,
            Storage::Paged(p) => p.dense.get_or_init(|| {
                let mut values = vec![0; p.len];
                for (&at, page) in p.pages.iter() {
                    let start = at * PAGE;
                    let end = (start + PAGE).min(p.len);
                    values[start..end].copy_from_slice(&page[..end - start]);
                }
                values
            }),
        }
    }
}
impl Default for ProvinceField {
    fn default() -> Self {
        Vec::new().into()
    }
}
impl std::ops::Deref for ProvinceField {
    type Target = Vec<u32>;
    fn deref(&self) -> &Vec<u32> {
        self.dense()
    }
}
impl std::ops::DerefMut for ProvinceField {
    fn deref_mut(&mut self) -> &mut Vec<u32> {
        if matches!(&*self.0, Storage::Paged(_)) {
            self.0 = Arc::new(Storage::Dense(Arc::new(self.dense().clone())));
        }
        let Storage::Dense(values) = Arc::make_mut(&mut self.0) else {
            unreachable!()
        };
        Arc::make_mut(values)
    }
}
impl std::ops::Index<usize> for ProvinceField {
    type Output = u32;
    fn index(&self, id: usize) -> &u32 {
        self.get(id).expect("province within plane")
    }
}
impl std::ops::IndexMut<usize> for ProvinceField {
    fn index_mut(&mut self, id: usize) -> &mut u32 {
        &mut std::ops::DerefMut::deref_mut(self)[id]
    }
}
impl From<Vec<u32>> for ProvinceField {
    fn from(values: Vec<u32>) -> Self {
        Self(Arc::new(Storage::Dense(Arc::new(values))))
    }
}
impl FromIterator<u32> for ProvinceField {
    fn from_iter<T: IntoIterator<Item = u32>>(values: T) -> Self {
        Vec::from_iter(values).into()
    }
}
pub struct Iter<'a> {
    plane: &'a ProvinceField,
    range: std::ops::Range<usize>,
}
impl<'a> Iterator for Iter<'a> {
    type Item = &'a u32;
    fn next(&mut self) -> Option<Self::Item> {
        self.range.next().map(|id| self.plane.get(id).unwrap())
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}
impl ExactSizeIterator for Iter<'_> {}
impl DoubleEndedIterator for Iter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.range.next_back().map(|id| self.plane.get(id).unwrap())
    }
}
impl<'a> IntoIterator for &'a ProvinceField {
    type Item = &'a u32;
    type IntoIter = Iter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        Iter {
            plane: self,
            range: 0..self.len(),
        }
    }
}
impl PartialEq for ProvinceField {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.len() == other.len() && self.iter().eq(other.iter())
    }
}
impl Eq for ProvinceField {}
impl PartialEq<Vec<u32>> for ProvinceField {
    fn eq(&self, other: &Vec<u32>) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}
impl<const N: usize> PartialEq<[u32; N]> for ProvinceField {
    fn eq(&self, other: &[u32; N]) -> bool {
        self.len() == N && self.iter().eq(other.iter())
    }
}
impl std::fmt::Debug for ProvinceField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}
impl Product for ProvinceField {
    const MIN_BYTES: usize = 8;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        (self.len() as u64).encode(out)?;
        for value in self.iter() {
            value.encode(out)?;
        }
        Ok(())
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> io::Result<Self> {
        let len = input.count64(size_of::<u32>())?;
        let mut pages = CopyIndex::default();
        for at in 0..len.div_ceil(PAGE) {
            let mut page = ZERO;
            for value in page.iter_mut().take((len - at * PAGE).min(PAGE)) {
                *value = u32::decode(input)?;
            }
            if page != ZERO {
                pages = pages.inserted(at, page)?;
            }
        }
        Self::paged(len, pages)
    }
    fn schema(schema: &mut product_codec::Schema) {
        schema.field("ProvinceField/u32");
        Vec::<u32>::schema(schema);
    }
}

macro_rules! slice_index {
    ($($ty:ty),* $(,)?) => {$ (
        impl std::ops::Index<$ty> for ProvinceField {
            type Output = [u32];
            fn index(&self, index: $ty) -> &[u32] { &self.as_slice()[index] }
        }
    )*};
}
slice_index!(
    std::ops::Range<usize>,
    std::ops::RangeTo<usize>,
    std::ops::RangeFrom<usize>,
    std::ops::RangeFull,
    std::ops::RangeInclusive<usize>,
    std::ops::RangeToInclusive<usize>
);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sorted_page_batches_keep_duplicates_last_and_the_exact_dense_wire() {
        let base = ProvinceField::filled(1501)
            .unwrap()
            .patched_sorted([(1, 9), (64, 4), (1500, 7)])
            .unwrap();
        let edits = [(0, 1), (1, 2), (1, 3), (63, 4), (64, 0), (65, 5), (1500, 8)];
        let next = base.patched_sorted(edits).unwrap();
        let mut oracle = vec![0; 1501];
        oracle[1] = 9;
        oracle[64] = 4;
        oracle[1500] = 7;
        for (key, value) in edits {
            oracle[key] = value;
        }
        assert_eq!(next, oracle);
        assert_eq!(
            product_codec::encode(&next).unwrap(),
            product_codec::encode(&oracle).unwrap()
        );
        assert!(Arc::ptr_eq(&next.0, &next.patched_sorted(edits).unwrap().0));
        assert!(base.patched_sorted([(2, 1), (1, 2)]).is_err());
        assert!(base.patched_sorted([(1501, 1)]).is_err());
        assert_eq!(base[1], 9);
        assert!(matches!(&*base.0,Storage::Paged(root) if root.dense.get().is_none()));
    }
    #[test]
    fn page_edits_resize_and_codec_match_an_independent_dense_plane() {
        let empty = ProvinceField::filled(5003).unwrap();
        let first = empty.patched([(1, 7), (130, 9), (5002, 11)]).unwrap();
        let second = first.patched([(1, 8)]).unwrap();
        let (Storage::Paged(a), Storage::Paged(b)) = (&*first.0, &*second.0) else {
            panic!("paged roots")
        };
        assert!(a.dense.get().is_none() && b.dense.get().is_none());
        let mut changed = 0;
        for (&at, value) in a.pages.iter() {
            if b.pages.get(&at) != Some(value) {
                changed += 1;
            }
        }
        assert_eq!(changed, 1);
        assert_eq!(first[1], 7);
        assert_eq!(second[1], 8);
        let shrunk = second.resized(131).unwrap().resized(5003).unwrap();
        assert_eq!(shrunk[130], 9);
        assert_eq!(shrunk[5002], 0);
        let mut oracle = vec![0; 5003];
        oracle[1] = 8;
        oracle[130] = 9;
        assert_eq!(shrunk, oracle);
        let bytes = product_codec::encode(&shrunk).unwrap();
        assert_eq!(bytes, product_codec::encode(&oracle).unwrap());
        assert_eq!(
            product_codec::decode::<ProvinceField>(&bytes).unwrap(),
            oracle
        );
        assert!(Arc::ptr_eq(&first.0, &first.patched([(1, 7)]).unwrap().0));
    }
}

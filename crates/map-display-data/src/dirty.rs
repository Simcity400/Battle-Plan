use crate::credit::{Credit, MemoryBudget};
use std::{collections::BTreeMap, io};

pub struct Dirty<const WORDS: usize> {
    pages: BTreeMap<usize, [u64; WORDS]>,
    credit: Credit,
    owner: MemoryBudget,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn broad_duplicate_memberships_charge_pages_not_population() {
        let owner = MemoryBudget::new(1_000_000);
        let mut dirty = Dirty::<64>::new(&owner).unwrap();
        for cell in 0..160_000 {
            dirty.insert(cell).unwrap();
            dirty.insert(cell).unwrap();
        }
        assert!(dirty.iter().eq(0..160_000));
        assert!(owner.used() < 300_000);
        drop(dirty);
        assert_eq!(owner.used(), 0);
        let tiny = MemoryBudget::new(1);
        assert_eq!(
            Dirty::<64>::new(&tiny)
                .unwrap()
                .insert(0)
                .unwrap_err()
                .kind(),
            io::ErrorKind::OutOfMemory
        );
    }
}
impl<const WORDS: usize> Dirty<WORDS> {
    pub fn new(owner: &MemoryBudget) -> io::Result<Self> {
        Ok(Self {
            pages: BTreeMap::new(),
            credit: owner.admit_item(0)?,
            owner: owner.clone(),
        })
    }
    pub fn insert(&mut self, item: usize) -> io::Result<()> {
        let page = item / (WORDS * 64);
        if !self.pages.contains_key(&page) {
            let bytes = self
                .credit
                .bytes()
                .checked_add(12 * (size_of::<usize>() + size_of::<[u64; WORDS]>()) + 128)
                .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
            if bytes > self.owner.limit() {
                return Err(io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    "dirty-page working set exceeds its owner; common paging is required",
                ));
            }
            self.credit.resize(bytes)?;
            self.pages.insert(page, [0; WORDS]);
        }
        let within = item % (WORDS * 64);
        self.pages.get_mut(&page).unwrap()[within / 64] |= 1 << (within % 64);
        Ok(())
    }
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.pages.iter().flat_map(|(&page, words)| {
            words.iter().enumerate().flat_map(move |(word, &bits)| {
                let mut bits = bits;
                std::iter::from_fn(move || {
                    if bits == 0 {
                        None
                    } else {
                        let bit = bits.trailing_zeros() as usize;
                        bits &= bits - 1;
                        Some(page * WORDS * 64 + word * 64 + bit)
                    }
                })
            })
        })
    }
}

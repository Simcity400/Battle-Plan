use std::collections::BTreeMap;

pub struct Dirty<const WORDS: usize> {
    pages: BTreeMap<usize, [u64; WORDS]>,
}

impl<const WORDS: usize> Default for Dirty<WORDS> {
    fn default() -> Self {
        Self {
            pages: BTreeMap::new(),
        }
    }
}

impl<const WORDS: usize> Dirty<WORDS> {
    pub fn insert(&mut self, item: usize) {
        let page = item / (WORDS * 64);
        let within = item % (WORDS * 64);
        self.pages.entry(page).or_insert([0; WORDS])[within / 64] |= 1 << (within % 64);
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_memberships_iterate_once_in_order() {
        let mut dirty = Dirty::<64>::default();
        for cell in (0..160_000).rev() {
            dirty.insert(cell);
            dirty.insert(cell);
        }
        assert!(dirty.iter().eq(0..160_000));
    }
}

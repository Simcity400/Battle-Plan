use crate::{
    credit::{Credit, MemoryBudget},
    retained::Retained,
};
use std::{collections::BTreeMap, io};

pub struct Accounting {
    owner: MemoryBudget,
    credit: Credit,
}
fn overflow() -> io::Error {
    io::Error::new(
        io::ErrorKind::OutOfMemory,
        "display allocation byte count overflow",
    )
}
impl Accounting {
    pub fn new(owner: &MemoryBudget, bytes: usize) -> io::Result<Self> {
        Ok(Self {
            owner: owner.clone(),
            credit: owner.admit_item(bytes)?,
        })
    }
    pub fn add(&mut self, bytes: usize) -> io::Result<()> {
        let total = self
            .credit
            .bytes()
            .checked_add(bytes)
            .ok_or_else(overflow)?;
        if total > self.owner.limit() {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "display working set exceeds its owner; common paging is required",
            ));
        }
        self.credit.resize(total)
    }
    pub fn reserve<T>(&mut self, values: &mut Vec<T>, additional: usize) -> io::Result<()> {
        let required = values.len().checked_add(additional).ok_or_else(overflow)?;
        if required <= values.capacity() {
            return Ok(());
        }
        let capacity = required.max(values.capacity().checked_mul(2).unwrap_or(required));
        let added = (capacity - values.capacity())
            .checked_mul(size_of::<T>())
            .and_then(|n| {
                n.checked_add(if values.capacity() == 0 {
                    2 * size_of::<usize>() + align_of::<T>()
                } else {
                    0
                })
            })
            .ok_or_else(overflow)?;
        let old = self.credit.bytes();
        let overlap = values
            .capacity()
            .checked_mul(size_of::<T>())
            .ok_or_else(overflow)?;
        if old
            .checked_add(added)
            .and_then(|n| n.checked_add(overlap))
            .is_none_or(|n| n > self.owner.limit())
        {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "display buffer growth exceeds its owner; common paging is required",
            ));
        }
        let _overlap = self.owner.admit_item(overlap)?;
        self.add(added)?;
        if values.try_reserve_exact(capacity - values.len()).is_err() {
            self.credit.resize(old)?;
            return Err(overflow());
        }
        Ok(())
    }
    pub fn push_index<K: Ord + Copy>(
        &mut self,
        index: &mut BTreeMap<K, Vec<usize>>,
        key: K,
        value: usize,
    ) -> io::Result<()> {
        if !index.contains_key(&key) {
            self.add(12 * (size_of::<K>() + size_of::<Vec<usize>>()) + 128)?;
        }
        let values = index.entry(key).or_default();
        self.reserve(values, 1)?;
        values.push(value);
        Ok(())
    }
    pub fn finish<T>(self, value: T) -> Retained<T> {
        Retained::with_credit(self.credit, || value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn growth_refuses_before_mutation_and_retries_after_independent_release() {
        let owner = MemoryBudget::new(4096);
        let mut account = Accounting::new(&owner, Retained::<Vec<u64>>::overhead()).unwrap();
        let mut values = Vec::new();
        account.reserve(&mut values, 8).unwrap();
        values.extend(0..8u64);
        let held = owner.admit_item(owner.limit() - owner.used()).unwrap();
        let error = account.reserve(&mut values, 256).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(values, (0..8).collect::<Vec<_>>());
        assert_eq!(values.capacity(), 8);
        drop(held);
        account.reserve(&mut values, 256).unwrap();
        let value = account.finish(values);
        let shared = value.clone();
        drop(value);
        assert!(owner.used() > 0);
        drop(shared);
        assert_eq!(owner.used(), 0);
    }
}

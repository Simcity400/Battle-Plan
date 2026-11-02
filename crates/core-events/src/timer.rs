use crate::{Hour, Key};
use std::{
    cmp::Ordering,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timer<'a> {
    pub hour: Hour,
    pub id: Key<'a>,
    pub key: Key<'a>,
}

pub trait TimerStore {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn reserve_len(&mut self, len: usize) -> io::Result<()>;
    // The id index must follow swaps/removals; lookup must not scan the heap.
    fn find(&mut self, id: Key<'_>) -> io::Result<Option<usize>>;
    fn set_hour(&mut self, index: usize, hour: Hour) -> io::Result<()>;
    fn read(&mut self, index: usize) -> io::Result<Timer<'_>>;
    fn compare(&mut self, left: usize, right: usize) -> io::Result<Ordering>;
    fn swap(&mut self, left: usize, right: usize) -> io::Result<()>;
    fn truncate(&mut self, len: usize) -> io::Result<()>;
    fn pop_last(&mut self) -> io::Result<Timer<'_>>;
}

pub trait TimerSink<'a>: TimerStore {
    fn push(&mut self, timer: Timer<'a>) -> io::Result<()>;
}

pub struct SliceTimers<'a, 's> {
    storage: &'s mut [Timer<'a>],
    index: &'s mut [usize],
    buckets: &'s mut [usize],
    len: usize,
}

impl<'a, 's> SliceTimers<'a, 's> {
    pub fn new(
        storage: &'s mut [Timer<'a>],
        index: &'s mut [usize],
        buckets: &'s mut [usize],
    ) -> io::Result<Self> {
        if storage
            .len()
            .checked_mul(2)
            .is_none_or(|needed| index.len() < needed)
            || buckets.len() < storage.len()
        {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        index.fill(usize::MAX);
        Ok(Self {
            storage,
            index,
            buckets,
            len: 0,
        })
    }
    fn bucket(&self, id: Key<'_>) -> usize {
        let mut hash = DefaultHasher::new();
        id.hash(&mut hash);
        let mut bucket = (hash.finish() % self.index.len() as u64) as usize;
        while self.index[bucket] != usize::MAX && self.storage[self.index[bucket]].id != id {
            bucket = (bucket + 1) % self.index.len();
        }
        bucket
    }
    fn unindex(&mut self, slot: usize) {
        let bucket = self.buckets[slot];
        self.index[bucket] = usize::MAX;
        let mut next = (bucket + 1) % self.index.len();
        while self.index[next] != usize::MAX {
            let displaced = self.index[next];
            self.index[next] = usize::MAX;
            let destination = self.bucket(self.storage[displaced].id);
            self.index[destination] = displaced;
            self.buckets[displaced] = destination;
            next = (next + 1) % self.index.len();
        }
    }
}

impl TimerStore for SliceTimers<'_, '_> {
    fn len(&self) -> usize {
        self.len
    }
    fn reserve_len(&mut self, len: usize) -> io::Result<()> {
        if len <= self.storage.len() {
            Ok(())
        } else {
            Err(io::ErrorKind::OutOfMemory.into())
        }
    }
    fn find(&mut self, id: Key<'_>) -> io::Result<Option<usize>> {
        if self.len == 0 {
            return Ok(None);
        }
        let slot = self.index[self.bucket(id)];
        Ok((slot != usize::MAX).then_some(slot))
    }
    fn set_hour(&mut self, index: usize, hour: Hour) -> io::Result<()> {
        self.storage[..self.len]
            .get_mut(index)
            .ok_or(io::ErrorKind::InvalidInput)?
            .hour = hour;
        Ok(())
    }
    fn read(&mut self, index: usize) -> io::Result<Timer<'_>> {
        self.storage[..self.len]
            .get(index)
            .copied()
            .ok_or_else(|| io::ErrorKind::InvalidInput.into())
    }
    fn compare(&mut self, left: usize, right: usize) -> io::Result<Ordering> {
        let timers = &self.storage[..self.len];
        Ok(timers
            .get(left)
            .ok_or(io::ErrorKind::InvalidInput)?
            .cmp(timers.get(right).ok_or(io::ErrorKind::InvalidInput)?))
    }
    fn swap(&mut self, left: usize, right: usize) -> io::Result<()> {
        if left >= self.len || right >= self.len {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let left_bucket = self.buckets[left];
        let right_bucket = self.buckets[right];
        self.index[left_bucket] = right;
        self.index[right_bucket] = left;
        self.storage.swap(left, right);
        self.buckets.swap(left, right);
        Ok(())
    }
    fn truncate(&mut self, len: usize) -> io::Result<()> {
        if len > self.len {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        while self.len > len {
            self.unindex(self.len - 1);
            self.len -= 1;
        }
        Ok(())
    }
    fn pop_last(&mut self) -> io::Result<Timer<'_>> {
        let last = self.len.checked_sub(1).ok_or(io::ErrorKind::InvalidInput)?;
        self.unindex(last);
        self.len = last;
        Ok(self.storage[self.len])
    }
}

impl<'a: 'keys, 'keys> TimerSink<'a> for SliceTimers<'keys, '_> {
    fn push(&mut self, timer: Timer<'a>) -> io::Result<()> {
        if self.len == self.storage.len() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let bucket = self.bucket(timer.id);
        if self.index[bucket] != usize::MAX {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let slot = self
            .storage
            .get_mut(self.len)
            .ok_or(io::ErrorKind::OutOfMemory)?;
        *slot = timer;
        self.index[bucket] = self.len;
        self.buckets[self.len] = bucket;
        self.len += 1;
        Ok(())
    }
}

pub struct TimerQueue<S: TimerStore> {
    store: S,
}

impl<S: TimerStore> TimerQueue<S> {
    pub fn new(mut store: S) -> io::Result<Self> {
        store.truncate(0)?;
        Ok(Self { store })
    }
    pub fn len(&self) -> usize {
        self.store.len()
    }
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }
    pub fn into_store(self) -> S {
        self.store
    }
    pub fn peek(&mut self) -> io::Result<Option<Timer<'_>>> {
        if self.is_empty() {
            Ok(None)
        } else {
            self.store.read(0).map(Some)
        }
    }
    pub fn is_due(&mut self, hour: Hour) -> io::Result<bool> {
        Ok(self.peek()?.is_some_and(|timer| timer.hour <= hour))
    }
    pub fn schedule<'a>(&mut self, timer: Timer<'a>) -> io::Result<()>
    where
        S: TimerSink<'a>,
    {
        if let Some(index) = self.store.find(timer.id)? {
            if self.store.read(index)?.key != timer.key {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            self.store.set_hour(index, timer.hour)?;
            return self.repair(index, self.len());
        }
        let index = self.len();
        self.store
            .reserve_len(index.checked_add(1).ok_or(io::ErrorKind::OutOfMemory)?)?;
        self.store.push(timer)?;
        self.sift_up(index).map(|_| ())
    }
    fn sift_up(&mut self, mut index: usize) -> io::Result<usize> {
        while index > 0 {
            let parent = (index - 1) / 2;
            if self.store.compare(parent, index)? != Ordering::Greater {
                break;
            }
            self.store.swap(parent, index)?;
            index = parent;
        }
        Ok(index)
    }
    fn repair(&mut self, index: usize, len: usize) -> io::Result<()> {
        if self.sift_up(index)? == index {
            self.sift_down(index, len)?;
        }
        Ok(())
    }
    pub fn cancel(&mut self, id: Key<'_>) -> io::Result<bool> {
        let Some(index) = self.store.find(id)? else {
            return Ok(false);
        };
        let last = self.len() - 1;
        self.store.swap(index, last)?;
        // Remove the id before repairing the heap so index updates never reach a removed slot.
        self.store.truncate(last)?;
        if index < last {
            self.repair(index, last)?;
        }
        Ok(true)
    }
    pub fn pop_due(&mut self, hour: Hour) -> io::Result<Option<Timer<'_>>> {
        if self.is_empty() || self.store.read(0)?.hour > hour {
            return Ok(None);
        }
        let last = self.len() - 1;
        self.store.swap(0, last)?;
        if last > 0 {
            self.sift_down(0, last)?;
        }
        self.store.pop_last().map(Some)
    }
    fn sift_down(&mut self, mut index: usize, len: usize) -> io::Result<()> {
        while index < len / 2 {
            let mut child = index * 2 + 1;
            if child + 1 < len && self.store.compare(child + 1, child)? == Ordering::Less {
                child += 1;
            }
            if self.store.compare(index, child)? != Ordering::Greater {
                break;
            }
            self.store.swap(index, child)?;
            index = child;
        }
        Ok(())
    }
    pub fn rebuild<'a>(&mut self, timers: &[Timer<'a>]) -> io::Result<()>
    where
        S: TimerSink<'a>,
    {
        self.store.reserve_len(timers.len())?;
        self.store.truncate(0)?;
        for timer in timers.iter().copied() {
            self.schedule(timer)?;
        }
        Ok(())
    }
}

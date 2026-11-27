use std::{
    collections::HashMap,
    hash::Hash,
    ops::Deref,
    sync::{
        atomic::{AtomicI32, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

static LANDED: AtomicU64 = AtomicU64::new(0);

pub fn count() -> u64 {
    LANDED.load(Ordering::Acquire)
}

fn bump() {
    LANDED.fetch_add(1, Ordering::AcqRel);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Landed<T>(T);

impl<T> Landed<T> {
    pub fn into_inner(self) -> T {
        self.0
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Landed<U> {
        Landed(f(self.0))
    }

    pub fn as_ref(&self) -> Landed<&T> {
        Landed(&self.0)
    }
}

impl<T: Clone> Landed<&T> {
    pub fn cloned(self) -> Landed<T> {
        Landed(self.0.clone())
    }
}

impl<T, E> Landed<Result<T, E>> {
    pub fn transpose(self) -> Result<Landed<T>, E> {
        self.0.map(Landed)
    }
}

impl<T> Landed<Option<T>> {
    pub fn transpose(self) -> Option<Landed<T>> {
        self.0.map(Landed)
    }
}

impl<A, B> Landed<(A, B)> {
    pub fn split(self) -> (Landed<A>, Landed<B>) {
        (Landed(self.0.0), Landed(self.0.1))
    }
}

impl<T> Deref for Landed<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

pub struct Sender<T>(mpsc::Sender<T>);

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Sender<T> {
    pub fn send(&self, value: T) -> Result<(), mpsc::SendError<T>> {
        self.0.send(value)?;
        bump();
        Ok(())
    }
}

pub struct Receiver<T>(mpsc::Receiver<T>);

impl<T> Receiver<T> {
    pub fn try_recv(&self) -> Result<Landed<T>, mpsc::TryRecvError> {
        self.0.try_recv().map(Landed)
    }

    pub fn recv(&self) -> Result<Landed<T>, mpsc::RecvError> {
        self.0.recv().map(Landed)
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<Landed<T>, mpsc::RecvTimeoutError> {
        self.0.recv_timeout(timeout).map(Landed)
    }
}

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let (send, receive) = mpsc::channel();
    (Sender(send), Receiver(receive))
}

#[derive(Debug, Default)]
pub struct Published<T>(T);

impl<T> Published<T> {
    pub fn get(&self) -> &T {
        &self.0
    }

    pub fn clear(&mut self)
    where
        T: Default,
    {
        self.0 = T::default();
    }
}

impl<T> Published<Option<T>> {
    pub fn set_some(&mut self, value: T) {
        self.0 = Some(value);
        bump();
    }

    pub fn set(&mut self, value: Option<T>) {
        let changed = value.is_some() || self.0.is_some();
        self.0 = value;
        if changed {
            bump();
        }
    }

    pub fn set_if_changed(&mut self, value: Option<T>) -> bool
    where
        T: PartialEq,
    {
        if self.0 == value {
            return false;
        }
        self.0 = value;
        bump();
        true
    }

    pub fn advance(&mut self, value: T) -> bool
    where
        T: Ord,
    {
        if self.0.as_ref().is_some_and(|held| *held >= value) {
            return false;
        }
        self.0 = Some(value);
        bump();
        true
    }

    pub fn take(&mut self) -> Option<Landed<T>> {
        self.0.take().map(Landed)
    }

    pub fn as_ref(&self) -> Option<&T> {
        self.0.as_ref()
    }

    pub fn landed_clone(&self) -> Option<Landed<T>>
    where
        T: Clone,
    {
        self.0.clone().map(Landed)
    }

    pub fn is_some(&self) -> bool {
        self.0.is_some()
    }

    pub fn is_none(&self) -> bool {
        self.0.is_none()
    }
}

impl<T> Published<Vec<T>> {
    pub fn push(&mut self, value: T) {
        self.0.push(value);
        bump();
    }

    pub fn extend_nonempty(&mut self, values: impl IntoIterator<Item = T>) {
        let before = self.0.len();
        self.0.extend(values);
        if self.0.len() != before {
            bump();
        }
    }

    pub fn take_all(&mut self) -> Option<Landed<Vec<T>>> {
        (!self.0.is_empty()).then(|| Landed(std::mem::take(&mut self.0)))
    }

    pub fn retain(&mut self, keep: impl FnMut(&T) -> bool) {
        self.0.retain(keep);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<K: Eq + Hash, V> Published<HashMap<K, V>> {
    pub fn insert(&mut self, key: K, value: V) {
        self.0.insert(key, value);
        bump();
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.0.contains_key(key)
    }

    pub fn get_entry(&self, key: &K) -> Option<&V> {
        self.0.get(key)
    }
}

impl Published<u64> {
    pub fn inc(&mut self) {
        self.0 = self.0.wrapping_add(1);
        bump();
    }
}

#[derive(Debug)]
pub struct PublishedKeyed<K, V> {
    last: Option<K>,
    value: Option<V>,
}

impl<K, V> Default for PublishedKeyed<K, V> {
    fn default() -> Self {
        Self { last: None, value: None }
    }
}

impl<K, V> PublishedKeyed<K, V> {
    pub fn publish(&mut self, key: K, value: V) -> bool
    where
        K: PartialEq,
    {
        let changed = self.last.as_ref() != Some(&key);
        self.last = Some(key);
        self.value = Some(value);
        if changed {
            bump();
        }
        changed
    }

    pub fn publish_new(&mut self, key: K, value: V) {
        self.last = Some(key);
        self.value = Some(value);
        bump();
    }

    pub fn take(&mut self) -> Option<Landed<V>> {
        self.value.take().map(Landed)
    }

    pub fn as_ref(&self) -> Option<&V> {
        self.value.as_ref()
    }

    pub fn landed_clone(&self) -> Option<Landed<V>>
    where
        V: Clone,
    {
        self.value.clone().map(Landed)
    }

    pub fn is_some(&self) -> bool {
        self.value.is_some()
    }

    pub fn clear(&mut self) {
        (self.last, self.value) = (None, None);
    }

    pub fn forget_key(&mut self) {
        self.last = None;
    }
}

pub struct PublishedHour(AtomicI32);

impl PublishedHour {
    pub const fn new(hour: i32) -> Self {
        Self(AtomicI32::new(hour))
    }

    pub fn load(&self) -> i32 {
        self.0.load(Ordering::Acquire)
    }

    pub fn set(&self, hour: i32) -> bool {
        if self.0.swap(hour, Ordering::AcqRel) != hour {
            bump();
            return true;
        }
        false
    }

    pub fn advance(&self, hour: i32) -> bool {
        if self.0.fetch_max(hour, Ordering::AcqRel) < hour {
            bump();
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_and_published_slots_move_count_only_when_new_data_is_written() {
        let (send, receive) = channel::<u32>();
        let before = count();
        send.send(7).unwrap();
        assert!(count() > before, "sending on a landed channel moves the count");
        let landed: Landed<u32> = receive.try_recv().unwrap();
        assert_eq!(*landed, 7);
        assert_eq!(landed.map(|value| value + 1).into_inner(), 8);

        let mut slot: Published<Option<i32>> = Published::default();
        let before = count();
        assert!(slot.advance(10));
        assert!(count() > before);
        let after = count();
        assert!(!slot.advance(10), "unchanged advance does not move the count");
        assert!(!slot.advance(5), "earlier advance does not move the count");
        assert_eq!(count(), after);
        assert!(slot.set_if_changed(Some(12)));
        assert!(count() > after);
        let after = count();
        assert!(!slot.set_if_changed(Some(12)));
        assert_eq!(count(), after);
        assert_eq!(slot.take().map(Landed::into_inner), Some(12));
        slot.clear();
        assert_eq!(count(), after, "clear does not move the count");

        let mut keyed: PublishedKeyed<(u64, i32), &'static str> = PublishedKeyed::default();
        let before = count();
        assert!(keyed.publish((1, 10), "first"));
        assert!(count() > before);
        assert_eq!(keyed.take().map(Landed::into_inner), Some("first"));
        let after = count();
        assert!(!keyed.publish((1, 10), "same key after take"), "republishing the same key after take does not move the count");
        assert_eq!(count(), after);
        keyed.publish_new((1, 10), "fresh layout");
        assert!(count() > after);
        let after = count();
        assert!(!keyed.publish((1, 10), "unchanged after fresh"), "unchanged follow-up after publish_new does not move the count");
        assert_eq!(count(), after);
        assert!(keyed.publish((1, 11), "next hour"));
        assert!(count() > after);

        let mut list: Published<Vec<u32>> = Published::default();
        let before = count();
        list.extend_nonempty(std::iter::empty());
        assert_eq!(count(), before);
        assert!(list.take_all().is_none());
        list.extend_nonempty([1, 2]);
        assert!(count() > before);
        assert_eq!(list.take_all().map(Landed::into_inner), Some(vec![1, 2]));
        assert!(list.take_all().is_none());
        let before = count();
        list.push(3);
        assert!(count() > before);

        let mut map: Published<HashMap<u32, u32>> = Published::default();
        let before = count();
        map.insert(1, 2);
        assert!(count() > before);
        assert_eq!(map.get_entry(&1), Some(&2));

        let hour = PublishedHour::new(10);
        let before = count();
        assert!(!hour.set(10));
        assert!(!hour.advance(10));
        assert!(!hour.advance(8));
        assert_eq!(count(), before);
        assert!(hour.advance(12));
        assert!(count() > before);
        let after = count();
        assert!(hour.set(9));
        assert!(count() > after);
    }
}

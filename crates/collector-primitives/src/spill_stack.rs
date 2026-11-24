use engine_allocation as allocation;
use std::{
    io,
    ops::{Deref, DerefMut},
    ptr,
    sync::atomic::{AtomicPtr, AtomicUsize, Ordering},
};

struct Node<T> {
    next: *mut Node<T>,
    value: T,
}

pub struct SpillStack<T> {
    head: AtomicPtr<Node<T>>,
    len: AtomicUsize,
}

unsafe impl<T: Send> Send for SpillStack<T> {}
unsafe impl<T: Send> Sync for SpillStack<T> {}

impl<T> Default for SpillStack<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> SpillStack<T> {
    pub const fn new() -> Self {
        Self {
            head: AtomicPtr::new(ptr::null_mut()),
            len: AtomicUsize::new(0),
        }
    }

    pub const fn node_size() -> usize {
        size_of::<Node<T>>()
    }

    pub fn push(&self, value: T) -> io::Result<()> {
        self.reserve(value).map(Reserved::push)
    }

    pub fn reserve(&self, value: T) -> io::Result<Reserved<'_, T>> {
        let node = allocation::advisory(|| {
            allocation::boxed(Node {
                next: ptr::null_mut(),
                value,
            })
        })?;
        Ok(Reserved { stack: self, node })
    }

    fn link(&self, node: *mut Node<T>) {
        self.len.fetch_add(1, Ordering::AcqRel);
        let mut head = self.head.load(Ordering::Relaxed);
        loop {
            unsafe { (*node).next = head };
            match self
                .head
                .compare_exchange_weak(head, node, Ordering::Release, Ordering::Relaxed)
            {
                Ok(_) => return,
                Err(current) => head = current,
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Acquire).is_null()
    }

    pub fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }

    pub fn take(&self) -> Taken<T> {
        let mut node = self.head.swap(ptr::null_mut(), Ordering::Acquire);
        let mut oldest = ptr::null_mut();
        let mut len = 0;
        while !node.is_null() {
            let next = unsafe { (*node).next };
            unsafe { (*node).next = oldest };
            oldest = node;
            node = next;
            len += 1;
        }
        if len != 0 {
            self.len.fetch_sub(len, Ordering::AcqRel);
        }
        Taken { head: oldest, len }
    }

    pub fn take_into(&self, held: &mut Vec<T>) -> Result<(), Taken<T>> {
        let taken = self.take();
        if taken.len == 0 {
            return Ok(());
        }
        if allocation::reserve(held, taken.len).is_err() {
            return Err(taken);
        }
        held.extend(taken);
        Ok(())
    }
}

impl<T> Drop for SpillStack<T> {
    fn drop(&mut self) {
        drop(self.take());
    }
}

pub struct Reserved<'a, T> {
    stack: &'a SpillStack<T>,
    node: Box<Node<T>>,
}

impl<T> Reserved<'_, T> {
    pub fn push(self) {
        let Self { stack, node } = self;
        stack.link(Box::into_raw(node));
    }
}

impl<T> Deref for Reserved<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.node.value
    }
}

impl<T> DerefMut for Reserved<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.node.value
    }
}

pub struct Taken<T> {
    head: *mut Node<T>,
    len: usize,
}

unsafe impl<T: Send> Send for Taken<T> {}

impl<T> Iterator for Taken<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        if self.head.is_null() {
            return None;
        }
        let node = unsafe { Box::from_raw(self.head) };
        self.head = node.next;
        self.len -= 1;
        Some(node.value)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len, Some(self.len))
    }
}

impl<T> ExactSizeIterator for Taken<T> {}

impl<T> Drop for Taken<T> {
    fn drop(&mut self) {
        for _ in self.by_ref() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn a_take_returns_every_push_in_order_and_frees_what_it_drops() {
        let stack = SpillStack::new();
        for value in 0..100_u64 {
            stack.push(Box::new(value)).unwrap();
        }
        let mut taken = stack.take();
        assert_eq!(taken.len(), 100);
        let first: Vec<u64> = taken.by_ref().take(40).map(|value| *value).collect();
        assert_eq!(first, (0..40).collect::<Vec<_>>());
        assert!(stack.is_empty());
        stack.push(Box::new(100)).unwrap();
        drop(taken);
        let mut held = vec![Box::new(7_u64)];
        stack.take_into(&mut held).map_err(|_| ()).unwrap();
        assert_eq!(held.iter().map(|value| **value).collect::<Vec<_>>(), [7, 100]);
        assert!(stack.is_empty());
    }

    #[test]
    fn a_refused_node_drops_its_record_and_keeps_the_rest() {
        let stack = SpillStack::new();
        stack.push(1_u64).unwrap();
        assert!(collector_test_support::with_allocations_denied(|| stack.push(2)).is_err());
        let mut reserved = stack.reserve(0).unwrap();
        *reserved = 3;
        drop(stack.reserve(4).unwrap());
        assert!(stack.take().eq([1]));
        reserved.push();
        assert_eq!(stack.take().collect::<Vec<_>>(), [3]);
    }

    #[test]
    fn producers_on_many_threads_lose_nothing_and_each_keeps_its_order() {
        const THREADS: u64 = 4;
        const EACH: u64 = 5_000;
        let stack = Arc::new(SpillStack::new());
        let start = Arc::new(Barrier::new(THREADS as usize));
        let producers: Vec<_> = (0..THREADS)
            .map(|thread| {
                let (stack, start) = (stack.clone(), start.clone());
                std::thread::spawn(move || {
                    start.wait();
                    for n in 0..EACH {
                        stack.push((thread, n)).unwrap();
                    }
                })
            })
            .collect();
        let mut taken = Vec::new();
        while taken.len() < (THREADS * EACH) as usize {
            stack.take_into(&mut taken).map_err(|_| ()).unwrap();
        }
        producers.into_iter().for_each(|producer| producer.join().unwrap());
        assert!(stack.is_empty());
        assert_eq!(stack.len(), 0);
        for thread in 0..THREADS {
            let own: Vec<u64> = taken
                .iter()
                .filter(|(from, _)| *from == thread)
                .map(|&(_, n)| n)
                .collect();
            assert_eq!(own, (0..EACH).collect::<Vec<_>>());
        }
    }

}

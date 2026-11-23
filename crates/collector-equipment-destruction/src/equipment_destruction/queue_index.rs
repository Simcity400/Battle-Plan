const NONE: usize = usize::MAX;

pub(super) trait Key: Copy + Eq {
    fn word(self) -> u64;
}
impl Key for u64 {
    fn word(self) -> u64 {
        self
    }
}
impl Key for usize {
    fn word(self) -> u64 {
        self as u64
    }
}

#[derive(Clone, Copy)]
struct Bucket {
    generation: u64,
    head: usize,
}
#[derive(Clone, Copy)]
struct Node<K, V> {
    generation: u64,
    next: usize,
    value: Option<(K, V)>,
}

pub(super) struct FixedIndex<K, V> {
    buckets: Vec<Bucket>,
    nodes: Vec<Node<K, V>>,
    generation: u64,
    unused: usize,
    free: usize,
    len: usize,
}
impl<K: Key, V: Copy> FixedIndex<K, V> {
    pub(super) fn new() -> Self {
        Self {
            buckets: Vec::new(),
            nodes: Vec::new(),
            generation: 1,
            unused: 0,
            free: NONE,
            len: 0,
        }
    }
    pub(super) fn try_new(capacity: usize) -> std::io::Result<Self> {
        let bucket_count = if capacity == 0 {
            0
        } else {
            capacity
                .checked_next_power_of_two()
                .ok_or_else(engine_allocation::oom)?
        };
        let mut index = Self::new();
        engine_allocation::reserve_exact(&mut index.buckets, bucket_count)?;
        engine_allocation::reserve_exact(&mut index.nodes, capacity)?;
        index.buckets.resize(
            bucket_count,
            Bucket {
                generation: 0,
                head: NONE,
            },
        );
        index.nodes.resize(
            capacity,
            Node {
                generation: 0,
                next: NONE,
                value: None,
            },
        );
        Ok(index)
    }
    pub(super) fn grown(&self, capacity: usize) -> std::io::Result<Self> {
        let mut index = Self::try_new(capacity.max(self.len))?;
        for (&key, &value) in self.iter() {
            index
                .insert(key, value)
                .map_err(|_| engine_allocation::oom())?;
        }
        Ok(index)
    }
    #[cfg(test)]
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self::try_new(capacity).unwrap()
    }
    pub(super) fn capacity(&self) -> usize {
        self.nodes.len()
    }
    pub(super) fn len(&self) -> usize {
        self.len
    }
    pub(super) fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub(super) fn allocated_bytes(&self) -> usize {
        self.buckets.capacity() * std::mem::size_of::<Bucket>()
            + self.nodes.capacity() * std::mem::size_of::<Node<K, V>>()
    }
    pub(super) fn clear(&mut self) -> Result<(), &'static str> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("equipment deletion provenance generation exhausted")?;
        self.unused = 0;
        self.free = NONE;
        self.len = 0;
        Ok(())
    }
    fn bucket(&self, key: K) -> usize {
        let mut word = key.word();
        word = (word ^ (word >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        word = (word ^ (word >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        (word ^ (word >> 31)) as usize & (self.buckets.len() - 1)
    }
    fn find(&self, key: K) -> Option<usize> {
        if self.buckets.is_empty() {
            return None;
        }
        let bucket = self.buckets[self.bucket(key)];
        if bucket.generation != self.generation {
            return None;
        }
        let mut node = bucket.head;
        while node != NONE {
            if self.nodes[node]
                .value
                .is_some_and(|(stored, _)| stored == key)
            {
                return Some(node);
            }
            node = self.nodes[node].next;
        }
        None
    }
    pub(super) fn get(&self, key: &K) -> Option<&V> {
        self.nodes[self.find(*key)?]
            .value
            .as_ref()
            .map(|(_, value)| value)
    }
    pub(super) fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        let node = self.find(*key)?;
        self.nodes[node].value.as_mut().map(|(_, value)| value)
    }
    pub(super) fn insert(&mut self, key: K, value: V) -> Result<(), &'static str> {
        if let Some(node) = self.find(key) {
            self.nodes[node].value = Some((key, value));
            return Ok(());
        }
        let node = if self.free != NONE {
            let node = self.free;
            self.free = self.nodes[node].next;
            node
        } else if self.unused < self.nodes.len() {
            let node = self.unused;
            self.unused += 1;
            node
        } else {
            return Err("equipment deletion provenance index overflow");
        };
        let bucket = self.bucket(key);
        if self.buckets[bucket].generation != self.generation {
            self.buckets[bucket] = Bucket {
                generation: self.generation,
                head: NONE,
            };
        }
        self.nodes[node] = Node {
            generation: self.generation,
            next: self.buckets[bucket].head,
            value: Some((key, value)),
        };
        self.buckets[bucket].head = node;
        self.len += 1;
        Ok(())
    }
    pub(super) fn remove(&mut self, key: &K) -> Option<V> {
        if self.buckets.is_empty() {
            return None;
        }
        let bucket = self.bucket(*key);
        if self.buckets[bucket].generation != self.generation {
            return None;
        }
        let mut previous = NONE;
        let mut node = self.buckets[bucket].head;
        while node != NONE {
            if self.nodes[node]
                .value
                .is_some_and(|(stored, _)| stored == *key)
            {
                if previous == NONE {
                    self.buckets[bucket].head = self.nodes[node].next;
                } else {
                    self.nodes[previous].next = self.nodes[node].next;
                }
                let (_, value) = self.nodes[node].value.take()?;
                self.nodes[node].next = self.free;
                self.free = node;
                self.len -= 1;
                return Some(value);
            }
            previous = node;
            node = self.nodes[node].next;
        }
        None
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.nodes.iter().filter_map(|node| {
            (node.generation == self.generation)
                .then_some(node.value.as_ref())
                .flatten()
                .map(|(key, value)| (key, value))
        })
    }
    #[cfg(test)]
    pub(super) fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, value)| value)
    }
}
#[cfg(test)]
impl<K: Key, V: Copy> std::ops::Index<&K> for FixedIndex<K, V> {
    type Output = V;
    fn index(&self, key: &K) -> &V {
        self.get(key).unwrap()
    }
}

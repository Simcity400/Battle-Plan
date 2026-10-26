use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct Position {
    pub batch: u64,
    pub offset: u64,
}
impl Position {
    fn encode(self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.batch.to_le_bytes());
        bytes.extend_from_slice(&self.offset.to_le_bytes());
    }
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let position = Self {
            batch: u64_at(bytes, 0),
            offset: u64_at(bytes, 8),
        };
        if position.offset < HEADER {
            return Err(invalid("invalid event order position"));
        }
        Ok(position)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Node {
    entry: (i32, u64),
    left: Option<Position>,
    right: Option<Position>,
    pub(super) count: u64,
    pub(super) height: u32,
}
impl Node {
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let child = |at| {
            if u64_at(bytes, at + 8) == 0 {
                Ok(None)
            } else {
                Position::decode(&bytes[at..at + 16]).map(Some)
            }
        };
        let node = Self {
            entry: (i32_at(bytes, 0), u64_at(bytes, 4)),
            left: child(12)?,
            right: child(28)?,
            count: u64_at(bytes, 44),
            height: u32::from_le_bytes(bytes[52..56].try_into().unwrap()),
        };
        if node.count == 0 || node.height == 0 || node.height > 2 * (node.count.ilog2() + 1) {
            return Err(invalid("invalid event order node"));
        }
        Ok(node)
    }
    fn encode(self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.entry.0.to_le_bytes());
        bytes.extend_from_slice(&self.entry.1.to_le_bytes());
        for child in [self.left, self.right] {
            match child {
                Some(position) => position.encode(bytes),
                None => bytes.extend_from_slice(&[0; 16]),
            }
        }
        bytes.extend_from_slice(&self.count.to_le_bytes());
        bytes.extend_from_slice(&self.height.to_le_bytes());
    }
}

pub(super) fn slot(bytes: &[u8], index: u64) -> io::Result<Option<u64>> {
    match u32::from_le_bytes(bytes[60..64].try_into().unwrap()) {
        0 => Ok(None),
        2 => Ok(Some(index)),
        _ => Err(invalid("invalid event order directory flag")),
    }
}
pub(super) fn root_offset(length: u64, count: u64, slot: u64) -> io::Result<u64> {
    if slot >= count {
        return Err(invalid("event order root is outside its directory"));
    }
    count
        .checked_sub(slot)
        .and_then(|n| n.checked_mul(16))
        .and_then(|bytes| length.checked_sub(bytes))
        .filter(|offset| *offset >= HEADER)
        .ok_or_else(|| invalid("truncated event order roots"))
}

pub(super) fn count_through(
    root: Position,
    hour: i32,
    mut read: impl FnMut(Position) -> io::Result<Node>,
) -> io::Result<u64> {
    let mut next = Some(root);
    let mut previous = None;
    let mut count = 0_u64;
    while let Some(position) = next {
        let node = read(position)?;
        if previous.is_some_and(|count| node.count >= count) {
            return Err(invalid("event order counts do not descend"));
        }
        previous = Some(node.count);
        if node.entry.0 <= hour {
            let left = node
                .left
                .map(&mut read)
                .transpose()?
                .map_or(0, |node| node.count);
            count = count
                .checked_add(left)
                .and_then(|n| n.checked_add(1))
                .ok_or_else(|| invalid("event count overflow"))?;
            next = node.right;
        } else {
            next = node.left;
        }
    }
    Ok(count)
}
pub(super) fn select(
    root: Position,
    mut ordinal: u64,
    mut read: impl FnMut(Position) -> io::Result<Node>,
) -> io::Result<(i32, u64)> {
    let mut next = Some(root);
    let mut previous = None;
    while let Some(position) = next {
        let node = read(position)?;
        if ordinal >= node.count || previous.is_some_and(|count| node.count >= count) {
            return Err(invalid("event order ordinal is out of range"));
        }
        previous = Some(node.count);
        let left = node
            .left
            .map(&mut read)
            .transpose()?
            .map_or(0, |node| node.count);
        if ordinal == left {
            return Ok(node.entry);
        } else if ordinal < left {
            next = node.left;
        } else {
            ordinal -= left + 1;
            next = node.right;
        }
    }
    Err(invalid("event order ordinal is out of range"))
}

struct Builder<'a, W: super::super::pages::Pages + ?Sized> {
    writer: &'a mut W,
    section: &'a str,
    batch: u64,
    bytes: &'a mut Vec<u8>,
    limit: usize,
    cancelled: &'a AtomicBool,
}
impl<W: super::super::pages::Pages + ?Sized> Builder<'_, W> {
    fn node(&mut self, position: Position) -> io::Result<Node> {
        check_cancel(self.cancelled)?;
        if position.batch == self.batch {
            let start = usize::try_from(position.offset)
                .map_err(|_| invalid("event order offset overflow"))?;
            let bytes = self
                .bytes
                .get(
                    start
                        ..start
                            .checked_add(56)
                            .ok_or_else(|| invalid("event order offset overflow"))?,
                )
                .ok_or_else(|| invalid("missing event order node"))?;
            return Node::decode(bytes);
        }
        let mut bytes = [0; 56];
        self.writer
            .read_slice(self.section, position.batch, position.offset, &mut bytes)?;
        Node::decode(&bytes)
    }
    fn dimensions(&mut self, position: Option<Position>) -> io::Result<(u64, u32)> {
        position
            .map(|position| self.node(position).map(|node| (node.count, node.height)))
            .transpose()
            .map(|value| value.unwrap_or((0, 0)))
    }
    fn write(&mut self, mut node: Node) -> io::Result<Position> {
        let (left, lh) = self.dimensions(node.left)?;
        let (right, rh) = self.dimensions(node.right)?;
        node.count = left
            .checked_add(right)
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| invalid("event count overflow"))?;
        node.height = lh
            .max(rh)
            .checked_add(1)
            .ok_or_else(|| invalid("event order height overflow"))?;
        let position = Position {
            batch: self.batch,
            offset: self.bytes.len() as u64,
        };
        if self
            .bytes
            .len()
            .checked_add(56)
            .is_none_or(|end| end > self.limit)
        {
            return Err(invalid(
                "event order exceeds its admitted construction bound",
            ));
        }
        node.encode(self.bytes);
        #[cfg(test)]
        BUILT_NODES.set(BUILT_NODES.get() + 1);
        Ok(position)
    }
    fn left(&mut self, mut node: Node) -> io::Result<Position> {
        let mut right = self.node(
            node.right
                .ok_or_else(|| invalid("missing event order right child"))?,
        )?;
        node.right = right.left;
        right.left = Some(self.write(node)?);
        self.write(right)
    }
    fn right(&mut self, mut node: Node) -> io::Result<Position> {
        let mut left = self.node(
            node.left
                .ok_or_else(|| invalid("missing event order left child"))?,
        )?;
        node.left = left.right;
        left.right = Some(self.write(node)?);
        self.write(left)
    }
    fn insert(
        &mut self,
        root: Option<Position>,
        entry: (i32, u64),
        height: u32,
        depth: usize,
    ) -> io::Result<Position> {
        check_cancel(self.cancelled)?;
        #[cfg(test)]
        PEAK_DEPTH.set(PEAK_DEPTH.get().max(depth));
        let Some(root) = root else {
            return self.write(Node {
                entry,
                left: None,
                right: None,
                count: 1,
                height: 1,
            });
        };
        let mut node = self.node(root)?;
        if node.height > height {
            return Err(invalid("event order heights do not descend"));
        }
        let height = node.height - 1;
        if entry < node.entry {
            node.left = Some(self.insert(node.left, entry, height, depth + 1)?);
        } else if entry > node.entry {
            node.right = Some(self.insert(node.right, entry, height, depth + 1)?);
        } else {
            return Err(invalid("duplicate event order entry"));
        }
        let (_, lh) = self.dimensions(node.left)?;
        let (_, rh) = self.dimensions(node.right)?;
        if lh.saturating_sub(rh) > 1 {
            let left = self.node(node.left.unwrap())?;
            if self.dimensions(left.left)?.1 < self.dimensions(left.right)?.1 {
                node.left = Some(self.left(left)?);
            }
            self.right(node)
        } else if rh.saturating_sub(lh) > 1 {
            let right = self.node(node.right.unwrap())?;
            if self.dimensions(right.right)?.1 < self.dimensions(right.left)?.1 {
                node.right = Some(self.right(right)?);
            }
            self.left(node)
        } else {
            self.write(node)
        }
    }
}

#[cfg(test)]
thread_local! {
    pub(super) static BUILT_NODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static PEAK_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static CAPACITY_GROWTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn allocation_refused() -> io::Error {
    io::Error::new(
        io::ErrorKind::OutOfMemory,
        "Prepared event order allocation refused",
    )
}
fn reserve(bytes: &mut Vec<u8>, additional: usize) -> io::Result<()> {
    engine_allocation::reserve_exact(bytes, additional).map_err(|_| allocation_refused())
}

// Only backdated keys add persistent rank/select nodes and a root table; ordinary pages stay identical.
pub(super) fn publish<W: super::super::pages::Pages + ?Sized>(
    writer: &mut W,
    section: &str,
    batch: u64,
    bytes: &mut Vec<u8>,
    keys: &mut [KeyRows<'_>],
    cancelled: &AtomicBool,
) -> io::Result<usize> {
    let original = bytes.len();
    #[cfg(test)]
    let capacity = bytes.capacity();
    let mut first = i32_at(bytes, 0);
    let mut last = i32_at(bytes, 4);
    if batch != 0 && keys.iter().any(|(_, _, events, _)| !events.is_empty()) {
        let mut previous = [0; HEADER as usize];
        writer.read_slice(section, batch - 1, 0, &mut previous)?;
        first = first.max(i32_at(&previous, 4));
        last = last.max(first);
    }
    let bounds_changed = first != i32_at(bytes, 0) || last != i32_at(bytes, 4);
    put(bytes, 0, &first.to_le_bytes());
    put(bytes, 4, &last.to_le_bytes());
    let mut builder = Builder {
        writer,
        section,
        batch,
        bytes,
        limit: original,
        cancelled,
    };
    let mut additional = 0_usize;
    let mut indexed = false;
    for (_, _, events, state) in keys.iter() {
        check_cancel(cancelled)?;
        let late = events.partition_point(|row| row.0 < first);
        indexed |= state.event_order.is_some() || late != 0;
        if late == 0 {
            continue;
        }
        let (count, height) = builder.dimensions(state.event_order)?;
        let count = count
            .checked_add(late as u64)
            .ok_or_else(allocation_refused)?;
        let height = if late == 1 {
            height
        } else {
            2 * ((count - 1).ilog2() + 1)
        };
        // Each insertion writes a leaf and at most four nodes per previous AVL level.
        let nodes = late
            .checked_mul(1 + 4 * height as usize)
            .ok_or_else(allocation_refused)?;
        additional = additional
            .checked_add(nodes.checked_mul(56).ok_or_else(allocation_refused)?)
            .ok_or_else(allocation_refused)?;
    }
    if indexed {
        additional = additional
            .checked_add(keys.len().checked_mul(16).ok_or_else(allocation_refused)?)
            .ok_or_else(allocation_refused)?;
    }
    builder.limit = original
        .checked_add(additional)
        .ok_or_else(allocation_refused)?;
    reserve(builder.bytes, additional)?;
    for (index, (_, _, events, state)) in keys.iter_mut().enumerate() {
        check_cancel(cancelled)?;
        let base = state.events - events.len() as u64;
        for (index, (date, _)) in events
            .iter()
            .enumerate()
            .take_while(|(_, row)| row.0 < first)
        {
            let height = builder.dimensions(state.event_order)?.1;
            state.event_order =
                Some(builder.insert(state.event_order, (*date, base + index as u64), height, 1)?);
        }
        if state.event_order.is_some() {
            put(
                builder.bytes,
                HEADER as usize + index * DIRECTORY as usize + 60,
                &2_u32.to_le_bytes(),
            );
        }
    }
    if keys
        .iter()
        .any(|(_, _, _, state)| state.event_order.is_some())
    {
        let roots = keys.len().checked_mul(16).ok_or_else(allocation_refused)?;
        if builder
            .bytes
            .len()
            .checked_add(roots)
            .is_none_or(|end| end > builder.limit)
        {
            return Err(invalid(
                "event order roots exceed their admitted construction bound",
            ));
        }
        for (_, _, _, state) in keys {
            match state.event_order {
                Some(root) => root.encode(builder.bytes),
                None => builder.bytes.extend_from_slice(&[0; 16]),
            }
        }
    }
    if bounds_changed || builder.bytes.len() != original {
        builder.writer.write_at(section, batch, builder.bytes)?;
    }
    #[cfg(test)]
    CAPACITY_GROWTH.set(
        CAPACITY_GROWTH
            .get()
            .max(builder.bytes.capacity().saturating_sub(capacity)),
    );
    Ok(builder.bytes.len() - original)
}

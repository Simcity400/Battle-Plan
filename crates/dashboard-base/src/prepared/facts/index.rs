use super::*;
use pages::{Admitted, Cache};
use std::{ops::Deref, rc::Rc};

const FANOUT: usize = 64;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct IndexRoot {
    page: Option<u64>,
    pub count: u64,
}
impl Resident for IndexRoot {
    fn heap_bytes(&self) -> usize {
        0
    }
    fn decode_budget() -> DecodeBudget {
        DecodeBudget {
            fixed_bytes: size_of::<Self>(),
            bytes_per_encoded_byte: 0,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) enum Node<K, V> {
    Leaf(Vec<(K, V)>),
    Branch(Vec<(K, u64)>),
}
impl<K: Resident, V: Resident> Resident for Node<K, V> {
    fn heap_bytes(&self) -> usize {
        match self {
            Self::Leaf(rows) => rows.heap_bytes(),
            Self::Branch(rows) => rows.heap_bytes(),
        }
    }
    fn decode_budget() -> DecodeBudget {
        let a = Vec::<(K, V)>::decode_budget();
        let b = Vec::<(K, u64)>::decode_budget();
        DecodeBudget {
            fixed_bytes: a
                .fixed_bytes
                .max(b.fixed_bytes)
                .saturating_add(size_of::<Self>()),
            bytes_per_encoded_byte: a.bytes_per_encoded_byte.max(b.bytes_per_encoded_byte),
        }
    }
}
impl<K: Ord, V> Node<K, V> {
    fn valid(&self) -> io::Result<()> {
        let valid = match self {
            Self::Leaf(rows) => !rows.is_empty() && rows.windows(2).all(|w| w[0].0 < w[1].0),
            Self::Branch(rows) => rows.len() >= 2 && rows.windows(2).all(|w| w[0].0 < w[1].0),
        };
        if valid {
            Ok(())
        } else {
            Err(invalid("Invalid private key index page"))
        }
    }
    fn first(&self) -> &K {
        match self {
            Self::Leaf(rows) => &rows[0].0,
            Self::Branch(rows) => &rows[0].0,
        }
    }
    fn len(&self) -> usize {
        match self {
            Self::Leaf(rows) => rows.len(),
            Self::Branch(rows) => rows.len(),
        }
    }
}
pub(super) struct Item<K, V> {
    page: Rc<Admitted<Node<K, V>>>,
    at: usize,
}
impl<K, V> Clone for Item<K, V> {
    fn clone(&self) -> Self {
        Self {
            page: self.page.clone(),
            at: self.at,
        }
    }
}
impl<K, V> Deref for Item<K, V> {
    type Target = (K, V);
    fn deref(&self) -> &Self::Target {
        match &self.page.value {
            Node::Leaf(rows) => &rows[self.at],
            _ => unreachable!(),
        }
    }
}
pub(super) struct IndexCursor<K, V> {
    section: String,
    root: IndexRoot,
    current: Option<Item<K, V>>,
    last: Option<K>,
    _credit: Credit,
}
fn seek<K, V, R>(
    cache: &mut Cache<'_, R>,
    section: &str,
    page: u64,
    first: Option<(&K, bool)>,
) -> io::Result<Option<Item<K, V>>>
where
    K: Ord + Clone + Serialize + DeserializeOwned + Resident + 'static,
    V: Copy + Serialize + DeserializeOwned + Resident + 'static,
    R: ReadPages,
{
    let node: Rc<Admitted<Node<K, V>>> = cache.read(section, page)?;
    node.valid()?;
    match &node.value {
        Node::Leaf(rows) => {
            let at = first.map_or(0, |(key, exclusive)| {
                rows.partition_point(|(k, _)| k < key || (exclusive && k == key))
            });
            Ok((at < rows.len()).then_some(Item { page: node, at }))
        }
        Node::Branch(rows) => {
            let start = first.map_or(0, |(key, _)| {
                rows.partition_point(|(k, _)| k <= key).saturating_sub(1)
            });
            let len = rows.len();
            let child = rows[start].1;
            drop(node);
            if let Some(found) = seek(cache, section, child, first)? {
                return Ok(Some(found));
            }
            for at in start + 1..len {
                let node: Rc<Admitted<Node<K, V>>> = cache.read(section, page)?;
                let Node::Branch(rows) = &node.value else {
                    return Err(invalid("Private key index changed"));
                };
                let child = rows[at].1;
                drop(node);
                if let Some(found) = seek(cache, section, child, None)? {
                    return Ok(Some(found));
                }
            }
            Ok(None)
        }
    }
}
pub(super) fn get<K, V, R>(
    cache: &mut Cache<'_, R>,
    section: &str,
    root: IndexRoot,
    key: &K,
) -> io::Result<Option<V>>
where
    K: Ord + Clone + Serialize + DeserializeOwned + Resident + 'static,
    V: Copy + Serialize + DeserializeOwned + Resident + 'static,
    R: ReadPages,
{
    let Some(mut page) = root.page else {
        return Ok(None);
    };
    loop {
        let node: Rc<Admitted<Node<K, V>>> = cache.read(section, page)?;
        node.valid()?;
        match &node.value {
            Node::Leaf(rows) => {
                return Ok(rows
                    .binary_search_by(|(k, _)| k.cmp(key))
                    .ok()
                    .map(|at| rows[at].1));
            }
            Node::Branch(rows) => {
                page = rows[rows.partition_point(|(k, _)| k <= key).saturating_sub(1)].1
            }
        }
    }
}
impl<K, V> IndexCursor<K, V>
where
    K: Ord + Clone + Serialize + DeserializeOwned + Resident + 'static,
    V: Copy + Serialize + DeserializeOwned + Resident + 'static,
{
    pub fn new<R: ReadPages>(
        cache: &mut Cache<'_, R>,
        section: String,
        root: IndexRoot,
        first: Option<&K>,
        last: Option<&K>,
    ) -> io::Result<Self> {
        if first.zip(last).is_some_and(|(a, b)| a > b) {
            return Err(invalid("Private key range is reversed"));
        }
        let credit = cache
            .admit(size_of::<Self>() + section.capacity() + last.map_or(0, Resident::heap_bytes))?;
        let current = match root.page {
            Some(page) => seek(cache, &section, page, first.map(|k| (k, false)))?,
            None => None,
        };
        Ok(Self {
            section,
            root,
            current,
            last: last.cloned(),
            _credit: credit,
        })
    }
    pub fn next<R: ReadPages>(
        &mut self,
        cache: &mut Cache<'_, R>,
    ) -> io::Result<Option<Item<K, V>>> {
        loop {
            let Some(found) = self.current.as_ref() else {
                return Ok(None);
            };
            let Node::Leaf(rows) = &found.page.value else {
                unreachable!()
            };
            if found.at < rows.len() {
                if self.last.as_ref().is_some_and(|last| found.0 > *last) {
                    self.current = None;
                    return Ok(None);
                }
                let result = found.clone();
                self.current.as_mut().unwrap().at += 1;
                return Ok(Some(result));
            }
            let next = seek(
                cache,
                &self.section,
                self.root.page.unwrap(),
                Some((&rows.last().unwrap().0, true)),
            )?;
            self.current = next;
        }
    }
}
struct Written<K> {
    pages: Admitted<Vec<(K, u64)>>,
    inserted: bool,
}
fn write_node<K, V>(
    writer: &mut store::Writer,
    budget: &MemoryBudget,
    section: &str,
    node: Node<K, V>,
    page_bytes: usize,
    inserted: bool,
) -> io::Result<Written<K>>
where
    K: Ord + Clone + Serialize + DeserializeOwned + Resident + 'static,
    V: Copy + Serialize + DeserializeOwned + Resident + 'static,
{
    let encoded = pages::encode(budget, &node)?;
    let split = node.len() > FANOUT
        || (encoded.len() > page_bytes
            && node.len()
                >= match node {
                    Node::Leaf(_) => 2,
                    Node::Branch(_) => 4,
                });
    drop(encoded);
    let mut parts = [Some(node), None];
    if split {
        let node = parts[0].as_mut().unwrap();
        let mid = node.len() / 2;
        parts[1] = Some(match node {
            Node::Leaf(rows) => Node::Leaf(rows.split_off(mid)),
            Node::Branch(rows) => Node::Branch(rows.split_off(mid)),
        });
    }
    let bytes = parts.iter().flatten().fold(
        size_of::<Written<K>>().saturating_add(2 * size_of::<(K, u64)>()),
        |bytes, node| bytes.saturating_add(node.first().heap_bytes()),
    );
    let credit = budget.admit(bytes)?;
    let mut output = Vec::with_capacity(2);
    for node in parts.into_iter().flatten() {
        let encoded = pages::encode(budget, &node)?;
        let id = pages::write(writer, budget, section, encoded.as_slice())?;
        output.push((node.first().clone(), id));
    }
    Ok(Written {
        pages: Admitted {
            value: output,
            _credit: credit,
        },
        inserted,
    })
}
fn put_node<K, V>(
    writer: &mut store::Writer,
    budget: &MemoryBudget,
    section: &str,
    page: u64,
    key: &K,
    value: V,
    page_bytes: usize,
) -> io::Result<Written<K>>
where
    K: Ord + Clone + Serialize + DeserializeOwned + Resident + 'static,
    V: Copy + Serialize + DeserializeOwned + Resident + 'static,
{
    let node: Rc<Admitted<Node<K, V>>> = pages::read(writer, budget, section, page)?;
    node.valid()?;
    let child = match &node.value {
        Node::Branch(rows) => Some(rows.partition_point(|(k, _)| k <= key).saturating_sub(1)),
        _ => None,
    };
    let children = if let Some(at) = child {
        let Node::Branch(rows) = &node.value else {
            unreachable!()
        };
        let id = rows[at].1;
        drop(node);
        Some(put_node(
            writer, budget, section, id, key, value, page_bytes,
        )?)
    } else {
        drop(node);
        None
    };
    let node: Rc<Admitted<Node<K, V>>> = pages::read(writer, budget, section, page)?;
    let work = budget.admit(
        node.heap_bytes()
            .saturating_mul(3)
            .saturating_add(key.heap_bytes().saturating_mul(4))
            .saturating_add(
                children
                    .as_ref()
                    .map_or(0, |c| c.pages.heap_bytes())
                    .saturating_mul(2),
            )
            .saturating_add(
                (node.len() + 4).saturating_mul(4 * (size_of::<K>() + size_of::<V>() + 32)),
            ),
    )?;
    let mut changed = node.value.clone();
    drop(node);
    let inserted = match &mut changed {
        Node::Leaf(rows) => match rows.binary_search_by(|(k, _)| k.cmp(key)) {
            Ok(at) => {
                rows[at].1 = value;
                false
            }
            Err(at) => {
                rows.insert(at, (key.clone(), value));
                true
            }
        },
        Node::Branch(rows) => {
            let children = children.as_ref().unwrap();
            let at = child.unwrap();
            rows.splice(at..at + 1, children.pages.iter().cloned());
            children.inserted
        }
    };
    let result = write_node(writer, budget, section, changed, page_bytes, inserted);
    drop(work);
    result
}
pub(super) fn put<K, V>(
    writer: &mut store::Writer,
    budget: &MemoryBudget,
    section: &str,
    root: IndexRoot,
    key: &K,
    value: V,
    page_bytes: usize,
) -> io::Result<IndexRoot>
where
    K: Ord + Clone + Serialize + DeserializeOwned + Resident + 'static,
    V: Copy + PartialEq + Serialize + DeserializeOwned + Resident + 'static,
{
    if let Some(page) = root.page {
        let mut cache = Cache::new(writer, budget.clone(), 0)?;
        if get(&mut cache, section, root, key)? == Some(value) {
            return Ok(root);
        }
        drop(cache);
        let written = put_node(writer, budget, section, page, key, value, page_bytes)?;
        finish::<K, V>(writer, budget, section, root, written)
    } else {
        let _work = budget.admit(key.heap_bytes().saturating_add(4 * size_of::<(K, V)>()))?;
        let written = write_node(
            writer,
            budget,
            section,
            Node::Leaf(vec![(key.clone(), value)]),
            page_bytes,
            true,
        )?;
        finish::<K, V>(writer, budget, section, root, written)
    }
}
fn finish<K, V>(
    writer: &mut store::Writer,
    budget: &MemoryBudget,
    section: &str,
    root: IndexRoot,
    written: Written<K>,
) -> io::Result<IndexRoot>
where
    K: Ord + Clone + Serialize + DeserializeOwned + Resident + 'static,
    V: Copy + Serialize + DeserializeOwned + Resident + 'static,
{
    let count = root
        .count
        .checked_add(u64::from(written.inserted))
        .ok_or_else(|| invalid("Private key count overflow"))?;
    let page = if written.pages.len() == 1 {
        written.pages[0].1
    } else {
        let _work = budget.admit(written.pages.heap_bytes())?;
        let parent = Node::<K, V>::Branch(written.pages.value.clone());
        let encoded = pages::encode(budget, &parent)?;
        pages::write(writer, budget, section, encoded.as_slice())?
    };
    Ok(IndexRoot {
        page: Some(page),
        count,
    })
}

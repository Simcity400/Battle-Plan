use super::*;
use crate::prepared::pages::Pages;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Root(Option<u64>);

// S2 replaces this byte-keyed interface; family folds never inspect tree pages.
pub(super) trait Facts {
    fn get(&mut self, root: Root, key: &str) -> io::Result<Option<Vec<u8>>>;
    fn put(&mut self, root: &mut Root, key: &str, value: Vec<u8>) -> io::Result<()>;
    fn next(
        &mut self,
        root: Root,
        after: &str,
        prefix: &str,
    ) -> io::Result<Option<(String, Vec<u8>)>>;
    #[cfg(test)]
    fn visit(
        &mut self,
        root: Root,
        prefix: &str,
        visitor: &mut dyn FnMut(&str, &[u8]) -> io::Result<()>,
    ) -> io::Result<()>;
}

pub(super) fn get<T: serde::de::DeserializeOwned>(
    facts: &mut dyn Facts,
    root: Root,
    key: &str,
) -> io::Result<Option<T>> {
    facts
        .get(root, key)?
        .map(|bytes| serde_json::from_slice(&bytes).map_err(io::Error::other))
        .transpose()
}
pub(super) fn put(
    facts: &mut dyn Facts,
    root: &mut Root,
    key: &str,
    value: &impl Serialize,
) -> io::Result<()> {
    facts.put(root, key, serde_json::to_vec(value)?)
}

#[derive(Serialize, Deserialize)]
struct Node {
    key: String,
    value: Vec<u8>,
    left: Root,
    right: Root,
}

pub(super) struct Store<'a, W: Pages + ?Sized> {
    pub pages: &'a mut W,
    section: String,
    pub branch: u64,
}
impl<'a, W: Pages + ?Sized> Store<'a, W> {
    pub fn new(pages: &'a mut W, branch: u64) -> Self {
        Self {
            pages,
            branch,
            section: format!("production-lines-private-v2-{branch}"),
        }
    }
    fn read(&mut self, root: Root) -> io::Result<Option<Node>> {
        root.0
            .map(|range| {
                let bytes = self
                    .pages
                    .read_at(&self.section, range)?
                    .ok_or_else(|| crate::invalid("Missing production private index page"))?;
                pages::read(&bytes)
            })
            .transpose()
    }
    fn write(&mut self, node: &Node) -> io::Result<Root> {
        let bytes = crate::prepared::series::compress(&serde_json::to_vec(node)?, 3)?;
        Ok(Root(Some(self.pages.write(&self.section, &bytes)?)))
    }
    fn insert(&mut self, root: Root, key: &str, value: Vec<u8>) -> io::Result<Root> {
        let Some(mut node) = self.read(root)? else {
            return self.write(&Node {
                key: key.into(),
                value,
                left: Root::default(),
                right: Root::default(),
            });
        };
        match key.cmp(&node.key) {
            std::cmp::Ordering::Equal => node.value = value,
            std::cmp::Ordering::Less => {
                node.left = self.insert(node.left, key, value)?;
                let mut left = self.read(node.left)?.expect("inserted private page");
                if priority(&left.key) < priority(&node.key) {
                    node.left = left.right;
                    left.right = self.write(&node)?;
                    return self.write(&left);
                }
            }
            std::cmp::Ordering::Greater => {
                node.right = self.insert(node.right, key, value)?;
                let mut right = self.read(node.right)?.expect("inserted private page");
                if priority(&right.key) < priority(&node.key) {
                    node.right = right.left;
                    right.left = self.write(&node)?;
                    return self.write(&right);
                }
            }
        }
        self.write(&node)
    }
}
fn priority(key: &str) -> (u64, &str) {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in key.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    hash ^= hash >> 30;
    hash = hash.wrapping_mul(0xbf58476d1ce4e5b9);
    hash ^= hash >> 27;
    hash = hash.wrapping_mul(0x94d049bb133111eb);
    (hash ^ (hash >> 31), key)
}
impl<W: Pages + ?Sized> Facts for Store<'_, W> {
    fn get(&mut self, mut root: Root, key: &str) -> io::Result<Option<Vec<u8>>> {
        while let Some(node) = self.read(root)? {
            root = match key.cmp(&node.key) {
                std::cmp::Ordering::Less => node.left,
                std::cmp::Ordering::Greater => node.right,
                std::cmp::Ordering::Equal => return Ok(Some(node.value)),
            };
        }
        Ok(None)
    }
    fn put(&mut self, root: &mut Root, key: &str, value: Vec<u8>) -> io::Result<()> {
        if self.get(*root, key)?.as_ref() == Some(&value) {
            return Ok(());
        }
        *root = self.insert(*root, key, value)?;
        Ok(())
    }
    fn next(
        &mut self,
        mut root: Root,
        after: &str,
        prefix: &str,
    ) -> io::Result<Option<(String, Vec<u8>)>> {
        let mut found = None;
        while let Some(node) = self.read(root)? {
            if node.key.as_str() > after && node.key.as_str() >= prefix {
                root = node.left;
                found = Some((node.key, node.value));
            } else {
                root = node.right;
            }
        }
        Ok(found.filter(|(key, _)| key.starts_with(prefix)))
    }
    #[cfg(test)]
    fn visit(
        &mut self,
        root: Root,
        prefix: &str,
        visitor: &mut dyn FnMut(&str, &[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        if let Some(node) = self.read(root)? {
            if node.key.as_str() >= prefix {
                self.visit(node.left, prefix, visitor)?;
            }
            if node.key.starts_with(prefix) {
                visitor(&node.key, &node.value)?;
            }
            if node.key.as_str() <= prefix || node.key.starts_with(prefix) {
                self.visit(node.right, prefix, visitor)?;
            }
        }
        Ok(())
    }
}

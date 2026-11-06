use super::*;
use std::io::Read;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    namespace: Namespace,
    pub(super) range: u64,
    pub cursor: u64,
    keys: index::IndexRoot,
}
struct Stream<'a, K, V> {
    rows: std::collections::btree_map::Iter<'a, K, Column<V>>,
    head: Vec<u8>,
    head_at: usize,
    value: Option<Vec<u8>>,
    at: usize,
}
impl<K: Serialize, V: Serialize> Read for Stream<'_, K, V> {
    fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
        if into.is_empty() {
            return Ok(0);
        }
        loop {
            if self.head_at < self.head.len() {
                let count = into.len().min(self.head.len() - self.head_at);
                into[..count].copy_from_slice(&self.head[self.head_at..self.head_at + count]);
                self.head_at += count;
                return Ok(count);
            }
            if let Some(value) = &self.value {
                if self.at < value.len() {
                    let count = into.len().min(value.len() - self.at);
                    into[..count].copy_from_slice(&value[self.at..self.at + count]);
                    self.at += count;
                    return Ok(count);
                }
                self.value = None;
            }
            let Some((key, column)) = self.rows.next() else {
                return Ok(0);
            };
            let value = pages::encode(&(key, column.root, column.open.as_ref().map(|v| &v.interval)))?;
            self.head.clear();
            self.head
                .extend_from_slice(&(value.len() as u64).to_le_bytes());
            self.head_at = 0;
            self.at = 0;
            self.value = Some(value);
        }
    }
}
impl<
    K: Ord + Clone + Serialize + DeserializeOwned + Resident + 'static,
    V: PartialEq + Serialize + DeserializeOwned + Resident + 'static,
> Facts<K, V>
{
    pub fn checkpoint(
        &mut self,
        writer: &mut store::Writer,
        cursor: u64,
    ) -> io::Result<Checkpoint> {
        self.spill(writer)?;
        for (key, column) in &mut self.columns {
            if column.indexed {
                continue;
            }
            let result = index::put(
                writer,
                &format!("{}/keys", self.namespace.section()),
                self.keys,
                key,
                column.root,
                self.limits.page_bytes,
            );
            match result {
                Ok(root) => {
                    self.keys = root;
                    column.indexed = true;
                }
                Err(error) => {
                    self.poisoned = true;
                    return Err(error);
                }
            }
        }
        let mut head = b"H4FACT03".to_vec();
        head.extend_from_slice(&(self.columns.len() as u64).to_le_bytes());
        let stream = Stream {
            rows: self.columns.iter(),
            head,
            head_at: 0,
            value: None,
            at: 0,
        };
        let result = pages::write(
            writer,
            &format!("{}/checkpoints", self.namespace.section()),
            stream,
        );
        match result {
            Ok(range) => Ok(Checkpoint {
                namespace: self.namespace.clone(),
                range,
                cursor,
                keys: self.keys,
            }),
            Err(error) => {
                self.poisoned = true;
                Err(error)
            }
        }
    }
    pub fn restore<R: ReadPages>(
        source: &mut R,
        checkpoint: &Checkpoint,
        limits: Limits,
    ) -> io::Result<Self> {
        let mut facts = Self::new(checkpoint.namespace.clone(), limits)?;
        facts.keys = checkpoint.keys;
        let section = format!("{}/checkpoints", facts.namespace.section());
        let input = source
            .open(&section, checkpoint.range)?
            .ok_or_else(|| invalid("Private fact checkpoint missing"))?;
        let mut input = io::BufReader::new(input);
        let mut magic = [0; 8];
        input.read_exact(&mut magic)?;
        if &magic != b"H4FACT03" {
            return Err(invalid("Private fact checkpoint schema changed"));
        }
        let count = word(&mut input)?;
        for _ in 0..count {
            let len = word(&mut input)?;
            let bytes = usize::try_from(len)
                .map_err(|_| invalid("Private fact checkpoint row too large"))?;
            let mut take = (&mut input).take(len);
            let (key, root, open): (K, Root, Option<Interval<V>>) =
                serde_json::from_reader(&mut take)?;
            if take.limit() != 0 {
                return Err(invalid("Private fact checkpoint row truncated"));
            }
            if (root.index.count == 0) != root.last.is_none()
                || open.as_ref().is_some_and(|v| {
                    v.first > v.last || root.last.is_some_and(|last| v.first <= last)
                })
                || facts.columns.contains_key(&key)
            {
                return Err(invalid("Invalid private fact checkpoint roots"));
            }
            let open = open.map(Self::held);
            facts.columns.insert(
                key,
                Column {
                    root,
                    indexed: true,
                    pending: Vec::new(),
                    open,
                },
            );
        }
        let mut extra = [0];
        if input.read(&mut extra)? != 0 {
            return Err(invalid("Private fact checkpoint has trailing bytes"));
        }
        Ok(facts)
    }
}
fn word(input: &mut impl Read) -> io::Result<u64> {
    let mut word = [0; 8];
    input.read_exact(&mut word)?;
    Ok(u64::from_le_bytes(word))
}

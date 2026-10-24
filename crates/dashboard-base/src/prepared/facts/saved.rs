use super::*;
use pages::Admitted;
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
    budget: &'a MemoryBudget,
    head: Vec<u8>,
    head_at: usize,
    value: Option<Admitted<Vec<u8>>>,
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
            let value = pages::encode(
                self.budget,
                &(key, column.root, column.open.as_ref().map(|v| &v.interval)),
            )?;
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
                &self.budget,
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
            budget: &self.budget,
            head,
            head_at: 0,
            value: None,
            at: 0,
        };
        let result = pages::write(
            writer,
            &self.budget,
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
        budget: MemoryBudget,
        limits: Limits,
    ) -> io::Result<Self> {
        let mut facts = Self::new(checkpoint.namespace.clone(), budget, limits)?;
        facts.keys = checkpoint.keys;
        let section = format!("{}/checkpoints", facts.namespace.section());
        let input = source
            .open(&section, checkpoint.range)?
            .ok_or_else(|| invalid("Private fact checkpoint missing"))?;
        let _scratch = facts.budget.admit(8192)?;
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
            let reserve = K::decode_budget()
                .for_encoded(bytes)?
                .checked_add(Option::<Interval<V>>::decode_budget().for_encoded(bytes)?)
                .and_then(|n| n.checked_add(size_of::<Root>()))
                .ok_or_else(|| invalid("Fact allocation overflow"))?;
            let mut credit = facts.budget.admit(reserve)?;
            let mut take = (&mut input).take(len);
            let (key, root, open): (K, Root, Option<Interval<V>>) =
                serde_json::from_reader(&mut take)?;
            if take.limit() != 0 {
                return Err(invalid("Private fact checkpoint row truncated"));
            }
            let actual = size_of::<(K, Root, Option<Interval<V>>)>()
                .saturating_add(key.heap_bytes())
                .saturating_add(open.heap_bytes());
            if actual > reserve {
                return Err(invalid(
                    "Private fact Resident checkpoint decode bound was too small",
                ));
            }
            if (root.index.count == 0) != root.last.is_none()
                || open.as_ref().is_some_and(|v| {
                    v.first > v.last || root.last.is_some_and(|last| v.first <= last)
                })
                || facts.columns.contains_key(&key)
            {
                return Err(invalid("Invalid private fact checkpoint roots"));
            }
            let key_bytes = key
                .heap_bytes()
                .saturating_add((size_of::<K>() + size_of::<Column<V>>() + 128).saturating_mul(12));
            let open_bytes = open.as_ref().map_or(0, |v| {
                v.heap_bytes()
                    .saturating_add(size_of::<Held<V>>().saturating_mul(4))
            });
            credit.resize(key_bytes.saturating_add(open_bytes))?;
            let key_credit = credit.split_off(key_bytes);
            let open = open.map(|interval| Held { interval, credit });
            facts.columns.insert(
                key,
                Column {
                    root,
                    indexed: true,
                    pending: Vec::new(),
                    open,
                    _credit: key_credit,
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

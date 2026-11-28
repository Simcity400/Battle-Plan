
use std::{io, sync::Arc};

fn damaged() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "a kept buffer is damaged")
}

pub fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend((bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

pub fn put_texts(out: &mut Vec<u8>, texts: &[String]) {
    out.extend((texts.len() as u64).to_le_bytes());
    for text in texts {
        put_bytes(out, text.as_bytes());
    }
}

pub struct Input<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Input<'a> {
    pub fn new(bytes: &'a [u8], at: usize) -> Self {
        Self { bytes, at }
    }

    pub fn at(&self) -> usize {
        self.at
    }

    pub fn take(&mut self, length: usize) -> io::Result<&'a [u8]> {
        let end = self.at.checked_add(length).ok_or_else(damaged)?;
        let taken = self.bytes.get(self.at..end).ok_or_else(damaged)?;
        self.at = end;
        Ok(taken)
    }

    pub fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().map_err(|_| damaged())?,
        ))
    }

    fn count(&mut self, least: usize) -> io::Result<usize> {
        let count = usize::try_from(self.u64()?).map_err(|_| damaged())?;
        let rest = self.bytes.len().saturating_sub(self.at);
        (count <= rest / least.max(1))
            .then_some(count)
            .ok_or_else(damaged)
    }

    pub fn bytes(&mut self) -> io::Result<&'a [u8]> {
        let length = self.count(1)?;
        self.take(length)
    }

    pub fn text(&mut self) -> io::Result<String> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|_| damaged())
    }

    pub fn texts(&mut self) -> io::Result<Vec<String>> {
        let count = self.count(8)?;
        (0..count).map(|_| self.text()).collect()
    }
}

#[derive(Clone)]
pub struct Table {
    bytes: Arc<Vec<u8>>,
    count: usize,
    ends: usize,
    data: usize,
}

impl std::fmt::Debug for Table {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Table")
            .field("count", &self.count)
            .finish()
    }
}

impl Table {
    const ENTRY_BYTES: usize = 2 * size_of::<u64>();

    pub fn pack<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        entries: impl ExactSizeIterator<Item = (K, V)>,
        payload_bytes: usize,
    ) -> io::Result<Self> {
        let header = size_of::<u64>() + Self::ENTRY_BYTES * entries.len();
        let mut bytes = Vec::with_capacity(header + payload_bytes);
        Self::write(&mut bytes, entries);
        Self::read(&Arc::new(bytes), 0).map(|(table, _)| table)
    }

    pub fn write<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        out: &mut Vec<u8>,
        entries: impl ExactSizeIterator<Item = (K, V)>,
    ) {
        let count = entries.len();
        out.extend((count as u64).to_le_bytes());
        let ends = out.len();
        out.resize(ends + Self::ENTRY_BYTES * count, 0);
        let data = out.len();
        let mut written = 0;
        for (index, (key, value)) in entries.enumerate() {
            out.extend_from_slice(key.as_ref());
            let key_end = ((out.len() - data) as u64).to_le_bytes();
            out.extend_from_slice(value.as_ref());
            let value_end = ((out.len() - data) as u64).to_le_bytes();
            let at = ends + Self::ENTRY_BYTES * index;
            out[at..at + 8].copy_from_slice(&key_end);
            out[at + 8..at + 16].copy_from_slice(&value_end);
            written += 1;
        }
        debug_assert_eq!(written, count, "an exact-size iterator yields its length");
    }

    pub fn read(bytes: &Arc<Vec<u8>>, at: usize) -> io::Result<(Self, usize)> {
        let mut input = Input::new(bytes, at);
        let count = usize::try_from(input.u64()?).map_err(|_| damaged())?;
        let ends = input.at();
        let data = count
            .checked_mul(Self::ENTRY_BYTES)
            .and_then(|length| ends.checked_add(length))
            .filter(|data| *data <= bytes.len())
            .ok_or_else(damaged)?;
        let table = Self {
            bytes: Arc::clone(bytes),
            count,
            ends,
            data,
        };
        let length = match count.checked_sub(1) {
            Some(last) => table.end(2 * last + 1).ok_or_else(damaged)?,
            None => 0,
        };
        let end = data
            .checked_add(length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(damaged)?;
        Ok((table, end))
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn end(&self, slot: usize) -> Option<usize> {
        let at = self.ends.checked_add(slot.checked_mul(8)?)?;
        let end = u64::from_le_bytes(self.bytes.get(at..at.checked_add(8)?)?.try_into().ok()?);
        usize::try_from(end).ok()
    }

    fn slot(&self, slot: usize) -> Option<&[u8]> {
        let start = match slot.checked_sub(1) {
            Some(before) => self.end(before)?,
            None => 0,
        };
        let end = self.end(slot)?;
        self.bytes
            .get(self.data.checked_add(start)?..self.data.checked_add(end)?)
    }

    pub fn key(&self, index: usize) -> Option<&[u8]> {
        if index < self.count {
            self.slot(2 * index)
        } else {
            None
        }
    }

    pub fn value(&self, index: usize) -> Option<&[u8]> {
        if index < self.count {
            self.slot(2 * index + 1)
        } else {
            None
        }
    }

    pub fn find(&self, key: &[u8]) -> Option<usize> {
        let (mut low, mut high) = (0, self.count);
        while low < high {
            let middle = low + (high - low) / 2;
            match self.key(middle)?.cmp(key) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => return Some(middle),
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_reads_back_in_place_and_refuses_a_cut_one() {
        let entries = [("", "empty key"), ("A", ""), ("B_KEY", "ünïcode"), ("b", "lower")];
        let mut out = b"head".to_vec();
        put_texts(&mut out, &["one".to_owned(), String::new()]);
        let at = out.len();
        Table::write(&mut out, entries.iter().map(|(k, v)| (k.as_bytes(), v.as_bytes())));
        out.extend(b"tail");
        let bytes = Arc::new(out);
        let mut input = Input::new(&bytes, 4);
        assert_eq!(input.texts().unwrap(), ["one", ""]);
        assert_eq!(input.at(), at);
        let (table, end) = Table::read(&bytes, at).unwrap();
        assert_eq!(&bytes[end..], b"tail");
        assert_eq!(table.len(), entries.len());
        for (index, (key, value)) in entries.iter().enumerate() {
            assert_eq!(table.find(key.as_bytes()), Some(index));
            assert_eq!(table.key(index), Some(key.as_bytes()));
            assert_eq!(table.value(index), Some(value.as_bytes()));
        }
        assert_eq!(table.find(b"AB"), None);
        assert_eq!(table.find(b"c"), None);
        assert_eq!(table.key(entries.len()), None);
        for cut in at..end {
            let short = Arc::new(bytes[..cut].to_vec());
            assert!(Table::read(&short, at).is_err(), "cut at {cut}");
        }
        let (empty, _) = {
            let mut out = Vec::new();
            Table::write(&mut out, std::iter::empty::<(&[u8], &[u8])>());
            Table::read(&Arc::new(out), 0).unwrap()
        };
        assert!(empty.is_empty());
        assert_eq!(empty.find(b""), None);
    }
}

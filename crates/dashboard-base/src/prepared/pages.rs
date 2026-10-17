//! Where prepared pages are written ([`Pages`]): the store's open transaction,
//! or pages held for a family's next commit ([`HeldPages`]). A family
//! prepared on its own publishes each block at the block's end, as hourly
//! preparation does, into pages it holds until its commit: they answer the
//! family's reads of what earlier blocks of the batch wrote, its committed
//! pages answer the rest, and its commit writes them, in order, into the
//! transaction it commits.

use std::{
    collections::{BTreeMap, HashMap},
    io::{self, Read},
};

/// The production publication budget, shared by independent family folds.
pub const BATCH_BYTES: usize = 64 * 1024 * 1024;

/// Whether this block publishes the family's pending rows and held pages.
pub fn publication_due(
    continues: bool,
    measured: bool,
    full: bool,
    bytes: impl FnOnce() -> (usize, usize),
) -> bool {
    !continues || measured || full || {
        let (pending, held) = bytes();
        pending.saturating_add(held) >= BATCH_BYTES
    }
}

/// A writer of prepared pages.
pub trait Pages {
    /// Write `range` of `section`: one it holds, or the next.
    fn write_at(&mut self, section: &str, range: u64, data: &[u8]) -> io::Result<()>;
    /// Append a range to `section`; its range number.
    fn write(&mut self, section: &str, data: &[u8]) -> io::Result<u64>;
    /// The latest bytes of `range` of `section`, written or committed.
    fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Vec<u8>>>;
    /// Immutable identities of the first `count` slots, including missing
    /// ones. None disables publication reuse for a page writer that cannot
    /// authenticate its current ranges without reading their payloads.
    fn range_ids(
        &mut self,
        _section: &str,
        _count: u64,
    ) -> io::Result<Option<Vec<Option<store::RangeId>>>> {
        Ok(None)
    }
}

impl Pages for store::Writer {
    fn write_at(&mut self, section: &str, range: u64, data: &[u8]) -> io::Result<()> {
        store::Writer::write_at(self, section, range, data)
    }

    fn write(&mut self, section: &str, data: &[u8]) -> io::Result<u64> {
        store::Writer::write(self, section, data)
    }

    fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Vec<u8>>> {
        store::Writer::read_at(self, section, range)?
            .map(|mut range| {
                let mut bytes = Vec::new();
                range.read_to_end(&mut bytes)?;
                Ok(bytes)
            })
            .transpose()
    }
    fn range_ids(
        &mut self,
        section: &str,
        count: u64,
    ) -> io::Result<Option<Vec<Option<store::RangeId>>>> {
        (0..count)
            .map(|range| self.range_id(section, range))
            .collect::<io::Result<Vec<_>>>()
            .map(Some)
    }
}

/// One held write: an explicit range, or an append.
enum Write {
    At(String, u64, Vec<u8>),
    Append(String, u64, Vec<u8>),
}

/// Pages written since a family's last commit, in order.
#[derive(Default)]
pub struct HeldPages {
    writes: Vec<Write>,
    /// The held write of each range, explicit or appended, by its position
    /// in `writes`.
    at: HashMap<(String, u64), usize>,
    /// Each section's range count: committed, then as held writes grow it.
    counts: BTreeMap<String, u64>,
    bytes: usize,
}

/// The pages committed before the held ones: a family's committed store
/// pages, read as its state is loaded ([`HeldPages::view`]).
pub struct Committed<'a> {
    pub store: &'a store::Store,
    pub key: &'a store::SourceKey,
    /// The last committed preparation; None before any.
    pub saved: Option<&'a store::SourceKey>,
}

impl Committed<'_> {
    fn read(&self, section: &str, range: u64) -> io::Result<Option<Vec<u8>>> {
        let file = match self.saved {
            None => return Ok(None),
            Some(previous) if previous != self.key => {
                self.store
                    .read_prefix(previous, self.key, section, range)?
            }
            Some(_) => self.store.read(self.key, section, range)?,
        };
        file.map(|mut file| {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            Ok(bytes)
        })
        .transpose()
    }

    fn count(&self, section: &str) -> io::Result<u64> {
        Ok(match self.saved {
            None => None,
            Some(previous) if previous != self.key => {
                self.store.prefix_range_count(previous, self.key, section)?
            }
            Some(_) => self.store.range_count(self.key, section)?,
        }
        .unwrap_or(0))
    }
}

/// [`HeldPages`] with the committed pages they follow.
pub struct HeldView<'h, 'c> {
    held: &'h mut HeldPages,
    committed: Committed<'c>,
}

impl HeldPages {
    /// Bytes held for the next commit.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }

    /// The held pages, written over `committed`.
    pub fn view<'h, 'c>(&'h mut self, committed: Committed<'c>) -> HeldView<'h, 'c> {
        HeldView {
            held: self,
            committed,
        }
    }

    /// Write the held pages into `writer` in the order they were written,
    /// and hold none.
    pub fn write(&mut self, writer: &mut store::Writer) -> io::Result<()> {
        let held = std::mem::take(self);
        for write in held.writes {
            match write {
                Write::At(section, range, data) => writer.write_at(&section, range, data.as_slice())?,
                Write::Append(section, range, data) => {
                    let written = writer.write(&section, data.as_slice())?;
                    if written != range {
                        return Err(io::Error::other(format!(
                            "Held range {range} of {section} was committed at {written}"
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

impl HeldView<'_, '_> {
    fn count(&mut self, section: &str) -> io::Result<u64> {
        if let Some(&count) = self.held.counts.get(section) {
            return Ok(count);
        }
        let count = self.committed.count(section)?;
        self.held.counts.insert(section.to_owned(), count);
        Ok(count)
    }
}

impl Pages for HeldView<'_, '_> {
    fn write_at(&mut self, section: &str, range: u64, data: &[u8]) -> io::Result<()> {
        let count = self.count(section)?;
        if range > count {
            return Err(io::Error::other(format!(
                "Held range {range} of {section} skips past its {count} ranges"
            )));
        }
        let held = &mut *self.held;
        held.bytes += data.len();
        let key = (section.to_owned(), range);
        match held.at.get(&key) {
            // A later write of a range replaces the held one where it stands,
            // an appended one keeping its append, as the store replaces it.
            Some(&position) => match &mut held.writes[position] {
                Write::At(_, _, old) | Write::Append(_, _, old) => {
                    held.bytes -= old.len();
                    *old = data.to_vec();
                }
            },
            None => {
                held.at.insert(key, held.writes.len());
                held.writes
                    .push(Write::At(section.to_owned(), range, data.to_vec()));
            }
        }
        if range == count {
            held.counts.insert(section.to_owned(), count + 1);
        }
        Ok(())
    }

    fn write(&mut self, section: &str, data: &[u8]) -> io::Result<u64> {
        let range = self.count(section)?;
        let held = &mut *self.held;
        held.bytes += data.len();
        held.at.insert((section.to_owned(), range), held.writes.len());
        held.writes
            .push(Write::Append(section.to_owned(), range, data.to_vec()));
        held.counts.insert(section.to_owned(), range + 1);
        Ok(range)
    }

    fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Vec<u8>>> {
        if let Some(&position) = self.held.at.get(&(section.to_owned(), range)) {
            return Ok(Some(match &self.held.writes[position] {
                Write::At(_, _, data) | Write::Append(_, _, data) => data.clone(),
            }));
        }
        self.committed.read(section, range)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prepared::series::tests::Fixture;

    /// Pages held over a commit read their own writes, then the committed
    /// pages, and their commit writes exactly what writing them directly
    /// would.
    #[test]
    fn held_pages_commit_what_direct_writes_do() {
        let direct = Fixture::new();
        let held_store = Fixture::new();
        for fixture in [&direct, &held_store] {
            let mut writer = fixture.writer();
            writer.write_at("page", 0, b"committed".as_slice()).unwrap();
            writer.write("log", b"first".as_slice()).unwrap();
            writer.commit().unwrap();
        }
        let ops = |pages: &mut dyn Pages| {
            assert_eq!(pages.read_at("page", 0).unwrap().unwrap(), b"committed");
            pages.write_at("page", 0, b"replaced").unwrap();
            pages.write_at("page", 1, b"next").unwrap();
            assert_eq!(pages.write("log", b"second").unwrap(), 1);
            pages.write_at("page", 0, b"again").unwrap();
            assert_eq!(pages.read_at("page", 0).unwrap().unwrap(), b"again");
            assert_eq!(pages.read_at("log", 0).unwrap().unwrap(), b"first");
            assert_eq!(pages.write("log", b"third").unwrap(), 2);
            // A range appended, then written again before the commit.
            let range = pages.write("section", b"old").unwrap();
            pages.write_at("section", range, b"new").unwrap();
            assert_eq!(pages.read_at("section", range).unwrap().unwrap(), b"new");
        };
        let mut writer = direct.writer();
        ops(&mut writer);
        writer.commit().unwrap();
        let mut held = HeldPages::default();
        ops(&mut held.view(Committed {
            store: &held_store.store,
            key: &held_store.key,
            saved: Some(&held_store.key),
        }));
        assert!(!held.is_empty());
        assert_eq!(
            held.bytes(),
            ["again", "next", "second", "third", "new"]
                .iter()
                .map(|data| data.len())
                .sum::<usize>(),
            "each held range counts its latest bytes once"
        );
        let mut writer = held_store.writer();
        held.write(&mut writer).unwrap();
        writer.commit().unwrap();
        assert!(held.is_empty() && held.bytes() == 0);
        for (section, ranges) in [("page", 2), ("log", 3), ("section", 1)] {
            for range in 0..ranges {
                let read = |fixture: &Fixture| {
                    let mut bytes = Vec::new();
                    fixture
                        .store
                        .read(&fixture.key, section, range)
                        .unwrap()
                        .unwrap()
                        .read_to_end(&mut bytes)
                        .unwrap();
                    bytes
                };
                assert_eq!(read(&held_store), read(&direct), "{section} {range}");
            }
        }
    }
}

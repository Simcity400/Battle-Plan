//! The runs and events a family's accumulators finished: one buffer their emit shares, spilled
//! in key-sorted stretches and read back by key.

use super::{EventRow, Run, invalid};
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{self, BufReader, Read},
    os::windows::fs::FileExt,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};
use store::{Piece, Spill};

/// What the shared buffer holds before it spills, as one stretch: the write block.
const BLOCK: usize = 1 << 20;
const READ_AHEAD: usize = 64 << 10;
const RUN: u8 = 0;
const EVENT: u8 = 1;

pub(super) enum Record {
    Run(Run),
    Event(EventRow),
}

/// The records every accumulator of one emit finished and has not yet written.
struct Shelf {
    /// None when made with no spill room: what it holds stays held.
    spill: Option<Spill>,
    /// Each held record's owner, key and place in `held`.
    index: Vec<(u64, u64, usize)>,
    held: Vec<u8>,
    /// Each owner's piece of every stretch spilled since it last wrote, in spill order.
    pieces: HashMap<u64, Vec<Piece>>,
    owners: u64,
    /// The first spill that failed; its records stay held.
    failed: Option<String>,
}

type Shared = Arc<Mutex<Shelf>>;

impl Shelf {
    fn new(spill: Option<Spill>) -> Self {
        Self {
            spill,
            index: Vec::new(),
            held: Vec::new(),
            pieces: HashMap::new(),
            owners: 0,
            failed: None,
        }
    }

    fn push(&mut self, owner: u64, key: u64, record: &[u8]) {
        self.index.push((owner, key, self.held.len()));
        self.held.extend_from_slice(record);
        if self.held.len() >= BLOCK {
            self.spill_held();
        }
    }

    /// Spills every held record as one stretch, sorted by owner and key.
    fn spill_held(&mut self) {
        let Some(spill) = &mut self.spill else {
            return;
        };
        if self.failed.is_some() || self.index.is_empty() {
            return;
        }
        // Stable within a key: the place in `held` follows the order records came in.
        self.index.sort_unstable();
        let mut placed: Vec<(u64, Piece)> = Vec::new();
        let (index, held) = (&self.index, &self.held);
        let spilled = (|| {
            for &(owner, key, at) in index {
                let start = spill.len();
                spill.append(&key.to_le_bytes())?;
                spill.append(record_bytes(held, at)?)?;
                match placed.last_mut() {
                    Some((last, piece)) if *last == owner => piece.len = spill.len() - piece.offset,
                    _ => placed.push((
                        owner,
                        Piece {
                            offset: start,
                            len: spill.len() - start,
                        },
                    )),
                }
            }
            spill.seal()
        })();
        match spilled {
            Ok(()) => {
                for (owner, piece) in placed {
                    self.pieces.entry(owner).or_default().push(piece);
                }
                self.index = Vec::new();
                self.held = Vec::new();
            }
            Err(error) => self.failed = Some(error.to_string()),
        }
    }

    /// `owner`'s spilled pieces, a reader of the spill file, and its held records by key.
    fn of(&self, owner: u64) -> io::Result<Owned> {
        let pieces = self.pieces.get(&owner).cloned().unwrap_or_default();
        let file = match (&self.spill, pieces.is_empty()) {
            (Some(spill), false) => Some(Arc::new(
                spill
                    .reader()?
                    .ok_or_else(|| invalid("A spilled series stretch was never written"))?,
            )),
            _ => None,
        };
        let mut held = Vec::new();
        for &(held_owner, key, at) in &self.index {
            if held_owner == owner {
                held.push((key, decode(record_bytes(&self.held, at)?)?));
            }
        }
        // Stable: a key's records keep their order.
        held.sort_by_key(|&(key, _)| key);
        Ok(Owned { pieces, file, held })
    }

    /// Forgets what `owner` finished, which it wrote; the spill is reused once none is left.
    fn forget(&mut self, owner: u64) {
        self.pieces.remove(&owner);
        if self.index.iter().any(|&(held, ..)| held == owner) {
            self.index.retain(|&(held, ..)| held != owner);
        }
        if self.index.is_empty() {
            self.held = Vec::new();
        }
        if self.pieces.is_empty()
            && let Some(spill) = &mut self.spill
        {
            spill.clear();
        }
    }
}

fn lock(shelf: &Shared) -> MutexGuard<'_, Shelf> {
    shelf.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What one owner finished, read apart from the shelf.
struct Owned {
    pieces: Vec<Piece>,
    file: Option<Arc<File>>,
    held: Vec<(u64, Record)>,
}

/// One accumulator's finished records, on the shelf of the emit it first finished one in.
#[derive(Default)]
pub(super) struct Finished {
    on: Option<(Shared, u64)>,
}

impl Finished {
    pub(super) fn push_run(&mut self, key: u64, run: Run) {
        let mut record = [0; 26];
        record[0] = RUN;
        record[1..5].copy_from_slice(&run.first_hour.to_le_bytes());
        record[5..9].copy_from_slice(&run.last_hour.to_le_bytes());
        record[9..25].copy_from_slice(&run.value.to_le_bytes());
        record[25] = u8::from(run.break_before);
        let (shelf, owner) = self.shelf();
        lock(shelf).push(owner, key, &record);
    }

    pub(super) fn push_event(&mut self, key: u64, hour: i32, row: &[u8]) {
        let mut record = Vec::with_capacity(13 + row.len());
        record.push(EVENT);
        record.extend_from_slice(&hour.to_le_bytes());
        record.extend_from_slice(&(row.len() as u64).to_le_bytes());
        record.extend_from_slice(row);
        let (shelf, owner) = self.shelf();
        lock(shelf).push(owner, key, &record);
    }

    /// Its shelf and owner number: the thread's emit's, taken at its first record.
    fn shelf(&mut self) -> (&Shared, u64) {
        let (shelf, owner) = self.on.get_or_insert_with(|| {
            let shelf = engine_allocation::spill::room()
                .and_then(|room| {
                    room.kept(|folder| Mutex::new(Shelf::new(Some(Spill::new(folder)))))
                })
                .unwrap_or_else(|| Arc::new(Mutex::new(Shelf::new(None))));
            let owner = {
                let mut held = lock(&shelf);
                held.owners += 1;
                held.owners
            };
            (shelf, owner)
        });
        (shelf, *owner)
    }

    fn owned(&self) -> io::Result<Option<Owned>> {
        self.on
            .as_ref()
            .map(|(shelf, owner)| lock(shelf).of(*owner))
            .transpose()
    }

    pub(super) fn every(&self) -> io::Result<BTreeMap<u64, Vec<Record>>> {
        let mut keys: BTreeMap<u64, Vec<Record>> = BTreeMap::new();
        let Some(owned) = self.owned()? else {
            return Ok(keys);
        };
        if let Some(file) = &owned.file {
            for &piece in &owned.pieces {
                let mut source = Stretch::open(file, piece);
                while let Some(key) = source.key()? {
                    keys.entry(key).or_default().push(source.take()?);
                }
            }
        }
        for (key, record) in owned.held {
            keys.entry(key).or_default().push(record);
        }
        Ok(keys)
    }

    /// Spills what its shelf holds, readying its records for [`Self::merge`]; fails with the
    /// first failed spill.
    pub(super) fn sort(&mut self) -> io::Result<()> {
        let Some((shelf, _)) = &self.on else {
            return Ok(());
        };
        let mut shelf = lock(shelf);
        shelf.spill_held();
        match &shelf.failed {
            Some(error) => Err(io::Error::other(error.clone())),
            None => Ok(()),
        }
    }

    pub(super) fn merge(&self) -> io::Result<Merge> {
        let Some(owned) = self.owned()? else {
            return Ok(Merge {
                sources: Vec::new(),
            });
        };
        let mut sources = Vec::with_capacity(owned.pieces.len() + 1);
        if let Some(file) = &owned.file {
            for &piece in &owned.pieces {
                sources.push(Source::Stretch(Stretch::open(file, piece)));
            }
        }
        sources.push(Source::Held(owned.held.into_iter().peekable()));
        Ok(Merge { sources })
    }

    /// The finished records of the next batch: none yet, on the same shelf.
    pub(super) fn into_next(mut self) -> Self {
        let on = self.on.take();
        if let Some((shelf, owner)) = &on {
            lock(shelf).forget(*owner);
        }
        Self { on }
    }
}

impl Drop for Finished {
    fn drop(&mut self) {
        if let Some((shelf, owner)) = &self.on {
            lock(shelf).forget(*owner);
        }
    }
}

pub(super) struct Merge {
    /// Stretches in spill order, then what is held: a key's records keep that order.
    sources: Vec<Source>,
}

impl Merge {
    /// `key`'s runs and events; keys must be taken in ascending order.
    pub(super) fn take(&mut self, key: u64) -> io::Result<(Vec<Run>, Vec<EventRow>)> {
        let (mut runs, mut events) = (Vec::new(), Vec::new());
        for source in &mut self.sources {
            loop {
                match source.key()? {
                    Some(next) if next == key => match source.take()? {
                        Record::Run(run) => runs.push(run),
                        Record::Event(event) => events.push(event),
                    },
                    Some(next) if next < key => {
                        return Err(invalid("A series key's finished rows were passed over"));
                    }
                    _ => break,
                }
            }
        }
        Ok((runs, events))
    }

    pub(super) fn done(&mut self) -> io::Result<()> {
        for source in &mut self.sources {
            if source.key()?.is_some() {
                return Err(invalid("A series key's finished rows were never taken"));
            }
        }
        Ok(())
    }
}

enum Source {
    Stretch(Stretch),
    Held(std::iter::Peekable<std::vec::IntoIter<(u64, Record)>>),
}

impl Source {
    fn key(&mut self) -> io::Result<Option<u64>> {
        match self {
            Self::Stretch(stretch) => stretch.key(),
            Self::Held(held) => Ok(held.peek().map(|&(key, _)| key)),
        }
    }

    fn take(&mut self) -> io::Result<Record> {
        match self {
            Self::Stretch(stretch) => stretch.take(),
            Self::Held(held) => held
                .next()
                .map(|(_, record)| record)
                .ok_or_else(|| invalid("A held series record was taken twice")),
        }
    }
}

/// One owner's piece of a stretch, read from the spill file at its place.
struct At {
    file: Arc<File>,
    piece: Piece,
}

impl Read for At {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let count = out
            .len()
            .min(usize::try_from(self.piece.len).unwrap_or(usize::MAX));
        if count == 0 {
            return Ok(0);
        }
        let read = self.file.seek_read(&mut out[..count], self.piece.offset)?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.piece.offset += read as u64;
        self.piece.len -= read as u64;
        Ok(read)
    }
}

struct Stretch {
    reader: BufReader<At>,
    next: Option<u64>,
    read: bool,
}

impl Stretch {
    fn open(file: &Arc<File>, piece: Piece) -> Self {
        Self {
            reader: BufReader::with_capacity(
                READ_AHEAD.min(usize::try_from(piece.len).unwrap_or(READ_AHEAD)),
                At {
                    file: Arc::clone(file),
                    piece,
                },
            ),
            next: None,
            read: false,
        }
    }

    fn key(&mut self) -> io::Result<Option<u64>> {
        if !self.read {
            let mut key = [0; 8];
            self.next = match self.reader.read_exact(&mut key) {
                Ok(()) => Some(u64::from_le_bytes(key)),
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => None,
                Err(error) => return Err(error),
            };
            self.read = true;
        }
        Ok(self.next)
    }

    fn take(&mut self) -> io::Result<Record> {
        self.read = false;
        let mut tag = [0; 1];
        self.reader.read_exact(&mut tag)?;
        match tag[0] {
            RUN => {
                let mut bytes = [0; 25];
                self.reader.read_exact(&mut bytes)?;
                Ok(Record::Run(run(&bytes)))
            }
            EVENT => {
                let mut head = [0; 12];
                self.reader.read_exact(&mut head)?;
                let hour = i32::from_le_bytes(head[..4].try_into().expect("four bytes"));
                let len = u64::from_le_bytes(head[4..].try_into().expect("eight bytes"));
                let mut row = vec![0; usize::try_from(len).map_err(io::Error::other)?];
                self.reader.read_exact(&mut row)?;
                Ok(Record::Event((hour, row)))
            }
            _ => Err(invalid("A spilled series record has no kind")),
        }
    }
}

pub(super) fn split(records: Vec<Record>) -> (Vec<Run>, Vec<EventRow>) {
    let (mut runs, mut events) = (Vec::new(), Vec::new());
    for record in records {
        match record {
            Record::Run(run) => runs.push(run),
            Record::Event(event) => events.push(event),
        }
    }
    (runs, events)
}

fn record_bytes(held: &[u8], at: usize) -> io::Result<&[u8]> {
    let len = match held.get(at) {
        Some(&RUN) => 26,
        Some(&EVENT) => {
            let len = held
                .get(at + 5..at + 13)
                .ok_or_else(|| invalid("A held series event is cut short"))?;
            13 + usize::try_from(u64::from_le_bytes(len.try_into().expect("eight bytes")))
                .map_err(io::Error::other)?
        }
        _ => return Err(invalid("A held series record has no kind")),
    };
    held.get(at..at + len)
        .ok_or_else(|| invalid("A held series record is cut short"))
}

fn decode(bytes: &[u8]) -> io::Result<Record> {
    match bytes[0] {
        RUN => Ok(Record::Run(run(&bytes[1..26]))),
        _ => Ok(Record::Event((
            i32::from_le_bytes(bytes[1..5].try_into().expect("four bytes")),
            bytes[13..].to_vec(),
        ))),
    }
}

fn run(bytes: &[u8]) -> Run {
    Run {
        first_hour: i32::from_le_bytes(bytes[0..4].try_into().expect("four bytes")),
        last_hour: i32::from_le_bytes(bytes[4..8].try_into().expect("four bytes")),
        value: i128::from_le_bytes(bytes[8..24].try_into().expect("sixteen bytes")),
        break_before: bytes[24] != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(at: i32) -> Run {
        Run {
            first_hour: at,
            last_hour: at + 1,
            value: i128::from(at) * 3,
            break_before: at % 2 == 0,
        }
    }

    fn records(finished: &mut Finished, keys: &[u64]) -> Vec<(u64, Vec<Run>, Vec<EventRow>)> {
        finished.sort().unwrap();
        let mut merge = finished.merge().unwrap();
        let taken = keys
            .iter()
            .map(|&key| {
                let (runs, events) = merge.take(key).unwrap();
                (key, runs, events)
            })
            .collect();
        merge.done().unwrap();
        taken
    }

    /// Accumulators sharing an emit's shelf each read back their own records, in order.
    #[test]
    fn accumulators_sharing_a_shelf_read_back_their_own_records_in_order() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let folder =
            std::env::temp_dir().join(format!("finished-shelf-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let room = engine_allocation::spill::Room::new(&folder).lane();
        let _room = engine_allocation::spill::enter(Some(room.clone()));
        let (mut first, mut second) = (Finished::default(), Finished::default());
        let mut expected = [BTreeMap::new(), BTreeMap::new()];
        let row = vec![7u8; 4_000];
        for at in 0..3_000 {
            let key = (at % 13) as u64;
            first.push_run(key, run(at));
            expected[0].entry(key).or_insert((Vec::new(), Vec::new())).0.push(run(at));
            second.push_event(key + 100, at, &row);
            expected[1].entry(key + 100).or_insert((Vec::new(), Vec::new())).1.push((at, row.clone()));
        }
        let shelf = room.kept(|_| -> Mutex<Shelf> { unreachable!() }).unwrap();
        assert!(lock(&shelf).pieces.len() == 2, "the block spilled for both accumulators");
        for (finished, expected) in [(&mut first, &expected[0]), (&mut second, &expected[1])] {
            let keys: Vec<u64> = expected.keys().copied().collect();
            let got: Vec<_> = records(finished, &keys);
            for (key, runs, events) in got {
                assert!(runs == expected[&key].0 && events == expected[&key].1, "key {key}");
            }
        }
        let mut next = std::mem::take(&mut first).into_next();
        assert!(next.every().unwrap().is_empty());
        next.push_run(5, run(1));
        assert_eq!(records(&mut next, &[5])[0].1, vec![run(1)]);
        drop((next, second));
        assert!(lock(&shelf).pieces.is_empty());
        drop(_room);
        std::fs::remove_dir_all(folder).ok();
    }
}

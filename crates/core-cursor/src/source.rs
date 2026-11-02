use crate::{Address, Buffer, Bytes, Coverage, Hour, Hours, KeyFilter, Memory, RecordRef};
use std::io;
use std::ops::ControlFlow;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Access {
    Resident,
    Load,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lookup<'a> {
    Record(RecordRef<'a>),
    NotSelected,
    NotResident,
}

// These are local dictionary slots; identities themselves remain arbitrary-width Key varints.
pub enum Selection {
    All {
        len: usize,
    },
    Ids(Buffer<usize>),
    Prepared {
        ordinals: Box<Selection>,
        source: Box<dyn std::any::Any + Send + Sync>,
        _memory: crate::Reservation,
    },
}

impl Selection {
    // Sources attach their typed, validated key handle once during worker restore.
    pub fn with_source<T: Send + Sync + 'static>(
        self,
        source: T,
        memory: &Memory,
    ) -> io::Result<Self> {
        let reservation = memory.reserve(
            crate::Owner::State,
            std::mem::size_of::<Self>() + std::mem::size_of::<T>(),
        )?;
        Ok(Self::Prepared {
            ordinals: Box::new(self),
            source: Box::new(source),
            _memory: reservation,
        })
    }
    pub fn source<T: 'static>(&self) -> Option<&T> {
        match self {
            Self::Prepared { source, .. } => source.downcast_ref(),
            _ => None,
        }
    }
    pub fn ordinals(&self) -> &Self {
        match self {
            Self::Prepared { ordinals, .. } => ordinals.ordinals(),
            other => other,
        }
    }
    pub fn len(&self) -> usize {
        match self {
            Self::All { len } => *len,
            Self::Ids(ids) => ids.len(),
            Self::Prepared { ordinals, .. } => ordinals.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn id(&self, slot: usize) -> usize {
        match self {
            Self::All { .. } => slot,
            Self::Ids(ids) => ids[slot],
            Self::Prepared { ordinals, .. } => ordinals.id(slot),
        }
    }
    pub fn slot(&self, id: usize) -> Option<usize> {
        match self {
            Self::All { len } => (id < *len).then_some(id),
            Self::Ids(ids) => {
                if ids.get(id) == Some(&id) {
                    Some(id)
                } else {
                    ids.binary_search(&id).ok()
                }
            }
            Self::Prepared { ordinals, .. } => ordinals.slot(id),
        }
    }
}

pub trait ValueStore: Send + 'static {
    fn selection(&self) -> Arc<Selection>;
    fn address(&self, slot: usize) -> &Address;
    fn get(&self, address: &Address) -> Lookup<'_>;
    fn record(&self, slot: usize, staged: bool) -> RecordRef<'_>;
    fn load(&mut self, address: &Address) -> io::Result<crate::Record>;
    fn begin(&mut self, previous: &[usize]);
    // Stage copies into reserved columns; failure leaves the committed state whole.
    fn stage(&mut self, slot: usize, record: RecordRef<'_>) -> io::Result<bool>;
    fn include_active_rates(&self, changed: &mut Buffer<usize>) -> bool;
    fn commit(&mut self, changed: &[usize]);
    fn resident_bytes(&self) -> usize;
    fn max_record_bytes(&self) -> usize;
    fn active_rates(&self) -> usize;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageInfo {
    pub id: u64,
    pub hours: Hours,
    pub records: usize,
    pub coverage: usize,
    /// Directory upper bound; actual loaded charges are reported by CursorStats::window_bytes.
    pub resident_bytes: usize,
}

// Selection maps dictionary ids to cursor slots; a decoder applies it exactly once.
pub trait PageDecoder: Send + Sync + 'static {
    fn decode<'a>(
        &'a self,
        bytes: &'a [u8],
        hours: Hours,
        selection: &Selection,
        visit: &mut dyn FnMut(Hour, usize, RecordRef<'a>) -> io::Result<()>,
    ) -> io::Result<()>;
}

pub struct Page<D: PageDecoder> {
    pub info: PageInfo,
    pub bytes: Bytes,
    pub decoder: D,
    pub coverage: Buffer<Coverage>,
    pub backing: Option<crate::Reservation>,
}

impl<D: PageDecoder> Page<D> {
    pub fn validate(&self) -> io::Result<()> {
        let mut next = i64::from(self.info.hours.first);
        let mut accepted = None;
        for coverage in self.coverage.iter() {
            if coverage.hours.first > coverage.hours.last
                || i64::from(coverage.hours.first) != next
                || accepted.is_some_and(|value| value != coverage.accepted_before)
                || coverage.missing.as_ref().is_some_and(|missing| {
                    missing.hours.first > coverage.hours.first
                        || missing.hours.last < coverage.hours.last
                })
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid page coverage",
                ));
            }
            next = i64::from(coverage.hours.last) + 1;
            accepted = Some(coverage.accepted_through(coverage.hours.last));
        }
        if next != i64::from(self.info.hours.last) + 1
            || self.info.hours.first > self.info.hours.last
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "page omitted coverage",
            ));
        }
        Ok(())
    }
}

// One source is one published log and branch view; corrections are resolved before exposure.
pub trait LogSource: Send + Sync + 'static {
    type State: ValueStore;
    type Decoder: PageDecoder;

    // All selects live addresses at the cut plus ids named by scope; never the campaign dictionary.
    fn version_at(
        &self,
        keys: &KeyFilter,
        hour: Hour,
        scope: Hours,
        memory: &Memory,
    ) -> io::Result<Self::State>;
    // Restore the nearest version and replay to the cut, including earlier splits of this hour.
    fn version_before_page(
        &self,
        keys: &KeyFilter,
        page: PageInfo,
        scope: Hours,
        memory: &Memory,
    ) -> io::Result<Self::State>;
    // Worker-only, streaming directory traversal: directory pages may do I/O and must obey memory.
    fn pages(
        &self,
        keys: &KeyFilter,
        hours: Hours,
        visit: &mut dyn FnMut(PageInfo) -> ControlFlow<()>,
    ) -> io::Result<()>;
    fn checkpoint_run(&self, page: PageInfo) -> u64;
    // Directory resident_bytes includes decoded columns and indexes; read_page decompresses once on the worker.
    fn read_page(&self, page: PageInfo, memory: &Memory) -> io::Result<Page<Self::Decoder>>;
    fn coverage(
        &self,
        hours: Hours,
        visit: &mut dyn FnMut(Coverage) -> ControlFlow<()>,
    ) -> io::Result<()>;

    // No I/O, allocations or owned changes: decode only this frame's span into reserved columns.
    fn merged_changes<'a>(
        &self,
        selection: &Selection,
        hours: Hours,
        pages: &mut dyn Iterator<Item = &'a Page<Self::Decoder>>,
        state: &mut Self::State,
        changed: &mut Buffer<usize>,
    ) -> io::Result<()> {
        for page in pages {
            page.decoder
                .decode(&page.bytes, hours, selection, &mut |_, slot, record| {
                    if state.stage(slot, record)? && !changed.push(slot) {
                        return Err(io::Error::new(
                            io::ErrorKind::OutOfMemory,
                            "frame slots not prepared",
                        ));
                    }
                    Ok(())
                })?;
        }
        Ok(())
    }
}

use super::{Progress, inflate, invalid};
use replay_data::{BranchId, Hour, codec::Decoder, rows::TableId};
use replay_frame::frame::endpoint_step::EndpointStep;
use std::io::{self, Read};
use store::{SourceKey, Store};

mod fold;
mod produce;
pub use fold::PageFold;
#[cfg(test)]
pub(crate) use produce::split;
pub use produce::{BlockPages, FAMILY, PageExtract, Producer, STRIDES};

const MAGIC: &[u8; 8] = b"HOIPAG01";
pub const HEADER_BYTES: usize = 8 + 8 + 4 + 4 + 4 + 8 + 2;

/// Hours per second of the playback presets above the native route.
const PRESETS: [u32; 2] = [720, 1_440];
/// Frames per second the playback cadence targets.
pub const CADENCE: u32 = 36;

pub const fn provisional_strides() -> [u32; 2] {
    [PRESETS[0] / CADENCE, PRESETS[1] / CADENCE]
}

pub fn section(branch: BranchId, stride: u32) -> String {
    format!("map-pages.{branch}.{stride}")
}

/// The store section holding one branch's committed chains ([`BranchPages`]).
pub fn progress_section(branch: BranchId) -> String {
    format!("map-pages.progress.{branch}")
}

pub fn state_section(branch: BranchId, stride: u32) -> String {
    format!("map-pages.state.{branch}.{stride}")
}

/// Whether the applied `hour` is a chain point of `stride`.
pub fn chain_point(hour: Hour, stride: u32) -> bool {
    stride != 0 && i64::from(hour).rem_euclid(i64::from(stride)) == 0
}

/// One stride's committed chain on one branch.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Chain {
    pub stride: u32,
    /// Committed page ranges, in chain order.
    pub ranges: u64,
    /// The chain's origin: its branch's first chain point, reached natively.
    pub first: Option<Hour>,
    /// The last committed page's end, or the origin before any page.
    pub last: Option<Hour>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BranchPages {
    /// The selected tables' indices, ascending.
    pub selected: Vec<u16>,
    pub chains: Vec<Chain>,
}

impl BranchPages {
    pub fn check(&self, selected: &[u16]) -> io::Result<()> {
        if self.selected != selected {
            return Err(invalid("map page table selection differs from progress"));
        }
        if self.chains.first().is_some_and(|chain| chain.stride == 0)
            || self
                .chains
                .windows(2)
                .any(|pair| pair[0].stride >= pair[1].stride)
        {
            return Err(invalid("map page strides are not distinct and ascending"));
        }
        Ok(())
    }
}

pub fn committed(
    store: &Store,
    key: &SourceKey,
    map: &Progress,
) -> io::Result<Vec<(BranchId, BranchPages)>> {
    let mut committed = engine_allocation::vec(map.branches.len())?;
    for &branch in map.branches.keys() {
        let Some(file) = store.read(key, &progress_section(branch), 0)? else {
            continue;
        };
        let pages: BranchPages = serde_json::from_reader(io::BufReader::new(file))?;
        pages.check(&map.selected)?;
        committed.push((branch, pages));
    }
    Ok(committed)
}

/// A page's fixed, uncompressed header, read alone to search a chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub branch: BranchId,
    pub stride: u32,
    pub from: Hour,
    pub to: Hour,
    pub block: usize,
}

impl Header {
    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.branch.to_le_bytes());
        out.extend_from_slice(&self.stride.to_le_bytes());
        out.extend_from_slice(&self.from.to_le_bytes());
        out.extend_from_slice(&self.to.to_le_bytes());
        out.extend_from_slice(&(self.block as u64).to_le_bytes());
        out.extend_from_slice(&replay_data::FORMAT_VERSION.to_le_bytes());
    }

    pub fn read(mut input: impl Read) -> io::Result<Self> {
        let mut bytes = [0; HEADER_BYTES];
        input.read_exact(&mut bytes)?;
        let mut input = Decoder::new(&bytes);
        if input.take(8)? != MAGIC {
            return Err(invalid("unsupported map page"));
        }
        let branch = input.u64()?;
        let stride = input.u32()?;
        let from = input.i32()?;
        let to = input.i32()?;
        let block = usize::try_from(input.u64()?)
            .map_err(|_| invalid("map block ordinal exceeds address space"))?;
        if input.u16()? != replay_data::FORMAT_VERSION {
            return Err(invalid("map page row contract differs"));
        }
        input.finish()?;
        if stride == 0 || from >= to {
            return Err(invalid("map page header is invalid"));
        }
        Ok(Self {
            branch,
            stride,
            from,
            to,
            block,
        })
    }
}

pub fn read_page(mut input: impl Read, selected: &[TableId]) -> io::Result<(Header, EndpointStep)> {
    let header = Header::read(&mut input)?;
    let step = EndpointStep::decode(&inflate(input)?)?;
    check_step(&header, &step, selected)?;
    Ok((header, step))
}

pub fn check_step(header: &Header, step: &EndpointStep, selected: &[TableId]) -> io::Result<()> {
    let endpoints = step
        .from
        .map(|from| (from.branch, from.hour, step.to.branch, step.to.hour));
    if endpoints != Some((header.branch, header.from, header.branch, header.to)) {
        return Err(invalid("map page step differs from its header"));
    }
    let mut tables = engine_allocation::map_slice(selected, |table| table.index())?;
    tables.sort_unstable();
    if !step.tables().map(TableId::index).eq(tables) {
        return Err(invalid("map page step tables differ from the selection"));
    }
    Ok(())
}

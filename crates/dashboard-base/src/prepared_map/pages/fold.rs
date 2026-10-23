//! One branch's pages joined in block order.
use super::super::{compress, inflate, invalid, validate_selection};
use super::produce::{BlockPages, Page};
use super::{BranchPages, Chain, HEADER_BYTES, Header, section, state_section};
use replay_data::{BranchId, Hour, rows::TableId};
use replay_frame::frame::{
    DecodedFrame,
    endpoint_step::{EndpointStep, EndpointWriter},
};
use std::io;

struct Held {
    chain: usize,
    range: u64,
    bytes: Vec<u8>,
}

pub struct PageFold {
    branch: BranchId,
    selected: Vec<TableId>,
    pages: BranchPages,
    /// Each chain's state at its last chain point, once folded or resumed.
    lasts: Vec<Option<DecodedFrame>>,
    held: Vec<Held>,
    held_bytes: usize,
}

impl PageFold {
    pub fn new(
        branch: BranchId,
        selected: &[TableId],
        strides: &[u32],
        committed: Option<BranchPages>,
    ) -> io::Result<Self> {
        validate_selection(selected)?;
        let mut selection = engine_allocation::map_slice(selected, |table| *table as u16)?;
        selection.sort_unstable();
        let mut wanted = engine_allocation::copy_vec(strides)?;
        wanted.sort_unstable();
        wanted.dedup();
        if wanted.first().is_none_or(|&stride| stride == 0) {
            return Err(invalid("map page strides are missing or zero"));
        }
        let pages = match committed {
            Some(pages) => {
                pages.check(&selection)?;
                if !pages.chains.iter().map(|chain| chain.stride).eq(wanted) {
                    return Err(invalid("map page strides differ from accepted progress"));
                }
                pages
            }
            None => BranchPages {
                chains: engine_allocation::collect_vec(wanted.iter().map(|&stride| Chain {
                    stride,
                    ranges: 0,
                    first: None,
                    last: None,
                }))?,
                selected: selection,
            },
        };
        Ok(Self {
            branch,
            selected: engine_allocation::copy_vec(selected)?,
            lasts: engine_allocation::collect_vec(pages.chains.iter().map(|_| None))?,
            pages,
            held: Vec::new(),
            held_bytes: 0,
        })
    }

    /// The branch's chains, counting every folded page.
    pub fn pages(&self) -> &BranchPages {
        &self.pages
    }

    /// Bytes of folded pages not yet written.
    pub fn pending(&self) -> usize {
        self.held_bytes
    }

    pub fn save_states(&self, writer: &mut store::Writer) -> io::Result<()> {
        for (chain, last) in self.pages.chains.iter().zip(&self.lasts) {
            let Some(state) = last.as_ref() else {
                continue;
            };
            if chain.last != Some(state.hour) {
                return Err(invalid("map page chain state differs from its chain"));
            }
            let mut root = Vec::new();
            EndpointWriter::root(state, &self.selected, &mut root)?;
            let mut bytes = Vec::new();
            compress(&root, &mut bytes)?;
            writer.write_at(
                &state_section(self.branch, chain.stride),
                0,
                bytes.as_slice(),
            )?;
        }
        Ok(())
    }

    pub fn resume(
        &mut self,
        mut read: impl FnMut(&str) -> io::Result<Option<store::Range>>,
    ) -> io::Result<()> {
        for at in 0..self.pages.chains.len() {
            let Some(hour) = self.pages.chains[at]
                .last
                .filter(|_| self.lasts[at].is_none())
            else {
                continue;
            };
            let file = read(&state_section(self.branch, self.pages.chains[at].stride))?
                .ok_or_else(|| invalid("map page chain state is missing"))?;
            let root = EndpointStep::decode(&inflate(file)?)?;
            if root.from.is_some()
                || (root.to.branch, root.to.hour) != (self.branch, hour)
                || !root.tables().map(|table| table.index()).eq(self
                    .pages
                    .selected
                    .iter()
                    .map(|&table| usize::from(table)))
            {
                return Err(invalid("map page chain state differs from its chain"));
            }
            self.lasts[at] = Some(root.apply_owned(None)?);
        }
        Ok(())
    }

    pub fn fold(&mut self, block: usize, pages: BlockPages) -> io::Result<Option<(Hour, String)>> {
        let BlockPages {
            branch,
            lanes,
            refused,
            ..
        } = pages;
        if branch.is_some_and(|branch| branch != self.branch) {
            return Err(invalid("map page block belongs to another branch"));
        }
        for lane in lanes {
            let at = self
                .pages
                .chains
                .binary_search_by_key(&lane.stride, |chain| chain.stride)
                .map_err(|_| invalid("map page stride differs from its chains"))?;
            let Some(head) = lane.head else {
                continue;
            };
            match self.lasts[at].take() {
                Some(previous) => {
                    let mut step = Vec::new();
                    EndpointWriter::diff(&self.selected, &previous, &head, &mut step)?;
                    self.hold(
                        at,
                        block,
                        Page {
                            from: previous.hour,
                            to: head.hour,
                            step,
                        },
                    )?;
                }
                None if self.pages.chains[at].last.is_none() => {
                    let chain = &mut self.pages.chains[at];
                    chain.first = Some(head.hour);
                    chain.last = Some(head.hour);
                }
                None => return Err(invalid("map page chain lost its last state")),
            }
            for page in lane.pages {
                self.hold(at, block, page)?;
            }
            self.lasts[at] = lane.last;
        }
        Ok(refused)
    }

    fn hold(&mut self, at: usize, block: usize, page: Page) -> io::Result<()> {
        let chain = &mut self.pages.chains[at];
        if chain.last != Some(page.from) {
            return Err(invalid("map page does not continue its chain"));
        }
        let header = Header {
            branch: self.branch,
            stride: chain.stride,
            from: page.from,
            to: page.to,
            block,
        };
        let mut bytes = engine_allocation::vec(HEADER_BYTES)?;
        header.encode(&mut bytes);
        compress(&page.step, &mut bytes)?;
        engine_allocation::reserve(&mut self.held, 1)?;
        self.held_bytes = self.held_bytes.saturating_add(bytes.len());
        self.held.push(Held {
            chain: at,
            range: chain.ranges,
            bytes,
        });
        chain.ranges += 1;
        chain.last = Some(page.to);
        Ok(())
    }

    /// Write the held pages, in fold order, as their chains' next ranges.
    pub fn publish(&mut self, writer: &mut store::Writer) -> io::Result<()> {
        self.held_bytes = 0;
        for held in self.held.drain(..) {
            let stride = self.pages.chains[held.chain].stride;
            let range = writer.write(&section(self.branch, stride), held.bytes.as_slice())?;
            if range != held.range {
                return Err(invalid("map page ordinal differs from progress"));
            }
        }
        Ok(())
    }
}

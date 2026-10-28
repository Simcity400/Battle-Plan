//! One block's pages, from the block alone.
use super::super::{invalid, selected_tables};
use super::{chain_point, provisional_strides};
use crate::prepared::{
    frame_changes::FrameChanges,
    prepass::{Extract, Family, Held, HeldBytes},
};
use replay::container::Replay;
use replay_data::{BranchId, Hour, format::FormatVersion, rows::TableId};
use replay_frame::frame::{
    DecodedFrame, FrameDecoder, FrameSource, endpoint_step, endpoint_step::EndpointWriter,
};
use std::{
    any::Any,
    io,
    sync::{LazyLock, atomic::AtomicBool},
};

/// The strides prepared now ([`provisional_strides`]).
pub static STRIDES: [u32; 2] = provisional_strides();

pub static FAMILY: LazyLock<Family> = LazyLock::new(|| Family {
    name: "map-pages",
    extract: "map-pages.extract",
    sequential: "map-pages.sequential",
    fallback: "map-pages.fallback",
    tables: selected_tables(),
    fields: None,
    start,
});

fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(PageExtract::new(selected_tables(), &STRIDES))
}

/// A step from one chain point to the next of its stride.
pub(super) struct Page {
    pub from: Hour,
    pub to: Hour,
    pub step: Vec<u8>,
}

/// One stride's pages within one block.
pub(super) struct Lane {
    pub stride: u32,
    /// The block's first chain point.
    pub head: Option<DecodedFrame>,
    /// Steps between the block's chain points, in chain order.
    pub pages: Vec<Page>,
    /// The block's last chain point so far.
    pub last: Option<DecodedFrame>,
}

impl Held for Page {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.add(self.step.capacity());
    }
}

impl Held for Lane {
    fn held(&self, bytes: &mut HeldBytes) {
        self.head.held(bytes);
        self.pages.held(bytes);
        self.last.held(bytes);
    }
}

pub struct Producer {
    selected: Vec<TableId>,
    lanes: Vec<Lane>,
    writers: Vec<Option<EndpointWriter>>,
    /// The block's branch and the last hour observed.
    previous: Option<(BranchId, Hour)>,
}

impl Producer {
    pub fn new(selected: &[TableId], strides: &[u32]) -> io::Result<Self> {
        if strides.contains(&0) {
            return Err(invalid("map page stride is zero"));
        }
        Ok(Self {
            selected: engine_allocation::copy_vec(selected)?,
            lanes: engine_allocation::collect_vec(strides.iter().map(|&stride| Lane {
                stride,
                head: None,
                pages: Vec::new(),
                last: None,
            }))?,
            writers: engine_allocation::collect_vec(strides.iter().map(|_| None))?,
            previous: None,
        })
    }

    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        recorded: Option<&(impl FrameSource + ?Sized)>,
    ) -> io::Result<()> {
        let follows = match self.previous {
            Some((branch, hour)) if branch != frame.branch || hour >= frame.hour => {
                return Err(invalid("map page hours do not advance within their block"));
            }
            Some((_, hour)) => hour.checked_add(1) == Some(frame.hour),
            None => false,
        };
        self.previous = Some((frame.branch, frame.hour));
        let recorded = recorded.filter(|_| follows);
        for (lane, writer) in self.lanes.iter_mut().zip(&mut self.writers) {
            match (&mut *writer, recorded) {
                (Some(steps), Some(source)) => steps.observe(source, frame)?,
                (stale, _) => *stale = None,
            }
            if !chain_point(frame.hour, lane.stride) {
                continue;
            }
            match &lane.last {
                Some(last) => {
                    let mut step = Vec::new();
                    match &mut *writer {
                        Some(steps) => steps.finish(frame, &mut step)?,
                        None => EndpointWriter::diff(&self.selected, last, frame, &mut step)?,
                    };
                    engine_allocation::reserve(&mut lane.pages, 1)?;
                    lane.pages.push(Page {
                        from: last.hour,
                        to: frame.hour,
                        step,
                    });
                }
                None => lane.head = Some(frame.clone()),
            }
            if writer.is_none() {
                *writer = Some(EndpointWriter::resume(&self.selected, frame)?);
            }
            lane.last = Some(frame.clone());
        }
        Ok(())
    }

    fn bytes(&self) -> u64 {
        let mut held = HeldBytes::default();
        held.add(size_of::<BlockPages>()).heap(&self.lanes);
        held.total()
    }

    /// The block's pages and chain points.
    pub fn finish(self) -> BlockPages {
        BlockPages {
            branch: self.previous.map(|(branch, _)| branch),
            lanes: self.lanes,
            reached: self.previous.map(|(_, hour)| hour),
            refused: None,
        }
    }
}

/// One block's pages of every stride and its first and last chain points.
pub struct BlockPages {
    pub(super) branch: Option<BranchId>,
    pub(super) lanes: Vec<Lane>,
    pub(super) reached: Option<Hour>,
    pub(super) refused: Option<(Hour, String)>,
}

impl BlockPages {
    pub fn produce(
        replay: &Replay,
        block: usize,
        selected: &[TableId],
        strides: &[u32],
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Self> {
        let index = replay
            .index
            .get(block)
            .ok_or_else(|| invalid("map page block exceeds recording"))?;
        let mut producer = Producer::new(selected, strides)?;
        check()?;
        let frames = match replay.read_block_recorded_selected(index, selected) {
            Ok(frames) => frames,
            Err(error) => return producer.finish().refuse(index.first_hour, error),
        };
        let mut decoder = FrameDecoder::for_tables(selected);
        for frame in &frames {
            check()?;
            let observed = decoder.advance(frame).and_then(|decoded| {
                crate::metadata::validate_campaign(decoded, replay)?;
                producer.observe(decoded, Some(frame))
            });
            if let Err(error) = observed {
                return producer.finish().refuse(frame.hour, error);
            }
        }
        Ok(producer.finish())
    }

    /// [`Self::produce`] with the selected tables split into `groups` groups, each read,
    /// reconstructed and stepped on its own thread, and their pages merged into the block's:
    /// the same pages and chain points, as each table's are its own. A block one group
    /// refuses, or whose groups disagree, is produced whole instead, so its refusal is the
    /// whole block's.
    pub fn produce_grouped(
        replay: &Replay,
        block: usize,
        selected: &[TableId],
        strides: &[u32],
        groups: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<Self> {
        let check = || crate::metadata::check_cancel(cancelled);
        let groups = groups.min(selected.len());
        if groups <= 1 {
            return Self::produce(replay, block, selected, strides, check);
        }
        let split = split(selected, groups);
        if split.len() <= 1 {
            return Self::produce(replay, block, selected, strides, check);
        }
        let priority = crate::prepared::priority::Caller::current();
        let cpu = replay_frame::cpu::Context::capture();
        let parts: Vec<io::Result<Self>> = std::thread::scope(|scope| {
            let handles: Vec<_> = split
                .iter()
                .enumerate()
                .map(|(group, tables)| {
                    let (priority, cpu) = (priority.clone(), cpu.clone());
                    std::thread::Builder::new()
                        .name(format!("preparation map pages {group}"))
                        .spawn_scoped(scope, move || {
                            let _cpu = cpu.enter("fold driver");
                            priority.enter();
                            Self::produce(replay, block, tables, strides, check)
                        })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| match handle {
                    Ok(handle) => handle
                        .join()
                        .unwrap_or_else(|_| Err(io::Error::other("A map page group panicked"))),
                    Err(error) => Err(error),
                })
                .collect()
        });
        let parts = parts.into_iter().collect::<io::Result<Vec<Self>>>()?;
        match Self::merged(parts, &split)? {
            Some(merged) => Ok(merged),
            None => Self::produce(replay, block, selected, strides, check),
        }
    }

    /// The pages of disjoint groups' `parts` as one block's, None when a group refused or
    /// the groups disagree on the block's chain points.
    fn merged(parts: Vec<Self>, split: &[Vec<TableId>]) -> io::Result<Option<Self>> {
        let first = &parts[0];
        if parts.iter().any(|part| {
            part.refused.is_some()
                || part.branch != first.branch
                || part.reached != first.reached
                || part.lanes.len() != first.lanes.len()
        }) {
            return Ok(None);
        }
        let frame = |frames: &[Option<&DecodedFrame>]| -> Option<Option<DecodedFrame>> {
            let base = frames[0];
            if frames.iter().any(|frame| frame.is_some() != base.is_some()) {
                return None;
            }
            let Some(base) = base else {
                return Some(None);
            };
            if frames.iter().flatten().any(|frame| {
                (frame.branch, frame.hour, &frame.metrics)
                    != (base.branch, base.hour, &base.metrics)
            }) {
                return None;
            }
            let mut merged = base.clone();
            for (frame, tables) in frames.iter().flatten().zip(split).skip(1) {
                for table in tables {
                    merged.tables[table.index()] = frame.tables[table.index()].clone();
                }
            }
            Some(Some(merged))
        };
        let mut lanes = engine_allocation::vec(first.lanes.len())?;
        for at in 0..first.lanes.len() {
            let each: Vec<&Lane> = parts.iter().map(|part| &part.lanes[at]).collect();
            let pages = each[0].pages.len();
            if each.iter().any(|lane| {
                lane.stride != each[0].stride
                    || lane.pages.len() != pages
                    || lane
                        .pages
                        .iter()
                        .zip(&each[0].pages)
                        .any(|(page, base)| (page.from, page.to) != (base.from, base.to))
            }) {
                return Ok(None);
            }
            let heads: Vec<Option<&DecodedFrame>> =
                each.iter().map(|lane| lane.head.as_ref()).collect();
            let lasts: Vec<Option<&DecodedFrame>> =
                each.iter().map(|lane| lane.last.as_ref()).collect();
            let (Some(head), Some(last)) = (frame(&heads), frame(&lasts)) else {
                return Ok(None);
            };
            let mut merged = engine_allocation::vec(pages)?;
            for page in 0..pages {
                let steps: Vec<&[u8]> = each
                    .iter()
                    .map(|lane| lane.pages[page].step.as_slice())
                    .collect();
                let mut step = Vec::new();
                endpoint_step::merge(&steps, &mut step)?;
                merged.push(Page {
                    from: each[0].pages[page].from,
                    to: each[0].pages[page].to,
                    step,
                });
            }
            lanes.push(Lane {
                stride: each[0].stride,
                head,
                pages: merged,
                last,
            });
        }
        Ok(Some(Self {
            branch: first.branch,
            lanes,
            reached: first.reached,
            refused: None,
        }))
    }

    fn refuse(mut self, hour: Hour, error: io::Error) -> io::Result<Self> {
        if engine_allocation::is_allocation_error(&error) {
            return Err(error);
        }
        self.refused = Some((hour, error.to_string()));
        Ok(self)
    }

    /// The last hour the block observed.
    pub fn reached(&self) -> Option<Hour> {
        self.reached
    }

    /// The hour whose read, decode or identity check refused, and why.
    pub fn refused(&self) -> Option<&(Hour, String)> {
        self.refused.as_ref()
    }

    /// Every lane's stride and pages, as hours and step bytes, for comparing productions.
    #[cfg(test)]
    pub(crate) fn steps(&self) -> Vec<(u32, Hour, Hour, &[u8])> {
        self.lanes
            .iter()
            .flat_map(|lane| {
                lane.pages
                    .iter()
                    .map(move |page| (lane.stride, page.from, page.to, page.step.as_slice()))
            })
            .collect()
    }

    /// The block's first chain point of `stride`.
    pub fn head(&self, stride: u32) -> Option<&DecodedFrame> {
        self.lanes
            .iter()
            .find(|lane| lane.stride == stride)
            .and_then(|lane| lane.head.as_ref())
    }

    /// The block's last chain point of `stride`.
    pub fn last(&self, stride: u32) -> Option<&DecodedFrame> {
        self.lanes
            .iter()
            .find(|lane| lane.stride == stride)
            .and_then(|lane| lane.last.as_ref())
    }
}

impl Held for BlockPages {
    fn held(&self, bytes: &mut HeldBytes) {
        self.lanes.held(bytes);
        if let Some((_, reason)) = &self.refused {
            reason.held(bytes);
        }
    }
}

/// `selected` in up to `groups` groups of tables read on their own. Tables an alias joins (one
/// reads the other's private aliases, or both read the same table's) stay in one group: each
/// group's reader resolves those sources and enters them in the block's shared decoded-section
/// cache, and two readers each holding a table the other resolves would wait on each other.
pub(crate) fn split(selected: &[TableId], groups: usize) -> Vec<Vec<TableId>> {
    use replay_data::rows::TABLE_COUNT;
    let mut root: Vec<usize> = (0..TABLE_COUNT).collect();
    fn find(root: &mut [usize], at: usize) -> usize {
        let mut at = at;
        while root[at] != at {
            root[at] = root[root[at]];
            at = root[at];
        }
        at
    }
    for &table in selected {
        for &source in replay::container::alias_sources(table) {
            let (a, b) = (
                find(&mut root, table.index()),
                find(&mut root, source.index()),
            );
            root[a] = b;
        }
    }
    let mut joined: Vec<Vec<TableId>> = Vec::new();
    let mut at: Vec<Option<usize>> = vec![None; TABLE_COUNT];
    for &table in selected {
        let key = find(&mut root, table.index());
        match at[key] {
            Some(set) => joined[set].push(table),
            None => {
                at[key] = Some(joined.len());
                joined.push(vec![table]);
            }
        }
    }
    joined.sort_by_key(|set| std::cmp::Reverse(set.len()));
    let mut split: Vec<Vec<TableId>> = vec![Vec::new(); groups.min(joined.len()).max(1)];
    for set in joined {
        let smallest = split
            .iter_mut()
            .min_by_key(|group| group.len())
            .expect("a group");
        smallest.extend(set);
    }
    for group in &mut split {
        group.sort_unstable_by_key(|table| table.index());
    }
    split
}

pub struct PageExtract {
    selected: &'static [TableId],
    strides: &'static [u32],
    producer: Option<Producer>,
}

impl PageExtract {
    pub fn new(selected: &'static [TableId], strides: &'static [u32]) -> Self {
        Self {
            selected,
            strides,
            producer: None,
        }
    }
}

impl Extract for PageExtract {
    fn observe(
        &mut self,
        replay: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        crate::metadata::validate_campaign(frame, replay)?;
        let producer = match &mut self.producer {
            Some(producer) => producer,
            empty => empty.insert(Producer::new(self.selected, self.strides)?),
        };
        producer.observe(frame, changes.recorded().map(|recorded| &**recorded))
    }

    fn bytes(&self) -> u64 {
        self.producer.as_ref().map_or(0, Producer::bytes)
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        let producer = match self.producer {
            Some(producer) => producer,
            None => Producer::new(self.selected, self.strides)?,
        };
        Ok(Box::new(producer.finish()))
    }
}

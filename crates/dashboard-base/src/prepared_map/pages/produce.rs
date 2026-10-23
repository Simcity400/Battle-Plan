//! One block's pages, from the block alone.
use super::super::{invalid, selected_tables};
use super::{chain_point, provisional_strides};
use crate::prepared::{
    frame_changes::FrameChanges,
    prepass::{Extract, Family, Held, HeldBytes},
};
use replay::container::Replay;
use replay_data::{BranchId, Hour, format::FormatVersion, rows::TableId};
use replay_frame::frame::{DecodedFrame, FrameDecoder, FrameSource, endpoint_step::EndpointWriter};
use std::{any::Any, io, sync::LazyLock};

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

//! Panel readers for dated playback.

use crate::live::Live;
use replay_data::{Hour, rows::TableId};
use replay_frame::frame::DecodedFrame;
use serde_json::Value;
use std::{io, sync::Arc};

/// A panel's value at an hour: its data, or why it has none (the request path's refusal).
pub type PanelData = Result<Value, String>;

pub type ExactReader = Arc<dyn Fn(Hour, &std::sync::atomic::AtomicBool) -> PanelData + Send + Sync>;

pub mod product;
pub use playhead::Declared;

#[derive(Clone, Copy)]
pub struct ProductStep<'a> {
    pub step: &'a playhead::Step,
    pub lanes: &'a playhead::Lanes<'a>,
}

pub struct ProductRead {
    pub value: Arc<PanelData>,
    pub rows: u64,
    pub encoded: Option<Arc<[u8]>>,
    pub through: Hour,
}

pub type ProductReader = Arc<dyn Fn(Hour, bool, &std::sync::atomic::AtomicBool, Option<ProductStep<'_>>) -> ProductRead + Send + Sync>;

pub struct AtHour {
    pub declared: Vec<Declared>,
    pub advance: ProductReader,
    pub cold: ExactReader,
    pub read: ExactReader,
}

impl Panel for AtHour {
    fn tables(&self) -> &[TableId] { &[] }
    fn data(&mut self, frame: &DecodedFrame) -> PanelData { (self.read)(frame.hour, &false.into()) }
    fn exact(&self) -> Option<&AtHour> { Some(self) }
}

/// A panel at the head.
pub trait Panel: Send {
    fn exact(&self) -> Option<&AtHour> { None }
    /// The tables its data reads, whole.
    fn tables(&self) -> &[TableId];
    /// Its data from rows at the head holding those tables.
    fn data(&mut self, frame: &DecodedFrame) -> PanelData;
    /// The live cells its last data names ([`crate::live`]): its host follows them by key while
    /// that data stands, and replaces them with the next data that differs.
    fn live(&self) -> Option<&Live> {
        None
    }
}

/// Rows at the head of one playhead, for the panels its host shows.
pub trait PanelRows: Send + Sync {
    /// A reader of `tables` at a panel's cadence; it holds them until dropped.
    fn reader(&self, tables: &[TableId]) -> io::Result<Box<dyn PanelReader>>;
}

/// One panel's rows at the head.
pub trait PanelReader: Send {
    /// The newest frame of its tables; none while one is loading. Never waits; the same `Arc`
    /// while nothing it reads changed.
    fn frame(&self) -> Option<Arc<DecodedFrame>>;
    /// The last confirmed hour and seek of this exact frame.
    fn through_seek(&self, _frame: &Arc<DecodedFrame>) -> Option<(Hour, u64)> {
        None
    }

    /// Why its rows stopped coming for good, and the hour that is said at: the panel is refused
    /// by name, never left loading.
    fn stopped(&self) -> Option<(Hour, String)> {
        None
    }
}

pub mod dated;

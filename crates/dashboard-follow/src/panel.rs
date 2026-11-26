
use crate::live::Live;
use replay_data::{Hour, rows::TableId};
use replay_frame::frame::DecodedFrame;
use serde_json::Value;
use std::{io, sync::Arc};

pub type PanelData = Result<Value, String>;

pub type ExactReader = Arc<dyn Fn(Hour, &std::sync::atomic::AtomicBool) -> PanelData + Send + Sync>;

pub mod product;
pub use playhead::Declared;

pub type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Copy)]
pub struct ProductStep<'a> {
    pub step: &'a playhead::Step,
    pub lanes: &'a playhead::Lanes<'a>,
    pub held: Option<&'a [bool]>,
    pub wake: Option<&'a Wake>,
}

impl ProductStep<'_> {
    pub fn lanes_table(&self, table: usize) -> bool {
        self.held.is_none_or(|held| held.get(table).copied().unwrap_or(false))
    }
}

pub struct ProductRead {
    pub value: Arc<PanelData>,
    pub rows: u64,
    pub fresh: bool,
    pub pending: bool,
    pub live: Option<Arc<Live>>,
    pub lanes: Vec<Declared>,
}

pub type ProductReader = Arc<dyn Fn(Hour, bool, &std::sync::atomic::AtomicBool, Option<ProductStep<'_>>) -> ProductRead + Send + Sync>;

pub struct AtHour {
    pub advance: ProductReader,
    pub cold: ExactReader,
    pub read: ExactReader,
}

impl Panel for AtHour {
    fn tables(&self) -> &[TableId] { &[] }
    fn data(&mut self, frame: &DecodedFrame) -> PanelData { (self.read)(frame.hour, &false.into()) }
    fn exact(&self) -> Option<&AtHour> { Some(self) }
}

pub trait Panel: Send {
    fn exact(&self) -> Option<&AtHour> { None }
    fn tables(&self) -> &[TableId];
    fn data(&mut self, frame: &DecodedFrame) -> PanelData;
    fn live(&self) -> Option<&Live> {
        None
    }
}

pub trait PanelRows: Send + Sync {
    fn reader(&self, tables: &[TableId]) -> io::Result<Box<dyn PanelReader>>;
    fn on_publish(&self, _wake: Wake) {}
}

pub trait PanelReader: Send {
    fn frame(&self) -> Option<Arc<DecodedFrame>>;
    fn through_seek(&self, _frame: &Arc<DecodedFrame>) -> Option<(Hour, u64)> {
        None
    }

    fn stopped(&self) -> Option<(Hour, String)> {
        None
    }
}

pub mod dated;

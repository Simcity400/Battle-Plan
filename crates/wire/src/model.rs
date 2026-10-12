use crate::errors::{Result, WireError};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u16 = 1;
pub const HEADER_BYTES: usize = 128;
pub const DESCRIPTOR_BYTES: usize = 64;
pub const NONE_INDEX: u32 = u32::MAX;
pub const TOP_LEVEL_LENGTH: u64 = u64::MAX;
pub const UNKNOWN_TOTAL: u64 = u64::MAX;
pub const FLAG_FINAL: u16 = 1;
pub const FLAG_CONTINUATION: u16 = 2;
pub const FLAG_BOUNDED: u16 = 4;
pub const FLAG_ERROR: u16 = 8;

macro_rules! wire_enum {
    ($name:ident, $repr:ty, $default:ident, {$($variant:ident = $number:literal),+ $(,)?}) => {
        #[repr($repr)]
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
        pub enum $name { $($variant = $number),+ }
        impl Default for $name { fn default() -> Self { Self::$default } }
        impl TryFrom<$repr> for $name {
            type Error = WireError;
            fn try_from(value: $repr) -> Result<Self> {
                match value { $($number => Ok(Self::$variant)),+, _ => Err(WireError::new("invalid-enum", stringify!($name))) }
            }
        }
    }
}
wire_enum!(LogicalType, u16, Bool, {Bool=1, Unsigned=2, Signed=3, Safe53=4, WideInt=5, FixedPoint=6, Float=7, Hour=8, NativeDate=9, Bytes=10, Text=11, Enum=12, Refusal=13, List=14, Struct=15});
wire_enum!(Encoding, u8, Plain, {Plain=0, BitPacked=1, Dictionary=2, RunLength=3});
wire_enum!(ReplyKind, u32, Series, {Series=1, Events=2, StateAt=3, Catalogue=4, Rank=5, Select=6, WindowTotal=7, Presentation=8, Description=9, Diagnostics=10});
wire_enum!(UnitKind, u32, Page, {Page=0, Window=1, Viewport=2, TopK=3, Complete=4});
wire_enum!(DictionaryKind, u32, Plain, {Plain=0, Text=1, Tag=2, Refusal=3, ImageKey=4, DiagnosticCode=5});
wire_enum!(BoundsKind, u32, Hours, {Hours=0, Viewport=1, Rank=2});
wire_enum!(Coordinate, u8, Element, {Element=1, Byte=2});

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Header {
    pub schema_version: u16,
    pub flags: u16,
    pub query_id: u64,
    pub campaign_id: [u8; 16],
    pub revision: u64,
    pub generation: u64,
    pub reply_kind: ReplyKind,
    pub shape_id: u32,
    pub shape_version: u32,
    pub unit_kind: UnitKind,
    pub unit_start: u64,
    pub unit_end: u64,
    pub row_count: u64,
    pub total_count: u64,
    pub diagnostic_total: u64,
}
impl Default for Header {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            flags: FLAG_FINAL,
            query_id: 0,
            campaign_id: [0; 16],
            revision: 0,
            generation: 0,
            reply_kind: ReplyKind::Series,
            shape_id: 0,
            shape_version: 1,
            unit_kind: UnitKind::Page,
            unit_start: 0,
            unit_end: 0,
            row_count: 0,
            total_count: UNKNOWN_TOTAL,
            diagnostic_total: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Descriptor {
    pub name_id: u32,
    pub logical_type: LogicalType,
    pub physical_encoding: Encoding,
    pub state_layers: u8,
    pub type_param_a: u32,
    pub type_param_b: u32,
    pub dictionary_ref: u32,
    pub child_first: u32,
    pub child_count: u32,
    pub data_buffer: u32,
    pub length: u64,
    pub aux_buffer: u32,
    pub state_buffer: u32,
    pub state_widths: u16,
    pub gate_column: u16,
    pub gate_value: u32,
    pub refusal_ref: u32,
    pub reserved: u32,
}
impl Default for Descriptor {
    fn default() -> Self {
        Self {
            name_id: 0,
            logical_type: LogicalType::Bool,
            physical_encoding: Encoding::Plain,
            state_layers: 0,
            type_param_a: 0,
            type_param_b: 1,
            dictionary_ref: NONE_INDEX,
            child_first: NONE_INDEX,
            child_count: 0,
            data_buffer: NONE_INDEX,
            length: TOP_LEVEL_LENGTH,
            aux_buffer: NONE_INDEX,
            state_buffer: NONE_INDEX,
            state_widths: 0,
            gate_column: u16::MAX,
            gate_value: 0,
            refusal_ref: NONE_INDEX,
            reserved: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DictionaryRef {
    pub dictionary_identity: [u8; 32],
    pub shard_identity: [u8; 32],
    pub first_entry: u64,
    pub entry_count: u64,
    pub kind: DictionaryKind,
    pub flags: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub kind: BoundsKind,
    pub start: i64,
    pub end: i64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Extent {
    pub column: u32,
    pub coordinate: Coordinate,
    pub final_extent: bool,
    pub index: u64,
    pub first: u64,
    pub end: u64,
    pub total: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Reply {
    pub header: Header,
    pub descriptors: Vec<Descriptor>,
    pub top_level: Vec<u32>,
    pub buffers: Vec<Vec<u8>>,
    pub dictionary_refs: Vec<DictionaryRef>,
    pub diagnostics: Option<Vec<u8>>,
    pub continuation: Option<Vec<u8>>,
    pub diagnostic_continuation: Option<Vec<u8>>,
    pub extents: Vec<Extent>,
    pub bounds: Option<Bounds>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StateMapping {
    pub native_absence: bool,
    pub computed_absence: bool,
    pub knownness_witness: Option<String>,
    pub refusal_witness: Option<String>,
    pub source_distinction: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ColumnSpec {
    pub descriptor: Descriptor,
    pub static_enum: Option<Vec<String>>,
    pub fixed_list_length: Option<u64>,
    pub dictionary_kind: Option<DictionaryKind>,
    pub state_mapping: StateMapping,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BoundsRule {
    Forbidden,
    Optional,
    Required,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Shape {
    pub id: u32,
    pub version: u32,
    pub name: String,
    pub reply_kind: ReplyKind,
    pub unit_kinds: Vec<UnitKind>,
    pub names: Vec<String>,
    pub columns: Vec<ColumnSpec>,
    pub top_level: Vec<u32>,
    pub bounds: BoundsRule,
}
pub trait ShapeRegistry {
    fn shape(&self, id: u32) -> Option<&Shape>;
}
impl ShapeRegistry for Vec<Shape> {
    fn shape(&self, id: u32) -> Option<&Shape> {
        self.iter().find(|s| s.id == id)
    }
}
impl ShapeRegistry for [Shape] {
    fn shape(&self, id: u32) -> Option<&Shape> {
        self.iter().find(|s| s.id == id)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ReadContext {
    pub expected_bounds: Option<Bounds>,
    pub require_bounds_match: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Section {
    pub kind: u32,
    pub count: u32,
    pub offset: u64,
    pub length: u64,
}

/// Retains the received bytes. Column and opaque-buffer views borrow them.
#[derive(Debug)]
pub struct ValidatedReply<'a> {
    pub header: Header,
    pub sections: Vec<Section>,
    pub descriptors: Vec<Descriptor>,
    pub top_level: Vec<u32>,
    pub buffers: Vec<&'a [u8]>,
    pub dictionary_refs: Vec<DictionaryRef>,
    pub diagnostics: Option<Box<ValidatedReply<'a>>>,
    pub continuation: Option<&'a [u8]>,
    pub diagnostic_continuation: Option<&'a [u8]>,
    pub extents: Vec<Extent>,
    pub bounds: Option<Bounds>,
    pub bytes: &'a [u8],
    pub(crate) parents: Vec<Option<u32>>,
    pub(crate) run_ends: Vec<Vec<u64>>,
}
impl ValidatedReply<'_> {
    pub fn column_length(&self, index: usize) -> u64 {
        let d = &self.descriptors[index];
        if d.length == TOP_LEVEL_LENGTH {
            self.header.row_count
        } else {
            d.length
        }
    }
    pub fn to_owned(&self) -> Reply {
        Reply {
            header: self.header.clone(),
            descriptors: self.descriptors.clone(),
            top_level: self.top_level.clone(),
            buffers: self.buffers.iter().map(|b| b.to_vec()).collect(),
            dictionary_refs: self.dictionary_refs.clone(),
            diagnostics: self.diagnostics.as_ref().map(|d| d.bytes.to_vec()),
            continuation: self.continuation.map(<[u8]>::to_vec),
            diagnostic_continuation: self.diagnostic_continuation.map(<[u8]>::to_vec),
            extents: self.extents.clone(),
            bounds: self.bounds,
        }
    }
}

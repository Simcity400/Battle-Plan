use crate::value::value_struct;

pub use game_model::modifier_breakdown::{
    SourceNode as DivisionPanelSourceNode, SourceValue as DivisionPanelSourceValue,
};
value_struct!(DivisionPanelSourceValue { key: u32, raw: i64 });
value_struct!(DivisionPanelSourceNode { children: u32, name: Vec<u8>, values: Vec<DivisionPanelSourceValue> });
opaque_value!(Option<DivisionPanelSourceNode>);

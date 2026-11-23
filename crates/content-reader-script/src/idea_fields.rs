use crate::native::{Field, Shape};
use content_reader_parser::parser::Item;

pub fn ledger(bits: u32, item: &Item) -> u32 {
    match item.value_keyword().as_deref() {
        Some("hidden") => 1,
        Some("army") => bits | 0x4,
        Some("navy") => bits | 0x8,
        Some("military") => bits | 0x1C,
        Some("all") => bits | 0x1E,
        Some("air") => bits | 0x10,
        Some("civilian") => bits | 0x2,
        Some("invalid") => 0xFFFF,
        _ => bits,
    }
}

pub fn ledger_shape<S>(field: &Field<'_>) -> Shape<S> {
    match field.value_keyword().as_deref() {
        Some(
            b"hidden" | b"army" | b"navy" | b"military" | b"all" | b"air" | b"civilian"
            | b"invalid",
        ) => Shape::One,
        _ => Shape::Skip,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleNode {
    Rule,
    Override,
}

pub fn rule_field(node: RuleNode, field: &Field<'_>) -> Shape<RuleNode> {
    match node {
        RuleNode::Rule => match field.keyword().as_deref() {
            Some(b"icon") => Shape::SkipIfOpen,
            Some(b"override") => Shape::Object(RuleNode::Override),
            _ => Shape::One,
        },
        RuleNode::Override => Shape::One,
    }
}

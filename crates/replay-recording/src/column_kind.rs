use replay_columnar::state::Kind as Column;
use replay_data::rows::Kind;
use std::io;

pub fn column(kind: Kind) -> Column {
    match kind {
        Kind::U8 => Column::U8,
        Kind::I8 => Column::I8,
        Kind::U16 => Column::U16,
        Kind::I16 => Column::I16,
        Kind::U32 => Column::U32,
        Kind::I32 => Column::I32,
        Kind::U64 => Column::U64,
        Kind::I64 => Column::I64,
        Kind::Bool => Column::Bool,
        Kind::OptionalI32 => Column::OptionalI32,
        Kind::OptionalU32 => Column::OptionalU32,
        Kind::OptionalI64 => Column::OptionalI64,
        Kind::OptionalU64 => Column::OptionalU64,
        Kind::Bytes => Column::Bytes,
        Kind::U32List => Column::U32List,
        Kind::U64List => Column::U64List,
        Kind::Opaque => Column::Opaque,
    }
}

pub fn columns(schema: &[Kind]) -> io::Result<Vec<Column>> {
    engine_allocation::collect_vec(schema.iter().map(|&kind| column(kind)))
}

//! CProductionLine base shared by equipment and general production lines.
//! The pinned executable's RTTI confirms these bases at offset zero. The old
//! cost/speed/produced reads use i32 despite eight-byte spacing; native width
//! requires an engine writer/serializer check before a new walker adopts it.

/// CProductionLine inherited CReferenceObject type u32; collector offsets.rs LINE_TYPE.
pub const REFERENCE_TYPE: usize = 0x08;
/// CProductionLine inherited CReferenceObject id u32; collector offsets.rs LINE_ID.
pub const ID: usize = 0x0c;
/// CProductionLine active factory count i32; decode_production_lines_snapshot and construction walk.
pub const ACTIVE_FACTORIES: usize = 0x18;
/// CProductionLine cost, old reader takes i32 /100000; native width unresolved (read_production_fixed_raw).
pub const COST: usize = 0x28;
/// CProductionLine speed, old reader takes i32 /100000; native width unresolved (decode_production_lines_snapshot).
pub const SPEED: usize = 0x30;
/// CProductionLine produced amount, old reader takes i32 /100000; native width unresolved (construction walk).
pub const PRODUCED: usize = 0x38;
/// CProductionLine priority i32; collector offsets.rs LINE_PRIORITY and decode_production_lines_snapshot.
pub const PRIORITY: usize = 0x44;
/// CProductionLine requested output amount i32; collector offsets.rs LINE_AMOUNT.
pub const AMOUNT: usize = 0x48;

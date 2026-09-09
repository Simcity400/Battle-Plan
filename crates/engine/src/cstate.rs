/// CState vtable, ported from the old build_compact_map_control_topology walker.
pub const VTABLE_RVA: usize = 0x0292_06c0;
/// CState id, ported from build_compact_map_control_topology's state-id witness.
pub const ID: usize = 0x58;
/// CState owning country id; old map topology walk reads this as u32.
pub const OWNER: usize = 0xc8;
/// CState controlling country id; old map topology walk reads this as u32.
pub const CONTROLLER: usize = 0xcc;

//! National-focus presentation item. State, highlight and progress are GUI
//! callback operands, excluded from hourly walks. Callback presentation codes
//! do not establish the corresponding in-memory field widths.

/// Item constructor entry.
pub const CONSTRUCTOR_RVA: usize = 0x0136_a5c0;
/// Item allocation size.
pub const ALLOCATION_SIZE: usize = 0x550;
/// Item primary vtable.
pub const PRIMARY_VTABLE_RVA: usize = 0x0299_dd40;
/// Item secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x0299_dde0;
/// Item secondary vtable-pointer offset.
pub const SECONDARY_VTABLE: usize = 0x18;
/// Item runtime GUI-root pointer.
pub const RUNTIME_GUI_ROOT: usize = 0x10;
/// Item focus-definition pointer.
pub const DEFINITION: usize = 0x20;
/// Item presentation-state field; in-memory width is unknown.
pub const PRESENTATION_STATE: usize = 0x540;
/// Item national-capability field; in-memory width is unknown.
pub const NATIONAL_CAPABILITY: usize = 0x54c;
/// Item joint-capability field; in-memory width is unknown.
pub const JOINT_CAPABILITY: usize = 0x54d;
/// Item state-producer entry.
pub const STATE_PRODUCER_RVA: usize = 0x0137_9a50;
/// Item state-producer entry sentinel.
pub const STATE_PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x10, 0x44, 0x88, 0x40, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54,
];
/// Item state-writer entry with a u32 state argument.
pub const STATE_WRITER_RVA: usize = 0x0137_78f0;
/// Item state-writer entry sentinel.
pub const STATE_WRITER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x55, 0x57, 0x41, 0x56, 0x48, 0x8d,
];
/// Highlight-producer entry; u32 code three selects visibility.
pub const HIGHLIGHT_PRODUCER_RVA: usize = 0x0137_7430;
/// Highlight-producer entry sentinel.
pub const HIGHLIGHT_PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x70, 0x10, 0x48, 0x89, 0x78, 0x20, 0x55,
];
/// Joint-country renderer call site in the state producer.
pub const JOINT_COUNTRY_CALLSITE_RVA: usize = 0x0137_9e5f;
/// Original joint-country renderer call bytes.
pub const JOINT_COUNTRY_CALLSITE_SENTINEL: [u8; 5] = [0xe8, 0x2c, 0xbb, 0x7b, 0xff];

//! Sampler declaration reader for presentation.

/// Reader vtable.
pub const VTABLE_RVA: usize = 0x02b3_afc8;
/// Sampler-name string at reader +0x10.
pub const NAME: usize = 0x10;
/// Sampler type, u32 at +0x30, separate from the lowered descriptor.
pub const TYPE: usize = 0x30;
/// Inline sampler descriptor copied from reader +0x40 to declaration +0x30.
pub const DESCRIPTION: usize = 0x40;
/// Field dispatcher.
pub const READ_FIELD_RVA: usize = 0x0234_8890;
/// Reader construction and append routine initializing defaults and 0x70-byte rows.
pub const READ_DECLARATION_RVA: usize = 0x0234_8d70;
/// Sampler-type token scanner targeting reader +0x30 for token 0xe1.
pub const READ_TYPE_RVA: usize = 0x023f_8070;

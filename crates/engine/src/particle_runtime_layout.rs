//! Particle-runtime presentation layout.

/// Particle-type pointer selecting subsystem arrays.
pub const TYPE: usize = 0x00;
/// Emitter-array data pointer, distinct from an inline emitter or elapsed-time value.
pub const EMITTERS: usize = 0x08;
/// Particle f32 scale.
pub const SCALE: usize = 0x48;
/// First f32 control field; runtime meaning is unknown.
pub const ALPHA: usize = 0x4c;
/// Second f32 control field; runtime meaning is unknown.
pub const PLAYBACK: usize = 0x50;
/// Runtime allocation size.
pub const SIZE: usize = 0x58;
/// Runtime back-pointer to the node transform subobject.
pub const NODE_TRANSFORM_POINTER: usize = 0x40;
/// Runtime enable u8 flag.
pub const ENABLED: usize = 0x54;
/// Second runtime u8 flag, uninitialized until its independent setter writes one.
pub const SECOND_FLAG: usize = 0x55;
/// Runtime enable setter.
pub const ENABLE_SETTER_RVA: usize = 0x023e_9150;
/// Independent second-flag setter.
pub const SECOND_FLAG_SETTER_RVA: usize = 0x023e_9020;
/// Node-transform pointer writer, following census publication.
pub const NODE_TRANSFORM_SETTER_RVA: usize = 0x009b_c400;
/// Runtime authored-scale setter.
pub const AUTHORED_SCALE_SETTER_RVA: usize = 0x023e_9190;
/// Object-returning runtime allocator.
pub const PARTICLE_RUNTIME_CREATE_RVA: u32 = 0x023E_8190;
/// Runtime allocation entry sentinel.
pub const CREATE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xf2, 0x48, 0x8b, 0xe9,
];
/// Runtime destruction entry.
pub const PARTICLE_RUNTIME_DESTROY_RVA: u32 = 0x023E_8400;
/// Runtime destruction entry sentinel.
pub const DESTROY_SENTINEL: [u8; 16] = [
    0x48, 0x85, 0xc9, 0x0f, 0x84, 0xf4, 0x00, 0x00, 0x00, 0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89,
];
/// Runtime normal/ribbon packet builder.
pub const PARTICLE_PACKET_BUILDER_RVA: u32 = 0x023E_8B90;
/// Packet-builder entry sentinel.
pub const BUILDER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x4c, 0x89, 0x4c, 0x24, 0x20, 0x55,
];

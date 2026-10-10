//! Particle presentation object derived from the 3D-object base.
//! This module enables no recorded table or hook. Inherited transform,
//! descriptor and flags are defined in `cpdx_3d_object`.

/// Particle runtime pointer.
pub const RUNTIME: usize = 0x70;
/// Particle-object primary vtable.
pub const VTABLE_RVA: usize = 0x02b2_a498;
/// Particle-object allocation size.
pub const SIZE: usize = 0x80;
/// Time-scale f32, initialized to 1.0.
pub const TIME_SCALE: usize = 0x78;
/// Propagate the node enabled bit to the particle runtime enabled byte.
pub const PROPAGATE_ENABLE_RVA: usize = 0x0227_ecb0;
/// Particle-object constructor.
pub const PARTICLE_OBJECT_CTOR_RVA: u32 = 0x0227_EA10;
/// Particle-object constructor entry sentinel bytes.
pub const OBJECT_CTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48,
];
/// Particle-bucket submit entry.
pub const PARTICLE_BUCKET_SUBMIT_RVA: u32 = 0x0227_ECD0;
/// Particle-bucket submit entry sentinel bytes.
pub const BUCKET_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0x41, 0x58, 0x4c, 0x8d,
];

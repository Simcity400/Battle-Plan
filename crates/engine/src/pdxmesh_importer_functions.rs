//! Pdxmesh import and source-stream layouts; descriptors do not invoke game code or enable capture.

/// Selects the first matching mesh-settings row in file order.
pub const IMPORT_MESH_RVA: usize = 0x0224_f250;
/// Absent chunks clear their source-stream bits.
pub const LOAD_SOURCE_STREAMS_RVA: usize = 0x0224_edd0;
/// Optional source-stream counts determine the packed allocation stride.
pub const PACK_VERTEX_STREAMS_RVA: usize = 0x0225_0c70;
/// Skin-stream packing for meshes with skin influences.
pub const PACK_SKIN_INFLUENCES_RVA: usize = 0x0225_0a90;
/// Material-loader entry receiving the mesh path's directory prefix.
pub const LOAD_MESH_TYPE_RVA: usize = 0x022f_55b0;
/// Float-count order for position, normal, tangent and four UV streams; absent streams are skipped.
pub const SOURCE_STREAM_FLOAT_COUNTS: [usize; 7] = [3, 3, 4, 2, 2, 2, 2];

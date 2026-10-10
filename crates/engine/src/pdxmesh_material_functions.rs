//! Pdxmesh material-loader and static-data layouts; descriptors do not enable capture or rendering.

/// Only nonempty override strings replace authored material slots.
pub const LOAD_RVA: usize = 0x0224_cc00;
/// UV-stream shader-define bytes loaded from the executable.
pub const UV1_SHADER_DEFINE_RVA: usize = 0x02b2_8c30;
/// UV-stream shader-define byte length.
pub const UV1_SHADER_DEFINE: &str = "PDX_MESH_UV1";
/// First material texture-key bytes.
pub const DIFFUSE_ATTRIBUTE_RVA: usize = 0x02b2_8b04;
/// Second material texture-key bytes.
pub const NORMAL_ATTRIBUTE_RVA: usize = 0x02b2_8ad0;
/// Third material texture-key bytes.
pub const SPECULAR_ATTRIBUTE_RVA: usize = 0x02b2_8b0c;

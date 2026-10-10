//! String-list header copied into fixed windows. Nodes hold MSVC strings
//! and are distinct from application reload vectors.

/// List-copy constructor traversing source nodes and cloning destination nodes.
pub const COPY_RVA: usize = 0x022d_0410;

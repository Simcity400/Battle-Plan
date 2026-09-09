//! CDeploymentStatus, reached through CCountry::DEPLOYMENT_MANAGER.
//!
//! The archived walk rechecked the owner and array after reading the queue;
//! hourly consistency and the engine writers remain unproved for this table.

/// CDeploymentStatus primary vtable; offsets.rs:410 and the pinned RTTI inventory identify the class.
pub const VTABLE_RVA: usize = 0x2949128;
/// CDeploymentStatus conveyor CPdxArray; read_deployment_country at attach_log.rs:172333 reads +0x48.
pub const CONVEYORS: usize = 0x48;
/// CDeploymentStatus conveyor-array pointer stride; offsets.rs:416 and read_deployment_country advance eight bytes.
pub const CONVEYOR_STRIDE: usize = 0x08;
/// CDeploymentStatus owning CCountry pointer; offsets.rs:411 and read_deployment_country check the +0x118 backlink.
pub const OWNER: usize = 0x118;

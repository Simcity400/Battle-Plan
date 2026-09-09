//! CMilitaryDeploymentConveyor queue entry and its deployment lines.
//!
//! Role token zero is retained as zero; the old reader's synthetic
//! "__unknown_0x0" string is not an engine value. A location interpretation
//! beyond the old reader's raw u32 has not been proved.

/// CMilitaryDeploymentConveyor primary vtable; offsets.rs:417 and pinned RTTI confirm the class.
pub const VTABLE_RVA: usize = 0x29a9e18;
/// CMilitaryDeploymentConveyor reference kind; offsets.rs:418 and read_deployment_conveyor require type 64.
pub const REFERENCE_TYPE: u32 = 64;
/// CMilitaryDeploymentConveyor reference type word, u32; read_deployment_conveyor uses offsets.rs:459 +8.
pub const TYPE: usize = 0x08;
/// CMilitaryDeploymentConveyor reference id, u32; read_deployment_conveyor uses offsets.rs:460 +0xC.
pub const ID: usize = 0x0c;
/// CMilitaryDeploymentConveyor CReferencedDivisionTemplate pointer; offsets.rs:419 and read_deployment_conveyor read +0x20.
pub const TEMPLATE: usize = 0x20;
/// CMilitaryDeploymentConveyor MSVC byte string; offsets.rs:420 and read_stable_deployment_string read +0x28.
pub const NAME: usize = 0x28;
/// CMilitaryDeploymentConveyor queued amount, i32; offsets.rs:421 and read_deployment_conveyor read +0x48.
pub const AMOUNT: usize = 0x48;
/// CMilitaryDeploymentConveyor raw location, u32; offsets.rs:422 and read_deployment_conveyor read +0x4C.
pub const LOCATION: usize = 0x4c;
/// CMilitaryDeploymentConveyor raw priority, u32; offsets.rs:423 and read_deployment_conveyor read +0x50.
pub const PRIORITY: usize = 0x50;
/// CMilitaryDeploymentConveyor line-pointer CPdxArray; offsets.rs:424 and read_deployment_conveyor read +0x78.
pub const LINES: usize = 0x78;
/// CMilitaryDeploymentConveyor line-pointer stride; offsets.rs:416 and read_deployment_conveyor advance eight bytes.
pub const LINE_STRIDE: usize = 0x08;
/// CMilitaryDeploymentConveyor CPdxLexer role token, u32; offsets.rs:425 and resolve_deployment_role preserve its raw source.
pub const ROLE_TOKEN: usize = 0x90;
/// CMilitaryDeploymentConveyor closed flag, u8; offsets.rs:426 and read_deployment_conveyor require zero or one.
pub const CLOSED: usize = 0x94;

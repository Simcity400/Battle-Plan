//! CMilitaryDeploymentLine identity, repetition count and deployment object.

/// CMilitaryDeploymentLine primary vtable; offsets.rs:427 and pinned RTTI confirm the class.
pub const VTABLE_RVA: usize = 0x29a9dc0;
/// CMilitaryDeploymentLine reference kind; offsets.rs:428 and read_deployment_line require type 63.
pub const REFERENCE_TYPE: u32 = 63;
/// CMilitaryDeploymentLine reference type word, u32; read_deployment_line uses offsets.rs:459 +8.
pub const TYPE: usize = 0x08;
/// CMilitaryDeploymentLine reference id, u32; read_deployment_line uses offsets.rs:460 +0xC.
pub const ID: usize = 0x0c;
/// CMilitaryDeploymentLine CNameGroupMember pointer; offsets.rs:429 and read_deployment_line read +0x20.
pub const NAME: usize = 0x20;
/// CMilitaryDeploymentLine amount, i32; offsets.rs:430 and read_deployment_line read +0x28.
pub const AMOUNT: usize = 0x28;
/// CMilitaryDeploymentLine CMilitaryDeployment pointer; offsets.rs:431 and read_deployment_line read +0x30.
pub const DEPLOYMENT: usize = 0x30;
/// CMilitaryDeploymentLine parent conveyor pointer; offsets.rs:432 and read_deployment_line verify +0x38.
pub const PARENT: usize = 0x38;

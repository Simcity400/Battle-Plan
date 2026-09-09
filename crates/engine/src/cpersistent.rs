//! `CPersistent` RTTI anchors from the pinned executable.
//!
//! Neither address below is a vtable. A derived object's CPersistent subobject
//! may start at a different displacement and use a different base descriptor;
//! the zero displacement here belongs to the CDelayedEvent hierarchy.

/// CPersistent RTTI type descriptor; attach_log.rs:97735 and the pinned PE name bytes identify .?AVCPersistent@@.
pub const TYPE_DESCRIPTOR_RVA: usize = 0x0312_4140;
/// CPersistent decorated RTTI name; pinned TypeDescriptor+0x10 bytes match the old CDelayedEvent RTTI walk.
pub const RTTI_NAME: &str = ".?AVCPersistent@@";
/// CPersistent nonvirtual zero-displacement base descriptor used by CDelayedEvent; attach_log.rs:97735 and pinned PE agree.
pub const ZERO_DISPLACEMENT_BASE_DESCRIPTOR_RVA: usize = 0x02b9_88e8;
/// CPersistent displacement in that specific CDelayedEvent base descriptor; pinned PMD has mdisp=0 and pdisp=-1.
pub const DELAYED_EVENT_BASE_DISPLACEMENT: i32 = 0;

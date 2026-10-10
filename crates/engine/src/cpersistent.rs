//! Persistent-base identity descriptors. Neither numeric identity is a
//! vtable. Derived objects can use a different subobject displacement and
//! base descriptor; this zero displacement is specific to delayed events.

/// Persistent-base type descriptor.
pub const TYPE_DESCRIPTOR_RVA: usize = 0x0312_4140;

/// Zero-displacement nonvirtual base descriptor for delayed events.
pub const ZERO_DISPLACEMENT_BASE_DESCRIPTOR_RVA: usize = 0x02b9_88e8;
/// Persistent-base displacement in delayed events: member zero and no virtual-base pointer.
pub const DELAYED_EVENT_BASE_DISPLACEMENT: i32 = 0;

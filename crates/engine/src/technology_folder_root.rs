//! Unnamed active folder-root object in CCountryTechTreeView's pointer array.
//!
//! Its +30 graphical subobject has only a nonzero vtable check in the old hook;
//! class identity and equality with other GUI classes have not been proved.

/// Active folder-root graphical subobject; archived dynamic_collection_trace_live/src/a729_technology_canvas_hook.rs:39,538-546 checks its vtable.
pub const GRAPHICAL_SUBOBJECT: usize = 0x30;

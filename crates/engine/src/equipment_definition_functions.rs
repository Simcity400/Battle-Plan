//! Equipment-definition parser metadata, separate from object layouts.

/// Category-mask reader; clears the u64 destination before each scalar or list, including empty lists.
pub const READ_CATEGORY_MASK_RVA: usize = 0x00f7_5bc0;

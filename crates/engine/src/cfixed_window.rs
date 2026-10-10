//! CFixedWindow runtime layout, relative to its enclosing window pointer.
//! No hourly state walk is enabled here.

/// CFixedWindow authored CGuiType pointer +0x90.
pub const GUI_TYPE: usize = 0x90;
/// CFixedWindow embedded CTernary<CTextBox*> map +0x1C60.
pub const TEXT_BOX_MAP: usize = 0x1c60;
/// CFixedWindow list +0x1E58.
pub const DROP_DESTINATION_LIST: usize = 0x1e58;
/// CFixedWindow constructor receiving the copied source list as argument 23.
pub const CONSTRUCT_WITH_DROP_LIST_RVA: usize = 0x0230_c610;

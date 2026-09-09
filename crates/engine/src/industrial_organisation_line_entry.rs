//! COrganisation production-line array entry; native nominal struct name unidentified.

/// Organisation line entry stride; a729_mio_history_hook production_candidates walks 0x18-byte records.
pub const STRIDE: usize = 0x18;
/// Organisation line entry CEquipmentProductionLine pointer; old history hook reads entry+0x10 then resolves its self CID.
pub const LINE: usize = 0x10;

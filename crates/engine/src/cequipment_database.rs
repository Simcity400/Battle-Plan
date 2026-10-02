//! CEquipmentDatabase, the loaded equipment types, and its archetype list.
//!
//! The archetype list is what the logistics series are indexed by: every
//! `CLogisticsStatus` series record holds one value per archetype, at the
//! archetype's `CEquipmentType+0x538`, which is its position in this list.

/// CEquipmentDatabase singleton pointer global; GetEquipmentDatabaseChecked@140161c20 and
/// GetEquipmentDatabase@14022f5f0 return it, and CLogisticsStatus::PostDailyUpdate@14135ffa0 reads it.
pub const SINGLETON_RVA: usize = 0x0331_6af0;
/// CEquipmentDatabase primary vtable 0x1429219c0; RTTI `.?AVCEquipmentDatabase@@` places it at object +0.
pub const VTABLE_RVA: usize = 0x0292_19c0;
/// CEquipmentDatabase archetype CPdxArray (data, capacity +0x88, count +0x8c) of `CEquipmentType*`.
/// CEquipmentDatabase::RegisterType@1409ecf30 sets an archetype's +0x538 to the count, then pushes it
/// here, or on a reload stores it back at its saved index, so +0x538 is always its position here.
/// PostDailyUpdate@14135ffa0 and the CLogisticsStatus constructor@14135aa20 size and walk the
/// logistics records by this count.
pub const ARCHETYPES: usize = 0x80;
/// Archetype pointer stride; RegisterType pushes through PtrVector::PushBack_stride8@140120090.
pub const ARCHETYPE_STRIDE: usize = 8;

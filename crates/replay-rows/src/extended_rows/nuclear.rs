use super::*;
use crate::event_feed_rows::EventFeedCountry;

impl OpaqueValue for NuclearWarSupport {}
impl OpaqueValue for NuclearBuildingDamage {}
impl OpaqueValue for NuclearUnitDamage {}
impl OpaqueValue for NuclearEquipmentLoss {}
impl OpaqueValue for NuclearStrikeRefusal {}
impl OpaqueValue for NuclearWingLoss {}
opaque_value!(EventFeedCountry);

typed_codec!(NuclearStrikeRow, 64, |r| r.id as u128,
|k| Self { id: k as u64, ..Default::default() },
 { 0 => (hour): I32, 1 => (launcher): Opaque, 2 => (nuke_type): U32, 3 => (province): U32,
   4 => (state): U32, 5 => (owner): Opaque, 6 => (controller): U32, 7 => (war_support): Opaque,
   8 => (buildings): Opaque, 9 => (units): Opaque, 10 => (equipment): Opaque, 11 => (refusals): Opaque,
   12 => (wings): Opaque }, |r| {
   if r.refusals.iter().any(|refusal| refusal.count == 0) {
       return Err(crate::codec::invalid("nuclear strike refusal without a count"));
   }
   if r.units.iter().any(|unit| unit.organisation_before.is_some() != unit.organisation_after.is_some()) {
       return Err(crate::codec::invalid("nuclear strike unit organisation half recorded"));
   }
   Ok(())
});

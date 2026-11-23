use super::*;

impl OpaqueValue for SettledPeaceAction {}
impl OpaqueValue for SettledPeaceTransfer {}
impl OpaqueValue for SettledPeaceSubject {}

typed_codec!(PeaceSettlementRow, 64, |r| r.sequence as u128,
|k| Self { sequence: k as u64, ..Default::default() },
 { 0 => (conference): OptionalU64, 1 => (hour): OptionalI32, 2 => (winners): U32List, 3 => (losers): U32List, 4 => (actions): Opaque, 5 => (transfers): Opaque, 6 => (failure): Opaque, 7 => (omitted): U64, 8 => (subjects): Opaque }, |r| {
   if r.failure.is_none() && (r.conference.is_none() || r.hour.is_none()) { return Err(crate::codec::invalid("successful peace settlement lacks identity or hour")); }
   Ok(())
});

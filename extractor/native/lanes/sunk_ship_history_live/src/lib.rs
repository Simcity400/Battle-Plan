//! A33 exact sunk-ship writer carrier (compact domain 86).
//!
//! The wire accepts only pointer-free observations from the same-thread
//! `CFEXMember -> CSunkShipInfo` writer bracket joined to the validated d49/d50
//! combat epoch. Disappearance, serializer polling, or time-window correlation
//! cannot implement the source trait.

pub mod a729_hook;

use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 86;
pub const DOMAIN_NAME: &str = "sunk_ship_history_live";
pub const MAGIC: u32 = u32::from_le_bytes(*b"SHP1");
pub const SCHEMA_VERSION: u32 = 1;
pub const EXECUTABLE_SHA256: [u8; 32] =
    hex32(b"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad");
pub const EXACT20_FUNCTION_MANIFEST_SHA256: [u8; 32] =
    hex32(b"37797c3175af53fc483a26555a492a65ac926b815eb4abd650ff01a9c3d5487e");
pub const EXACT20_DATA_ARTIFACT_SHA256: [u8; 32] =
    hex32(b"c09db5903cf588bc792bbe63a3a34929316d2136ec0e104f4c99931b86b24369");

pub const PRODUCTION_SCHEDULING_ENABLED: bool = false;
pub const LIVE_FIXTURE_ACCEPTED: bool = false;
pub const MAX_EVENTS: usize = 65_536;
pub const MAX_TEXT_BYTES: usize = 4_096;
pub const EVENT_FLAG_KILLED: u32 = 1;
pub const EVENT_FLAG_ASSISTED_KILL: u32 = 1 << 1;
pub const EVENT_KNOWN_FLAGS: u32 = EVENT_FLAG_KILLED | EVENT_FLAG_ASSISTED_KILL;

const fn hex32(input: &[u8; 64]) -> [u8; 32] {
    const fn nibble(value: u8) -> u8 {
        match value { b'0'..=b'9' => value-b'0', b'a'..=b'f' => value-b'a'+10, b'A'..=b'F' => value-b'A'+10, _ => 0xff }
    }
    let mut out=[0;32]; let mut i=0;
    while i<32 { out[i]=(nibble(input[i*2])<<4)|nibble(input[i*2+1]); i+=1; }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityState { Unavailable, StaticExact20Receipt }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SunkShipEvent {
    pub event_sequence: u64,
    pub combat_id: u32,
    pub combat_generation: u64,
    /// Exact two-word country identity block consumed at CSunkShipInfo +0x48.
    pub victim_country_identity_raw: [u32; 2],
    /// Exact country id consumed by the item populator from +0x78.
    pub killer_country_id_raw: u32,
    pub sink_date_raw: i32,
    /// Slot index copied from the outer, d49/d50-bound victim CFEXMember.
    /// This is not the unrelated CSunkShipInfo +0x84 reference handle.
    pub victim_member_battle_slot_index: u32,
    pub flags: u32,
    pub ship_name: String,
    pub ship_class_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SunkShipSnapshot {
    pub source_sequence: u64,
    /// True only when a baseline proves the history prefix complete and no
    /// hook/queue/binding/read failure has occurred since that baseline.
    pub producer_complete: bool,
    pub events: Vec<SunkShipEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SunkShipFrame {
    pub elapsed_wall_ms: u64,
    pub frame_hour: u64,
    pub status_flags: u32,
    pub authority: AuthorityState,
    pub snapshot: Option<SunkShipSnapshot>,
}

pub trait ExactSunkShipSource {
    fn is_exact_a729_writer_source(&self) -> bool;
    fn read_sunk_ship_snapshot(&self, frame_hour: u64) -> Result<SunkShipSnapshot, String>;
}

/// The only production source implementation. External trait implementations
/// remain useful for codec fixtures but cannot bypass the two compile-time
/// activation latches checked by [`collect_and_encode`].
#[cfg(windows)]
pub struct InstalledA729HookSource;

#[cfg(windows)]
impl ExactSunkShipSource for InstalledA729HookSource {
    fn is_exact_a729_writer_source(&self) -> bool {
        a729_hook::installed()
    }

    fn read_sunk_ship_snapshot(&self, _frame_hour: u64) -> Result<SunkShipSnapshot, String> {
        a729_hook::current_complete_snapshot()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierError { ActivationDisabled, Source(String), Invalid(String), Truncated, TrailingBytes }

fn invalid<T>(message: impl Into<String>) -> Result<T, CarrierError> { Err(CarrierError::Invalid(message.into())) }

fn valid_text(value: &str, required: bool) -> bool {
    value.len() <= MAX_TEXT_BYTES && (!required || !value.is_empty()) && !value.as_bytes().contains(&0)
}

pub fn validate_snapshot(snapshot: &SunkShipSnapshot) -> Result<(), CarrierError> {
    if snapshot.source_sequence == 0 || !snapshot.producer_complete || snapshot.events.len() > MAX_EVENTS { return invalid("SHP1 snapshot sequence/completeness/count invalid"); }
    let mut identities=BTreeSet::new();
    for event in &snapshot.events {
        if event.event_sequence==0 || event.combat_id==0 || event.combat_generation==0
            || event.victim_country_identity_raw == [0, 0]
            || event.killer_country_id_raw==0 || event.flags & !EVENT_KNOWN_FLAGS != 0
            || (event.flags & EVENT_FLAG_KILLED != 0) == (event.flags & EVENT_FLAG_ASSISTED_KILL != 0)
            || !valid_text(&event.ship_name,true) || !valid_text(&event.ship_class_name,true)
            || !identities.insert(event.event_sequence)
        { return invalid("SHP1 event is incomplete, ambiguous, or duplicated"); }
    }
    Ok(())
}

pub fn validate_frame(frame: &SunkShipFrame) -> Result<(), CarrierError> {
    match frame.authority {
        AuthorityState::Unavailable if frame.status_flags != 0 && frame.snapshot.is_none() => Ok(()),
        AuthorityState::StaticExact20Receipt if frame.status_flags == 0 && frame.snapshot.is_some() => validate_snapshot(frame.snapshot.as_ref().unwrap()),
        _ => invalid("SHP1 authority/availability mismatch"),
    }
}

pub fn collect_and_encode<S: ExactSunkShipSource>(source: &S, frame_hour: u64, elapsed_wall_ms: u64) -> Result<Vec<u8>, CarrierError> {
    if !PRODUCTION_SCHEDULING_ENABLED || !LIVE_FIXTURE_ACCEPTED { return Err(CarrierError::ActivationDisabled); }
    if !source.is_exact_a729_writer_source() { return Err(CarrierError::Source("source is not the receipt-bound a729 writer bracket".into())); }
    let snapshot=source.read_sunk_ship_snapshot(frame_hour).map_err(CarrierError::Source)?;
    encode_payload(&SunkShipFrame { elapsed_wall_ms, frame_hour, status_flags:0, authority:AuthorityState::StaticExact20Receipt, snapshot:Some(snapshot) })
}

pub fn encode_unavailable(frame_hour:u64, elapsed_wall_ms:u64, status_flags:u32) -> Result<Vec<u8>,CarrierError> {
    encode_payload(&SunkShipFrame { elapsed_wall_ms, frame_hour, status_flags, authority:AuthorityState::Unavailable, snapshot:None })
}

pub fn encode_payload(frame:&SunkShipFrame)->Result<Vec<u8>,CarrierError>{
    validate_frame(frame)?; let mut out=Vec::new();
    u32v(&mut out,MAGIC);u32v(&mut out,SCHEMA_VERSION);u32v(&mut out,match frame.authority{AuthorityState::Unavailable=>0,AuthorityState::StaticExact20Receipt=>1});u32v(&mut out,frame.status_flags);
    u64v(&mut out,frame.elapsed_wall_ms);u64v(&mut out,frame.frame_hour);out.extend_from_slice(&EXECUTABLE_SHA256);out.extend_from_slice(&EXACT20_FUNCTION_MANIFEST_SHA256);out.extend_from_slice(&EXACT20_DATA_ARTIFACT_SHA256);
    out.push(u8::from(frame.snapshot.is_some()));out.extend_from_slice(&[0;3]);
    if let Some(snapshot)=&frame.snapshot { u64v(&mut out,snapshot.source_sequence);out.push(u8::from(snapshot.producer_complete));out.extend_from_slice(&[0;3]);u32v(&mut out,snapshot.events.len() as u32); for e in &snapshot.events {
        u64v(&mut out,e.event_sequence);u32v(&mut out,e.combat_id);u64v(&mut out,e.combat_generation);u32v(&mut out,e.victim_country_identity_raw[0]);u32v(&mut out,e.victim_country_identity_raw[1]);u32v(&mut out,e.killer_country_id_raw);i32v(&mut out,e.sink_date_raw);u32v(&mut out,e.victim_member_battle_slot_index);u32v(&mut out,e.flags);textv(&mut out,&e.ship_name);textv(&mut out,&e.ship_class_name);
    }} Ok(out)
}

pub fn decode_payload(payload:&[u8])->Result<SunkShipFrame,CarrierError>{
    let mut r=Reader{payload,at:0}; if r.u32()?!=MAGIC||r.u32()?!=SCHEMA_VERSION{return invalid("SHP1 magic/schema mismatch")};
    let authority=match r.u32()?{0=>AuthorityState::Unavailable,1=>AuthorityState::StaticExact20Receipt,_=>return invalid("SHP1 authority unknown")};let status_flags=r.u32()?;let elapsed_wall_ms=r.u64()?;let frame_hour=r.u64()?;
    if r.bytes(32)?!=EXECUTABLE_SHA256||r.bytes(32)?!=EXACT20_FUNCTION_MANIFEST_SHA256||r.bytes(32)?!=EXACT20_DATA_ARTIFACT_SHA256{return invalid("SHP1 receipt identity mismatch")};
    let has=r.u8()?;if has>1||r.bytes(3)?!=[0,0,0]{return invalid("SHP1 availability marker invalid")};
    let snapshot=if has==1{let source_sequence=r.u64()?;let producer_complete=r.u8()?!=0;if r.bytes(3)?!=[0,0,0]{return invalid("SHP1 completeness reserved bytes invalid")};let count=r.u32()? as usize;if count>MAX_EVENTS{return invalid("SHP1 event count exceeds bound")};let mut events=Vec::with_capacity(count);for _ in 0..count{events.push(SunkShipEvent{event_sequence:r.u64()?,combat_id:r.u32()?,combat_generation:r.u64()?,victim_country_identity_raw:[r.u32()?,r.u32()?],killer_country_id_raw:r.u32()?,sink_date_raw:r.i32()?,victim_member_battle_slot_index:r.u32()?,flags:r.u32()?,ship_name:r.text()?,ship_class_name:r.text()?});}Some(SunkShipSnapshot{source_sequence,producer_complete,events})}else{None};
    if r.at!=payload.len(){return Err(CarrierError::TrailingBytes)};let frame=SunkShipFrame{elapsed_wall_ms,frame_hour,status_flags,authority,snapshot};validate_frame(&frame)?;Ok(frame)
}

fn u32v(out:&mut Vec<u8>,v:u32){out.extend_from_slice(&v.to_le_bytes())} fn i32v(out:&mut Vec<u8>,v:i32){out.extend_from_slice(&v.to_le_bytes())} fn u64v(out:&mut Vec<u8>,v:u64){out.extend_from_slice(&v.to_le_bytes())}
fn textv(out:&mut Vec<u8>,v:&str){u32v(out,v.len() as u32);out.extend_from_slice(v.as_bytes())}
struct Reader<'a>{payload:&'a[u8],at:usize} impl<'a> Reader<'a>{fn bytes(&mut self,n:usize)->Result<&'a[u8],CarrierError>{let end=self.at.checked_add(n).ok_or(CarrierError::Truncated)?;let b=self.payload.get(self.at..end).ok_or(CarrierError::Truncated)?;self.at=end;Ok(b)}fn u8(&mut self)->Result<u8,CarrierError>{Ok(self.bytes(1)?[0])}fn u32(&mut self)->Result<u32,CarrierError>{Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))}fn i32(&mut self)->Result<i32,CarrierError>{Ok(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))}fn u64(&mut self)->Result<u64,CarrierError>{Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))}fn text(&mut self)->Result<String,CarrierError>{let n=self.u32()? as usize;if n>MAX_TEXT_BYTES{return invalid("SHP1 text exceeds bound")};String::from_utf8(self.bytes(n)?.to_vec()).map_err(|_|CarrierError::Invalid("SHP1 text is not UTF-8".into()))}fn opt_text(&mut self)->Result<Option<String>,CarrierError>{match self.u8()?{0=>Ok(None),1=>Ok(Some(self.text()?)),_=>invalid("SHP1 optional text marker invalid")}}}

#[cfg(test)] mod tests{use super::*;fn frame()->SunkShipFrame{SunkShipFrame{elapsed_wall_ms:7,frame_hour:900,status_flags:0,authority:AuthorityState::StaticExact20Receipt,snapshot:Some(SunkShipSnapshot{source_sequence:1,producer_complete:true,events:vec![SunkShipEvent{event_sequence:1,combat_id:4,combat_generation:2,victim_country_identity_raw:[3,9],killer_country_id_raw:5,sink_date_raw:123,victim_member_battle_slot_index:7,flags:EVENT_FLAG_KILLED,ship_name:"HMS Exact".into(),ship_class_name:"Destroyer".into()}]})}}#[test]fn roundtrip_preserves_exact_epoch_and_writer_fields(){let f=frame();assert_eq!(decode_payload(&encode_payload(&f).unwrap()).unwrap(),f)}#[test]fn production_source_is_inert_until_fixture_promotion(){struct S;impl ExactSunkShipSource for S{fn is_exact_a729_writer_source(&self)->bool{true}fn read_sunk_ship_snapshot(&self,_:u64)->Result<SunkShipSnapshot,String>{Ok(frame().snapshot.unwrap())}}assert_eq!(collect_and_encode(&S,900,0),Err(CarrierError::ActivationDisabled));}#[test]fn disappearance_style_ambiguous_flags_are_rejected(){let mut f=frame();f.snapshot.as_mut().unwrap().events[0].flags=0;assert!(encode_payload(&f).is_err())}}
